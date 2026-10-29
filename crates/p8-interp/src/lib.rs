#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The p8 interpreter: one loop over register instructions, all frames in
//! one register file, calls resolved once per call site.

mod arrayaccess;
mod builtin_classes;
mod callable;
mod classes;
mod handlers;
mod fiber;
mod generators;
mod lazy;
mod include;
mod reflect_info;
mod reflection;
mod refs;
mod throw;

use p8_bytecode::{CastKind, ClassDef, Cmp, Function, Instr, Reg, Rare};
pub use include::{LoadError, Loader};
use p8_runtime::array::{Key, KeyIssue, PhpArray};
use p8_runtime::ext::{Ctx, Registry};
pub use p8_runtime::ext::Throw;
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::object::{Class, HOOK_GET, HOOK_SET, Ids, Object};
use p8_runtime::ops::{self, BinOp, Diag, OpError};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{OnceCell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// A function as one context runs it: the shared compiled code, plus what
/// belongs to this context alone: its constants as values, and what each
/// call site and constant fetch resolved to (a feedback vector, in V8's
/// terms). Compiled code is shared by every context; this is not.
pub struct Closure {
    code: Arc<Function>,
    consts: Box<[Value]>,
    sites: Box<[OnceCell<Callee>]>,
}

impl Closure {
    fn new(code: Arc<Function>) -> Rc<Closure> {
        let consts = code.consts.iter().map(|c| c.to_value()).collect();
        let sites = code.sites.iter().map(|_| OnceCell::new()).collect();
        Rc::new(Closure { code, consts, sites })
    }
}

/// What a call site or constant fetch resolved to, in one context.
#[derive(Clone)]
enum Callee {
    /// A user function: its closure's index in the context's table (an
    /// index, not a reference: a recursive function's call site names the
    /// function itself, and a reference would be a cycle that is never
    /// freed).
    User(u32),
    Builtin(usize),
    Constant(Value),
}

struct Frame {
    func: Rc<Closure>,
    /// Where the frame resumes (written when it calls out).
    pc: u32,
    base: u32,
    /// Absolute register of the caller's result (NO_RET for the main frame).
    ret: u32,
    argc: u32,
    /// Arguments past the declared parameters (func_get_args()).
    /// and the variables only named at run time (`$$name`, extract()):
    /// rare, so one box that is usually none.
    extra: Option<Box<Extra>>,
    /// A method's `$this`, the class its code is in (visibility, self::)
    /// and the class it was called on (static::); None in a function.
    this: Option<Rc<Object>>,
    scope: Option<Rc<Class>>,
    called: Option<Rc<Class>>,
    /// The Generator whose code this frame runs, if any.
    generator: Option<Rc<Object>>,
    /// Its variables are the global ones: the main script, and a file
    /// included at global scope (at any depth of includes).
    global: bool,
}

const NO_RET: u32 = u32::MAX;

/// A frame's arguments past its parameters and its run-time variables.
#[derive(Default)]
struct Extra {
    args: Box<[Value]>,
    vars: Vars,
}

impl Frame {
    /// The arguments past the declared parameters.
    fn args(&self) -> &[Value] {
        self.extra.as_ref().map_or(&[], |e| &e.args)
    }

    fn vars(&self) -> Option<&Vars> {
        self.extra.as_ref().map(|e| &e.vars)
    }

    fn vars_mut(&mut self) -> &mut Vars {
        &mut self.extra.get_or_insert_with(Default::default).vars
    }
}

/// A frame's variables only named at run time, in the order made.
#[derive(Default)]
struct Vars(Vec<(Vec<u8>, Rc<RefCell<Value>>)>);

impl Vars {
    fn get(&self, name: &[u8]) -> Option<&Rc<RefCell<Value>>> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, c)| c)
    }

    /// The variable `name`, made (null) if there is none.
    fn cell(&mut self, name: &[u8]) -> Rc<RefCell<Value>> {
        if let Some(c) = self.get(name) {
            return Rc::clone(c);
        }
        let c = Rc::new(RefCell::new(Value::Null));
        self.0.push((name.to_vec(), Rc::clone(&c)));
        c
    }

    fn remove(&mut self, name: &[u8]) {
        self.0.retain(|(n, _)| n != name);
    }
}

/// PHP's error levels (E_*), for error_reporting.
const E_WARNING: i64 = 2;
const E_NOTICE: i64 = 8;
const E_USER_WARNING: i64 = 512;
const E_USER_NOTICE: i64 = 1024;
const E_DEPRECATED: i64 = 8192;
const E_USER_DEPRECATED: i64 = 16384;
/// The CLI's level with no php.ini: E_ALL except E_STRICT.
const E_DEFAULT: i64 = 30719;

/// The call stack. A popped frame keeps its slot and the closure in it, so
/// calling the same function again from the same depth (what a loop does)
/// reuses the slot without touching the closure's reference count.
#[derive(Default)]
struct Frames {
    slots: Vec<Frame>,
    /// Frames in use: `slots[..len]`.
    len: usize,
    /// The frame the running `execute` started at: a call from a builtin
    /// back into PHP runs its own `execute` above the frames below.
    floor: usize,
}

impl Frames {
    #[inline(always)]
    fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    fn last(&self) -> Option<&Frame> {
        self.slots[..self.len].last()
    }

    #[inline(always)]
    fn last_mut(&mut self) -> Option<&mut Frame> {
        self.slots[..self.len].last_mut()
    }

    fn first(&self) -> Option<&Frame> {
        self.slots[..self.len].first()
    }

    /// Push a frame running `func`, its window at `base`.
    #[inline(always)]
    fn push(&mut self, func: &Rc<Closure>, base: usize, argc: usize, ret: u32) {
        match self.slots.get_mut(self.len) {
            Some(slot) => {
                if !Rc::ptr_eq(&slot.func, func) {
                    slot.func = Rc::clone(func);
                }
                (slot.pc, slot.base, slot.ret, slot.argc, slot.global) = (0, base as u32, ret, argc as u32, false);
            }
            None => self.slots.push(Frame {
                func: Rc::clone(func),
                pc: 0,
                base: base as u32,
                ret,
                argc: argc as u32,
                extra: None,
                this: None,
                scope: None,
                called: None,
                generator: None,
                global: false,
            }),
        }
        self.len += 1;
    }

    /// Pop the top frame; its slot stays for the next push.
    #[inline(always)]
    fn pop(&mut self) -> &mut Frame {
        self.len -= 1;
        &mut self.slots[self.len]
    }

    /// The frames from `from` on, taken out (their slots left empty).
    fn take_above(&mut self, from: usize) -> Vec<Frame> {
        let mut out = Vec::with_capacity(self.len.saturating_sub(from));
        for i in from..self.len {
            let blank = Frame {
                func: Rc::clone(&self.slots[i].func),
                pc: 0,
                base: 0,
                ret: NO_RET,
                argc: 0,
                extra: None,
                this: None,
                scope: None,
                called: None,
                generator: None,
                global: false,
            };
            out.push(std::mem::replace(&mut self.slots[i], blank));
        }
        self.len = from.min(self.len);
        out
    }

    /// Put a frame taken out back on top.
    fn restore(&mut self, frame: Frame) {
        match self.slots.get_mut(self.len) {
            Some(slot) => *slot = frame,
            None => self.slots.push(frame),
        }
        self.len += 1;
    }

    /// Drop every frame and slot (and the closures they hold).
    fn clear(&mut self) {
        self.slots.clear();
        self.len = 0;
        self.floor = 0;
    }
}

/// The globals a request starts with, in the order PHP makes them.
const REQUEST_GLOBALS: &[&str] = &["_GET", "_POST", "_COOKIE", "_FILES", "argv", "argc", "_SERVER", "_ENV", "_REQUEST"];

/// The builtin constants PHP marks deprecated (CONST_DEPRECATED), as one
/// a FetchConst names (or falls back to).
fn deprecated_constant(name: &[u8], fallback: Option<&[u8]>) -> Option<&'static str> {
    const DEPRECATED: &[&str] = &[
        "FILE_TEXT", "FILE_BINARY", "ASSERT_ACTIVE", "ASSERT_CALLBACK", "ASSERT_BAIL", "ASSERT_WARNING", "ASSERT_EXCEPTION",
        "DOM_PHP_ERR", "SUNFUNCS_RET_TIMESTAMP", "SUNFUNCS_RET_STRING", "SUNFUNCS_RET_DOUBLE", "MT_RAND_PHP", "E_STRICT",
        "FILTER_SANITIZE_STRING", "FILTER_SANITIZE_STRIPPED",
    ];
    let global = |n: &[u8]| n.strip_prefix(b"\\").unwrap_or(n).to_vec();
    let names = [Some(global(name)), fallback.map(global)];
    DEPRECATED.iter().copied().find(|d| names.iter().flatten().any(|n| n == d.as_bytes()))
}

/// A builtin that is running PHP code (array_map's callback): where it
/// was called (the frame count then), its name and arguments.
struct NativeCall {
    depth: usize,
    name: &'static str,
    args: Vec<Value>,
    /// Shown in a trace even when the error is raised inside it (no PHP
    /// frame above): the Fiber methods.
    top: bool,
}

/// What `step` leaves the frame loop to do.
enum Next {
    /// Continue this frame at the pc.
    Go(usize),
    /// A call or return changed the current frame.
    Switch,
}

/// Run the common cases of the hottest instructions, from `pc` of the top
/// frame on: those that need only the frame's registers (ints and floats,
/// jumps), plus calls to a user function the call site already resolved
/// and returns of a value, which switch frames here. Returns the pc (in
/// the top frame as it is then) of the first instruction it cannot run,
/// for `step`.
///
/// Kept apart from everything else so its state stays in machine registers.
#[inline(never)]
fn fast(regs: &mut Vec<Value>, frames: &mut Frames, closures: &[Rc<Closure>], mut pc: usize, stop: &AtomicU64, ids: &Ids) -> usize {
    // A loop's backward branch is where an interrupt is noticed: `step`
    // then raises it.
    let back = |target: u32, pc: usize| target as usize <= pc && stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT;
    loop {
        let top = frames.last().expect("a frame");
        let (clo, base) = (&*top.func, top.base as usize);
        let code = &clo.code.code[..];
        let w = &mut regs[base..base + clo.code.nregs as usize];
        // A compare-and-branch on two numbers (anything else goes to `step`).
        macro_rules! jump_if {
            ($cmp:expr, $when:expr, $a:expr, $b:expr, $target:expr) => {
                match num_test($cmp, &w[$a as usize], &w[$b as usize]) {
                    Some(t) if t == $when => {
                        if back($target, pc) {
                            return pc;
                        }
                        pc = $target as usize;
                    }
                    Some(_) => pc += 1,
                    None => return pc,
                }
            };
        }
        // `dst = a op b` on numbers (and `.` on strings).
        macro_rules! arith {
            ($op:expr, $dst:expr, $a:expr, $b:expr) => {
                match num_op($op, &w[$a as usize], &w[$b as usize]) {
                    // A variable holding a reference is written through it (`step`).
                    Some(_) if matches!(w[$dst as usize], Value::Ref(_)) => return pc,
                    Some(r) => {
                        w[$dst as usize] = r;
                        pc += 1;
                    }
                    None if $op == BinOp::Concat && concat(w, $dst, $a, $b) => pc += 1,
                    None => return pc,
                }
            };
        }
        // A fused step and test (`IncJumpLt` and co.) on ints: step, then
        // jump or go past the test. A non-int bound leaves the test to the
        // next instruction; a non-int variable leaves both to `step`.
        macro_rules! step_jump {
            ($var:expr, $b:expr, $target:expr, $step:expr, $holds:expr) => {{
                // Written in place: no drop of the old value to compile.
                let Value::Int(i) = &mut w[$var as usize] else { return pc };
                let Some(n) = $step(*i, 1) else { return pc };
                *i = n;
                match w[$b as usize] {
                    Value::Int(b) if $holds(n, b) => {
                        if back($target, pc) {
                            // The step is done: the test (next) sees the interrupt.
                            return pc + 1;
                        }
                        pc = $target as usize;
                    }
                    Value::Int(_) => pc += 2,
                    _ => pc += 1,
                }
            }};
        }
        let switch = loop {
            match code[pc] {
                Instr::Jump { target } => {
                    if back(target, pc) {
                        return pc;
                    }
                    pc = target as usize;
                }
                Instr::JumpLt { a, b, target } => jump_if!(Cmp::Lt, true, a, b, target),
                Instr::JumpNotLt { a, b, target } => jump_if!(Cmp::Lt, false, a, b, target),
                Instr::JumpLe { a, b, target } => jump_if!(Cmp::Le, true, a, b, target),
                Instr::JumpNotLe { a, b, target } => jump_if!(Cmp::Le, false, a, b, target),
                Instr::JumpEq { a, b, target } => jump_if!(Cmp::Eq, true, a, b, target),
                Instr::JumpNotEq { a, b, target } => jump_if!(Cmp::Eq, false, a, b, target),
                Instr::JumpIdentical { a, b, target } => jump_if!(Cmp::Identical, true, a, b, target),
                Instr::JumpNotIdentical { a, b, target } => jump_if!(Cmp::Identical, false, a, b, target),
                Instr::Compare { cmp, dst, a, b } => match num_test(cmp, &w[a as usize], &w[b as usize]) {
                    Some(t) if cmp != Cmp::Spaceship && !matches!(w[dst as usize], Value::Ref(_)) => {
                        w[dst as usize] = Value::Bool(t);
                        pc += 1;
                    }
                    _ => return pc,
                },
                Instr::IncVar { var } => match &mut w[var as usize] {
                    Value::Int(i) if *i != i64::MAX => {
                        *i += 1;
                        pc += 1;
                    }
                    _ => return pc,
                },
                Instr::IncJumpLt { var, b, target } => step_jump!(var, b, target, i64::checked_add, |n, b| n < b),
                Instr::IncJumpLe { var, b, target } => step_jump!(var, b, target, i64::checked_add, |n, b| n <= b),
                Instr::DecJumpGt { var, b, target } => step_jump!(var, b, target, i64::checked_sub, |n, b| n > b),
                Instr::DecJumpGe { var, b, target } => step_jump!(var, b, target, i64::checked_sub, |n, b| n >= b),
                Instr::DecVar { var } => match &mut w[var as usize] {
                    Value::Int(i) if *i != i64::MIN => {
                        *i -= 1;
                        pc += 1;
                    }
                    _ => return pc,
                },
                Instr::Binary { op, dst, a, b } => arith!(op, dst, a, b),
                Instr::Add { dst, a, b } => arith!(BinOp::Add, dst, a, b),
                Instr::Sub { dst, a, b } => arith!(BinOp::Sub, dst, a, b),
                Instr::Mul { dst, a, b } => arith!(BinOp::Mul, dst, a, b),
                Instr::ConcatN { dst, start, n } => {
                    if !concat_n(w, dst, start, n) {
                        return pc;
                    }
                    pc += 1;
                }
                // A write that freed an object with a destructor stops here:
                // `execute` runs it before the next instruction.
                Instr::Move { dst, src } => match copied(&w[src as usize]) {
                    Some(v) => {
                        w[dst as usize] = v;
                        pc += 1;
                        if ids.has_pending() {
                            return pc;
                        }
                    }
                    None => return pc,
                },
                Instr::Assign { var, src } => match copied(&w[src as usize]) {
                    Some(v) => {
                        assign(w, var, v);
                        pc += 1;
                        if ids.has_pending() {
                            return pc;
                        }
                    }
                    None => return pc,
                },
                Instr::AssignTake { var, src } => match std::mem::replace(&mut w[src as usize], Value::Undef) {
                    // Through a reference, or undefined: `step`'s.
                    v @ (Value::Ref(_) | Value::Undef) => {
                        w[src as usize] = v;
                        return pc;
                    }
                    v => {
                        assign(w, var, v);
                        pc += 1;
                        if ids.has_pending() {
                            return pc;
                        }
                    }
                },
                Instr::Free { reg } => {
                    clear(&mut w[reg as usize]);
                    pc += 1;
                    if ids.has_pending() {
                        return pc;
                    }
                }
                Instr::JumpIf { cond, target } => match w[cond as usize] {
                    Value::Bool(true) if back(target, pc) => return pc,
                    Value::Bool(c) => pc = if c { target as usize } else { pc + 1 },
                    _ => return pc,
                },
                Instr::JumpIfNot { cond, target } => match w[cond as usize] {
                    Value::Bool(false) if back(target, pc) => return pc,
                    Value::Bool(c) => pc = if c { pc + 1 } else { target as usize },
                    _ => return pc,
                },
                // $a[int] on an array (or a reference to one) holding a plain
                // value at that key
                Instr::FetchDim { dst, arr, key, .. } => {
                    let v = match w[key as usize] {
                        Value::Int(k) => match &w[arr as usize] {
                            Value::Array(a) => int_elem(a, k),
                            Value::Ref(cell) => match &*cell.borrow() {
                                Value::Array(a) => int_elem(a, k),
                                _ => None,
                            },
                            _ => None,
                        },
                        Value::Str(_) => str_elem(w, arr, key),
                        _ => None,
                    };
                    match v {
                        Some(v) => {
                            w[dst as usize] = v;
                            pc += 1;
                            if ids.has_pending() {
                                return pc;
                            }
                        }
                        None => return pc,
                    }
                }
                // $a[int] = int|float|bool|null, $a an array (or a reference to one)
                Instr::AssignDim { var, keys, n: 1, append: false, val, dst } => {
                    if let Value::Str(_) = w[keys as usize] {
                        if !set_str_elem(w, var, keys, val, dst) {
                            return pc;
                        }
                        pc += 1;
                        continue;
                    }
                    let (Value::Int(k), Some(v)) = (&w[keys as usize], plain(&w[val as usize])) else { return pc };
                    let k = *k;
                    let done = match &mut w[var as usize] {
                        Value::Array(a) => set_int_elem(a, k, &v),
                        Value::Ref(cell) => match &mut *cell.borrow_mut() {
                            Value::Array(a) => set_int_elem(a, k, &v),
                            _ => false,
                        },
                        _ => false,
                    };
                    if !done {
                        return pc;
                    }
                    w[dst as usize] = v;
                    pc += 1;
                    if ids.has_pending() {
                        return pc;
                    }
                }
                // $a[int] op= number, $a an array holding a number at
                // that key
                Instr::AssignDimOp { op, var, keys, n: 1, val, dst } => {
                    if !dim_op(w, op, var, keys, val, dst) {
                        return pc;
                    }
                    pc += 1;
                }
                Instr::IterNext { iter, val, key, has_key, end } => {
                    // An object (a generator, an Iterator) is `step`'s.
                    if !matches!(w[iter as usize], Value::Array(_)) {
                        return pc;
                    }
                    pc = if iter_next(w, iter, val, key, has_key) { pc + 1 } else { end as usize };
                }
                // $var op= number, $var holding a number
                Instr::AssignOp { op, var, src } => match num_op(op, &w[var as usize], &w[src as usize]) {
                    Some(r) => {
                        w[var as usize] = r;
                        pc += 1;
                    }
                    // $str .= string
                    None if op == BinOp::Concat && append_str(w, var, src) => pc += 1,
                    None => return pc,
                },
                // $i++ / ++$i (and --) whose value is used, on an int
                Instr::IncDec { var, dst, inc, post } => {
                    let Value::Int(slot) = &mut w[var as usize] else { return pc };
                    let i = *slot;
                    let Some(n) = (if inc { i.checked_add(1) } else { i.checked_sub(1) }) else { return pc };
                    *slot = n;
                    w[dst as usize] = Value::Int(if post { i } else { n });
                    pc += 1;
                }
                // Every other instruction (listed, so dispatch needs no range check).
                Instr::AssignDim { .. }
                | Instr::AssignDimOp { .. }
                | Instr::LoadNull { .. }
                | Instr::Not { .. }
                | Instr::Neg { .. }
                | Instr::BitNot { .. }
                | Instr::ToBool { .. }
                | Instr::Cast { .. }
                | Instr::JumpIfSet { .. }
                | Instr::NewArray { .. }
                | Instr::ArrayPush { .. }
                | Instr::ArraySet { .. }
                | Instr::IssetDim { .. }
                | Instr::IssetVar { .. }
                | Instr::UnsetVar { .. }
                | Instr::UnsetDim { .. }
                | Instr::IterInit { .. }
                | Instr::SendVar { .. }
                | Instr::SendVarMethod { .. }
                | Instr::SendVarStatic { .. }
                | Instr::MethodArgByRef { .. }
                | Instr::StaticArgByRef { .. }
                | Instr::FuncArgByRef { .. }
                | Instr::Echo { .. }
                | Instr::FetchConst { .. }
                | Instr::BindGlobal { .. }
                | Instr::ArgDefault { .. }
                | Instr::Exit { .. }
                | Instr::MatchError { .. }
                | Instr::DeclareClass { .. }
                | Instr::DeclareFunction { .. }
                | Instr::Include { .. }
                | Instr::Silence { .. }
                | Instr::MakeClosure { .. }
                | Instr::Capture { .. }
                | Instr::CallValue { .. }
                | Instr::FromCallable { .. }
                | Instr::BindRef { .. }
                | Instr::RefDim { .. }
                | Instr::DimSetRef { .. }
                | Instr::IterInitRef { .. }
                | Instr::IterNextRef { .. }
                | Instr::StaticVar { .. }
                | Instr::FetchList { .. }
                | Instr::CollectVariadic { .. }
                | Instr::DimIncDec { .. }
                | Instr::CallSpread { .. }
                | Instr::ArrayUnpack { .. }
                | Instr::Eval { .. }
                | Instr::Yield { .. }
                | Instr::YieldFrom { .. }
                | Instr::Unsilence { .. }
                | Instr::New { .. }
                | Instr::Construct { .. }
                | Instr::Rare { .. }
                | Instr::ConstructSpread { .. }
                | Instr::This { .. }
                | Instr::FetchProp { .. }
                | Instr::AssignProp { .. }
                | Instr::AssignPropOp { .. }
                | Instr::PropIncDec { .. }
                | Instr::AssignPropDim { .. }
                | Instr::IssetProp { .. }
                | Instr::UnsetProp { .. }
                | Instr::MethodCall { .. }
                | Instr::StaticCall { .. }
                | Instr::ClassConst { .. }
                | Instr::FetchStaticProp { .. }
                | Instr::AssignStaticProp { .. }
                | Instr::AssignStaticPropOp { .. }
                | Instr::InstanceOf { .. }
                | Instr::Clone { .. }
                | Instr::Throw { .. }
                | Instr::CatchIf { .. } => return pc,
                // Calls and returns leave this loop, so the loop itself
                // keeps only the code and the registers live.
                Instr::Call { site, base: args, argc: n, dst } => break Switch::Call(site, args, n, dst),
                Instr::Return { src } => break Switch::Return(src),
                Instr::ReturnNull => break Switch::ReturnNull,
            }
        };
        match switch {
            Switch::Call(site, args, n, dst) => {
                // A user function the call site resolved, with its required
                // arguments, and no interrupt pending: anything else is `step`'s.
                let Some(&Callee::User(target)) = clo.sites[site as usize].get() else { return pc };
                let target = &closures[target as usize];
                if (n as usize) < target.code.required || target.code.is_generator || stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                    return pc;
                }
                if let Some(caller) = frames.last_mut() {
                    caller.pc = (pc + 1) as u32;
                }
                frames.push(target, base + args as usize, n as usize, (base + dst as usize) as u32);
                enter(frames, regs);
                pc = 0;
            }
            // The main frame's return ends the run: `step` does that.
            // The bottom frame of this `execute` returns there: `step` does it.
            Switch::Return(_) | Switch::ReturnNull if frames.len() == frames.floor + 1 => return pc,
            Switch::Return(src) => {
                let value = match &regs[base + src as usize] {
                    Value::Undef => return pc,
                    Value::Ref(cell) => cell.borrow().clone(),
                    v => v.clone(),
                };
                leave(frames, regs, value);
                pc = frames.last().expect("the caller's frame").pc as usize;
                if ids.has_pending() {
                    return pc;
                }
            }
            Switch::ReturnNull => {
                leave(frames, regs, Value::Null);
                pc = frames.last().expect("the caller's frame").pc as usize;
                if ids.has_pending() {
                    return pc;
                }
            }
        }
    }
}

/// Why `fast`'s inner loop stopped: a call (its site, arguments' start,
/// count and result register) or a return (of a register, or of null).
enum Switch {
    Call(u16, Reg, u8, Reg),
    Return(Reg),
    ReturnNull,
}

/// Set up the registers of the frame just pushed: its window starts at
/// its base, with its arguments already in the first registers.
#[inline(always)]
fn enter(frames: &mut Frames, regs: &mut Vec<Value>) {
    let frame = frames.last_mut().expect("the frame just pushed");
    let (func, base, argc) = (&frame.func, frame.base as usize, frame.argc as usize);
    let code = &*func.code;
    let need = base + code.nregs as usize;
    if regs.len() < need {
        regs.resize(need, Value::Undef);
    }
    let nparams = code.params.len();
    if argc > nparams {
        frame.extra = take_extra(regs, code, base, nparams, argc);
    }
    let consts_at = code.consts_at as usize;
    for r in &mut regs[base + argc.min(nparams)..base + consts_at] {
        clear(r);
    }
    for (r, c) in regs[base + consts_at..base + consts_at + func.consts.len()].iter_mut().zip(&func.consts) {
        clear(r);
        // `r` is undefined now: nothing to drop.
        std::mem::forget(std::mem::replace(r, c.clone()));
    }
}

/// Pop the current frame, putting `value` in the caller's result
/// register; the caller (if any) resumes at its saved pc.
#[inline(always)]
fn leave(frames: &mut Frames, regs: &mut [Value], value: Value) {
    let frame = frames.pop();
    frame.extra = None;
    // Locals die with the frame (before the result lands: the result
    // register may be inside the window).
    let base = frame.base as usize;
    let code = &frame.func.code;
    // Variables and temporaries; the constants' registers are left as
    // they are (the next frame there overwrites them).
    let consts = base + code.consts_at as usize..base + code.consts_at as usize + frame.func.consts.len();
    for r in &mut regs[base..consts.start] {
        clear(r);
    }
    for r in &mut regs[consts.end..base + code.nregs as usize] {
        clear(r);
    }
    // `$this` goes after the arguments, as in PHP (it decides which
    // object id is free first).
    if frame.scope.is_some() {
        frame.this = None;
        frame.scope = None;
        frame.called = None;
    }
    if frame.ret != NO_RET {
        regs[frame.ret as usize] = value;
    }
}

/// Make a register undefined, releasing what it held. Scalars hold
/// nothing to release, so they are only re-tagged.
#[inline(always)]
fn clear(r: &mut Value) {
    if matches!(r, Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Undef) {
        // Nothing to drop: `forget` skips the drop check entirely. (The
        // scalars are listed, so any value holding memory is dropped.)
        std::mem::forget(std::mem::replace(r, Value::Undef));
    } else {
        *r = Value::Undef;
    }
}

/// Element `k` of an array if it is there and not a reference.
#[inline(always)]
fn int_elem(a: &PhpArray, k: i64) -> Option<Value> {
    match a.get_int(k) {
        Some(Value::Ref(_)) | None => None,
        Some(v) => Some(v.clone()),
    }
}

/// `$a[k] = v` for a plain `v`, unless the element is a reference (false).
#[inline(always)]
fn set_int_elem(a: &mut Rc<PhpArray>, k: i64, v: &Value) -> bool {
    if matches!(a.get_int(k), Some(Value::Ref(_))) {
        return false;
    }
    let a = Rc::make_mut(a);
    match a.get_int_mut(k) {
        Some(slot) => *slot = v.clone(),
        None => a.insert(Key::Int(k), v.clone()),
    }
    true
}

/// A copy of a register's value, as `step` makes one (strings and arrays
/// are shared until written), unless it is undefined (a warning) or a
/// reference (read through its cell).
#[inline(always)]
fn copied(v: &Value) -> Option<Value> {
    match *v {
        // The commonest cases first, without the general clone's dispatch.
        Value::Int(i) => Some(Value::Int(i)),
        Value::Float(f) => Some(Value::Float(f)),
        Value::Undef | Value::Ref(_) => None,
        ref v => Some(v.clone()),
    }
}

/// A copy of a value that holds nothing reference-counted.
#[inline(always)]
fn plain(v: &Value) -> Option<Value> {
    match *v {
        Value::Null => Some(Value::Null),
        Value::Bool(b) => Some(Value::Bool(b)),
        Value::Int(i) => Some(Value::Int(i)),
        Value::Float(f) => Some(Value::Float(f)),
        _ => None,
    }
}

/// `x cmp y` for two numbers, when that is a plain numeric comparison
/// (not NAN, and not === between an int and a float).
#[inline(always)]
fn num_test(cmp: Cmp, x: &Value, y: &Value) -> Option<bool> {
    let (x, y) = match (x, y) {
        (Value::Int(x), Value::Int(y)) => return Some(int_test(cmp, *x, *y)),
        (Value::Float(x), Value::Float(y)) => (*x, *y),
        _ if matches!(cmp, Cmp::Identical | Cmp::NotIdentical) => return None,
        (Value::Int(x), Value::Float(y)) => (*x as f64, *y),
        (Value::Float(x), Value::Int(y)) => (*x, *y as f64),
        _ => return None,
    };
    let o = x.partial_cmp(&y)?;
    Some(match cmp {
        Cmp::Lt => o.is_lt(),
        Cmp::Le => o.is_le(),
        Cmp::Gt => o.is_gt(),
        Cmp::Ge => o.is_ge(),
        Cmp::Eq | Cmp::Identical => o.is_eq(),
        Cmp::NotEq | Cmp::NotIdentical | Cmp::Spaceship => o.is_ne(),
    })
}

/// Where a context's output goes: the embedder's side of a request (a
/// terminal, an HTTP response).
pub trait Host {
    fn write(&mut self, bytes: &[u8]);
    fn flush(&mut self) {}
}

/// An output buffer (ob_start): its contents, and the handler (PHP code)
/// that turns them into what is passed on.
struct Buffer {
    data: Vec<u8>,
    handler: Option<Value>,
    chunk: usize,
    /// The handler was called (later calls lack the START phase).
    started: bool,
}

/// Output, with output buffers (ob_start) stacked on top of the host.
struct Output<'i> {
    buffers: Vec<Buffer>,
    host: &'i mut dyn Host,
    /// Some output has reached the host (the headers are sent).
    started: bool,
}

impl Output<'_> {
    fn write(&mut self, bytes: &[u8]) {
        match self.buffers.last_mut() {
            Some(buffer) => buffer.data.extend_from_slice(bytes),
            None => self.write_host(bytes),
        }
    }

    fn write_host(&mut self, bytes: &[u8]) {
        self.started |= !bytes.is_empty();
        self.host.write(bytes);
    }

    /// Write to the level under the innermost buffer.
    fn write_below(&mut self, bytes: &[u8]) {
        let n = self.buffers.len();
        match n.checked_sub(2).and_then(|i| self.buffers.get_mut(i)) {
            Some(below) => below.data.extend_from_slice(bytes),
            None => self.write_host(bytes),
        }
    }

    /// Everything buffered goes out as it is (a fatal error's report).
    fn flush_all(&mut self) {
        while let Some(buffer) = self.buffers.pop() {
            self.write(&buffer.data);
        }
        self.host.flush();
    }
}

/// How ob_list_handlers() names a buffer's handler.
fn handler_name(handler: &Option<Value>) -> String {
    match handler {
        None => "default output handler".into(),
        Some(Value::Str(s)) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
        Some(Value::Object(o)) if o.class.lower == b"closure" => "Closure::__invoke".into(),
        Some(Value::Array(a)) => {
            let parts: Vec<Value> = a.values().map(Value::deref).collect();
            let class = match parts.first() {
                Some(Value::Object(o)) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                Some(v) => String::from_utf8_lossy(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned(),
                None => String::new(),
            };
            let method = parts.get(1).map_or_else(String::new, |m| String::from_utf8_lossy(m.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned());
            format!("{class}::{method}")
        }
        Some(Value::Object(o)) => format!("{}::__invoke", String::from_utf8_lossy(o.class.name.as_bytes())),
        Some(_) => "???".into(),
    }
}

/// PHP_OUTPUT_HANDLER_* phases a handler is told.
const OB_START: i64 = 1;
const OB_CLEAN: i64 = 2;
const OB_FLUSH: i64 = 4;
const OB_FINAL: i64 = 8;

/// No interrupt pending.
const NO_INTERRUPT: u64 = 0;
/// InterruptHandle::terminate (other values: a timeout of that many seconds).
const TERMINATE: u64 = u64::MAX;

/// Stops the request an isolate is running, from any thread: what a host's
/// watchdog holds. The request ends with a fatal error at the next loop
/// iteration or call; the isolate stays usable for the next request.
#[derive(Clone)]
pub struct InterruptHandle(Arc<AtomicU64>);

impl InterruptHandle {
    /// End the request with PHP's max_execution_time error.
    pub fn timeout(&self, seconds: u64) {
        self.0.store(seconds.clamp(1, TERMINATE - 1), AtomicOrdering::Relaxed);
    }

    /// End the request (the host is shutting it down).
    pub fn terminate(&self) {
        self.0.store(TERMINATE, AtomicOrdering::Relaxed);
    }
}

/// One engine on one thread. It runs one context (request) at a time: a
/// context borrows it mutably for as long as it lives. What it keeps
/// between requests (the register file, buffers) stays allocated, so a warm
/// isolate starts the next request without allocating them again.
///
/// `!Send`: values are reference-counted without atomics, so an isolate
/// and everything in it stays on the thread that made it.
pub struct Isolate {
    /// The builtins and their constants (shared by every isolate).
    registry: Arc<Registry>,
    /// All frames' registers. While code runs, the interpreter holds the
    /// register file itself (taken out of here) so the hot loop works on a
    /// plain local slice.
    regs: Vec<Value>,
    /// Set by an InterruptHandle; cleared when a request starts.
    interrupt: Arc<AtomicU64>,
    /// The classes every request starts with.
    builtin_classes: builtin_classes::Classes,
}

impl Isolate {
    pub fn new(registry: Arc<Registry>) -> Isolate {
        Isolate {
            regs: Vec::with_capacity(1024),
            interrupt: Arc::new(AtomicU64::new(NO_INTERRUPT)),
            builtin_classes: builtin_classes::build(&registry),
            registry,
        }
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// A handle that stops this isolate's running request from another thread.
    pub fn interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle(Arc::clone(&self.interrupt))
    }

    /// Start a request whose output goes to `host`. An interrupt left from
    /// an earlier request is cleared.
    pub fn new_context<'i>(&'i mut self, host: &'i mut dyn Host) -> Context<'i> {
        self.interrupt.store(NO_INTERRUPT, AtomicOrdering::Relaxed);
        let stop = Arc::clone(&self.interrupt);
        let classes = self.builtin_classes.clone();
        Context {
            stop,
            isolate: self,
            frames: Frames::default(),
            functions: HashMap::new(),
            closures: Vec::new(),
            constants: HashMap::new(),
            globals: indexmap::IndexMap::new(),
            out: Output { buffers: Vec::new(), host, started: false },
            output_start: None,
            diags: Vec::new(),
            ini: HashMap::new(),
            error_level: E_DEFAULT,
            display_errors: true,
            pc: 0,
            classes,
            failed_call: None,
            class_defs: Vec::new(),
            declared: HashMap::new(),
            class_origins: HashMap::new(),
            binding_file: None,
            loader: None,
            included: std::collections::HashSet::new(),
            included_order: Vec::new(),
            ids: Rc::new(Ids::default()),
            native_class: None,
            ext_state: HashMap::new(),
            resource_classes: HashMap::new(),
            resource_ids: Rc::new(Ids::monotonic()),
            enum_objects: HashMap::new(),
            resolved: std::collections::HashSet::new(),
            stack_base: 0,
            ref_wanted: Vec::new(),
            named_extra: None,
            class_defaults: HashMap::new(),
            const_cache: HashMap::new(),
            const_evaluating: Vec::new(),
            autoloaders: Vec::new(),
            autoload_tried: std::collections::HashSet::new(),
            error_handlers: Vec::new(),
            exception_handlers: Vec::new(),
            shutdown: Vec::new(),
            pending_errors: Vec::new(),
            last_error: None,
            in_handler: false,
            arg_buf: Vec::new(),
            lambdas: HashMap::new(),
            natives: Vec::new(),
            statics: HashMap::new(),
            magic_guard: Vec::new(),
            deferred_write: None,
            fibers: Vec::new(),
            fiber_suspended: None,
            fiber_throw: None,
            fiber_exit: None,
            gc_runs: 0,
            gc_collected: 0,
            cfg: Vec::new(),
            argv: None,
        }
    }
}

/// One request: its declared functions, constants and globals, its output.
/// Dropping it ends the request.
pub struct Context<'i> {
    isolate: &'i mut Isolate,
    /// The isolate's interrupt flag (kept here: read on every step).
    stop: Arc<AtomicU64>,
    frames: Frames,
    /// Declared functions by lowercase name, as indices in `closures`
    /// (only ever grows, so call sites
    /// can cache what they resolve).
    functions: HashMap<Vec<u8>, u32>,
    /// Every closure this context made. Call sites refer to them by index.
    closures: Vec<Rc<Closure>>,
    /// Constants the script defined (define(), const).
    constants: HashMap<Vec<u8>, Value>,
    /// Globals not held in the main frame's registers.
    /// In the order made, as PHP's symbol table ($GLOBALS, get_defined_vars).
    globals: indexmap::IndexMap<Vec<u8>, Rc<RefCell<Value>>>,
    out: Output<'i>,
    diags: Vec<Diag>,
    /// ini settings this request changed (ini_set, error_reporting()).
    ini: HashMap<Vec<u8>, Value>,
    /// error_reporting's level: which diagnostics are shown.
    error_level: i64,
    /// display_errors: whether diagnostics are shown at all.
    display_errors: bool,
    /// The running frame's pc (Frame.pc is written when it calls out).
    pc: usize,
    /// Linked classes by lowercase name.
    classes: HashMap<Vec<u8>, Rc<Class>>,
    /// The frame of a call that failed before it ran, for the trace of
    /// the error it raised (see `failed_call`).
    failed_call: Option<PhpArray>,
    /// The script's class declarations (DeclareClass indexes them), and
    /// which are linked.
    class_defs: Vec<Arc<ClassDef>>,
    /// Keyed by the declaration's address, holding it so that address
    /// is not reused by another file's class while it is remembered.
    declared: HashMap<usize, Arc<ClassDef>>,
    /// Where each class this request declared was declared (file, line),
    /// for "previously declared in"; and the file whose classes are being
    /// bound before it runs.
    class_origins: HashMap<Vec<u8>, (Rc<str>, u32)>,
    binding_file: Option<Rc<str>>,
    /// Compiles the files `include` runs (the embedder's compiler).
    loader: Option<Rc<dyn Loader>>,
    /// Files included so far, for `_once` (resolved paths).
    included: std::collections::HashSet<std::path::PathBuf>,
    /// The files run so far, in order (get_included_files).
    included_order: Vec<String>,
    /// Object ids (handles), reused as PHP reuses them.
    ids: Rc<Ids>,
    /// A builtin's arguments while it runs (kept for its capacity).
    arg_buf: Vec<Value>,
    /// Extensions' per-request state (Ctx::state).
    ext_state: HashMap<&'static str, Value>,
    /// The class of each kind of resource, and their numbers.
    resource_classes: HashMap<&'static str, Rc<Class>>,
    resource_ids: Rc<Ids>,
    /// The class a static builtin method runs for (an enum's cases()).
    native_class: Option<Rc<Class>>,
    /// Enum case objects, one per (enum, case) in a request.
    enum_objects: HashMap<(Vec<u8>, Vec<u8>), Rc<Object>>,
    /// Result registers (absolute) of calls in a reference context: a
    /// function returning by reference returns the reference there.
    ref_wanted: Vec<usize>,
    /// Named arguments for the variadic parameter of the function being
    /// called (at this frame depth), by name.
    named_extra: Option<(usize, PhpArray)>,
    /// The native stack's address where the script started (0: unknown),
    /// to stop runaway nesting (PHP's "Maximum call stack size").
    stack_base: usize,
    /// Lazy classes (CLASS_LAZY) whose default code has run, by address.
    resolved: std::collections::HashSet<usize>,
    /// A lazy class's computed instance defaults: (slot, value).
    class_defaults: HashMap<usize, Rc<Vec<(usize, Value)>>>,
    /// Class constants computed by code: (declaring class, name) → value.
    const_cache: HashMap<(Vec<u8>, Vec<u8>), Value>,
    /// The ones being computed (a constant that needs itself is an error).
    const_evaluating: Vec<(Vec<u8>, Vec<u8>)>,
    /// spl_autoload_register's callables, and the classes they were asked
    /// for (once each).
    autoloaders: Vec<Value>,
    autoload_tried: std::collections::HashSet<Vec<u8>>,
    /// set_error_handler's (callable, levels), innermost last.
    error_handlers: Vec<(Value, i64)>,
    exception_handlers: Vec<Value>,
    /// register_shutdown_function's (callable, arguments).
    shutdown: Vec<(Value, Vec<Value>)>,
    /// Diagnostics for the error handler, raised by the running instruction.
    pending_errors: Vec<(i64, String, Rc<str>, u32)>,
    /// error_get_last(): the last error that reached the default handler.
    last_error: Option<(i64, String, String, u32)>,
    /// Where output first reached the host (headers_sent()).
    output_start: Option<(Rc<str>, u32)>,
    in_handler: bool,
    /// The closure of each function a Closure object runs (by its code).
    lambdas: HashMap<usize, Rc<Closure>>,
    /// Builtins running that may call back into PHP, innermost last.
    natives: Vec<NativeCall>,
    /// `static` variables, by function and index.
    statics: HashMap<(usize, u16), Rc<RefCell<Value>>>,
    /// The magic property methods running: (object, property, which), so
    /// a property access inside `__get('x')` is a plain one (zend's guards).
    magic_guard: Vec<(usize, PhpStr, crate::classes::Magic)>,
    /// A write held back until the error handler has run (`$undef op= v`).
    deferred_write: Option<(usize, Value)>,
    /// The fibers running, innermost last.
    fibers: Vec<fiber::FiberRun>,
    /// Fiber::suspend()'s value, while the running fiber is being left.
    fiber_suspended: Option<Value>,
    /// Fiber::throw()'s exception, raised where the fiber goes on.
    fiber_throw: Option<Rc<Object>>,
    /// The exit unwinding a fiber being destroyed (no catch matches it).
    fiber_exit: Option<Rc<Object>>,
    /// gc_status(): collections run, values they freed.
    gc_runs: i64,
    gc_collected: i64,
    /// The host's settings, as given (get_cfg_var).
    cfg: Vec<(Vec<u8>, String)>,
    /// The command line, when the host gives one: $argv, $argc, $_SERVER.
    argv: Option<Vec<String>>,
}

impl Context<'_> {
    pub fn declare(&mut self, func: Arc<Function>) -> Result<(), String> {
        let key = func.name.as_bytes().to_ascii_lowercase();
        let name = String::from_utf8_lossy(func.name.as_bytes());
        if let Some(&i) = self.functions.get(&key) {
            let first = &self.closures[i as usize].code;
            return Err(format!("Cannot redeclare function {name}() (previously declared in {}:{})", first.file, first.line));
        }
        if self.isolate.registry.lookup(&key).is_some() {
            return Err(format!("Cannot redeclare function {name}()"));
        }
        self.closures.push(Closure::new(func));
        self.functions.insert(key, (self.closures.len() - 1) as u32);
        Ok(())
    }

    /// Make a closure of `func` (a method's code) and return its index.
    fn add_closure(&mut self, func: Arc<Function>) -> u32 {
        self.closures.push(Closure::new(func));
        (self.closures.len() - 1) as u32
    }

    /// The script's class declarations: those whose parent and
    /// interfaces are known are linked when the run starts, the others
    /// when their declaration runs.
    pub fn declare_classes(&mut self, classes: &[Arc<ClassDef>]) {
        self.class_defs.extend(classes.iter().cloned());
    }

    /// What compiles the files the script includes.
    pub fn set_loader(&mut self, loader: Rc<dyn Loader>) {
        self.loader = Some(loader);
    }

    pub fn function_exists(&self, name: &[u8]) -> bool {
        let key = name.strip_prefix(b"\\").unwrap_or(name).to_ascii_lowercase();
        self.functions.contains_key(&key) || self.isolate.registry.lookup(&key).is_some()
    }

    /// An ini value that may be an expression (`E_ALL & ~E_NOTICE`):
    /// integers and constants with | & ^ ~ (left to right, ~ binding tightest).
    fn ini_bits(&self, text: &str) -> i64 {
        let mut acc: Option<i64> = None;
        let mut pending: Option<char> = None;
        let mut not = false;
        let mut chars = text.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
                continue;
            }
            if matches!(c, '|' | '&' | '^') {
                pending = Some(c);
                chars.next();
                continue;
            }
            if c == '~' || c == '!' {
                not = !not;
                chars.next();
                continue;
            }
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' && word.is_empty() {
                    word.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            if word.is_empty() {
                chars.next();
                continue;
            }
            let mut v = word.parse::<i64>().ok().or_else(|| self.isolate.registry.constant(word.as_bytes()).map(|c| c.to_value().to_int())).unwrap_or(0);
            if std::mem::take(&mut not) {
                v = !v;
            }
            acc = Some(match (acc, pending.take()) {
                (Some(a), Some('|')) => a | v,
                (Some(a), Some('&')) => a & v,
                (Some(a), Some('^')) => a ^ v,
                _ => v,
            });
        }
        acc.unwrap_or(0)
    }

    /// An ini setting from the host (`-d`); false when unknown.
    pub fn set_ini(&mut self, name: &[u8], value: &str) -> bool {
        self.cfg.push((name.to_vec(), value.to_string()));
        Ctx::ini_set(self, name, Value::Str(PhpStr::from(value))).is_some()
    }

    pub fn set_args(&mut self, argv: Vec<String>) {
        self.argv = Some(argv);
    }

    /// The globals the CLI starts a script with (php_cli.c and
    /// php_variables.c): $argv and $argc, $_SERVER (the environment, the
    /// script's names, the request time, argv/argc), $_ENV when
    /// variables_order has E, and the empty request arrays.
    fn request_globals(&mut self) -> Vec<(&'static str, Value)> {
        let Some(argv) = self.argv.clone() else { return Vec::new() };
        let text = |s: &str| Value::Str(PhpStr::from(s));
        let key = |s: &str| Key::Str(PhpStr::from(s));
        let args = Value::Array(Rc::new(PhpArray::from_list(argv.iter().map(|a| text(a)).collect())));
        let argc = Value::Int(argv.len() as i64);
        let mut env = PhpArray::new();
        for (k, v) in std::env::vars_os() {
            env.insert(Key::Str(PhpStr::from(k.as_encoded_bytes())), Value::Str(PhpStr::from(v.as_encoded_bytes())));
        }
        let mut server = env.clone();
        let script = argv.first().cloned().unwrap_or_default();
        for name in ["PHP_SELF", "SCRIPT_NAME", "SCRIPT_FILENAME", "PATH_TRANSLATED"] {
            server.insert(key(name), text(&script));
        }
        server.insert(key("DOCUMENT_ROOT"), text(""));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        server.insert(key("REQUEST_TIME_FLOAT"), Value::Float(now.as_secs_f64()));
        server.insert(key("REQUEST_TIME"), Value::Int(now.as_secs() as i64));
        let register = match Ctx::ini_get(self, b"register_argc_argv") {
            Some(Value::Str(s)) => !matches!(s.as_bytes().to_ascii_lowercase().as_slice(), b"" | b"0" | b"off" | b"no" | b"false"),
            _ => true,
        };
        if register {
            server.insert(key("argv"), args.clone());
            server.insert(key("argc"), argc.clone());
        }
        let order = match Ctx::ini_get(self, b"variables_order") {
            Some(Value::Str(s)) => s.as_bytes().to_ascii_uppercase(),
            _ => b"EGPCS".to_vec(),
        };
        let empty = || Value::Array(Rc::new(PhpArray::new()));
        let mut out = vec![("_SERVER", Value::Array(Rc::new(server)))];
        if register {
            out.extend([("argv", args), ("argc", argc)]);
        }
        out.push(("_ENV", if order.contains(&b'E') { Value::Array(Rc::new(env)) } else { empty() }));
        for name in ["_GET", "_POST", "_COOKIE", "_FILES", "_REQUEST"] {
            out.push((name, empty()));
        }
        out
    }

    /// What PHP says about the host's settings once they are all set: the
    /// assert.* ones are deprecated away from their defaults (in the order
    /// ext/standard registers them).
    pub fn startup_diagnostics(&mut self) {
        let on = |v: Option<Value>| match v {
            Some(Value::Str(s)) => {
                let t = s.as_bytes().to_ascii_lowercase();
                matches!(t.as_slice(), b"true" | b"yes" | b"on") || Value::Str(s).to_int() != 0
            }
            Some(v) => v.to_bool(),
            None => false,
        };
        for (name, default) in [("assert.active", true), ("assert.bail", false), ("assert.warning", true), ("assert.callback", false), ("assert.exception", true)] {
            let v = Ctx::ini_get(self, name.as_bytes());
            let deprecated = if name == "assert.callback" {
                matches!(&v, Some(Value::Str(s)) if !s.as_bytes().is_empty())
            } else {
                on(v) != default
            };
            if deprecated {
                self.print_diag(E_DEPRECATED, &format!("PHP Startup: {name} INI setting is deprecated"), "Unknown", 0);
            }
        }
    }

    /// Run the innermost buffer's handler on its contents (`phase`); what it
    /// gives back (its contents, without a handler or when it returns false).
    fn ob_handle(&mut self, regs: &mut Vec<Value>, phase: i64) -> Result<Vec<u8>, Throw> {
        let Some(buffer) = self.out.buffers.last_mut() else { return Ok(Vec::new()) };
        let data = std::mem::take(&mut buffer.data);
        let Some(handler) = buffer.handler.clone() else { return Ok(data) };
        let phase = if buffer.started { phase } else { phase | OB_START };
        buffer.started = true;
        // The handler's own output is not buffered output.
        let r = self.call_with_regs(regs, &handler, &mut [Value::Str(PhpStr::new(data.clone())), Value::Int(phase)])?;
        Ok(match r {
            Value::Bool(false) => data,
            v => v.to_php_string().map_or_else(Vec::new, |s| s.as_bytes().to_vec()),
        })
    }

    pub(crate) fn ob_with(&mut self, regs: &mut Vec<Value>, op: p8_runtime::ext::Ob) -> Result<Value, Throw> {
        use p8_runtime::ext::Ob;
        let none = self.out.buffers.is_empty();
        Ok(match op {
            Ob::Start { handler, chunk } => {
                let handler = (!matches!(handler, Value::Null)).then_some(handler);
                self.out.buffers.push(Buffer { data: Vec::new(), handler, chunk: chunk.max(0) as usize, started: false });
                Value::Bool(true)
            }
            _ if none && !matches!(op, Ob::Level | Ob::Handlers | Ob::Status { .. }) => Value::Bool(false),
            Ob::Contents => Value::Str(PhpStr::new(self.out.buffers.last().map_or_else(Vec::new, |b| b.data.clone()))),
            Ob::Length => Value::Int(self.out.buffers.last().map_or(0, |b| b.data.len()) as i64),
            Ob::Level => Value::Int(self.out.buffers.len() as i64),
            Ob::Clean => {
                self.ob_handle(regs, OB_CLEAN)?;
                Value::Bool(true)
            }
            Ob::Flush => {
                let out = self.ob_handle(regs, OB_FLUSH)?;
                self.out.write_below(&out);
                self.note_output_start();
                Value::Bool(true)
            }
            Ob::End { flush } => {
                let phase = if flush { OB_FINAL } else { OB_CLEAN | OB_FINAL };
                let out = self.ob_handle(regs, phase)?;
                if flush {
                    self.out.write_below(&out);
                    self.note_output_start();
                }
                self.out.buffers.pop();
                Value::Bool(true)
            }
            Ob::Handlers => {
                let names: Vec<Value> = self.out.buffers.iter().map(|b| Value::Str(PhpStr::from(handler_name(&b.handler)))).collect();
                Value::Array(Rc::new(PhpArray::from_list(names)))
            }
            Ob::Status { full } => {
                let status = |level: usize, b: &Buffer| {
                    let mut a = PhpArray::new();
                    let user = b.handler.is_some();
                    a.insert(Key::from_string(PhpStr::from("name")), Value::Str(PhpStr::from(handler_name(&b.handler))));
                    a.insert(Key::from_string(PhpStr::from("type")), Value::Int(i64::from(user)));
                    // PHP_OUTPUT_HANDLER_STDFLAGS (| USER); STARTED | PROCESSED once called.
                    a.insert(Key::from_string(PhpStr::from("flags")), Value::Int(if user { 0x71 } else { 0x70 } | if b.started { 0x5000 } else { 0 }));
                    a.insert(Key::from_string(PhpStr::from("level")), Value::Int(level as i64));
                    a.insert(Key::from_string(PhpStr::from("chunk_size")), Value::Int(b.chunk as i64));
                    a.insert(Key::from_string(PhpStr::from("buffer_size")), Value::Int(16384));
                    a.insert(Key::from_string(PhpStr::from("buffer_used")), Value::Int(b.data.len() as i64));
                    Value::Array(Rc::new(a))
                };
                if full {
                    let all: Vec<Value> = self.out.buffers.iter().enumerate().map(|(i, b)| status(i, b)).collect();
                    Value::Array(Rc::new(PhpArray::from_list(all)))
                } else {
                    match self.out.buffers.last() {
                        Some(b) => status(self.out.buffers.len() - 1, b),
                        None => Value::Array(Rc::new(PhpArray::new())),
                    }
                }
            }
        })
    }

    /// The request's end: each buffer left is flushed through its handler.
    fn end_buffers(&mut self, regs: &mut Vec<Value>) {
        while !self.out.buffers.is_empty() {
            if let Err(t) = self.ob_with(regs, p8_runtime::ext::Ob::End { flush: true }) {
                let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
                self.report_end(Err(t));
                self.out.buffers.pop();
            }
        }
    }

    /// Write to the output and flush it (messages the host shows as they come).
    pub fn echo_and_flush(&mut self, bytes: &[u8]) {
        self.write_out(bytes);
        self.out.flush_all();
        self.note_output_start();
    }

    /// Write to the output (the innermost buffer, or the host).
    fn write_out(&mut self, bytes: &[u8]) {
        self.out.write(bytes);
        self.note_output_start();
    }

    /// The first output to reach the host sends the headers: remember
    /// where the code was (headers_sent(), "headers already sent").
    fn note_output_start(&mut self) {
        if self.out.started && self.output_start.is_none() {
            self.output_start = Some(self.location());
        }
    }

    /// Run a file's top-level code; returns the exit status.
    pub fn run_main(&mut self, main: Arc<Function>) -> i32 {
        if let Ok(path) = std::fs::canonicalize(&*main.file) {
            self.included_order.push(path.to_string_lossy().into_owned());
            self.included.insert(path);
        }
        self.standard_streams();
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let ncv = main.consts_at as usize;
        let here = 0u8;
        self.stack_base = std::ptr::addr_of!(here) as usize;
        self.frames.push(&Closure::new(Arc::clone(&main)), 0, 0, NO_RET);
        self.frames.last_mut().expect("the main frame").global = true;
        enter(&mut self.frames, &mut regs);
        for (name, value) in self.request_globals() {
            match main.cv_names.iter().position(|n| n.as_bytes() == name.as_bytes()) {
                Some(r) => regs[r] = value,
                None => {
                    self.globals.insert(name.as_bytes().to_vec(), Rc::new(RefCell::new(value)));
                }
            }
        }
        let defs = std::mem::take(&mut self.class_defs);
        let mut result = match self.early_bind(&defs, Some(Rc::from(&*main.file))) {
            Ok(()) => self.execute(&mut regs),
            Err(e) => Err(self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), e)),
        };
        if let Err(t) = result {
            result = Err(self.handle_uncaught(&mut regs, t));
        }
        // exit() and an uncaught exception end the script cleanly enough
        // for destructors (a fatal error does not).
        let clean_end = matches!(result, Err(Throw::Exit(_) | Throw::Object(_) | Throw::Error { .. } | Throw::Autoload(_)));
        let mut status = self.report_end(result);
        // Shutdown functions (when the main frame returned, they ran then).
        if !self.shutdown.is_empty() {
            self.frames.floor = 0;
            let r = self.run_shutdown(&mut regs);
            if let Err(t) = r {
                let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
                let t = self.handle_uncaught(&mut regs, t);
                status = self.report_end(Err(t));
            }
        }
        if clean_end {
            self.frames.floor = 0;
            if let Err(t) = self.run_destructors(&mut regs).and_then(|()| self.end_destructors(&mut regs, ncv)) {
                let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
                let t = self.handle_uncaught(&mut regs, t);
                status = self.report_end(Err(t));
            }
        }
        self.end_buffers(&mut regs);
        // The request's variables go now (the isolate keeps the capacity).
        regs.clear();
        self.frames.clear();
        self.isolate.regs = regs;
        self.out.flush_all();
        status
    }

    /// The exit status for how the code ended, with PHP's message for an
    /// error nothing caught.
    fn report_end(&mut self, result: Result<(), Throw>) -> i32 {
        match result {
            Ok(()) => 0,
            Err(Throw::Exit(status)) => status,
            Err(Throw::Fatal { message, file, line }) => {
                self.write_out(format!("\nFatal error: {message} in {file} on line {line}\n").as_bytes());
                255
            }
            Err(Throw::Parse { message, file, line }) => {
                self.write_out(format!("\nParse error: {message} in {file} on line {line}\n").as_bytes());
                255
            }
            Err(Throw::Object(e)) => {
                let shown = builtin_classes::describe(&e);
                let root: &[u8] = if e.class.is_a(b"exception") { b"exception" } else { b"error" };
                let file = e.class.read(&e, b"file", root).to_php_string().unwrap_or_else(PhpStr::empty);
                let line = e.class.read(&e, b"line", root).to_int();
                let file = String::from_utf8_lossy(file.as_bytes());
                self.write_out(format!("\nFatal error: Uncaught {shown}\n  thrown in {file} on line {line}\n").as_bytes());
                255
            }
            Err(Throw::Error { class, message, file, line, .. }) => {
                self.write_out(
                    format!(
                        "\nFatal error: Uncaught {class}: {message} in {file}:{line}\nStack trace:\n#0 {{main}}\n  thrown in {file} on line {line}\n"
                    )
                    .as_bytes(),
                );
                255
            }
            Err(Throw::Autoload(name)) => {
                let (file, line) = self.location();
                let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
                self.write_out(format!("\nFatal error: Uncaught Error: Class \"{name}\" not found in {file}:{line}\nStack trace:\n#0 {{main}}\n  thrown in {file} on line {line}\n").as_bytes());
                255
            }
        }
    }

    /// The fatal error an interrupt ends the request with.
    #[cold]
    #[inline(never)]
    fn interrupted(&self) -> Throw {
        let (file, line) = self.location();
        let message = match self.stop.load(AtomicOrdering::Relaxed) {
            TERMINATE => "Execution was terminated by the host".to_string(),
            seconds => format!("Maximum execution time of {seconds} second{} exceeded", if seconds == 1 { "" } else { "s" }),
        };
        Throw::Fatal { message, file, line }
    }

    fn throw(&self, class: &'static str, message: impl Into<String>) -> Throw {
        let (file, line) = self.location();
        Throw::Error { class, message: message.into(), file, line, props: Vec::new() }
    }

    fn location(&self) -> (Rc<str>, u32) {
        match self.frames.last() {
            // A constant expression's code: where it was needed.
            Some(f) if f.func.code.name.as_bytes() == p8_bytecode::CONST_EXPR.as_bytes() => {
                let n = self.frames.len();
                match self.frames.slots[..n].iter().rev().find(|f| f.func.code.name.as_bytes() != p8_bytecode::CONST_EXPR.as_bytes()) {
                    Some(f) => (Rc::from(&*f.func.code.file), f.func.code.lines.get((f.pc as usize).saturating_sub(1)).copied().unwrap_or(0)),
                    None => (Rc::from(""), 0),
                }
            }
            Some(f) => (Rc::from(&*f.func.code.file), f.func.code.lines.get(self.pc.saturating_sub(1)).copied().unwrap_or(0)),
            None => (Rc::from(""), 0),
        }
    }

    fn op_error(&self, e: OpError) -> Throw {
        match e {
            OpError::Type(m) => self.throw("TypeError", m),
            OpError::DivisionByZero(m) => self.throw("DivisionByZeroError", m),
            OpError::Arithmetic(m) => self.throw("ArithmeticError", m),
            OpError::Error(m) => self.throw("Error", m),
        }
    }

    /// Show the diagnostics the last operation raised, at the current line.
    #[inline]
    fn report_diags(&mut self) {
        if !self.diags.is_empty() {
            self.print_diags();
        }
    }

    #[cold]
    #[inline(never)]
    fn print_diags(&mut self) {
        for d in std::mem::take(&mut self.diags) {
            match d {
                Diag::Warning(m) => self.show(E_WARNING, &m),
                Diag::Deprecated(m) => self.show(E_DEPRECATED, &m),
            }
        }
    }

    /// Print a diagnostic of PHP error `level` at the current line, as the
    /// CLI does, if error_reporting and display_errors let it through.
    fn show(&mut self, level: i64, message: &str) {
        // A user error handler gets it (after the instruction), whatever
        // error_reporting says.
        if !self.in_handler
            && let Some((_, mask)) = self.error_handlers.last()
            && mask & level != 0
            && !matches!(self.error_handlers.last(), Some((Value::Null, _)))
        {
            let (file, line) = self.location();
            self.pending_errors.push((level, message.to_string(), file, line));
            return;
        }
        let (file, line) = self.location();
        self.print_diag(level, message, &file, line);
    }

    /// Print a diagnostic as the CLI does, if error_reporting and
    /// display_errors let it through.
    fn print_diag(&mut self, level: i64, message: &str, file: &str, line: u32) {
        // Recorded even when silenced (`@`, error_reporting), as PHP does.
        self.last_error = Some((level, message.to_string(), file.to_string(), line));
        if !self.display_errors || self.error_level & level == 0 {
            return;
        }
        let label = match level {
            E_WARNING | E_USER_WARNING => "Warning",
            E_NOTICE | E_USER_NOTICE => "Notice",
            E_DEPRECATED | E_USER_DEPRECATED => "Deprecated",
            _ => "Fatal error",
        };
        self.write_out(format!("\n{label}: {message} in {file} on line {line}\n").as_bytes());
    }

    fn warn(&mut self, message: impl Into<String>) {
        self.diags.push(Diag::Warning(message.into()));
        self.report_diags();
    }

    fn notice(&mut self, message: &str) {
        self.show(E_NOTICE, message);
    }

    /// The value in register `r` of the frame window `w`, as an operand:
    /// looks through references; an undefined variable warns and reads as null.
    #[inline(always)]
    fn operand(&mut self, w: &[Value], func: &Function, r: Reg) -> Value {
        match &w[r as usize] {
            Value::Undef => self.undefined(func, r),
            // A global slot another file unset.
            Value::Ref(cell) if matches!(*cell.borrow(), Value::Undef) => self.undefined(func, r),
            Value::Ref(cell) => cell.borrow().clone(),
            v => v.clone(),
        }
    }

    /// Reading an undefined register: a warning for a variable, then null.
    #[cold]
    #[inline(never)]
    fn undefined(&mut self, func: &Function, r: Reg) -> Value {
        if let Some(name) = func.cv_names.get(r as usize) {
            let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
            self.warn(format!("Undefined variable ${name}"));
        }
        Value::Null
    }

    /// Move the value out of variable register `r` (leaving null), so it
    /// can be changed in place without a copy; undefined warns.
    fn take_var(&mut self, w: &mut [Value], func: &Function, r: Reg) -> Value {
        match &mut w[r as usize] {
            Value::Undef => self.undefined(func, r),
            Value::Ref(cell) => std::mem::take(&mut *cell.borrow_mut()),
            slot => std::mem::take(slot),
        }
    }

    /// What call site `site` names (cached after the first time).
    #[inline(always)]
    fn resolve(&mut self, clo: &Closure, site: u16) -> Result<Callee, Throw> {
        match clo.sites[site as usize].get() {
            Some(c) => Ok(c.clone()),
            None => self.resolve_slow(clo, site),
        }
    }

    #[cold]
    #[inline(never)]
    fn resolve_slow(&mut self, clo: &Closure, site: u16) -> Result<Callee, Throw> {
        let s = &clo.code.sites[site as usize];
        let key = s.name.as_bytes();
        let global = s.fallback.as_ref().map(|g| g.as_bytes());
        let callee = if let Some(f) = self.functions.get(key) {
            Callee::User(*f)
        } else if let Some(i) = self.isolate.registry.lookup(key) {
            Callee::Builtin(i)
        } else if let Some(f) = global.and_then(|g| self.functions.get(g)) {
            Callee::User(*f)
        } else if let Some(i) = global.and_then(|g| self.isolate.registry.lookup(g)) {
            Callee::Builtin(i)
        } else {
            let name = String::from_utf8_lossy(s.display.as_bytes()).into_owned();
            return Err(self.throw("Error", format!("Call to undefined function {name}()")));
        };
        let _ = clo.sites[site as usize].set(callee.clone());
        Ok(callee)
    }

    /// Run frames until the bottom one returns: `fast` runs the common
    /// cases (cached calls and returns included), `step` the one
    /// instruction it stopped at.
    fn execute(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        let stop = Arc::clone(&self.stop);
        let mut pc = self.frames.last().expect("a frame").pc as usize;
        // The function `step` last ran in (refreshed when `fast` switched).
        let mut func = Rc::clone(&self.frames.last().expect("a frame").func);
        // Fiber::throw(): the exception is raised where the fiber went on.
        if let Some(e) = self.fiber_throw.take() {
            pc = self.unwind(regs, Throw::Object(e), pc.saturating_sub(1))?;
        }
        loop {
            pc = fast(regs, &mut self.frames, &self.closures, pc, &stop, &self.ids);
            if self.ids.has_pending() {
                pc = self.after_errors(regs, pc, pc.saturating_sub(1))?;
            }
            let frame = self.frames.last().expect("a frame");
            if !Rc::ptr_eq(&func, &frame.func) {
                func = Rc::clone(&frame.func);
            }
            let (base, argc) = (frame.base as usize, frame.argc as usize);
            self.pc = pc + 1;
            // A builtin the call site already resolved is called from here,
            // not through `step`.
            if let Instr::Call { site, base: args, argc: n, dst } = func.code.code[pc]
                && let Some(&Callee::Builtin(i)) = func.sites[site as usize].get()
            {
                let at = pc;
                match self.call_builtin(regs, base, i, args, n, dst) {
                    Ok(()) => pc += 1,
                    Err(Throw::Autoload(name)) => pc = self.autoload_then(regs, name, at)?,
                    Err(t) => pc = self.unwind(regs, t, pc)?,
                }
                if !self.pending_errors.is_empty() || self.ids.has_pending() {
                    pc = self.after_errors(regs, pc, at)?;
                }
                continue;
            }
            let at = pc;
            match self.step(regs, &func, base, argc, pc) {
                Ok(Next::Go(next)) => {
                    pc = next;
                    // Fiber::suspend(): leave the fiber's frames where they are.
                    if self.fiber_suspended.is_some() {
                        if let Some(top) = self.frames.last_mut() {
                            top.pc = pc as u32;
                        }
                        return Ok(());
                    }
                }
                Ok(Next::Switch) => {
                    if self.frames.len() == self.frames.floor {
                        if !self.pending_errors.is_empty() {
                            self.dispatch_errors(regs)?;
                        }
                        if self.ids.has_pending() {
                            self.run_destructors(regs)?;
                        }
                        return Ok(());
                    }
                    pc = self.frames.last().expect("a frame").pc as usize;
                }
                // A class an autoloader may declare: run them, then this
                // instruction again.
                Err(Throw::Autoload(name)) => pc = self.autoload_then(regs, name, at)?,
                Err(t) => pc = self.unwind(regs, t, pc)?,
            }
            if !self.pending_errors.is_empty() || self.ids.has_pending() {
                pc = self.after_errors(regs, pc, at)?;
            }
        }
    }

    /// Run the autoloaders for `name`, then go on at `at` (the instruction
    /// that wanted the class, again).
    #[cold]
    fn autoload_then(&mut self, regs: &mut Vec<Value>, name: PhpStr, at: usize) -> Result<usize, Throw> {
        match self.run_autoload(regs, name) {
            Ok(()) => Ok(at),
            Err(t) => self.unwind(regs, t, at),
        }
    }

    /// Call the error handler for the diagnostics the last instruction
    /// raised, then the destructors of the objects it freed; an exception
    /// either throws is raised there.
    #[cold]
    fn after_errors(&mut self, regs: &mut Vec<Value>, pc: usize, at: usize) -> Result<usize, Throw> {
        let r = if self.pending_errors.is_empty() { Ok(()) } else { self.dispatch_errors(regs) };
        if let Some((slot, v)) = self.deferred_write.take()
            && let Some(cell) = regs.get_mut(slot)
        {
            match cell {
                Value::Ref(r) => *r.borrow_mut() = v,
                other => *other = v,
            }
        }
        match r.and_then(|()| self.run_destructors_at(regs, Some(at + 1))) {
            Ok(()) => Ok(pc),
            Err(t) => self.unwind(regs, t, at),
        }
    }

    /// Runs a magic property method found by prop_magic, its guard held.
    fn run_magic(&mut self, regs: &mut Vec<Value>, found: (crate::classes::MethodFrame, (usize, PhpStr, crate::classes::Magic)), args: &mut [Value]) -> Result<Value, Throw> {
        let (m, key) = found;
        self.magic_guard.push(key.clone());
        let r = self.run_nested(regs, m, args);
        if let Some(i) = self.magic_guard.iter().rposition(|k| *k == key) {
            self.magic_guard.remove(i);
        }
        r
    }

    /// `$o->p op= …`, `$o->p++` on an overloaded property: read through
    /// __get (or the property), compute, write through __set (or the
    /// property). None: no magic method takes part.
    fn magic_update(
        &mut self,
        regs: &mut Vec<Value>,
        target: &Value,
        name: &Value,
        compute: impl FnOnce(&mut Self, &Value) -> Result<(Value, Value), Throw>,
    ) -> Result<Option<Value>, Throw> {
        use crate::classes::Magic;
        let get = self.prop_magic(target, name, Magic::Get);
        let has_set = self.prop_magic(target, name, Magic::Set).is_some();
        if get.is_none() && !has_set {
            return Ok(None);
        }
        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
        let x = match get {
            Some(found) => self.run_magic(regs, found, &mut [pname.clone()])?.deref(),
            None => self.read_for_update(target, name)?,
        };
        let (stored, result) = compute(self, &x)?;
        match self.prop_magic(target, name, Magic::Set) {
            Some(found) => {
                self.run_magic(regs, found, &mut [pname, stored])?;
            }
            None => self.write_prop(target, name, stored)?,
        }
        Ok(Some(result))
    }

    /// `$o->p op= …`, `$o->p++` on a hooked property: read through its get
    /// hook (or the stored value), compute (the stored value, the
    /// expression's), write through its set hook. None: not hooked.
    fn hooked_update(
        &mut self,
        regs: &mut Vec<Value>,
        target: &Value,
        name: &Value,
        compute: impl FnOnce(&mut Self, &Value) -> Result<(Value, Value), Throw>,
    ) -> Result<Option<Value>, Throw> {
        let get = self.prop_hook(target, name, HOOK_GET);
        let set = self.prop_hook(target, name, HOOK_SET);
        if get.is_none() && set.is_none() {
            return Ok(None);
        }
        let x = match get {
            Some(hook) => self.run_nested(regs, hook?, &mut [])?,
            None => self.read_for_update(target, name)?,
        };
        let (stored, result) = compute(self, &x)?;
        match self.prop_hook(target, name, HOOK_SET) {
            Some(hook) => {
                self.run_nested(regs, hook?, &mut [stored])?;
            }
            None => self.write_prop(target, name, stored)?,
        }
        Ok(Some(result))
    }

    /// Execute the instruction at `pc` of `func`'s frame (at `base`).
    #[inline(never)]
    fn step(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, argc: usize, at: usize) -> Result<Next, Throw> {
        // This instruction's index (a lazy object's initialization runs it again).
        let insn_at = at;
        let func = &*clo.code;
        if self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
            return Err(self.interrupted());
        }
        // The one-operator arithmetic opcodes run here as `Binary`.
        let instr = match func.code[at].as_binary() {
            Some((op, dst, a, b)) => Instr::Binary { op, dst, a, b },
            None => func.code[at],
        };
        let mut pc = at + 1;
        // `w` is this frame's window (registers are frame-relative); `below`
        // holds the frames under it, the main frame's globals first.
        let in_main = base == 0;
        let (below, w) = regs.split_at_mut(base);
        let w = &mut w[..func.nregs as usize];
        macro_rules! reg {
            ($r:expr) => {
                w[$r as usize]
            };
        }
        match instr {
                Instr::Move { dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = v;
                }
                Instr::Assign { var, src } => {
                    let v = self.operand(w, func, src);
                    assign(w, var, v);
                }
                Instr::AssignTake { var, src } => {
                    let v = self.operand(w, func, src);
                    clear(&mut reg!(src));
                    assign(w, var, v);
                }
                Instr::Free { reg } => clear(&mut reg!(reg)),
                Instr::LoadNull { dst } => reg!(dst) = Value::Null,
                Instr::Add { .. } | Instr::Sub { .. } | Instr::Mul { .. } => unreachable!("normalized to Binary above"),
                Instr::Binary { op, dst, a, b } => {
                    // Fast paths for the common int cases.
                    let fast = match (&reg!(a), &reg!(b), op) {
                        (Value::Int(x), Value::Int(y), BinOp::Add) => x.checked_add(*y).map(Value::Int),
                        (Value::Int(x), Value::Int(y), BinOp::Sub) => x.checked_sub(*y).map(Value::Int),
                        (Value::Int(x), Value::Int(y), BinOp::Mul) => x.checked_mul(*y).map(Value::Int),
                        (Value::Float(x), Value::Float(y), _) => float_op(op, *x, *y),
                        (Value::Int(x), Value::Float(y), _) => float_op(op, *x as f64, *y),
                        (Value::Float(x), Value::Int(y), _) => float_op(op, *x, *y as f64),
                        _ => None,
                    };
                    let v = match fast {
                        Some(v) => v,
                        None => {
                            let mut x = self.operand(w, func, a);
                            let mut y = self.operand(w, func, b);
                            if op == BinOp::Concat && (matches!(x, Value::Object(_)) || matches!(y, Value::Object(_))) {
                                x = self.string_of(regs, x)?;
                                y = self.string_of(regs, y)?;
                                let r = ops::binary(op, &x, &y, &mut self.diags);
                                self.report_diags();
                                let v = r.map_err(|e| self.op_error(e))?;
                                assign(&mut regs[base..], dst, v);
                                return Ok(Next::Go(pc));
                            }
                            let r = ops::binary(op, &x, &y, &mut self.diags);
                            self.report_diags();
                            r.map_err(|e| self.op_error(e))?
                        }
                    };
                    // The result may go straight to a variable (`$x = $a + $b`).
                    assign(w, dst, v);
                }
                Instr::AssignOp { op, var, src } => {
                    let y = self.operand(w, func, src);
                    if op == BinOp::Concat {
                        let y = self.string_of(regs, y)?;
                        // An object on the left: its __toString() first.
                        if matches!(regs[base + var as usize].deref(), Value::Object(_)) {
                            let x = regs[base + var as usize].deref();
                            let x = self.string_of(regs, x)?;
                            assign(&mut regs[base..], var, x);
                        }
                        let w = &mut regs[base..base + func.nregs as usize];
                        // `.=` appends in place when the string is not shared
                        let x = self.take_var(w, func, var);
                        if let Some(e) = ops::object_string_error(&x).or_else(|| ops::object_string_error(&y)) {
                            assign(w, var, x);
                            return Err(self.op_error(e));
                        }
                        let mut s = match x {
                            Value::Str(s) => s,
                            other => ops::to_string_lossy(&other, &mut self.diags),
                        };
                        let tail = ops::to_string_lossy(&y, &mut self.diags);
                        self.report_diags();
                        s.push_bytes(tail.as_bytes());
                        assign(w, var, Value::Str(s));
                    } else {
                        let undefined = matches!(w[var as usize], Value::Undef);
                        let x = self.operand(w, func, var);
                        let r = ops::binary(op, &x, &y, &mut self.diags);
                        self.report_diags();
                        let v = r.map_err(|e| self.op_error(e))?;
                        // An undefined variable's warning goes to the error
                        // handler before the write, as in PHP (it may unset
                        // the variable): the write waits for it.
                        if undefined && !self.pending_errors.is_empty() {
                            self.deferred_write = Some((base + var as usize, v));
                        } else {
                            assign(w, var, v);
                        }
                    }
                }
                Instr::Compare { cmp, dst, a, b } => {
                    let v = if cmp == Cmp::Spaceship {
                        let x = self.operand(w, func, a);
                        let y = self.operand(w, func, b);
                        compare(cmp, &x, &y)
                    } else {
                        Value::Bool(self.test_regs(w, func, cmp, a, b))
                    };
                    assign(w, dst, v);
                }
                Instr::JumpLt { .. }
                | Instr::JumpNotLt { .. }
                | Instr::JumpLe { .. }
                | Instr::JumpNotLe { .. }
                | Instr::JumpEq { .. }
                | Instr::JumpNotEq { .. }
                | Instr::JumpIdentical { .. }
                | Instr::JumpNotIdentical { .. } => {
                    let Some((cmp, a, b, when, target)) = instr.as_jump_cmp() else { unreachable!("a compare-and-branch") };
                    if self.test_regs(w, func, cmp, a, b) == when {
                        pc = target as usize;
                    }
                }
                // A fused step and test runs here as its step alone; the
                // test is the next instruction.
                Instr::IncVar { var }
                | Instr::DecVar { var }
                | Instr::IncJumpLt { var, .. }
                | Instr::IncJumpLe { var, .. }
                | Instr::DecJumpGt { var, .. }
                | Instr::DecJumpGe { var, .. } => {
                    let inc = matches!(instr, Instr::IncVar { .. } | Instr::IncJumpLt { .. } | Instr::IncJumpLe { .. });
                    if let Value::Int(i) = reg!(var)
                        && let Some(n) = if inc { i.checked_add(1) } else { i.checked_sub(1) }
                    {
                        reg!(var) = Value::Int(n);
                        return Ok(Next::Go(pc));
                    }
                    let old = self.operand(w, func, var);
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    assign(w, var, new);
                }
                Instr::Not { dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = Value::Bool(!v.to_bool());
                }
                Instr::ToBool { dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = Value::Bool(v.to_bool());
                }
                Instr::Neg { dst, src } => {
                    let v = self.operand(w, func, src);
                    let r = ops::negate(&v, &mut self.diags);
                    self.report_diags();
                    reg!(dst) = r.map_err(|e| self.op_error(e))?;
                }
                Instr::BitNot { dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = ops::bit_not(&v).map_err(|e| self.op_error(e))?;
                }
                Instr::Cast { kind: CastKind::String, dst, src } => {
                    let v = self.operand(w, func, src);
                    let v = self.string_of(regs, v)?;
                    if let Some(e) = ops::object_string_error(&v) {
                        return Err(self.op_error(e));
                    }
                    let s = ops::to_string_lossy(&v, &mut self.diags);
                    self.report_diags();
                    regs[base + dst as usize] = Value::Str(s);
                }
                Instr::Cast { kind, dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = match kind {
                        CastKind::Int => Value::Int(v.to_int()),
                        CastKind::Float => Value::Float(v.to_float()),
                        CastKind::Bool => Value::Bool(v.to_bool()),
                        CastKind::String => {
                            let v = self.object_to_string(v)?;
                            if let Some(e) = ops::object_string_error(&v) {
                                return Err(self.op_error(e));
                            }
                            let s = ops::to_string_lossy(&v, &mut self.diags);
                            self.report_diags();
                            Value::Str(s)
                        }
                        CastKind::Array => match v {
                            Value::Array(a) => Value::Array(a),
                            // A lazy object is initialized; a proxy is its instance.
                            Value::Object(o) if o.is_uninit_lazy() => {
                                self.lazy_init(regs, &o)?;
                                return Ok(Next::Go(insn_at));
                            }
                            Value::Object(o) => Value::Array(Rc::new(classes::object_to_array(&o.proxy_instance().unwrap_or(o)))),
                            Value::Null => Value::Array(Rc::new(PhpArray::new())),
                            other => Value::Array(Rc::new(PhpArray::from_list(vec![other]))),
                        },
                        CastKind::Unset => Value::Null,
                    };
                }
                Instr::IncDec { var, dst, inc, post } => {
                    if let Value::Int(i) = reg!(var)
                        && let Some(n) = if inc { i.checked_add(1) } else { i.checked_sub(1) }
                    {
                        reg!(var) = Value::Int(n);
                        reg!(dst) = Value::Int(if post { i } else { n });
                        return Ok(Next::Go(pc));
                    }
                    let old = self.operand(w, func, var);
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    assign(w, var, new.clone());
                    reg!(dst) = if post { old } else { new };
                }
                Instr::Jump { target } => pc = target as usize,
                Instr::JumpIf { cond, target } => {
                    let c = match &reg!(cond) {
                        Value::Bool(b) => *b,
                        _ => self.operand(w, func, cond).to_bool(),
                    };
                    if c {
                        pc = target as usize;
                    }
                }
                Instr::JumpIfNot { cond, target } => {
                    let c = match &reg!(cond) {
                        Value::Bool(b) => *b,
                        _ => self.operand(w, func, cond).to_bool(),
                    };
                    if !c {
                        pc = target as usize;
                    }
                }
                Instr::JumpIfSet { src, dst, target } => {
                    let v = match &reg!(src) {
                        Value::Undef | Value::Null => None,
                        Value::Ref(cell) => {
                            let v = cell.borrow().clone();
                            (!matches!(v, Value::Null | Value::Undef)).then_some(v)
                        }
                        v => Some(v.clone()),
                    };
                    if let Some(v) = v {
                        reg!(dst) = v;
                        pc = target as usize;
                    }
                }
                Instr::NewArray { dst } => reg!(dst) = Value::Array(Rc::new(PhpArray::new())),
                Instr::ArrayPush { arr, val } => {
                    let v = self.operand(w, func, val);
                    if let Value::Array(a) = &mut reg!(arr)
                        && Rc::make_mut(a).push(v).is_err()
                    {
                        return Err(self.throw("Error", "Cannot add element to the array as the next element is already occupied"));
                    }
                }
                Instr::ArraySet { arr, key, val } => {
                    let k = self.key(w, func, key)?;
                    let v = self.operand(w, func, val);
                    if let Value::Array(a) = &mut reg!(arr) {
                        Rc::make_mut(a).insert(k, v);
                    }
                }
                Instr::FetchDim { dst, arr, key, quiet } => {
                    let container = if quiet {
                        match &reg!(arr) {
                            Value::Ref(cell) => cell.borrow().clone(),
                            v => v.clone(),
                        }
                    } else {
                        self.operand(w, func, arr)
                    };
                    // An ArrayAccess object: its offsetGet.
                    if let Value::Object(o) = &container {
                        let k = self.operand(w, func, key);
                        let v = self.aa_get(regs, o, k, quiet)?;
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    let v = self.fetch_dim(&container, w, func, key, quiet)?;
                    reg!(dst) = v;
                }
                Instr::AssignDim { var, keys, n, append, val, dst } => {
                    let v = self.operand(w, func, val);
                    let key_values: Vec<Value> = (0..n).map(|i| reg!(keys + i as Reg).deref()).collect();
                    if arrayaccess::path_has_object(&reg!(var), &key_values, append) {
                        let root = reg!(var).deref();
                        let r = self.aa_write(regs, &root, &key_values, append, arrayaccess::DimOp::Set(v))?;
                        regs[base + dst as usize] = r;
                        return Ok(Next::Go(pc));
                    }
                    self.assign_dim(w, func, var, keys, n, append, v.clone(), None)?;
                    reg!(dst) = v;
                }
                Instr::AssignDimOp { op, var, keys, n, val, dst } => {
                    let v = self.operand(w, func, val);
                    let key_values: Vec<Value> = (0..n).map(|i| reg!(keys + i as Reg).deref()).collect();
                    if arrayaccess::path_has_object(&reg!(var), &key_values, false) {
                        let root = reg!(var).deref();
                        let r = self.aa_write(regs, &root, &key_values, false, arrayaccess::DimOp::Op(op, v))?;
                        regs[base + dst as usize] = r;
                        return Ok(Next::Go(pc));
                    }
                    let result = self.assign_dim(w, func, var, keys, n, false, v, Some(op))?;
                    reg!(dst) = result;
                }
                Instr::IssetDim { dst, var, keys, n } => {
                    let key_values: Vec<Value> = (0..n).map(|i| reg!(keys + i as Reg).deref()).collect();
                    if arrayaccess::path_has_object(&reg!(var), &key_values, false) {
                        let root = reg!(var).deref();
                        let set = self.aa_isset(regs, &root, &key_values)?;
                        regs[base + dst as usize] = Value::Bool(set);
                        return Ok(Next::Go(pc));
                    }
                    let mut cur = match &reg!(var) {
                        Value::Ref(cell) => cell.borrow().clone(),
                        v => v.clone(),
                    };
                    let mut set = true;
                    for i in 0..n {
                        let k = match Key::from_value(&reg!(keys + i as Reg)) {
                            Ok((k, _)) => k,
                            Err(_) => {
                                set = false;
                                break;
                            }
                        };
                        let next = match &cur {
                            Value::Array(a) => a.get(&k).map(|v| v.deref()),
                            Value::Str(s) => match k {
                                Key::Int(i) if (i >= 0 && (i as usize) < s.len()) || (i < 0 && i.unsigned_abs() as usize <= s.len()) => Some(Value::Null),
                                _ => None,
                            },
                            _ => None,
                        };
                        match next {
                            Some(v) => cur = v,
                            None => {
                                set = false;
                                break;
                            }
                        }
                    }
                    reg!(dst) = Value::Bool(set && !matches!(cur, Value::Null | Value::Undef));
                }
                Instr::IssetVar { dst, var } => {
                    let set = match &reg!(var) {
                        Value::Undef | Value::Null => false,
                        Value::Ref(cell) => !matches!(*cell.borrow(), Value::Null | Value::Undef),
                        _ => true,
                    };
                    reg!(dst) = Value::Bool(set);
                }
                Instr::UnsetVar { var } => {
                    // In a file included at global scope the variable is the
                    // global itself: unset removes it there too.
                    if !in_main
                        && self.frames.last().is_some_and(|f| f.global)
                        && let Some(name) = func.cv_names.get(var as usize)
                    {
                        let name = name.as_bytes().to_vec();
                        self.global_clear(below, &name);
                    }
                    reg!(var) = Value::Undef;
                }
                Instr::UnsetDim { var, keys, n } => {
                    let key_values: Vec<Value> = (0..n).map(|i| reg!(keys + i as Reg).clone()).collect();
                    let derefed: Vec<Value> = key_values.iter().map(Value::deref).collect();
                    if arrayaccess::path_has_object(&reg!(var), &derefed, false) {
                        let root = reg!(var).deref();
                        self.aa_write(regs, &root, &derefed, false, arrayaccess::DimOp::Unset)?;
                        return Ok(Next::Go(pc));
                    }
                    let slot = &mut reg!(var);
                    let mut holder = match slot {
                        Value::Ref(cell) => cell.borrow().clone(),
                        v => v.clone(),
                    };
                    if unset_path(&mut holder, &key_values) {
                        assign(w, var, holder);
                    }
                }
                Instr::IterInit { src, iter, end } => {
                    let v = self.operand(w, func, src);
                    match v {
                        Value::Array(a) if !a.is_empty() => {
                            reg!(iter) = Value::Array(a);
                            reg!(iter + 1) = Value::Int(0);
                        }
                        Value::Array(_) => pc = end as usize,
                        Value::Object(o) => {
                            let it = self.iter_object(regs, o)?;
                            let w = &mut regs[base..base + func.nregs as usize];
                            match it {
                                Some(v) => {
                                    w[iter as usize] = v;
                                    w[iter as usize + 1] = Value::Int(0);
                                }
                                None => pc = end as usize,
                            }
                        }
                        other => {
                            self.warn(format!(
                                "foreach() argument must be of type array|object, {} given",
                                other.type_name()
                            ));
                            pc = end as usize;
                        }
                    }
                }
                Instr::IterNext { iter, val, key, has_key, end } if matches!(reg!(iter), Value::Object(_)) => {
                    let Value::Object(object) = reg!(iter).clone() else { unreachable!("an object") };
                    let first = matches!(reg!(iter + 1), Value::Int(0));
                    let next = self.iter_object_next(regs, &object, first)?;
                    let w = &mut regs[base..base + func.nregs as usize];
                    w[iter as usize + 1] = Value::Int(1);
                    match next {
                        Some((v, k)) => {
                            assign(w, val, v);
                            if has_key {
                                assign(w, key, k);
                            }
                        }
                        None => pc = end as usize,
                    }
                }
                Instr::IterNext { iter, val, key, has_key, end } => {
                    if !iter_next(w, iter, val, key, has_key) {
                        pc = end as usize;
                    }
                }
                Instr::SendVar { site, n, arg, var } => {
                    let by_ref = match self.resolve(clo, site)? {
                        Callee::User(f) => self.closures[f as usize].code.param_by_ref(n as usize),
                        Callee::Builtin(i) => self.isolate.registry.by_ref(i, n as usize),
                        Callee::Constant(_) => false,
                    };
                    reg!(arg) = if by_ref {
                        Value::Ref(make_ref(w, var))
                    } else {
                        self.operand(w, func, var)
                    };
                }
                Instr::SendVarMethod { obj, method, n, arg, var } => {
                    let site = &func.sites[method as usize];
                    let by_ref = match reg!(obj).deref() {
                        Value::Object(o) => self.method_takes_ref(&o.class, site.name.as_bytes(), n as usize),
                        _ => false,
                    };
                    reg!(arg) = if by_ref { Value::Ref(make_ref(w, var)) } else { self.operand(w, func, var) };
                }
                Instr::SendVarStatic { class, method, n, arg, var } => {
                    let site = &func.sites[method as usize];
                    let target = self.class_of(func, class, w)?;
                    let by_ref = self.method_takes_ref(&target, site.name.as_bytes(), n as usize);
                    reg!(arg) = if by_ref { Value::Ref(make_ref(w, var)) } else { self.operand(w, func, var) };
                }
                Instr::MethodArgByRef { dst, obj, method, n } => {
                    let site = &func.sites[method as usize];
                    reg!(dst) = Value::Bool(match reg!(obj).deref() {
                        Value::Object(o) => self.method_takes_ref(&o.class, site.name.as_bytes(), n as usize),
                        _ => false,
                    });
                }
                Instr::FuncArgByRef { dst, site, n } => {
                    reg!(dst) = Value::Bool(match self.resolve(clo, site)? {
                        Callee::User(f) => self.closures[f as usize].code.param_by_ref(n as usize),
                        Callee::Builtin(i) => self.isolate.registry.by_ref(i, n as usize),
                        Callee::Constant(_) => false,
                    });
                }
                Instr::StaticArgByRef { dst, class, method, n } => {
                    let site = &func.sites[method as usize];
                    let target = self.class_of(func, class, w)?;
                    reg!(dst) = Value::Bool(self.method_takes_ref(&target, site.name.as_bytes(), n as usize));
                }
                // Passed (and not a hole a named argument skipped): no default.
                Instr::ArgDefault { n, target } => {
                    if (n as usize) < argc && !matches!(reg!(n as Reg), Value::Undef) {
                        pc = target as usize;
                    }
                }
                Instr::Echo { src } => {
                    let v = self.operand(w, func, src);
                    let v = self.string_of(regs, v)?;
                    if let Some(e) = ops::object_string_error(&v) {
                        return Err(self.op_error(e));
                    }
                    match &v {
                        Value::Str(s) => self.write_out(s.as_bytes()),
                        other => {
                            let s = ops::to_string_lossy(other, &mut self.diags);
                            self.report_diags();
                            self.write_out(s.as_bytes());
                        }
                    }
                }
                Instr::FetchConst { site, dst } => {
                    let s = &func.sites[site as usize];
                    let v = match clo.sites[site as usize].get() {
                        Some(Callee::Constant(v)) => v.clone(),
                        _ => match self.constant_value(s.name.as_bytes()).or_else(|| s.fallback.as_ref().and_then(|g| self.constant_value(g.as_bytes()))) {
                            // Deprecated: said on every fetch, so never cached.
                            Some(v) if let Some(name) = deprecated_constant(s.name.as_bytes(), s.fallback.as_ref().map(|g| g.as_bytes())) => {
                                self.diags.push(Diag::Deprecated(format!("Constant {name} is deprecated")));
                                self.report_diags();
                                v
                            }
                            Some(v) => {
                                let _ = clo.sites[site as usize].set(Callee::Constant(v.clone()));
                                v
                            }
                            None => {
                                let name = String::from_utf8_lossy(s.display.as_bytes()).into_owned();
                                return Err(self.throw("Error", format!("Undefined constant \"{name}\"")));
                            }
                        },
                    };
                    reg!(dst) = v;
                }
                Instr::ConcatN { dst, start, n } if w[start as usize..start as usize + n as usize].iter().any(|v| matches!(v.deref(), Value::Object(_))) => {
                    // An object part: its __toString() may run PHP code.
                    let values: Vec<Value> = (0..n).map(|i| self.operand(w, func, start + i as Reg)).collect();
                    let mut bytes = Vec::new();
                    for v in values {
                        let v = self.string_of(regs, v)?;
                        if let Some(e) = ops::object_string_error(&v) {
                            return Err(self.op_error(e));
                        }
                        ops::push_string(&mut bytes, &v, &mut self.diags);
                    }
                    self.report_diags();
                    regs[base + dst as usize] = Value::Str(PhpStr::new(bytes));
                }
                Instr::ConcatN { dst, start, n } => {
                    let parts = start as usize..start as usize + n as usize;
                    let mut bytes = Vec::with_capacity(w[parts].iter().map(ops::string_len_hint).sum());
                    for i in 0..n {
                        let v = self.operand(w, func, start + i as Reg);
                        let v = self.object_to_string(v)?;
                        if let Some(e) = ops::object_string_error(&v) {
                            return Err(self.op_error(e));
                        }
                        ops::push_string(&mut bytes, &v, &mut self.diags);
                    }
                    self.report_diags();
                    reg!(dst) = Value::Str(PhpStr::new(bytes));
                }
                Instr::BindGlobal { var, name } => {
                    let name = match &clo.consts[name as usize] {
                        Value::Str(s) => s.as_bytes().to_vec(),
                        _ => Vec::new(),
                    };
                    // The main frame's window starts at 0: it is `w` when
                    // the main frame is running, else the start of `below`.
                    let cell = if in_main { self.global_cell(w, &name) } else { self.global_cell(below, &name) };
                    reg!(var) = Value::Ref(cell);
                }
                Instr::MatchError { src } => {
                    let v = self.operand(w, func, src);
                    return Err(self.throw("UnhandledMatchError", format!("Unhandled match case {}", match_case(&v))));
                }
                Instr::Exit { src, has } => {
                    let v = self.operand(w, func, src);
                    let status = match (has, &v) {
                        (true, Value::Int(i)) => *i as i32,
                        (true, other) => {
                            let s = ops::to_string_lossy(other, &mut self.diags);
                            self.write_out(s.as_bytes());
                            0
                        }
                        (false, _) => 0,
                    };
                    return Err(Throw::Exit(status));
                }
                Instr::DeclareClass { class } => {
                    let def = Arc::clone(&func.decls[class as usize]);
                    self.declare_class(&def)?;
                }
                Instr::DeclareFunction { func: i } => {
                    let declared = Arc::clone(&func.lambdas[i as usize]);
                    if let Err(message) = self.declare(Arc::clone(&declared)) {
                        return Err(Throw::Fatal { message, file: Rc::from(&*declared.file), line: declared.line });
                    }
                }
                Instr::MakeClosure { dst, func: i, is_static } => {
                    let object = self.make_closure(&func.lambdas[i as usize], is_static);
                    reg!(dst) = Value::Object(object);
                }
                Instr::Capture { closure, var, by_ref, quiet } => {
                    let v = if by_ref {
                        Value::Ref(make_ref(w, var))
                    } else if quiet {
                        reg!(var).deref()
                    } else {
                        self.operand(w, func, var)
                    };
                    if let Value::Object(o) = &reg!(closure)
                        && let Some(data) = callable::closure_data(o)
                    {
                        data.captured.borrow_mut().push(v);
                    }
                }
                Instr::Rare { op, a, b, c, class } => match op {
                    Rare::PropRef => {
                            let (dst, obj, name) = (a, b, c);
                        let target = self.operand(w, func, obj);
                        let name = reg!(name).clone();
                        if let Some(lo) = self.lazy_pending(&target, &name) {
                            self.lazy_init(regs, &lo)?;
                            return Ok(Next::Go(insn_at));
                        }
                        let target = self.lazy_forward(&target, &name).unwrap_or(target);
                        let cell = self.prop_ref(&target, &name)?;
                        reg!(dst) = Value::Ref(cell);
                    }
                    Rare::AssignPropRef => {
                            let (obj, name, src) = (a, b, c);
                        let target = self.operand(w, func, obj);
                        let name = reg!(name).clone();
                        let cell = make_ref(w, src);
                        if let Some(lo) = self.lazy_pending(&target, &name) {
                            self.lazy_init(regs, &lo)?;
                            return Ok(Next::Go(insn_at));
                        }
                        let target = self.lazy_forward(&target, &name).unwrap_or(target);
                        self.prop_bind(&target, &name, cell)?;
                    }
                    Rare::StaticRef => {
                            let (dst, name) = (a, b);
                        if let Some(c) = self.lazy_class(func, class, w)? {
                            self.resolve_class(regs, &c)?;
                            return Ok(Next::Go(at));
                        }
                        let name = reg!(name).clone();
                        let cell = self.static_cell(func, class, &name, w)?;
                        let inner = match &*cell.borrow() {
                            Value::Ref(r) => Some(Rc::clone(r)),
                            _ => None,
                        };
                        let r = match inner {
                            Some(r) => r,
                            None => {
                                let v = match std::mem::take(&mut *cell.borrow_mut()) {
                                    Value::Undef => Value::Null,
                                    v => v,
                                };
                                let r = Rc::new(RefCell::new(v));
                                *cell.borrow_mut() = Value::Ref(Rc::clone(&r));
                                r
                            }
                        };
                        reg!(dst) = Value::Ref(r);
                    }
                    Rare::AssignStaticRef => {
                            let (name, src) = (a, b);
                        if let Some(c) = self.lazy_class(func, class, w)? {
                            self.resolve_class(regs, &c)?;
                            return Ok(Next::Go(at));
                        }
                        let name = reg!(name).clone();
                        let cell = self.static_cell(func, class, &name, w)?;
                        let r = make_ref(w, src);
                        *cell.borrow_mut() = Value::Ref(r);
                    }
                    Rare::WantRef => self.ref_wanted.push(base + a as usize),
                    Rare::BindRefResult => {
                            let (var, src) = (a, b);
                        let at_reg = base + src as usize;
                        self.ref_wanted.retain(|&r| r != at_reg);
                        match reg!(src).clone() {
                            Value::Ref(cell) => reg!(var) = Value::Ref(cell),
                            v => {
                                self.notice("Only variables should be assigned by reference");
                                assign(w, var, v);
                            }
                        }
                    }
                    Rare::ArrayPushRef => {
                        let cell = make_ref(w, b);
                        if let Value::Array(arr) = &mut reg!(a) {
                            let _ = Rc::make_mut(arr).push(Value::Ref(cell));
                        }
                    }
                    Rare::ArraySetRef => {
                        let key = self.key(w, func, b)?;
                        let cell = make_ref(w, c);
                        if let Value::Array(arr) = &mut reg!(a) {
                            Rc::make_mut(arr).insert(key, Value::Ref(cell));
                        }
                    }
                    Rare::ToObject => {
                        let v = self.operand(w, func, b);
                        reg!(a) = self.object_cast(v);
                    }
                    Rare::VerifyArgs => self.verify_args(regs, base, argc)?,
                    Rare::VerifyReturn => {
                        if let Some(ty) = func.ret_type.as_ref().filter(|_| !func.is_generator) {
                            let v = self.operand(w, func, a);
                            match self.check_type(ty, &v, func.strict) {
                                Ok(Some(converted)) => reg!(a) = converted,
                                Ok(None) => {}
                                Err(()) => {
                                    // b: 0 a value returned, else none (`return;`, the end).
                                    let given = if b != 0 { "none".into() } else { p8_runtime::args::value_name(&v) };
                                    let name = self.function_display();
                                    return Err(self.throw("TypeError", format!("{name}(): Return value must be of type {}, {given} returned", ty.shown_resolved())));
                                }
                            }
                        }
                    }
                    Rare::VarRef => {
                        let name = self.var_name(w, func, b);
                        let global = c == 1 || self.frames.last().is_some_and(|f| f.global);
                        let cell = if global && !in_main { self.global_cell(below, &name) } else { self.var_cell(w, in_main, &name) };
                        reg!(a) = Value::Ref(cell);
                    }
                    Rare::VarGet => {
                        let name = self.var_name(w, func, b);
                        let global = c & 2 != 0;
                        let scope = global || self.frames.last().is_some_and(|f| f.global);
                        let v = if scope && !in_main { self.global_value(below, &name) } else { self.var_value(w, in_main, &name) };
                        match v {
                            Some(v) => reg!(a) = v,
                            None => {
                                if c & 1 == 0 {
                                    let what = if global { "global variable" } else { "variable" };
                                    self.warn(format!("Undefined {what} ${}", String::from_utf8_lossy(&name)));
                                }
                                reg!(a) = Value::Null;
                            }
                        }
                    }
                    Rare::VarUnset => {
                        let name = self.var_name(w, func, a);
                        let at_global = self.frames.last().is_some_and(|f| f.global);
                        if at_global && !in_main {
                            // A file included at global scope: the global is
                            // emptied, and this file's binding to it goes.
                            self.global_clear(below, &name);
                            if let Some(r) = func.cv_names.iter().position(|n| n.as_bytes() == name.as_slice()) {
                                w[r] = Value::Undef;
                            }
                        } else if b == 1 && !in_main {
                            self.global_unset(below, &name);
                        } else {
                            self.var_unset(w, in_main, &name);
                        }
                    }
                    Rare::GlobalBind => {
                        let name = self.var_name(w, func, a);
                        let cell = make_ref(w, b);
                        let main: &mut [Value] = if in_main { w } else { below };
                        self.global_bind(main, &name, cell);
                    }
                    Rare::Globals => {
                        let main: &[Value] = if in_main { w } else { below };
                        reg!(a) = Value::Array(Rc::new(self.globals_array(main)));
                    }
                    Rare::IssetStatic => {
                        let set = self.isset_static(func, class, &reg!(b).clone(), w)?;
                        reg!(a) = Value::Bool(set);
                    }
                    Rare::CallableArgByRef => {
                        let callee = reg!(b).clone();
                        let by_ref = self.resolve_callable(&callee).is_ok_and(|t| self.takes_ref(&t, class as usize));
                        reg!(a) = Value::Bool(by_ref);
                    }
                    Rare::ReturnRef => {
                            let src = a;
                        let ret = self.frames.last().map_or(NO_RET, |f| f.ret);
                        let wanted = ret != NO_RET && self.ref_wanted.contains(&(ret as usize));
                        if wanted {
                            self.ref_wanted.retain(|&r| r != ret as usize);
                        }
                        let v = if wanted { Value::Ref(make_ref(w, src)) } else { self.operand(w, func, src) };
                        if let Some(g) = self.frames.last_mut().and_then(|f| f.generator.take()) {
                            self.finish(&g, v);
                            leave(&mut self.frames, regs, Value::Null);
                            return Ok(Next::Switch);
                        }
                        leave(&mut self.frames, regs, v);
                        return Ok(Next::Switch);
                    }
                },
                Instr::BindRef { var, src } => {
                    if var != src {
                        let cell = make_ref(w, src);
                        reg!(var) = Value::Ref(cell);
                    }
                }
                Instr::RefDim { var, root, keys, n, append } => {
                    let mut path = Vec::with_capacity(n as usize);
                    for i in 0..n {
                        path.push(self.key(w, func, keys + i as Reg)?);
                    }
                    let cell = make_ref(w, root);
                    let mut value = std::mem::take(&mut *cell.borrow_mut());
                    let r = self.dim_ref(&mut value, &path, append);
                    *cell.borrow_mut() = value;
                    reg!(var) = Value::Ref(r?);
                }
                Instr::DimSetRef { root, keys, n, append, src } => {
                    let mut path = Vec::with_capacity(n as usize);
                    for i in 0..n {
                        path.push(self.key(w, func, keys + i as Reg)?);
                    }
                    let target = make_ref(w, src);
                    let cell = make_ref(w, root);
                    let mut value = std::mem::take(&mut *cell.borrow_mut());
                    let r = self.dim_set_ref(&mut value, &path, append, target);
                    *cell.borrow_mut() = value;
                    r?;
                }
                Instr::IterInitRef { src, iter, end } => {
                    let cell = make_ref(w, src);
                    let state = match &*cell.borrow() {
                        Value::Array(a) => Ok(!a.is_empty()),
                        other => Err(other.type_name()),
                    };
                    match state {
                        Ok(true) => {
                            reg!(iter) = Value::Int(0);
                            reg!(iter + 1) = Value::Ref(cell);
                        }
                        Ok(false) => pc = end as usize,
                        Err(ty) => {
                            self.warn(format!("foreach() argument must be of type array|object, {ty} given"));
                            pc = end as usize;
                        }
                    }
                }
                Instr::IterNextRef { iter, val, key, has_key, end } => {
                    let pos = match reg!(iter) {
                        Value::Int(p) => p as usize,
                        _ => 0,
                    };
                    let next = match &reg!(iter + 1) {
                        Value::Ref(cell) => refs::next_ref(cell, pos),
                        _ => None,
                    };
                    match next {
                        Some((p, k, cell)) => {
                            reg!(iter) = Value::Int(p as i64 + 1);
                            reg!(val) = Value::Ref(cell);
                            if has_key {
                                assign(w, key, k.into_value());
                            }
                        }
                        None => pc = end as usize,
                    }
                }
                Instr::CollectVariadic { var } => {
                    // Its own argument (if passed), then those past it.
                    let mut list = Vec::new();
                    if argc > var as usize {
                        list.push(std::mem::replace(&mut reg!(var), Value::Undef));
                    }
                    if let Some(frame) = self.frames.last() {
                        list.extend(frame.args().iter().cloned());
                    }
                    let mut arr = PhpArray::from_list(list);
                    // Named arguments no parameter takes: by their names.
                    if let Some((depth, named)) = self.named_extra.take() {
                        if depth == self.frames.len() {
                            for (k, v) in named.iter() {
                                arr.insert(k, v.clone());
                            }
                        } else {
                            self.named_extra = Some((depth, named));
                        }
                    }
                    reg!(var) = Value::Array(Rc::new(arr));
                }
                Instr::FetchList { dst, arr, key } => {
                    let container = match &reg!(arr) {
                        Value::Ref(cell) => cell.borrow().clone(),
                        v => v.clone(),
                    };
                    reg!(dst) = match container {
                        Value::Array(_) => self.fetch_dim(&container, w, func, key, false)?,
                        // An ArrayAccess object: its offsetGet.
                        Value::Object(o) => {
                            let k = self.operand(w, func, key);
                            let v = self.aa_get(regs, &o, k, false)?;
                            regs[base + dst as usize] = v;
                            return Ok(Next::Go(pc));
                        }
                        _ => Value::Null,
                    };
                }
                Instr::StaticVar { var, index, skip } => {
                    let (cell, new) = self.static_var(func, index);
                    reg!(var) = Value::Ref(cell);
                    if !new {
                        pc = skip as usize;
                    }
                }
                Instr::FromCallable { dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = self.closure_from_callable(&v)?;
                }
                Instr::CallValue { dst, callee, base: args, argc } => {
                    let target = self.operand(w, func, callee);
                    return self.call_value(regs, &target, base + args as usize, argc, (base + dst as usize) as u32, pc);
                }
                Instr::Silence { saved } => {
                    reg!(saved) = Value::Int(self.error_level);
                    // E_ERROR | E_CORE_ERROR | E_COMPILE_ERROR | E_USER_ERROR | E_RECOVERABLE_ERROR | E_PARSE
                    self.error_level &= 4437;
                }
                Instr::Unsilence { saved } => {
                    // A level the silenced code set stays (as in PHP),
                    // unless it only narrowed it to the silenced one.
                    if let Value::Int(level) = reg!(saved)
                        && self.error_level == level & 4437
                    {
                        self.error_level = level;
                    }
                }
                Instr::Include { dst, src, kind } => {
                    let v = self.operand(w, func, src);
                    let Some(program) = self.load_include(&v, kind, &func.file)? else {
                        reg!(dst) = Value::Bool(kind.once());
                        return Ok(Next::Go(pc));
                    };
                    let program = match program {
                        Ok(p) => p,
                        Err(()) => {
                            reg!(dst) = Value::Bool(false);
                            return Ok(Next::Go(pc));
                        }
                    };
                    return Ok(self.run_in_scope(regs, base, func, &program.main, dst, pc));
                }
                Instr::Eval { dst, src } => {
                    let code = self.operand(w, func, src);
                    let program = self.load_eval(&code)?;
                    return Ok(self.run_in_scope(regs, base, func, &program.main, dst, pc));
                }
                Instr::DimIncDec { var, keys, n, dst, inc, post } => {
                    let key_values: Vec<Value> = (0..n).map(|i| reg!(keys + i as Reg).deref()).collect();
                    if arrayaccess::path_has_object(&reg!(var), &key_values, false) {
                        let root = reg!(var).deref();
                        let r = self.aa_write(regs, &root, &key_values, false, arrayaccess::DimOp::IncDec { inc, post })?;
                        regs[base + dst as usize] = r;
                        return Ok(Next::Go(pc));
                    }
                    let mut path = Vec::with_capacity(n as usize);
                    for i in 0..n {
                        path.push(self.key(w, func, keys + i as Reg)?);
                    }
                    let mut root = match &mut reg!(var) {
                        Value::Ref(cell) => std::mem::take(&mut *cell.borrow_mut()),
                        other => std::mem::take(other),
                    };
                    let r = self.incdec_path(&mut root, &path, inc);
                    match &mut reg!(var) {
                        Value::Ref(cell) => *cell.borrow_mut() = root,
                        other => *other = root,
                    }
                    let (old, new) = r?;
                    reg!(dst) = if post { old } else { new };
                }
                Instr::CallSpread { dst, callee, args } => {
                    let target = self.operand(w, func, callee);
                    let list: Vec<Value> = match self.operand(w, func, args) {
                        // Named arguments: placed by the target's parameters.
                        Value::Array(a) if a.iter().any(|(k, _)| matches!(k, p8_runtime::array::Key::Str(_))) => {
                            let t = self.resolve_callable(&target)?;
                            self.place_args(&t, &a)?
                        }
                        // References (the arguments compiled as ones) kept
                        // for the parameters that take them.
                        Value::Array(a) if a.values().any(|v| matches!(v, Value::Ref(_))) => {
                            let t = self.resolve_callable(&target)?;
                            a.values().enumerate().map(|(n, v)| if matches!(v, Value::Ref(_)) && self.takes_ref(&t, n) { v.clone() } else { v.deref() }).collect()
                        }
                        Value::Array(a) => a.values().map(Value::deref).collect(),
                        _ => Vec::new(),
                    };
                    let at = base + func.nregs as usize;
                    if regs.len() < at + list.len() + 1 {
                        regs.resize(at + list.len() + 1, Value::Undef);
                    }
                    let argc = list.len() as u8;
                    for (i, v) in list.into_iter().enumerate() {
                        regs[at + i] = v;
                    }
                    return self.call_value(regs, &target, at, argc, (base + dst as usize) as u32, pc);
                }
                Instr::ArrayUnpack { arr, src } => {
                    let v = self.operand(w, func, src);
                    let items = match v {
                        // A reference held elsewhere too stays one (as PHP's
                        // unpacking does): a call takes it by reference.
                        Value::Array(a) => a
                            .iter()
                            .map(|(k, v)| {
                                let v = match v {
                                    Value::Ref(cell) if Rc::strong_count(cell) > 1 => v.clone(),
                                    other => other.deref(),
                                };
                                (k.into_value(), v)
                            })
                            .collect(),
                        Value::Object(ref o) if o.class.is_a(b"traversable") => {
                            let items = self.iterate_all(regs, &v)?;
                            let w = &mut regs[base..base + func.nregs as usize];
                            return self.unpack_into(w, arr, items).map(|()| Next::Go(pc));
                        }
                        _ => return Err(self.throw("Error", "Only arrays and Traversables can be unpacked")),
                    };
                    self.unpack_into(w, arr, items)?;
                }
                Instr::New { dst, class } => {
                    let class = self.class_of(func, class, w)?;
                    if self.unresolved(&class) {
                        self.resolve_class(regs, &class)?;
                        return Ok(Next::Go(at));
                    }
                    let (object, ctor) = {
                        let read = |i: usize| if i < base { below[i].clone() } else { w[i - base].clone() };
                        self.instantiate(&read, class)?
                    };
                    // Construct finds the constructor again, after the arguments.
                    drop(ctor);
                    // Not destructed unless constructed (an argument may throw first).
                    object.destructed.set(true);
                    reg!(dst) = Value::Object(object);
                }
                Instr::ConstructSpread { obj, args } => {
                    let Value::Object(object) = reg!(obj).clone() else { unreachable!("ConstructSpread follows New") };
                    object.destructed.set(false);
                    let arr = self.operand(w, func, args);
                    let Some(m) = self.constructor(&object) else { return Ok(Next::Go(pc)) };
                    let target = callable::Target::Frame(m);
                    let list = match &arr {
                        Value::Array(a) => self.place_args(&target, a)?,
                        _ => Vec::new(),
                    };
                    let callable::Target::Frame(m) = target else { unreachable!("a frame") };
                    let at = base + func.nregs as usize;
                    if regs.len() < at + list.len() + 1 {
                        regs.resize(at + list.len() + 1, Value::Undef);
                    }
                    let argc = list.len() as u8;
                    for (i, v) in list.into_iter().enumerate() {
                        regs[at + i] = v;
                    }
                    return self.call_method(regs, m, at, argc, NO_RET, pc);
                }
                Instr::Construct { obj, base: args, argc } => {
                    let Value::Object(object) = reg!(obj).clone() else { unreachable!("Construct follows New") };
                    object.destructed.set(false);
                    match self.constructor(&object) {
                        Some(m) => return self.call_method(regs, m, base + args as usize, argc, NO_RET, pc),
                        None => {
                            for r in &mut w[args as usize..args as usize + argc as usize] {
                                clear(r);
                            }
                        }
                    }
                }
                Instr::This { dst } => match self.frames.last().and_then(|f| f.this.clone()) {
                    Some(this) => reg!(dst) = Value::Object(this),
                    None => return Err(self.throw("Error", "Using $this when not in object context")),
                },
                Instr::FetchProp { dst, obj, name, quiet } => {
                    let target = if quiet { reg!(obj).deref() } else { self.operand(w, func, obj) };
                    let name = reg!(name).clone();
                    if let Some(hook) = self.prop_hook(&target, &name, HOOK_GET) {
                        let v = self.run_nested(regs, hook?, &mut [])?;
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    if let Value::Object(o) = &target
                        && o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS != 0
                        && (self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || (quiet && self.prop_magic(&target, &name, crate::classes::Magic::Isset).is_some()))
                    {
                        use crate::classes::Magic;
                        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        // `??`: __isset decides when there is one, then __get reads.
                        let set = if quiet {
                            match self.prop_magic(&target, &name, Magic::Isset) {
                                Some(found) => self.run_magic(regs, found, &mut [pname.clone()])?.to_bool(),
                                None => true,
                            }
                        } else {
                            true
                        };
                        let v = match self.prop_magic(&target, &name, Magic::Get) {
                            Some(found) if set => self.run_magic(regs, found, &mut [pname])?.deref(),
                            Some(_) => Value::Null,
                            None => self.fetch_prop(&target, &name, quiet)?,
                        };
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    reg!(dst) = self.fetch_prop(&target, &name, quiet)?;
                }
                Instr::AssignProp { obj, name, val, dst } => {
                    let target = self.operand(w, func, obj);
                    let v = self.operand(w, func, val);
                    let name = reg!(name).clone();
                    if let Some(hook) = self.prop_hook(&target, &name, HOOK_SET) {
                        self.run_nested(regs, hook?, &mut [v.clone()])?;
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Set) {
                        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        self.run_magic(regs, found, &mut [pname, v.clone()])?;
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    self.write_prop(&target, &name, v.clone())?;
                    reg!(dst) = v;
                }
                Instr::AssignPropOp { op, obj, name, val, dst } => {
                    let target = self.operand(w, func, obj);
                    let y = self.operand(w, func, val);
                    let name = reg!(name).clone();
                    if self.prop_hook(&target, &name, HOOK_GET).is_some() || self.prop_hook(&target, &name, HOOK_SET).is_some() {
                        let v = self.hooked_update(regs, &target, &name, |c, x| {
                        let r = ops::binary(op, x, &y, &mut c.diags);
                        c.report_diags();
                        r.map(|v| (v.clone(), v)).map_err(|e| c.op_error(e))
                    })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::Null);
                        return Ok(Next::Go(pc));
                    }
                    if self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || self.prop_magic(&target, &name, crate::classes::Magic::Set).is_some() {
                        let v = self.magic_update(regs, &target, &name, |c, x| {
                            let r = ops::binary(op, x, &y, &mut c.diags);
                            c.report_diags();
                            r.map(|v| (v.clone(), v)).map_err(|e| c.op_error(e))
                        })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::Null);
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    let x = self.read_for_update(&target, &name)?;
                    let r = ops::binary(op, &x, &y, &mut self.diags);
                    self.report_diags();
                    let v = r.map_err(|e| self.op_error(e))?;
                    self.write_prop(&target, &name, v.clone())?;
                    reg!(dst) = v;
                }
                Instr::PropIncDec { obj, name, dst, inc, post } => {
                    let target = self.operand(w, func, obj);
                    let name = reg!(name).clone();
                    if self.prop_hook(&target, &name, HOOK_GET).is_some() || self.prop_hook(&target, &name, HOOK_SET).is_some() {
                        let v = self.hooked_update(regs, &target, &name, |c, x| {
                        let r = if inc { ops::increment(x, &mut c.diags) } else { ops::decrement(x, &mut c.diags) };
                        c.report_diags();
                        r.map(|new| (new.clone(), if post { x.clone() } else { new })).map_err(|e| c.op_error(e))
                    })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::Null);
                        return Ok(Next::Go(pc));
                    }
                    if self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || self.prop_magic(&target, &name, crate::classes::Magic::Set).is_some() {
                        let v = self.magic_update(regs, &target, &name, |c, x| {
                            let r = if inc { ops::increment(x, &mut c.diags) } else { ops::decrement(x, &mut c.diags) };
                            c.report_diags();
                            r.map(|new| (new.clone(), if post { x.clone() } else { new })).map_err(|e| c.op_error(e))
                        })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::Null);
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    let old = self.read_for_update(&target, &name)?;
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    self.write_prop(&target, &name, new.clone())?;
                    reg!(dst) = if post { old } else { new };
                }
                Instr::AssignPropDim { op, base: at, n, append, val, dst } => {
                    let target = self.operand(w, func, at);
                    let name = reg!(at + 1).clone();
                    // A hooked property's value is not modified in place.
                    if let (Value::Object(o), Some(pn)) = (&target, name.to_php_string())
                        && let Some(p) = self.hooked_property(o, pn.as_bytes())
                    {
                        let class = String::from_utf8_lossy(p.declaring_name.as_bytes()).into_owned();
                        return Err(self.throw("Error", format!("Indirect modification of {class}::${} is not allowed", String::from_utf8_lossy(pn.as_bytes()))));
                    }
                    let mut path = Vec::with_capacity(n as usize);
                    for i in 0..n {
                        path.push(self.key(w, func, at + 2 + i as Reg)?);
                    }
                    let v = self.operand(w, func, val);
                    // An overloaded property: __get's value is written into
                    // only when it is an object; otherwise nothing changes.
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Get) {
                        let pname = name.to_php_string().unwrap_or_else(PhpStr::empty);
                        let got = self.run_magic(regs, found, &mut [Value::Str(pname.clone())])?;
                        if matches!(got, Value::Object(_)) {
                            // Into the object as `$obj[k]... = v` writes (ArrayAccess, SimpleXML).
                            let keys: Vec<Value> = path.iter().map(|k| match k {
                                Key::Int(i) => Value::Int(*i),
                                Key::Str(s) => Value::Str(s.clone()),
                            }).collect();
                            let dim_op = match op {
                                Some(op) => arrayaccess::DimOp::Op(op, v),
                                None => arrayaccess::DimOp::Set(v),
                            };
                            regs[base + dst as usize] = self.aa_write(regs, &got, &keys, append, dim_op)?;
                        } else {
                            let class = match &target {
                                Value::Object(o) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                                _ => String::new(),
                            };
                            self.notice(&format!("Indirect modification of overloaded property {class}::${} has no effect", String::from_utf8_lossy(pname.as_bytes())));
                            regs[base + dst as usize] = v;
                        }
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    reg!(dst) = self.update_prop_dim(&target, &name, &path, append, v, op)?;
                }
                Instr::IssetProp { dst, obj, name } => {
                    let target = reg!(obj).deref();
                    let name = reg!(name).clone();
                    if let Some(hook) = self.prop_hook(&target, &name, HOOK_GET) {
                        let v = self.run_nested(regs, hook?, &mut [])?;
                        regs[base + dst as usize] = Value::Bool(!matches!(v.deref(), Value::Null | Value::Undef));
                        return Ok(Next::Go(pc));
                    }
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Isset) {
                        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        let v = self.run_magic(regs, found, &mut [pname])?;
                        regs[base + dst as usize] = Value::Bool(v.to_bool());
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    reg!(dst) = Value::Bool(self.isset_prop(&target, &name)?);
                }
                Instr::UnsetProp { obj, name } => {
                    let target = reg!(obj).deref();
                    let name = reg!(name).clone();
                    if let (Value::Object(o), Some(n)) = (&target, name.to_php_string())
                        && let Some(p) = self.hooked_property(o, n.as_bytes())
                    {
                        let class = String::from_utf8_lossy(p.declaring_name.as_bytes()).into_owned();
                        return Err(self.throw("Error", format!("Cannot unset hooked property {class}::${}", String::from_utf8_lossy(n.as_bytes()))));
                    }
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Unset) {
                        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        self.run_magic(regs, found, &mut [pname])?;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    self.unset_prop(&target, &name)?;
                }
                Instr::MethodCall { dst, obj, method, base: args, argc } => {
                    let target = self.operand(w, func, obj);
                    // A temporary (`(new A)->f()`) is released as it is
                    // used: the call's `$this` holds the object.
                    if is_temp(func, obj) {
                        clear(&mut reg!(obj));
                    }
                    let site = &func.sites[method as usize];
                    let m = self.method_frame(&target, site.name.as_bytes(), site.display.as_bytes())?;
                    return self.call_method(regs, m, base + args as usize, argc, (base + dst as usize) as u32, pc);
                }
                Instr::StaticCall { dst, class, method, base: args, argc } => {
                    let site = &func.sites[method as usize];
                    // `parent::$prop::get()` on a parent whose property has
                    // no such hook: the stored value of `$this->prop`.
                    if let Some(rest) = site.display.as_bytes().strip_prefix(b"$")
                        && let Some(at_sep) = rest.windows(2).position(|x| x == b"::")
                    {
                        let target = self.class_of(func, class, w)?;
                        if target.method(site.name.as_bytes()).is_none() {
                            let this = self.frames.last().and_then(|f| f.this.clone());
                            let Some(this) = this else { return Err(self.throw("Error", "Using $this when not in object context")) };
                            let prop = Value::Str(PhpStr::from(&rest[..at_sep]));
                            let v = if rest.ends_with(b"::set") {
                                let v = reg!(args).deref();
                                self.write_prop(&Value::Object(this), &prop, v.clone())?;
                                v
                            } else {
                                self.fetch_prop(&Value::Object(this), &prop, false)?
                            };
                            for r in &mut w[args as usize..args as usize + argc as usize] {
                                clear(r);
                            }
                            reg!(dst) = v;
                            return Ok(Next::Go(pc));
                        }
                    }
                    let m = self.static_frame(func, class, site.name.as_bytes(), site.display.as_bytes(), w)?;
                    return self.call_method(regs, m, base + args as usize, argc, (base + dst as usize) as u32, pc);
                }
                Instr::ClassConst { dst, class, name } => {
                    let name = reg!(name).clone();
                    if let Some((thunk, scope, key)) = self.pending_constant(func, class, &name, w)? {
                        self.eval_constant(regs, &thunk, scope, key)?;
                        return Ok(Next::Go(at));
                    }
                    reg!(dst) = self.class_constant(func, class, &name, w)?;
                }
                Instr::FetchStaticProp { dst, class, name } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = reg!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let v = match cell.borrow().deref() {
                        Value::Undef => Value::Null,
                        v => v,
                    };
                    reg!(dst) = v;
                }
                Instr::AssignStaticProp { class, name, val, dst } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = reg!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let v = self.operand(w, func, val);
                    store(&cell, v.clone());
                    reg!(dst) = v;
                }
                Instr::AssignStaticPropOp { op, class, name, val, dst } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = reg!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let y = self.operand(w, func, val);
                    let x = cell.borrow().deref();
                    let r = ops::binary(op, &x, &y, &mut self.diags);
                    self.report_diags();
                    let v = r.map_err(|e| self.op_error(e))?;
                    store(&cell, v.clone());
                    reg!(dst) = v;
                }
                Instr::InstanceOf { dst, src, class } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = Value::Bool(self.instance_of(func, &v, class, w));
                }
                Instr::Clone { dst, src } => {
                    let v = self.operand(w, func, src);
                    // A lazy object is initialized before it is cloned.
                    if let Value::Object(o) = &v
                        && o.is_uninit_lazy()
                    {
                        let o = Rc::clone(o);
                        self.lazy_init(regs, &o)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let (copy, hook) = self.clone_object(&v)?;
                    reg!(dst) = Value::Object(copy);
                    if let Some(m) = hook {
                        // __clone runs in a window past this frame's.
                        return self.call_method(regs, m, base + func.nregs as usize, 0, NO_RET, pc);
                    }
                }
                Instr::Throw { src } => {
                    let v = self.operand(w, func, src);
                    return Err(match v {
                        Value::Object(o) if o.class.is_a(b"throwable") => Throw::Object(o),
                        // A force-closed fiber's exit, rethrown past a catch or finally.
                        Value::Object(o) if self.fiber_exit.as_ref().is_some_and(|m| Rc::ptr_eq(m, &o)) => Throw::Object(o),
                        _ => self.throw("Error", "Can only throw objects"),
                    });
                }
                Instr::CatchIf { src, class, target } => {
                    if self.instance_of(func, &reg!(src), class, w) {
                        pc = target as usize;
                    }
                }
                Instr::Call { site, base: args, argc, dst } => {
                    match self.resolve(clo, site)? {
                        Callee::User(target) => {
                            let target = Rc::clone(&self.closures[target as usize]);
                            if (argc as usize) < target.code.required {
                                let passed = &w[args as usize..args as usize + argc as usize];
                                return Err(self.missing_args(&target.code, passed, None));
                            }
                            if target.code.is_generator {
                                let m = classes::MethodFrame { func: classes::Code::User(Rc::clone(&target)), this: None, scope: None, called: None, captured: Vec::new(), trampoline: None };
                                let g = self.make_generator(regs, m, target, base + args as usize, argc as usize);
                                regs[base + dst as usize] = Value::Object(g);
                                return Ok(Next::Go(pc));
                            }
                            if let Some(caller) = self.frames.last_mut() {
                                caller.pc = pc as u32;
                            }
                            // Missing optional arguments are filled by the callee's defaults.
                            self.frames.push(&target, base + args as usize, argc as usize, (base + dst as usize) as u32);
                            enter(&mut self.frames, regs);
                            return Ok(Next::Switch);
                        }
                        Callee::Builtin(i) => self.call_builtin(regs, base, i, args, argc, dst)?,
                        Callee::Constant(_) => unreachable!("a call site resolves to a function"),
                    }
                }
                Instr::Return { src } => {
                    let v = self.operand(w, func, src);
                    if self.frames.len() == 1 && self.frames.floor == 0 {
                        // The script ends: shutdown functions and destructors,
                        // with its globals still there.
                        self.script_end(regs)?;
                    }
                    if let Some(g) = self.frames.last_mut().and_then(|f| f.generator.take()) {
                        self.finish(&g, v);
                        leave(&mut self.frames, regs, Value::Null);
                        return Ok(Next::Switch);
                    }
                    leave(&mut self.frames, regs, v);
                    return Ok(Next::Switch);
                }

                Instr::ReturnNull => {
                    if self.frames.len() == 1 && self.frames.floor == 0 {
                        self.script_end(regs)?;
                    }
                    if let Some(g) = self.frames.last_mut().and_then(|f| f.generator.take()) {
                        self.finish(&g, Value::Null);
                    }
                    leave(&mut self.frames, regs, Value::Null);
                    return Ok(Next::Switch);
                }
                Instr::Yield { dst, value, key, has_key } => {
                    let v = self.operand(w, func, value);
                    let k = if has_key { Some(self.operand(w, func, key)) } else { None };
                    return Ok(self.suspend(regs, v, k, pc, Some(dst)));
                }
                Instr::YieldFrom { dst, src } => {
                    match self.yield_from(regs, base, src, at)? {
                        Ok(next) => return Ok(next),
                        Err(result) => regs[base + dst as usize] = result,
                    }
                }
        }
        Ok(Next::Go(pc))
    }

    /// Push the frame of a method call: its arguments at `args`
    /// (absolute), its result to `ret` (NO_RET: discarded, as a
    /// constructor's is).
    fn call_method(&mut self, regs: &mut Vec<Value>, mut m: classes::MethodFrame, args: usize, argc: u8, ret: u32, pc: usize) -> Result<Next, Throw> {
        // __call / __callStatic: the arguments become (name, [arguments]).
        let argc = match m.trampoline.take() {
            Some(name) => {
                let packed: Vec<Value> = regs[args..args + argc as usize].iter().map(Value::deref).collect();
                if regs.len() < args + 2 {
                    regs.resize(args + 2, Value::Undef);
                }
                regs[args] = Value::Str(name);
                regs[args + 1] = Value::Array(Rc::new(PhpArray::from_list(packed)));
                2
            }
            None => argc,
        };
        let func = match m.func {
            classes::Code::User(func) => func,
            classes::Code::Native(native, name) => {
                let mut buf: Vec<Value> = regs[args..args + argc as usize].iter_mut().map(|r| std::mem::replace(r, Value::Undef)).collect();
                let saved = std::mem::replace(&mut self.native_class, m.called.clone());
                // Lent, as to a builtin: Closure::call runs PHP code.
                self.isolate.regs = std::mem::take(regs);
                let r = self.run_native(native, m.this.as_ref(), &mut buf);
                *regs = std::mem::take(&mut self.isolate.regs);
                self.native_class = saved;
                // An error the method raised: the trace shows its call on
                // top (not a hook's, `$name::get`: PHP has no frame there).
                if let (Err(Throw::Error { .. }), Some(scope)) = (&r, &m.scope)
                    && name.as_bytes().first() != Some(&b'$')
                {
                    self.failed_call(name.as_bytes(), Some((&scope.name, m.this.is_some())), &buf);
                }
                drop(buf);
                let v = r?;
                if ret != NO_RET {
                    regs[ret as usize] = v;
                }
                return Ok(Next::Go(pc));
            }
        };
        if (argc as usize) < func.code.required {
            let class = m.scope.as_ref().map(|c| (c.name.clone(), m.this.is_some()));
            let class = class.as_ref().map(|(n, this)| (n, *this));
            return Err(self.missing_args(&func.code, &regs[args..args + argc as usize], class));
        }
        if func.code.is_generator {
            let frame = classes::MethodFrame { func: classes::Code::User(Rc::clone(&func)), this: m.this, scope: m.scope, called: m.called, captured: m.captured, trampoline: None };
            let g = self.make_generator(regs, frame, Rc::clone(&func), args, argc as usize);
            if ret != NO_RET {
                regs[ret as usize] = Value::Object(g);
            }
            return Ok(Next::Go(pc));
        }
        if let Some(caller) = self.frames.last_mut() {
            caller.pc = pc as u32;
        }
        self.frames.push(&func, args, argc as usize, ret);
        let frame = self.frames.last_mut().expect("the frame just pushed");
        frame.this = m.this;
        frame.scope = m.scope;
        frame.called = m.called;
        enter(&mut self.frames, regs);
        // A closure's captured values go into their variables.
        if !m.captured.is_empty() {
            for (r, v) in func.code.bound.iter().zip(m.captured) {
                regs[args + *r as usize] = v;
            }
        }
        Ok(Next::Switch)
    }

    /// Run a file's code (include, eval) as a frame above the running one,
    /// with its variables, `$this` and class scope; its result into `dst`.
    fn run_in_scope(&mut self, regs: &mut Vec<Value>, base: usize, func: &Function, main: &Arc<Function>, dst: Reg, pc: usize) -> Next {
        // The file runs in the including code's scope: the globals when
        // that is global scope (however deep in includes), else its locals.
        let global = self.frames.last().is_some_and(|f| f.global);
        let mut cells: Vec<Rc<RefCell<Value>>> = Vec::with_capacity(main.cv_names.len());
        for name in &main.cv_names {
            cells.push(match func.cv_names.iter().position(|n| n == name) {
                Some(r) => make_ref(&mut regs[base..base + func.nregs as usize], r as Reg),
                // (The main frame's window starts at register 0.)
                None if global => self.global_cell(regs, name.as_bytes()),
                None => self.frames.last_mut().expect("the including frame").vars_mut().cell(name.as_bytes()),
            });
        }
        let (this, scope, called) = match self.frames.last() {
            Some(f) => (f.this.clone(), f.scope.clone(), f.called.clone()),
            None => (None, None, None),
        };
        if let Some(caller) = self.frames.last_mut() {
            caller.pc = pc as u32;
        }
        let at = base + func.nregs as usize;
        self.frames.push(&Closure::new(Arc::clone(main)), at, 0, (base + dst as usize) as u32);
        enter(&mut self.frames, regs);
        for (i, cell) in cells.into_iter().enumerate() {
            regs[at + i] = Value::Ref(cell);
        }
        let frame = self.frames.last_mut().expect("the file's frame");
        (frame.this, frame.scope, frame.called, frame.global) = (this, scope, called, global);
        Next::Switch
    }

    /// `[...$x]`: int keys renumbered, string keys kept (PHP 8.1).
    fn unpack_into(&mut self, w: &mut [Value], arr: Reg, items: Vec<(Value, Value)>) -> Result<(), Throw> {
        let Value::Array(a) = &mut w[arr as usize] else { return Ok(()) };
        let a = Rc::make_mut(a);
        for (k, v) in items {
            match k {
                Value::Str(s) => a.insert(Key::from_string(s), v),
                _ => {
                    if a.push(v).is_err() {
                        return Err(self.throw("Error", "Cannot add element to the array as the next element is already occupied"));
                    }
                }
            }
        }
        Ok(())
    }

    /// `++`/`--` on `target[k1]..[kn]`: (old, new). A missing element
    /// warns and counts as null; arrays are made along the way.
    fn incdec_path(&mut self, target: &mut Value, path: &[Key], inc: bool) -> Result<(Value, Value), Throw> {
        if matches!(target, Value::Null | Value::Undef) {
            *target = Value::Array(Rc::new(PhpArray::new()));
        }
        let arr = match target {
            Value::Array(a) => Rc::make_mut(a),
            Value::Ref(cell) => {
                let mut inner = std::mem::take(&mut *cell.borrow_mut());
                let r = self.incdec_path(&mut inner, path, inc);
                *cell.borrow_mut() = inner;
                return r;
            }
            Value::Str(_) => return Err(self.throw("Error", "Cannot increment/decrement string offsets")),
            _ => return Err(self.throw("Error", "Cannot use a scalar value as an array")),
        };
        let Some((key, rest)) = path.split_first() else { return Ok((Value::Null, Value::Null)) };
        if !rest.is_empty() {
            if !arr.contains_key(key) {
                arr.insert(key.clone(), Value::Null);
            }
            let mut child = std::mem::take(arr.get_mut(key).expect("present"));
            let r = self.incdec_path(&mut child, rest, inc);
            *arr.get_mut(key).expect("present") = child;
            return r;
        }
        let old = match arr.get(key) {
            Some(v) => v.deref(),
            None => {
                let shown = match key {
                    Key::Int(i) => i.to_string(),
                    Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
                };
                self.warn(format!("Undefined array key {shown}"));
                Value::Null
            }
        };
        let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
        self.report_diags();
        let new = r.map_err(|e| self.op_error(e))?;
        match arr.get_mut(key) {
            Some(Value::Ref(cell)) => *cell.borrow_mut() = new.clone(),
            _ => arr.insert(key.clone(), new.clone()),
        }
        Ok((old, new))
    }

    /// A resource object (numbered by the resources' own counter).
    fn make_resource(&mut self, resource: p8_runtime::stream::Resource) -> Rc<Object> {
        let class = self
            .resource_classes
            .entry(resource.kind)
            .or_insert_with(|| {
                Rc::new(Class {
                    name: PhpStr::from(resource.kind),
                    lower: resource.kind.as_bytes().to_vec(),
                    flags: p8_runtime::object::CLASS_RESOURCE | p8_runtime::object::CLASS_FINAL,
                    parent: None,
                    interfaces: Vec::new(),
                    properties: Vec::new(),
                    slots: HashMap::new(),
                    methods: Default::default(),
                    constants: p8_runtime::object::Constants::new(),
                    statics: HashMap::new(),
                    debug_info: None,
                    get_properties: None,
                    cast_bool: None,
                    cast_string: None,
                    clone_data: None,
                    forward_calls: None,
                    make_data: None,
                    native_dtor: None,
                    enum_cases: Vec::new(),
                    enum_backing: None,
                    static_inits: Vec::new(),
                })
            })
            .clone();
        Object::with_data(class, &self.resource_ids, Some(Rc::new(resource)))
    }

    /// STDIN, STDOUT, STDERR (resources 1 to 3; the next one is 5, as in the CLI).
    fn standard_streams(&mut self) {
        use p8_runtime::stream::{Resource, Stream, Target};
        for (name, target, path, mode) in [
            ("STDIN", Target::Stdin, "php://stdin", "r"),
            ("STDOUT", Target::Stdout, "php://stdout", "w"),
            ("STDERR", Target::Stderr, "php://stderr", "w"),
        ] {
            let r = Resource { kind: "stream", closed: std::cell::Cell::new(false), stream: RefCell::new(Stream::new(target, path, mode)) };
            let v = Value::Object(self.make_resource(r));
            self.constants.insert(name.as_bytes().to_vec(), v);
        }
        self.resource_ids.take();
    }

    /// Run a builtin class's method.
    pub(crate) fn run_native(&mut self, native: p8_runtime::object::Native, this: Option<&Rc<Object>>, args: &mut [Value]) -> Result<Value, Throw> {
        let r = match (native, this) {
            (p8_runtime::object::Native::Method(f), Some(this)) => f(self, this, args),
            (p8_runtime::object::Native::Static(f), _) => f(self, args),
            (p8_runtime::object::Native::Method(_), None) => Err(self.throw("Error", "Non-static method cannot be called statically")),
        };
        self.report_diags();
        r
    }

    /// Objects passed to `string` parameters become their __toString(), as
    /// zend_parse_arg_str does (when the registers are lent, so PHP code
    /// can run).
    fn stringify_args(&mut self, sig: &p8_runtime::stubs::Sig, args: &mut [Value]) -> Result<(), Throw> {
        use p8_runtime::stubs::{T_CALLABLE, T_MIXED, T_OBJECT, T_STRING};
        if self.isolate.regs.is_empty() {
            return Ok(());
        }
        for (i, arg) in args.iter_mut().enumerate() {
            let Some(param) = sig.params.get(i).or_else(|| sig.params.last().filter(|p| p.variadic)) else { break };
            if param.by_ref || param.mask & T_STRING == 0 || param.mask & (T_OBJECT | T_MIXED | T_CALLABLE) != 0 {
                continue;
            }
            let Value::Object(o) = &*arg else { continue };
            if param.classes.iter().any(|c| o.class.is_a(c.to_ascii_lowercase().as_bytes())) || o.class.method(b"__tostring").is_none() {
                continue;
            }
            let method = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(Rc::clone(o)), Value::Str(PhpStr::from("__toString"))])));
            *arg = self.call_from_builtin(&method, &mut [])?;
        }
        Ok(())
    }

    /// An object as a string, when its class's __toString is native (a
    /// Throwable's); anything else as it is.
    /// A value in a string context (echo, `.`, interpolation, a string
    /// cast): an object becomes what its __toString() returns, run here
    /// as PHP code when it is; one without __toString stays for the caller
    /// to report.
    fn string_of(&mut self, regs: &mut Vec<Value>, v: Value) -> Result<Value, Throw> {
        let v = match v {
            Value::Ref(cell) => cell.borrow().clone(),
            v => v,
        };
        let Value::Object(o) = &v else { return Ok(v) };
        if o.class.flags & p8_runtime::object::CLASS_RESOURCE != 0 || o.class.cast_string.is_some() {
            return Ok(v);
        }
        match o.class.method(b"__tostring") {
            Some(m) if m.native.is_some() => self.object_to_string(v),
            Some(_) => {
                let callable = Value::Array(Rc::new(PhpArray::from_list(vec![v.clone(), Value::Str(PhpStr::from("__toString"))])));
                self.call_with_regs(regs, &callable, &mut [])
            }
            None => Ok(v),
        }
    }

    fn object_to_string(&mut self, v: Value) -> Result<Value, Throw> {
        let Value::Object(o) = &v else { return Ok(v) };
        let Some(p8_runtime::object::Native::Method(native)) = o.class.method(b"__tostring").and_then(|m| m.native) else {
            return Ok(v);
        };
        let o = Rc::clone(o);
        native(self, &o, &mut [])
    }

    /// Call builtin `i` on the `argc` arguments at `args` of the frame at
    /// `base`, its result into `dst`. The arguments move into a buffer
    /// and the register file is lent to the context while the builtin
    /// runs, so it can call back into PHP (array_map, usort).
    #[inline]
    fn call_builtin(&mut self, regs: &mut Vec<Value>, base: usize, i: usize, args: Reg, argc: u8, dst: Reg) -> Result<(), Throw> {
        let start = base + args as usize;
        // An object argument may need its __toString() (PHP code) to become
        // a string parameter: that takes the path that can call back.
        let objects = regs[start..start + argc as usize].iter().any(|v| matches!(v, Value::Object(_)));
        if !self.isolate.registry.reenters(i) && !objects {
            // The common case: the builtin works on the registers directly.
            let args = &mut regs[start..start + argc as usize];
            let r = self.run_builtin(i, args);
            if let Err(Throw::Autoload(_)) = r {
                // The call runs again after the autoloaders: its arguments stay.
                return r.map(|_| ());
            }
            for v in args {
                clear(v);
            }
            regs[base + dst as usize] = r?;
            return Ok(());
        }
        let mut buf = std::mem::take(&mut self.arg_buf);
        buf.extend(regs[start..start + argc as usize].iter_mut().map(|r| std::mem::replace(r, Value::Undef)));
        self.isolate.regs = std::mem::take(regs);
        // Running: frames it calls show it in their traces.
        let name = self.isolate.registry.function(i).name;
        self.natives.push(NativeCall { depth: self.frames.len(), name, args: buf.iter().map(Value::deref).collect(), top: false });
        let r = self.run_builtin(i, &mut buf);
        self.natives.pop();
        *regs = std::mem::take(&mut self.isolate.regs);
        if let Err(Throw::Autoload(_)) = r {
            for (slot, v) in regs[start..start + argc as usize].iter_mut().zip(buf.drain(..)) {
                *slot = v;
            }
        }
        buf.clear();
        self.arg_buf = buf;
        regs[base + dst as usize] = r?;
        Ok(())
    }

    /// Run builtin `i` on `args`: PHP's argument checks and conversions
    /// (from the stub signature), then the function.
    #[inline(always)]
    fn run_builtin(&mut self, i: usize, args: &mut [Value]) -> Result<Value, Throw> {
        if let Some(message) = self.isolate.registry.deprecation(i) {
            self.diags.push(Diag::Deprecated(message.to_string()));
        }
        if let Some(sig) = self.isolate.registry.sig(i) {
            self.stringify_args(sig, args)?;
            let checked = p8_runtime::args::check(sig, args, &mut self.diags);
            self.report_diags();
            if let Err((class, message)) = checked {
                let name = self.isolate.registry.function(i).name;
                if !opcode_function(name, args.len()) {
                    self.failed_call(name.as_bytes(), None, args);
                }
                return Err(self.throw(class, message));
            }
        }
        let r = (self.isolate.registry.function(i).run)(self, args);
        if let Err(Throw::Error { .. }) = r {
            self.failed_call(self.isolate.registry.function(i).name.as_bytes(), None, args);
        }
        self.report_diags();
        r
    }

    /// Whether `a cmp b` holds for two registers (`<=>`: whether it is
    /// nonzero), with fast paths for numbers.
    #[inline(always)]
    fn test_regs(&mut self, w: &[Value], func: &Function, cmp: Cmp, a: Reg, b: Reg) -> bool {
        let fast = match (&w[a as usize], &w[b as usize]) {
            (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
            (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
            // === between an int and a float is false: no fast path there
            (Value::Int(x), Value::Float(y)) if !matches!(cmp, Cmp::Identical | Cmp::NotIdentical) => {
                (*x as f64).partial_cmp(y)
            }
            (Value::Float(x), Value::Int(y)) if !matches!(cmp, Cmp::Identical | Cmp::NotIdentical) => {
                x.partial_cmp(&(*y as f64))
            }
            _ => None,
        };
        match fast {
            Some(o) => match cmp {
                Cmp::Lt => o.is_lt(),
                Cmp::Le => o.is_le(),
                Cmp::Gt => o.is_gt(),
                Cmp::Ge => o.is_ge(),
                Cmp::Eq | Cmp::Identical => o.is_eq(),
                Cmp::NotEq | Cmp::NotIdentical | Cmp::Spaceship => o.is_ne(),
            },
            None => self.test_slow(w, func, cmp, a, b),
        }
    }

    #[cold]
    #[inline(never)]
    fn test_slow(&mut self, w: &[Value], func: &Function, cmp: Cmp, a: Reg, b: Reg) -> bool {
        let x = self.operand(w, func, a);
        let y = self.operand(w, func, b);
        match compare(cmp, &x, &y) {
            Value::Bool(t) => t,
            v => v.to_bool(),
        }
    }

    /// ArgumentCountError for a call passing fewer than `func.required`
    /// arguments: reported at the declaration, naming the call's line.
    #[cold]
    fn missing_args(&mut self, func: &Function, args: &[Value], class: Option<(&PhpStr, bool)>) -> Throw {
        let passed = args.len();
        self.failed_call(func.name.as_bytes(), class, args);
        let (file, line) = self.location();
        let declared = func.params.iter().filter(|p| !p.variadic).count();
        let exactly = if func.required == declared { "exactly" } else { "at least" };
        let name = match &func.class {
            Some(class) => format!("{}::{}", String::from_utf8_lossy(class.as_bytes()), String::from_utf8_lossy(func.name.as_bytes())),
            None => String::from_utf8_lossy(func.name.as_bytes()).into_owned(),
        };
        Throw::Error {
            class: "ArgumentCountError",
            message: format!(
                "Too few arguments to function {name}(), {passed} passed in {file} on line {line} and {exactly} {} expected",
                func.required
            ),
            file: Rc::from(&*func.file),
            line: func.line,
            props: Vec::new(),
        }
    }

    /// The cell of global `$name`: the main frame's variable if it has one.
    /// `main` is the main frame's window (it starts at register 0).
    /// The name in register `r`, for `$$name`.
    fn var_name(&mut self, w: &[Value], func: &Function, r: Reg) -> Vec<u8> {
        let v = self.operand(w, func, r);
        ops::to_string_lossy(&v, &mut self.diags).as_bytes().to_vec()
    }

    /// The running frame's variable `name`, made a reference: a compiled
    /// variable, a main-script global, or one only named at run time.
    fn var_cell(&mut self, w: &mut [Value], in_main: bool, name: &[u8]) -> Rc<RefCell<Value>> {
        if in_main {
            return self.global_cell(w, name);
        }
        let frame = self.frames.last_mut().expect("a running frame");
        if let Some(r) = frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            return make_ref(w, r as Reg);
        }
        frame.vars_mut().cell(name)
    }

    /// The running frame's variable `name`, if set.
    fn var_value(&self, w: &[Value], in_main: bool, name: &[u8]) -> Option<Value> {
        let frame = self.frames.last()?;
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => match w[r].deref() {
                Value::Undef => None,
                v => Some(v),
            },
            None if in_main => self.globals.get(name).map(|c| c.borrow().clone()).filter(|v| !matches!(v, Value::Undef)),
            None => frame.vars()?.get(name).map(|c| c.borrow().clone()),
        }
    }

    fn var_unset(&mut self, w: &mut [Value], in_main: bool, name: &[u8]) {
        let Some(frame) = self.frames.last_mut() else { return };
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => w[r] = Value::Undef,
            None if in_main => {
                self.globals.shift_remove(name);
            }
            None => {
                if let Some(e) = &mut frame.extra {
                    e.vars.remove(name);
                }
            }
        }
    }

    /// The global variable `name`'s value, if set (`main`: the main frame's window).
    fn global_value(&self, main: &[Value], name: &[u8]) -> Option<Value> {
        let code = &self.frames.first()?.func.code;
        match code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => match main[r].deref() {
                Value::Undef => None,
                v => Some(v),
            },
            None => self.globals.get(name).map(|c| c.borrow().deref()).filter(|v| !matches!(v, Value::Undef)),
        }
    }

    /// The global variable `name` becomes `cell` (a reference binding).
    fn global_bind(&mut self, main: &mut [Value], name: &[u8], cell: Rc<RefCell<Value>>) {
        let Some(frame) = self.frames.first() else { return };
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => main[r] = Value::Ref(cell),
            None => {
                self.globals.insert(name.to_vec(), cell);
            }
        }
    }

    fn global_unset(&mut self, main: &mut [Value], name: &[u8]) {
        let Some(frame) = self.frames.first() else { return };
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => main[r] = Value::Undef,
            None => {
                self.globals.shift_remove(name);
            }
        }
    }

    /// `unset($x)` in a file included at global scope: the global's slot is
    /// emptied, not removed, so every file bound to it (and a later
    /// assignment through any of them) still shares it, as PHP's files share
    /// symbol-table slots.
    fn global_clear(&mut self, main: &mut [Value], name: &[u8]) {
        let Some(frame) = self.frames.first() else { return };
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => match &main[r] {
                Value::Ref(cell) => *cell.borrow_mut() = Value::Undef,
                _ => main[r] = Value::Undef,
            },
            None => {
                if let Some(cell) = self.globals.get(name) {
                    *cell.borrow_mut() = Value::Undef;
                }
            }
        }
    }

    /// `$GLOBALS` read whole: the set global variables, by name, in the
    /// order get_defined_vars() gives them: the ones the request starts
    /// with, the main script's, then the others as they were made.
    fn globals_array(&self, main: &[Value]) -> p8_runtime::array::PhpArray {
        let mut out = p8_runtime::array::PhpArray::new();
        for n in REQUEST_GLOBALS {
            if let Some(c) = self.globals.get(n.as_bytes()) {
                match c.borrow().deref() {
                    Value::Undef => {}
                    v => out.insert(Key::from_string(PhpStr::from(*n)), v),
                }
            }
        }
        if let Some(frame) = self.frames.first() {
            for (r, n) in frame.func.code.cv_names.iter().enumerate() {
                if n.as_bytes() == b"GLOBALS" {
                    continue;
                }
                match main.get(r).map(Value::deref) {
                    None | Some(Value::Undef) => {}
                    Some(v) => out.insert(Key::from_string(PhpStr::new(n.as_bytes().to_vec())), v),
                }
            }
        }
        for (n, c) in &self.globals {
            match c.borrow().deref() {
                Value::Undef => {}
                v => out.insert(Key::from_string(PhpStr::new(n.clone())), v),
            }
        }
        out
    }

    fn global_cell(&mut self, main: &mut [Value], name: &[u8]) -> Rc<RefCell<Value>> {
        if let Some(r) = self.frames.first().expect("the main frame").func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            return make_ref(main, r as Reg);
        }
        self.globals.entry(name.to_vec()).or_insert_with(|| Rc::new(RefCell::new(Value::Null))).clone()
    }

    /// An array key from register `r`, reporting what PHP reports.
    fn key(&mut self, w: &[Value], func: &Function, r: Reg) -> Result<Key, Throw> {
        let v = self.operand(w, func, r);
        match Key::from_value(&v) {
            Ok((k, None)) => Ok(k),
            Ok((k, Some(KeyIssue::LosesPrecision(f)))) => {
                let shown = float_to_string(f, Precision::Shortest, 'E');
                self.diags.push(Diag::Deprecated(format!("Implicit conversion from float {shown} to int loses precision")));
                self.report_diags();
                Ok(k)
            }
            // from_value reports illegal keys as errors only
            Ok((k, Some(KeyIssue::Illegal(_)))) => Ok(k),
            Err(KeyIssue::Illegal(t)) => Err(self.throw("TypeError", format!("Cannot access offset of type {t} on array"))),
            Err(KeyIssue::LosesPrecision(f)) => Ok(Key::Int(f as i64)),
        }
    }

    fn fetch_dim(&mut self, container: &Value, w: &[Value], func: &Function, key: Reg, quiet: bool) -> Result<Value, Throw> {
        match container {
            Value::Array(a) => {
                let k = self.key(w, func, key)?;
                match a.get(&k) {
                    Some(v) => Ok(v.deref()),
                    None => {
                        if !quiet {
                            let shown = match &k {
                                Key::Int(i) => i.to_string(),
                                Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
                            };
                            self.warn(format!("Undefined array key {shown}"));
                        }
                        Ok(Value::Null)
                    }
                }
            }
            Value::Str(s) => {
                let k = self.operand(w, func, key);
                let i = k.to_int();
                let len = s.len() as i64;
                let at = if i < 0 { len + i } else { i };
                if at < 0 || at >= len {
                    if !quiet {
                        self.warn(format!("Uninitialized string offset {i}"));
                    }
                    return Ok(if quiet { Value::Null } else { Value::Str(PhpStr::empty()) });
                }
                Ok(Value::Str(PhpStr::new(vec![s.as_bytes()[at as usize]])))
            }
            Value::Null | Value::Undef => {
                if !quiet {
                    self.warn("Trying to access array offset on null");
                }
                Ok(Value::Null)
            }
            other => {
                if !quiet {
                    self.warn(format!("Trying to access array offset on {}", other.type_name()));
                }
                Ok(Value::Null)
            }
        }
    }

    /// var[k1]..[kn] = val (or op= val), copy-on-write down the path.
    // The operands of AssignDim/AssignDimOp, decoded once by the caller.
    #[allow(clippy::too_many_arguments)]
    fn assign_dim(
        &mut self,
        w: &mut [Value],
        func: &Function,
        var: Reg,
        keys: Reg,
        n: u8,
        append: bool,
        val: Value,
        op: Option<BinOp>,
    ) -> Result<Value, Throw> {
        let mut path = Vec::with_capacity(n as usize);
        for i in 0..n {
            path.push(self.key(w, func, keys + i as Reg)?);
        }
        // Take the variable's value out, update it, put it back.
        let slot = var as usize;
        let mut root = match &mut w[slot] {
            Value::Ref(cell) => std::mem::take(&mut *cell.borrow_mut()),
            other => std::mem::take(other),
        };
        let result = self.update_path(&mut root, &path, append, val, op);
        match &mut w[slot] {
            Value::Ref(cell) => *cell.borrow_mut() = root,
            other => *other = root,
        }
        result
    }

    fn update_path(&mut self, target: &mut Value, path: &[Key], append: bool, val: Value, op: Option<BinOp>) -> Result<Value, Throw> {
        if matches!(target, Value::Null | Value::Undef) {
            *target = Value::Array(Rc::new(PhpArray::new()));
        }
        let arr = match target {
            Value::Array(a) => Rc::make_mut(a),
            Value::Ref(cell) => {
                let mut inner = std::mem::take(&mut *cell.borrow_mut());
                let r = self.update_path(&mut inner, path, append, val, op);
                *cell.borrow_mut() = inner;
                return r;
            }
            Value::Bool(false) => {
                self.diags.push(Diag::Deprecated("Automatic conversion of false to array is deprecated".into()));
                self.report_diags();
                *target = Value::Array(Rc::new(PhpArray::new()));
                return self.update_path(target, path, append, val, op);
            }
            Value::Str(_) => return Err(self.throw("Error", "String offsets are not supported by this engine yet")),
            _ => return Err(self.throw("Error", "Cannot use a scalar value as an array")),
        };
        match path.split_first() {
            None => {
                // `$a[] = v`
                if arr.push(val.clone()).is_err() {
                    return Err(self.throw("Error", "Cannot add element to the array as the next element is already occupied"));
                }
                Ok(val)
            }
            Some((key, rest)) if rest.is_empty() && !append => match op {
                None => {
                    match arr.get_mut(key) {
                        Some(Value::Ref(cell)) => *cell.borrow_mut() = val.clone(),
                        _ => arr.insert(key.clone(), val.clone()),
                    }
                    Ok(val)
                }
                Some(op) => {
                    let old = match arr.get(key) {
                        Some(v) => v.deref(),
                        None => {
                            let shown = match key {
                                Key::Int(i) => i.to_string(),
                                Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
                            };
                            self.warn(format!("Undefined array key {shown}"));
                            Value::Null
                        }
                    };
                    let r = ops::binary(op, &old, &val, &mut self.diags);
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    arr.insert(key.clone(), new.clone());
                    Ok(new)
                }
            },
            Some((key, rest)) => {
                if !arr.contains_key(key) {
                    arr.insert(key.clone(), Value::Null);
                }
                let mut child = std::mem::take(arr.get_mut(key).expect("just inserted"));
                let r = self.update_path(&mut child, rest, append, val, op);
                *arr.get_mut(key).expect("present") = child;
                r
            }
        }
    }

    /// Arguments past the parameters, for func_get_args().
    fn extra_args(&self) -> &[Value] {
        self.frames.last().map_or(&[], |f| f.args())
    }
}


impl Context<'_> {
    /// The constant `name`: the script's own, else an extension's.
    fn constant_value(&self, name: &[u8]) -> Option<Value> {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        let key = const_key(name);
        match self.constants.get(key.as_ref()) {
            Some(v) => Some(v.clone()),
            None => self.isolate.registry.constant(name).map(|c| c.to_value()),
        }
    }
}

impl Ctx for Context<'_> {
    fn echo(&mut self, bytes: &[u8]) {
        self.write_out(bytes);
    }

    fn ob(&mut self, op: p8_runtime::ext::Ob) -> Result<Value, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = self.ob_with(&mut regs, op);
        self.isolate.regs = regs;
        r
    }

    fn warn(&mut self, message: &str) {
        Context::warn(self, message);
    }

    fn deprecated(&mut self, message: &str) {
        self.diags.push(Diag::Deprecated(message.to_string()));
        self.report_diags();
    }

    fn notice(&mut self, message: &str) {
        Context::notice(self, message);
    }

    fn report(&mut self, level: i64, message: &str) {
        self.report_diags();
        self.show(level, message);
    }

    fn fatal(&self, message: String) -> Throw {
        let (file, line) = self.location();
        Throw::Fatal { message, file, line }
    }

    fn ini_get(&self, name: &[u8]) -> Option<Value> {
        let default = self.isolate.registry.ini_default(name)?;
        Some(match self.ini.get(name) {
            Some(v) => v.clone(),
            None => default.map_or(Value::Null, |d| Value::Str(PhpStr::from(d))),
        })
    }

    fn ini_set(&mut self, name: &[u8], value: Value) -> Option<Value> {
        let old = self.ini_get(name)?;
        let text = value.to_php_string().unwrap_or_else(PhpStr::empty);
        match name {
            // An expression of E_* constants (`E_ALL & ~E_NOTICE`).
            b"error_reporting" => self.error_level = self.ini_bits(&String::from_utf8_lossy(text.as_bytes())),
            b"display_errors" => {
                let t = text.as_bytes().to_ascii_lowercase();
                self.display_errors = !matches!(t.as_slice(), b"" | b"0" | b"off" | b"no" | b"false" | b"stderr");
            }
            _ => {}
        }
        self.ini.insert(name.to_vec(), Value::Str(text));
        Some(old)
    }

    fn error_level(&mut self, new: Option<i64>) -> i64 {
        let old = self.error_level;
        if let Some(level) = new {
            self.error_level = level;
            self.ini.insert(b"error_reporting".to_vec(), Value::Str(PhpStr::from(level.to_string())));
        }
        old
    }

    fn constant(&self, name: &[u8]) -> Option<Value> {
        self.constant_value(name)
    }

    fn define(&mut self, name: &[u8], value: Value) -> bool {
        if self.constant_value(name).is_some() {
            return false;
        }
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        self.constants.insert(const_key(name).into_owned(), value);
        true
    }

    fn builtin_name(&self) -> &'static str {
        // The innermost builtin call (as stack traces record it).
        self.natives.last().map_or("", |n| n.name)
    }

    fn modules(&self) -> &[&'static str] {
        self.isolate.registry.modules()
    }

    fn function_exists(&self, name: &[u8]) -> bool {
        Context::function_exists(self, name)
    }

    fn extra_args(&self) -> &[Value] {
        Context::extra_args(self)
    }

    fn throw(&self, class: &'static str, message: String) -> Throw {
        Context::throw(self, class, message)
    }

    fn class(&mut self, name: &[u8]) -> Option<Rc<Class>> {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        let lower = name.to_ascii_lowercase();
        match self.classes.get(&lower) {
            Some(c) => Some(Rc::clone(c)),
            None => self.builtin_on_demand(&lower),
        }
    }

    fn scope(&self) -> Option<Rc<Class>> {
        Context::scope(self).0
    }

    fn call(&mut self, callable: &Value, args: &mut [Value]) -> Result<Value, Throw> {
        if self.isolate.regs.is_empty() {
            return Err(Ctx::fatal(self, "a builtin that takes no callable called back into PHP".into()));
        }
        self.call_from_builtin(callable, args)
    }

    fn is_callable(&mut self, v: &Value) -> bool {
        self.callable(v)
    }

    fn output_started(&self) -> Option<(Rc<str>, u32)> {
        self.output_start.clone()
    }

    fn last_error(&mut self, clear: bool) -> Option<(i64, String, String, u32)> {
        if clear { self.last_error.take() } else { self.last_error.clone() }
    }

    fn bind_closure(&mut self, closure: &Rc<Object>, new_this: Value, scope: Option<Value>) -> Result<Value, Throw> {
        self.bind(closure, new_this, scope)
    }

    fn closure_from(&mut self, callable: &Value) -> Result<Value, Throw> {
        self.closure_from_callable(callable)
    }

    fn prop_value(&mut self, o: &Rc<Object>, slot: usize) -> Result<Option<Value>, Throw> {
        let p = &o.class.properties[slot];
        let raw = || match o.slots.borrow()[slot].deref() {
            Value::Undef => None,
            v => Some(v),
        };
        if p.hooks == 0 || self.isolate.regs.is_empty() {
            return Ok(raw());
        }
        let name = Value::Str(p.name.clone());
        match self.prop_hook_seen(&Value::Object(Rc::clone(o)), &name, HOOK_GET, false) {
            Some(Ok(m)) => {
                let mut regs = std::mem::take(&mut self.isolate.regs);
                let r = self.run_nested(&mut regs, m, &mut []);
                self.isolate.regs = regs;
                r.map(Some)
            }
            // Virtual and write-only: nothing to read.
            Some(Err(_)) => Ok(None),
            None => Ok(raw()),
        }
    }

    fn function_info(&mut self, callee: &p8_runtime::reflect::Callee) -> Option<p8_runtime::reflect::FnInfo> {
        self.info_of(callee)
    }

    fn invoke(&mut self, callee: &p8_runtime::reflect::Callee, this: Option<Rc<Object>>, args: &mut [Value]) -> Result<Value, Throw> {
        self.invoke_callee(callee, this, args)
    }

    fn new_instance(&mut self, class: &Rc<Class>, args: &mut [Value], construct: bool) -> Result<Rc<Object>, Throw> {
        self.new_instance_of(class, args, construct)
    }

    fn class_defaults(&mut self, class: &Rc<Class>) -> Result<Vec<Value>, Throw> {
        self.class_defaults_of(class)
    }

    fn closure_of(&mut self, callee: &p8_runtime::reflect::Callee, this: Option<Rc<Object>>) -> Result<Value, Throw> {
        self.closure_of_callee(callee, this)
    }

    fn class_load(&mut self, name: &[u8]) -> Result<Option<Rc<Class>>, Throw> {
        self.class_if_loaded(name)
    }

    fn called_class(&self) -> Option<Rc<Class>> {
        Context::scope(self).1
    }

    fn new_object(&mut self, class: &Rc<Class>) -> Rc<Object> {
        Object::new(Rc::clone(class), &self.ids)
    }

    fn call_args(&self) -> Vec<Value> {
        let Some(frame) = self.frames.last() else { return Vec::new() };
        let regs = &self.isolate.regs;
        let base = frame.base as usize;
        let n = (frame.argc as usize).min(frame.func.code.params.len());
        let mut out: Vec<Value> = (0..n).map(|i| regs.get(base + i).map(Value::deref).unwrap_or(Value::Null)).collect();
        out.extend(frame.args().iter().map(Value::deref));
        out
    }

    fn local(&self, name: &[u8]) -> Option<Value> {
        let frame = self.frames.last()?;
        let regs = &self.isolate.regs;
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => match regs.get(frame.base as usize + r)?.deref() {
                Value::Undef => None,
                v => Some(v),
            },
            None if frame.global => self.global_value(regs, name),
            None => frame.vars()?.get(name).map(|c| c.borrow().clone()),
        }
    }

    fn set_local(&mut self, name: &[u8], value: Value) -> bool {
        let Some(frame) = self.frames.last() else { return false };
        let base = frame.base as usize;
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => {
                if let Some(slot) = self.isolate.regs.get_mut(base + r) {
                    assign(std::slice::from_mut(slot), 0, value);
                }
                true
            }
            None if frame.global => {
                // (The main frame's window starts at register 0.)
                let mut regs = std::mem::take(&mut self.isolate.regs);
                self.global_cell(&mut regs, name).replace(value);
                self.isolate.regs = regs;
                true
            }
            None => {
                let frame = self.frames.last_mut().expect("the frame");
                frame.vars_mut().cell(name).replace(value);
                true
            }
        }
    }

    fn locals(&self) -> Vec<(PhpStr, Value)> {
        let Some(frame) = self.frames.last() else { return Vec::new() };
        // At global scope (a file included there too): the main frame's
        // variables and the other globals.
        let frame = if frame.global { self.frames.first().expect("the main frame") } else { frame };
        let regs = &self.isolate.regs;
        let base = frame.base as usize;
        let mut out: Vec<(PhpStr, Value)> = frame
            .func
            .code
            .cv_names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| match regs.get(base + i).map(Value::deref) {
                Some(Value::Undef) | None => None,
                Some(v) => Some((PhpStr::from(n.as_bytes()), v)),
            })
            .collect();
        if base == 0 {
            // The ones the request starts with come first, in the order
            // PHP makes them; then the others.
            let first: Vec<(PhpStr, Value)> = REQUEST_GLOBALS
                .iter()
                .filter_map(|n| self.globals.get(n.as_bytes()).map(|c| (PhpStr::from(*n), c.borrow().clone())))
                .filter(|(_, v)| !matches!(v, Value::Undef))
                .collect();
            out.splice(0..0, first);
            for (n, c) in &self.globals {
                if !REQUEST_GLOBALS.iter().any(|r| r.as_bytes() == n.as_slice()) && !matches!(&*c.borrow(), Value::Undef) {
                    out.push((PhpStr::from(n.as_slice()), c.borrow().clone()));
                }
            }
        } else if let Some(vars) = frame.vars() {
            for (n, c) in &vars.0 {
                out.push((PhpStr::from(n.as_slice()), c.borrow().clone()));
            }
        }
        out
    }

    fn backtrace(&self) -> PhpArray {
        let regs = &self.isolate.regs;
        self.trace(&|i| regs.get(i).cloned().unwrap_or_default())
    }

    fn included_files(&self) -> Vec<String> {
        self.included_order.clone()
    }

    fn state(&mut self, key: &'static str) -> &mut Value {
        self.ext_state.entry(key).or_insert(Value::Null)
    }

    fn new_resource(&mut self, resource: p8_runtime::stream::Resource) -> Value {
        Value::Object(self.make_resource(resource))
    }

    fn write_stdout(&mut self, bytes: &[u8]) {
        self.out.host.write(bytes);
    }

    fn native_class(&self) -> Option<Rc<Class>> {
        self.native_class.clone()
    }

    fn enum_case(&mut self, class: &Rc<Class>, case: &[u8]) -> Value {
        Value::Object(Context::enum_case(self, class, case))
    }

    fn error_handler(&mut self, handler: Option<(Value, i64)>) -> Value {
        let old = self.error_handlers.last().map(|(h, _)| h.clone()).unwrap_or(Value::Null);
        match handler {
            Some(h) => self.error_handlers.push(h),
            None => {
                self.error_handlers.pop();
            }
        }
        old
    }

    fn exception_handler(&mut self, handler: Option<Value>) -> Value {
        let old = self.exception_handlers.last().cloned().unwrap_or(Value::Null);
        match handler {
            Some(h) => self.exception_handlers.push(h),
            None => {
                self.exception_handlers.pop();
            }
        }
        old
    }

    fn autoloader(&mut self, callable: Value, register: Option<bool>) -> bool {
        let at = self.autoloaders.iter().position(|a| a.identical(&callable) || a.loose_eq(&callable));
        match (register, at) {
            (Some(_), Some(_)) => true,
            (Some(true), None) => {
                self.autoloaders.insert(0, callable);
                true
            }
            (Some(false), None) => {
                self.autoloaders.push(callable);
                true
            }
            (None, Some(i)) => {
                self.autoloaders.remove(i);
                true
            }
            (None, None) => false,
        }
    }

    fn autoloaders(&self) -> Vec<Value> {
        self.autoloaders.clone()
    }

    fn on_shutdown(&mut self, callable: Value, args: Vec<Value>) {
        self.shutdown.push((callable, args));
    }

    fn class_alias(&mut self, class: Rc<Class>, alias: &[u8]) -> bool {
        let lower = alias.strip_prefix(b"\\").unwrap_or(alias).to_ascii_lowercase();
        if self.classes.contains_key(&lower) {
            return false;
        }
        self.classes.insert(lower, class);
        true
    }

    fn iterate(&mut self, v: &Value) -> Result<Vec<(Value, Value)>, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        if regs.is_empty() {
            return Err(Ctx::fatal(self, "an iteration with no registers lent".into()));
        }
        let r = self.iterate_all(&mut regs, v);
        self.isolate.regs = regs;
        r
    }

    fn fiber_op(&mut self, fiber: Option<&Rc<Object>>, op: u8, args: &mut [Value]) -> Result<Value, Throw> {
        Context::fiber_op(self, fiber, op, args)
    }

    fn gc_collect(&mut self) -> Result<i64, Throw> {
        let garbage = p8_runtime::gc::find_cycles(&self.ids);
        if garbage.objects.is_empty() {
            return Ok(0);
        }
        // Destructors first, oldest first; one may keep an object alive
        // (store it somewhere), so the garbage is found again after.
        let mut ran = false;
        // A destructor's exception doesn't stop the others: each new one
        // takes the ones before as its previous.
        let mut thrown: Option<Rc<Object>> = None;
        for o in &garbage.objects {
            if o.destructed.get() || o.class.method(b"__destruct").is_none() || o.lazy.get() != 0 {
                continue;
            }
            o.destructed.set(true);
            ran = true;
            let m = self.method_frame(&Value::Object(Rc::clone(o)), b"__destruct", b"__destruct")?;
            let r = if self.fibers.is_empty() {
                let mut regs = std::mem::take(&mut self.isolate.regs);
                let r = self.run_nested(&mut regs, m, &mut []).map(|_| ());
                self.isolate.regs = regs;
                r
            } else {
                self.destruct_in_fiber(m)
            };
            match r {
                Ok(()) => {}
                Err(Throw::Object(e)) if e.class.is_a(b"throwable") => {
                    if let Some(before) = thrown.take() {
                        builtin_classes::set_previous(&e, before);
                    }
                    thrown = Some(e);
                }
                Err(t) => return Err(t),
            }
        }
        let garbage = if ran {
            drop(garbage);
            // PHP counts the pass after destructors as a run of its own.
            self.gc_runs += 1;
            p8_runtime::gc::find_cycles(&self.ids)
        } else {
            garbage
        };
        for o in &garbage.objects {
            o.destructed.set(true);
            p8_runtime::gc::clear(o);
        }
        let count = garbage.count as i64;
        drop(garbage);
        self.gc_runs += 1;
        self.gc_collected += count;
        match thrown {
            Some(e) => Err(Throw::Object(e)),
            None => Ok(count),
        }
    }

    fn gc_counts(&self) -> (i64, i64) {
        (self.gc_runs, self.gc_collected)
    }

    fn cfg(&self) -> Vec<(Vec<u8>, String)> {
        self.cfg.clone()
    }

    fn parse_check(&self, source: &[u8]) -> Option<(String, u32)> {
        let loader = self.loader.as_ref()?;
        match loader.load(source, "") {
            Err(e) if e.parse => Some((e.message, e.line)),
            _ => None,
        }
    }

    fn request_argv(&self) -> Option<Vec<String>> {
        let off = matches!(Ctx::ini_get(self, b"register_argc_argv"), Some(Value::Str(s)) if matches!(s.as_bytes().to_ascii_lowercase().as_slice(), b"" | b"0" | b"off" | b"no" | b"false"));
        if off { None } else { self.argv.clone() }
    }

    fn global_value(&self, name: &[u8]) -> Option<Value> {
        let main = self.frames.first()?;
        match main.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => self.isolate.regs.get(r).map(Value::deref).filter(|v| !matches!(v, Value::Undef)),
            None => self.globals.get(name).map(|c| c.borrow().deref()).filter(|v| !matches!(v, Value::Undef)),
        }
    }

    fn lazy_make(&mut self, class: &Rc<Class>, obj: Option<Rc<Object>>, initializer: Value, options: i64, proxy: bool) -> Result<Rc<Object>, Throw> {
        self.make_lazy(class, obj, initializer, options, proxy)
    }

    fn lazy_initialize(&mut self, o: &Rc<Object>) -> Result<Rc<Object>, Throw> {
        self.lazy_init_from_builtin(o)
    }

    fn lazy_mark_initialized(&mut self, o: &Rc<Object>) -> Result<(), Throw> {
        Context::lazy_mark_initialized(self, o)
    }

    fn lazy_slot_done(&mut self, o: &Rc<Object>, slot: usize) {
        Context::lazy_slot_done(self, o, slot)
    }

    fn generator(&mut self, generator: &Rc<Object>, op: &str, arg: Value) -> Result<Value, Throw> {
        use generators::Op;
        let op = match op {
            "current" => Op::Current,
            "key" => Op::Key,
            "next" => Op::Next,
            "send" => Op::Send,
            "valid" => Op::Valid,
            "rewind" => Op::Rewind,
            "getreturn" => Op::GetReturn,
            _ => Op::Throw,
        };
        let mut regs = std::mem::take(&mut self.isolate.regs);
        if regs.is_empty() {
            return Err(Ctx::fatal(self, "a generator resumed with no registers lent".into()));
        }
        let r = self.generator_op(&mut regs, generator, op, arg);
        self.isolate.regs = regs;
        r
    }
}

/// `x cmp y` on two ints (`<=>`: whether it is nonzero).
#[inline(always)]
fn int_test(cmp: Cmp, x: i64, y: i64) -> bool {
    match cmp {
        Cmp::Lt => x < y,
        Cmp::Le => x <= y,
        Cmp::Gt => x > y,
        Cmp::Ge => x >= y,
        Cmp::Eq | Cmp::Identical => x == y,
        Cmp::NotEq | Cmp::NotIdentical | Cmp::Spaceship => x != y,
    }
}

/// Write `value` into variable register `r` of window `w` (through a reference).
#[inline(always)]
fn assign(w: &mut [Value], r: Reg, value: Value) {
    let slot = &mut w[r as usize];
    match slot {
        Value::Ref(cell) => *cell.borrow_mut() = value,
        _ => *slot = value,
    }
}

/// Whether PHP compiles a call of builtin `name` with `argc` arguments
/// to an opcode (zend_try_compile_special_func): its errors then come
/// with no frame of their own in the trace.
fn opcode_function(name: &str, argc: usize) -> bool {
    matches!((name, argc), ("strlen" | "count" | "sizeof", 1) | ("array_key_exists", 2))
}

/// A constant's key: its namespace's part is case-insensitive (lowered),
/// its own name is not.
fn const_key(name: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    match name.iter().rposition(|&c| c == b'\\') {
        Some(at) => std::borrow::Cow::Owned([name[..at].to_ascii_lowercase().as_slice(), &name[at..]].concat()),
        None => std::borrow::Cow::Borrowed(name),
    }
}

/// Whether register `r` holds a temporary (not a variable or constant).
fn is_temp(func: &Function, r: Reg) -> bool {
    r as usize >= func.consts_at as usize + func.consts.len()
}

/// Write `value` into a cell (through a reference it holds).
fn store(cell: &RefCell<Value>, value: Value) {
    let inner = cell.borrow().clone();
    match inner {
        Value::Ref(r) => *r.borrow_mut() = value,
        _ => *cell.borrow_mut() = value,
    }
}

/// Make variable register `r` a reference (if it is not one) and return its cell.
fn make_ref(w: &mut [Value], r: Reg) -> Rc<RefCell<Value>> {
    let slot = &mut w[r as usize];
    if let Value::Ref(cell) = slot {
        return cell.clone();
    }
    let value = match std::mem::take(slot) {
        Value::Undef => Value::Null,
        v => v,
    };
    let cell = Rc::new(RefCell::new(value));
    *slot = Value::Ref(cell.clone());
    cell
}

/// Arguments past the parameters, moved out for func_get_args() (and a
/// variadic parameter's `CollectVariadic`).
#[cold]
fn take_extra(regs: &mut [Value], _func: &Function, base: usize, nparams: usize, argc: usize) -> Option<Box<Extra>> {
    Some(Box::new(Extra { args: regs[base + nparams..base + argc].iter_mut().map(std::mem::take).collect(), vars: Vars::default() }))
}

/// unset(a[k1]..[kn]) on a value; whether anything changed.
fn unset_path(target: &mut Value, keys: &[Value]) -> bool {
    let Value::Array(a) = target else { return false };
    let Some((first, rest)) = keys.split_first() else { return false };
    let Ok((k, _)) = Key::from_value(first) else { return false };
    if rest.is_empty() {
        if !a.contains_key(&k) {
            return false;
        }
        Rc::make_mut(a).remove(&k);
        return true;
    }
    let Some(child) = Rc::make_mut(a).get_mut(&k) else { return false };
    unset_path(child, rest)
}

/// `$a[k] op= v` for an int or string `k`, `$a` an array holding a number
/// there and `v` a number; false (nothing done) for anything else. Out of
/// line, so the fast loop's registers stay its own.
#[inline(never)]
fn dim_op(w: &mut [Value], op: BinOp, var: Reg, keys: Reg, val: Reg, dst: Reg) -> bool {
    let r = match (&w[var as usize], &w[keys as usize]) {
        (Value::Array(a), &Value::Int(k)) => {
            let Some(r) = a.get_int(k).and_then(|x| num_op(op, x, &w[val as usize])) else { return false };
            let Value::Array(a) = &mut w[var as usize] else { return false };
            match Rc::make_mut(a).get_int_mut(k) {
                Some(slot) => *slot = r.clone(),
                None => return false,
            }
            r
        }
        (Value::Array(_), Value::Str(s)) => {
            // One lookup: the array is made writable first (a copy, if it is
            // shared, is invisible to PHP code even when nothing is written).
            let key = Key::from_string(s.clone());
            let v = w[val as usize].clone();
            let Value::Array(a) = &mut w[var as usize] else { return false };
            let Some(slot) = Rc::make_mut(a).get_mut(&key) else { return false };
            let Some(r) = num_op(op, slot, &v) else { return false };
            *slot = r.clone();
            r
        }
        _ => return false,
    };
    w[dst as usize] = r;
    true
}

/// `$a[string]` on an array (or a reference to one) holding a value that is
/// not a reference at that key; None for anything else (a missing key
/// warns, in `step`).
#[inline(never)]
fn str_elem(w: &[Value], arr: Reg, key: Reg) -> Option<Value> {
    let Value::Str(s) = &w[key as usize] else { return None };
    let key = Key::from_string(s.clone());
    let elem = |a: &PhpArray| match a.get(&key) {
        Some(Value::Ref(_)) | None => None,
        Some(v) => Some(v.clone()),
    };
    match &w[arr as usize] {
        Value::Array(a) => elem(a),
        Value::Ref(cell) => match &*cell.borrow() {
            Value::Array(a) => elem(a),
            _ => None,
        },
        _ => None,
    }
}

/// `$a[string] = v`, `$a` an array whose element there (if any) is not a
/// reference, `v` defined and not a reference; false (nothing done) for
/// anything else.
#[inline(never)]
fn set_str_elem(w: &mut [Value], var: Reg, keys: Reg, val: Reg, dst: Reg) -> bool {
    let (Value::Str(s), Some(v)) = (&w[keys as usize], copied(&w[val as usize])) else { return false };
    let key = Key::from_string(s.clone());
    let Value::Array(a) = &mut w[var as usize] else { return false };
    if matches!(a.get(&key), Some(Value::Ref(_))) {
        return false;
    }
    let a = Rc::make_mut(a);
    match a.get_mut(&key) {
        Some(slot) => *slot = v.clone(),
        None => a.insert(key, v.clone()),
    }
    w[dst as usize] = v;
    true
}

/// `$str .= $s` with both strings (and `$str` not a reference): appends in
/// place unless the string is shared. False (nothing done) otherwise.
#[inline(never)]
fn append_str(w: &mut [Value], var: Reg, src: Reg) -> bool {
    let Value::Str(tail) = &w[src as usize] else { return false };
    // A handle of its own: `$s .= $s` reads what it appends to.
    let tail = tail.clone();
    let Value::Str(s) = &mut w[var as usize] else { return false };
    s.push_bytes(tail.as_bytes());
    true
}

/// Whether a value becomes a string with no warning and no side effect
/// (so when it is converted cannot be observed).
#[inline(always)]
fn silent_str(v: &Value) -> bool {
    matches!(v, Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null)
}

/// `a . b` on values that convert silently, the result not into a
/// reference. A temporary string on the left that nothing else holds is
/// appended to in place (`a . b . c` builds one string). False otherwise.
#[inline(never)]
fn concat(w: &mut [Value], dst: Reg, a: Reg, b: Reg) -> bool {
    let (x, y) = (&w[a as usize], &w[b as usize]);
    if !silent_str(x) || !silent_str(y) || matches!(w[dst as usize], Value::Ref(_)) {
        return false;
    }
    let mut diags = Vec::new();
    if dst == a && a != b && matches!(w[a as usize], Value::Str(_)) {
        let Value::Str(mut s) = std::mem::take(&mut w[a as usize]) else { unreachable!("checked") };
        ops::push_string(s.make_mut(), &w[b as usize], &mut diags);
        w[dst as usize] = Value::Str(s);
        return true;
    }
    let mut bytes = Vec::with_capacity(ops::string_len_hint(x) + ops::string_len_hint(y));
    ops::push_string(&mut bytes, x, &mut diags);
    ops::push_string(&mut bytes, y, &mut diags);
    w[dst as usize] = Value::Str(PhpStr::new(bytes));
    true
}

/// Interpolation (`"a $b c"`) when every part converts silently.
#[inline(never)]
fn concat_n(w: &mut [Value], dst: Reg, start: Reg, n: u8) -> bool {
    let parts = &w[start as usize..start as usize + n as usize];
    if !parts.iter().all(silent_str) || matches!(w[dst as usize], Value::Ref(_)) {
        return false;
    }
    let mut diags = Vec::new();
    let mut bytes = Vec::with_capacity(parts.iter().map(ops::string_len_hint).sum());
    for v in parts {
        ops::push_string(&mut bytes, v, &mut diags);
    }
    w[dst as usize] = Value::Str(PhpStr::new(bytes));
    true
}

/// How UnhandledMatchError names the value no arm matched, as PHP's
/// smart_str_append_scalar does: scalars as written (a string quoted,
/// escaped and cut at 15 characters), anything else by type.
fn match_case(v: &Value) -> String {
    match v {
        Value::Null | Value::Undef => "NULL".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => float_to_string(*f, Precision::ECHO, 'E'),
        Value::Str(s) => {
            let bytes = s.as_bytes();
            let mut out = String::from("'");
            for &c in &bytes[..bytes.len().min(15)] {
                match c {
                    b'\n' => out.push_str("\\n"),
                    b'\r' => out.push_str("\\r"),
                    b'\t' => out.push_str("\\t"),
                    0x0c => out.push_str("\\f"),
                    0x0b => out.push_str("\\v"),
                    b'\\' => out.push_str("\\\\"),
                    0x1b => out.push_str("\\e"),
                    c if !(0x20..0x7f).contains(&c) => out.push_str(&format!("\\x{c:02X}")),
                    c => out.push(c as char),
                }
            }
            if bytes.len() > 15 {
                out.push_str("...");
            }
            out.push('\'');
            out
        }
        Value::Ref(cell) => match_case(&cell.borrow()),
        other => format!("of type {}", other.type_name()),
    }
}

/// foreach's next element: its value into `val` (and key into `key`), the
/// position past it into the iterator. False when there is none.
#[inline(never)]
fn iter_next(w: &mut [Value], iter: Reg, val: Reg, key: Reg, has_key: bool) -> bool {
    let pos = match w[iter as usize + 1] {
        Value::Int(p) => p as usize,
        _ => 0,
    };
    let next = match &w[iter as usize] {
        // By value: an element that is a reference (a by-reference foreach
        // made it one) gives its value, not the reference.
        Value::Array(a) => a.entry_at(pos).map(|(p, k, v)| (p, k, v.deref())),
        _ => None,
    };
    let Some((p, k, v)) = next else { return false };
    w[iter as usize + 1] = Value::Int(p as i64 + 1);
    assign(w, val, v);
    if has_key {
        assign(w, key, k.into_value());
    }
    true
}

/// Arithmetic on two numbers (and the bit operators on two ints), when no
/// warning, error or overflow is involved (the fast path: anything else
/// takes the slow one).
#[inline(always)]
fn num_op(op: BinOp, a: &Value, b: &Value) -> Option<Value> {
    match (a, b) {
        (&Value::Int(x), &Value::Int(y)) => match op {
            BinOp::Add => x.checked_add(y).map(Value::Int),
            BinOp::Sub => x.checked_sub(y).map(Value::Int),
            BinOp::Mul => x.checked_mul(y).map(Value::Int),
            // % by zero throws and PHP_INT_MIN % -1 is 0: both take `step`.
            BinOp::Mod => x.checked_rem(y).filter(|_| y != 0).map(Value::Int),
            BinOp::BitAnd => Some(Value::Int(x & y)),
            BinOp::BitOr => Some(Value::Int(x | y)),
            BinOp::BitXor => Some(Value::Int(x ^ y)),
            // A negative shift throws (`step`); 64 or more shifts every bit out.
            BinOp::Shl if (0..64).contains(&y) => Some(Value::Int(x << y)),
            BinOp::Shl if y >= 64 => Some(Value::Int(0)),
            BinOp::Shr if (0..64).contains(&y) => Some(Value::Int(x >> y)),
            BinOp::Shr if y >= 64 => Some(Value::Int(if x < 0 { -1 } else { 0 })),
            _ => None,
        },
        (Value::Float(x), Value::Float(y)) => float_op(op, *x, *y),
        (Value::Int(x), Value::Float(y)) => float_op(op, *x as f64, *y),
        (Value::Float(x), Value::Int(y)) => float_op(op, *x, *y as f64),
        _ => None,
    }
}

/// +, -, * and / on two floats (the fast path; / by zero takes the slow one).
#[inline]
fn float_op(op: BinOp, x: f64, y: f64) -> Option<Value> {
    Some(Value::Float(match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div if y != 0.0 => x / y,
        _ => return None,
    }))
}

fn compare(cmp: Cmp, x: &Value, y: &Value) -> Value {
    Value::Bool(match cmp {
        Cmp::Eq => x.loose_eq(y),
        Cmp::NotEq => !x.loose_eq(y),
        Cmp::Identical => x.identical(y),
        Cmp::NotIdentical => !x.identical(y),
        Cmp::Lt => x.compare(y) == Some(Ordering::Less),
        Cmp::Le => matches!(x.compare(y), Some(Ordering::Less | Ordering::Equal)),
        Cmp::Gt => y.compare(x) == Some(Ordering::Less),
        Cmp::Ge => matches!(y.compare(x), Some(Ordering::Less | Ordering::Equal)),
        Cmp::Spaceship => {
            return Value::Int(match x.compare(y) {
                Some(Ordering::Less) => -1,
                Some(Ordering::Equal) => 0,
                _ => 1,
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink;
    impl Host for Sink {
        fn write(&mut self, _: &[u8]) {}
    }

    fn compile(source: &str, registry: &Registry) -> p8_compiler::Program {
        let arena = bumpalo::Bump::new();
        let mut parser = p8_syntax::parser::Parser::new(p8_syntax::lexer::Lexer::new(source.as_bytes()), &arena);
        let program = parser.parse_program();
        assert!(program.errors.is_empty(), "parse errors");
        p8_compiler::Compiler::new(source.as_bytes(), "t.php", registry).compile(program.statements).expect("compiles")
    }

    /// A recursive function's call site resolves to the function itself.
    /// When the request ends, every closure it made must be freed: a cycle
    /// between a closure and its call sites would leak one per request.
    #[test]
    fn a_request_frees_its_closures() {
        let registry = Arc::new(Registry::default());
        let program = compile("<?php function f($n) { return $n < 1 ? 0 : f($n - 1); } f(5); f(3);", &registry);
        let mut isolate = Isolate::new(Arc::clone(&registry));
        let mut host = Sink;
        let closures: Vec<std::rc::Weak<Closure>> = {
            let mut cx = isolate.new_context(&mut host);
            for f in &program.functions {
                cx.declare(Arc::clone(f)).expect("declares");
            }
            assert_eq!(cx.run_main(Arc::clone(&program.main)), 0);
            assert!(cx.closures.iter().any(|c| c.sites.iter().any(|s| s.get().is_some())), "a call site was resolved");
            cx.closures.iter().map(Rc::downgrade).collect()
        };
        assert!(!closures.is_empty());
        assert!(closures.iter().all(|c| c.upgrade().is_none()), "a closure outlived its request");
    }
}

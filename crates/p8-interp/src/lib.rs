#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The p8 interpreter: one loop over register instructions, all frames in
//! one register file, calls resolved once per call site.

use p8_bytecode::{CastKind, Cmp, Function, Instr, Reg};
use p8_runtime::array::{Key, KeyIssue, PhpArray};
use p8_runtime::ext::{Ctx, Registry};
pub use p8_runtime::ext::Throw;
use p8_runtime::number::{Precision, float_to_string};
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
    extra: Option<Box<[Value]>>,
}

const NO_RET: u32 = u32::MAX;

/// The call stack. A popped frame keeps its slot and the closure in it, so
/// calling the same function again from the same depth (what a loop does)
/// reuses the slot without touching the closure's reference count.
#[derive(Default)]
struct Frames {
    slots: Vec<Frame>,
    /// Frames in use: `slots[..len]`.
    len: usize,
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
                (slot.pc, slot.base, slot.ret, slot.argc) = (0, base as u32, ret, argc as u32);
            }
            None => self.slots.push(Frame { func: Rc::clone(func), pc: 0, base: base as u32, ret, argc: argc as u32, extra: None }),
        }
        self.len += 1;
    }

    /// Pop the top frame; its slot stays for the next push.
    #[inline(always)]
    fn pop(&mut self) -> &mut Frame {
        self.len -= 1;
        &mut self.slots[self.len]
    }

    /// Drop every frame and slot (and the closures they hold).
    fn clear(&mut self) {
        self.slots.clear();
        self.len = 0;
    }
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
fn fast(regs: &mut Vec<Value>, frames: &mut Frames, closures: &[Rc<Closure>], mut pc: usize, stop: &AtomicU64) -> usize {
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
                let Value::Int(i) = w[$var as usize] else { return pc };
                let Some(n) = $step(i, 1) else { return pc };
                w[$var as usize] = Value::Int(n);
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
                Instr::IncVar { var } => match w[var as usize] {
                    Value::Int(i) if i != i64::MAX => {
                        w[var as usize] = Value::Int(i + 1);
                        pc += 1;
                    }
                    _ => return pc,
                },
                Instr::IncJumpLt { var, b, target } => step_jump!(var, b, target, i64::checked_add, |n, b| n < b),
                Instr::IncJumpLe { var, b, target } => step_jump!(var, b, target, i64::checked_add, |n, b| n <= b),
                Instr::DecJumpGt { var, b, target } => step_jump!(var, b, target, i64::checked_sub, |n, b| n > b),
                Instr::DecJumpGe { var, b, target } => step_jump!(var, b, target, i64::checked_sub, |n, b| n >= b),
                Instr::DecVar { var } => match w[var as usize] {
                    Value::Int(i) if i != i64::MIN => {
                        w[var as usize] = Value::Int(i - 1);
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
                Instr::Move { dst, src } => match copied(&w[src as usize]) {
                    Some(v) => {
                        w[dst as usize] = v;
                        pc += 1;
                    }
                    None => return pc,
                },
                Instr::Assign { var, src } => match copied(&w[src as usize]) {
                    Some(v) => {
                        assign(w, var, v);
                        pc += 1;
                    }
                    None => return pc,
                },
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
                    let Value::Int(i) = w[var as usize] else { return pc };
                    let Some(n) = (if inc { i.checked_add(1) } else { i.checked_sub(1) }) else { return pc };
                    w[var as usize] = Value::Int(n);
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
                | Instr::Echo { .. }
                | Instr::FetchConst { .. }
                | Instr::BindGlobal { .. }
                | Instr::ArgDefault { .. }
                | Instr::Exit { .. } => return pc,
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
                if (n as usize) < target.code.required || stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
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
            Switch::Return(_) | Switch::ReturnNull if frames.len() == 1 => return pc,
            Switch::Return(src) => {
                let value = match &regs[base + src as usize] {
                    Value::Undef => return pc,
                    Value::Ref(cell) => cell.borrow().clone(),
                    v => v.clone(),
                };
                leave(frames, regs, value);
                pc = frames.last().expect("the caller's frame").pc as usize;
            }
            Switch::ReturnNull => {
                leave(frames, regs, Value::Null);
                pc = frames.last().expect("the caller's frame").pc as usize;
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
    if frame.ret != NO_RET {
        regs[frame.ret as usize] = value;
    }
}

/// Make a register undefined, releasing what it held. Scalars hold
/// nothing to release, so they are only re-tagged.
#[inline(always)]
fn clear(r: &mut Value) {
    if matches!(r, Value::Str(_) | Value::Array(_) | Value::Ref(_)) {
        *r = Value::Undef;
    } else {
        // Nothing to drop: `forget` skips the drop check entirely.
        std::mem::forget(std::mem::replace(r, Value::Undef));
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

/// Output, with output buffers (ob_start) stacked on top of the host.
struct Output<'i> {
    buffers: Vec<Vec<u8>>,
    host: &'i mut dyn Host,
}

impl Output<'_> {
    fn write(&mut self, bytes: &[u8]) {
        match self.buffers.last_mut() {
            Some(buffer) => buffer.extend_from_slice(bytes),
            None => self.host.write(bytes),
        }
    }

    fn start_buffer(&mut self) {
        self.buffers.push(Vec::new());
    }

    /// Drop the innermost buffer's contents (ob_end_clean); false if none.
    fn end_clean(&mut self) -> bool {
        self.buffers.pop().is_some()
    }

    fn flush_all(&mut self) {
        while let Some(buffer) = self.buffers.pop() {
            self.write(&buffer);
        }
        self.host.flush();
    }
}

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
}

impl Isolate {
    pub fn new(registry: Arc<Registry>) -> Isolate {
        Isolate { registry, regs: Vec::with_capacity(1024), interrupt: Arc::new(AtomicU64::new(NO_INTERRUPT)) }
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
        Context {
            stop,
            isolate: self,
            frames: Frames::default(),
            functions: HashMap::new(),
            closures: Vec::new(),
            constants: HashMap::new(),
            globals: HashMap::new(),
            out: Output { buffers: Vec::new(), host },
            diags: Vec::new(),
            pc: 0,
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
    globals: HashMap<Vec<u8>, Rc<RefCell<Value>>>,
    out: Output<'i>,
    diags: Vec<Diag>,
    /// The running frame's pc (Frame.pc is written when it calls out).
    pc: usize,
}

impl Context<'_> {
    pub fn declare(&mut self, func: Arc<Function>) -> Result<(), String> {
        let key = func.name.as_bytes().to_ascii_lowercase();
        if self.functions.contains_key(&key) || self.isolate.registry.lookup(&key).is_some() {
            return Err(format!("Cannot redeclare function {}()", String::from_utf8_lossy(func.name.as_bytes())));
        }
        self.closures.push(Closure::new(func));
        self.functions.insert(key, (self.closures.len() - 1) as u32);
        Ok(())
    }

    pub fn function_exists(&self, name: &[u8]) -> bool {
        let key = name.strip_prefix(b"\\").unwrap_or(name).to_ascii_lowercase();
        self.functions.contains_key(&key) || self.isolate.registry.lookup(&key).is_some()
    }

    /// Write to the output and flush it (messages the host shows as they come).
    pub fn echo_and_flush(&mut self, bytes: &[u8]) {
        self.out.write(bytes);
        self.out.flush_all();
    }

    /// Run a file's top-level code; returns the exit status.
    pub fn run_main(&mut self, main: Arc<Function>) -> i32 {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        self.frames.push(&Closure::new(main), 0, 0, NO_RET);
        enter(&mut self.frames, &mut regs);
        let result = self.execute(&mut regs);
        // The request's variables go now (the isolate keeps the capacity).
        regs.clear();
        self.frames.clear();
        self.isolate.regs = regs;
        let status = match result {
            Ok(()) => 0,
            Err(Throw::Exit(status)) => status,
            Err(Throw::Fatal { message, file, line }) => {
                self.out.write(format!("\nFatal error: {message} in {file} on line {line}\n").as_bytes());
                255
            }
            Err(Throw::Error { class, message, file, line }) => {
                self.out.write(
                    format!(
                        "\nFatal error: Uncaught {class}: {message} in {file}:{line}\nStack trace:\n#0 {{main}}\n  thrown in {file} on line {line}\n"
                    )
                    .as_bytes(),
                );
                255
            }
        };
        self.out.flush_all();
        status
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
        Throw::Error { class, message: message.into(), file, line }
    }

    fn location(&self) -> (Rc<str>, u32) {
        match self.frames.last() {
            Some(f) => (Rc::from(&*f.func.code.file), f.func.code.lines.get(self.pc.saturating_sub(1)).copied().unwrap_or(0)),
            None => (Rc::from(""), 0),
        }
    }

    fn op_error(&self, e: OpError) -> Throw {
        match e {
            OpError::Type(m) => self.throw("TypeError", m),
            OpError::DivisionByZero(m) => self.throw("DivisionByZeroError", m),
            OpError::Arithmetic(m) => self.throw("ArithmeticError", m),
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
        let (file, line) = self.location();
        for d in std::mem::take(&mut self.diags) {
            let (label, message) = match d {
                Diag::Warning(m) => ("Warning", m),
                Diag::Deprecated(m) => ("Deprecated", m),
            };
            self.out.write(format!("\n{label}: {message} in {file} on line {line}\n").as_bytes());
        }
    }

    fn warn(&mut self, message: impl Into<String>) {
        self.diags.push(Diag::Warning(message.into()));
        self.report_diags();
    }

    /// The value in register `r` of the frame window `w`, as an operand:
    /// looks through references; an undefined variable warns and reads as null.
    #[inline(always)]
    fn operand(&mut self, w: &[Value], func: &Function, r: Reg) -> Value {
        match &w[r as usize] {
            Value::Undef => self.undefined(func, r),
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
        let callee = if let Some(f) = self.functions.get(key) {
            Callee::User(*f)
        } else if let Some(i) = self.isolate.registry.lookup(key) {
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
        loop {
            pc = fast(regs, &mut self.frames, &self.closures, pc, &stop);
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
                let w = &mut regs[base..base + func.code.nregs as usize];
                self.call_builtin(w, i, args, n, dst)?;
                pc += 1;
                continue;
            }
            match self.step(regs, &func, base, argc, pc)? {
                Next::Go(next) => pc = next,
                Next::Switch => match self.frames.last() {
                    Some(frame) => pc = frame.pc as usize,
                    None => return Ok(()),
                },
            }
        }
    }

    /// Execute the instruction at `pc` of `func`'s frame (at `base`).
    #[inline(never)]
    fn step(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, argc: usize, at: usize) -> Result<Next, Throw> {
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
                            let x = self.operand(w, func, a);
                            let y = self.operand(w, func, b);
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
                        // `.=` appends in place when the string is not shared
                        let x = self.take_var(w, func, var);
                        let mut s = match x {
                            Value::Str(s) => s,
                            other => ops::to_string_lossy(&other, &mut self.diags),
                        };
                        let tail = ops::to_string_lossy(&y, &mut self.diags);
                        self.report_diags();
                        s.push_bytes(tail.as_bytes());
                        assign(w, var, Value::Str(s));
                    } else {
                        let x = self.operand(w, func, var);
                        let r = ops::binary(op, &x, &y, &mut self.diags);
                        self.report_diags();
                        let v = r.map_err(|e| self.op_error(e))?;
                        assign(w, var, v);
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
                Instr::Cast { kind, dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = match kind {
                        CastKind::Int => Value::Int(v.to_int()),
                        CastKind::Float => Value::Float(v.to_float()),
                        CastKind::Bool => Value::Bool(v.to_bool()),
                        CastKind::String => {
                            let s = ops::to_string_lossy(&v, &mut self.diags);
                            self.report_diags();
                            Value::Str(s)
                        }
                        CastKind::Array => match v {
                            Value::Array(a) => Value::Array(a),
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
                            (!matches!(v, Value::Null)).then_some(v)
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
                    let v = self.fetch_dim(&container, w, func, key, quiet)?;
                    reg!(dst) = v;
                }
                Instr::AssignDim { var, keys, n, append, val, dst } => {
                    let v = self.operand(w, func, val);
                    self.assign_dim(w, func, var, keys, n, append, v.clone(), None)?;
                    reg!(dst) = v;
                }
                Instr::AssignDimOp { op, var, keys, n, val, dst } => {
                    let v = self.operand(w, func, val);
                    let result = self.assign_dim(w, func, var, keys, n, false, v, Some(op))?;
                    reg!(dst) = result;
                }
                Instr::IssetDim { dst, var, keys, n } => {
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
                        Value::Ref(cell) => !matches!(*cell.borrow(), Value::Null),
                        _ => true,
                    };
                    reg!(dst) = Value::Bool(set);
                }
                Instr::UnsetVar { var } => reg!(var) = Value::Undef,
                Instr::UnsetDim { var, keys, n } => {
                    let key_values: Vec<Value> = (0..n).map(|i| reg!(keys + i as Reg).clone()).collect();
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
                        other => {
                            self.warn(format!(
                                "foreach() argument must be of type array|object, {} given",
                                other.type_name()
                            ));
                            pc = end as usize;
                        }
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
                Instr::ArgDefault { n, target } => {
                    if (n as usize) < argc {
                        pc = target as usize;
                    }
                }
                Instr::Echo { src } => {
                    let v = self.operand(w, func, src);
                    match &v {
                        Value::Str(s) => self.out.write(s.as_bytes()),
                        other => {
                            let s = ops::to_string_lossy(other, &mut self.diags);
                            self.report_diags();
                            self.out.write(s.as_bytes());
                        }
                    }
                }
                Instr::FetchConst { site, dst } => {
                    let s = &func.sites[site as usize];
                    let v = match clo.sites[site as usize].get() {
                        Some(Callee::Constant(v)) => v.clone(),
                        _ => match self.constant_value(s.name.as_bytes()) {
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
                Instr::ConcatN { dst, start, n } => {
                    let parts = start as usize..start as usize + n as usize;
                    let mut bytes = Vec::with_capacity(w[parts].iter().map(ops::string_len_hint).sum());
                    for i in 0..n {
                        let v = self.operand(w, func, start + i as Reg);
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
                Instr::Exit { src, has } => {
                    let v = self.operand(w, func, src);
                    let status = match (has, &v) {
                        (true, Value::Int(i)) => *i as i32,
                        (true, other) => {
                            let s = ops::to_string_lossy(other, &mut self.diags);
                            self.out.write(s.as_bytes());
                            0
                        }
                        (false, _) => 0,
                    };
                    return Err(Throw::Exit(status));
                }
                Instr::Call { site, base: args, argc, dst } => {
                    match self.resolve(clo, site)? {
                        Callee::User(target) => {
                            let target = Rc::clone(&self.closures[target as usize]);
                            if (argc as usize) < target.code.required {
                                return Err(self.missing_args(&target.code, argc as usize));
                            }
                            if let Some(caller) = self.frames.last_mut() {
                                caller.pc = pc as u32;
                            }
                            // Missing optional arguments are filled by the callee's defaults.
                            self.frames.push(&target, base + args as usize, argc as usize, (base + dst as usize) as u32);
                            enter(&mut self.frames, regs);
                            return Ok(Next::Switch);
                        }
                        Callee::Builtin(i) => self.call_builtin(w, i, args, argc, dst)?,
                        Callee::Constant(_) => unreachable!("a call site resolves to a function"),
                    }
                }
                Instr::Return { src } => {
                    let v = self.operand(w, func, src);
                    leave(&mut self.frames, regs, v);
                    return Ok(Next::Switch);
                }

                Instr::ReturnNull => {
                    leave(&mut self.frames, regs, Value::Null);
                    return Ok(Next::Switch);
                }
        }
        Ok(Next::Go(pc))
    }

    /// Call builtin `i` on the `argc` arguments at `args`. The builtin
    /// works on those registers directly (the register file is not part of
    /// the context, so it can lend them alongside itself); they are cleared
    /// after, so nothing the call was given outlives it.
    #[inline]
    fn call_builtin(&mut self, w: &mut [Value], i: usize, args: Reg, argc: u8, dst: Reg) -> Result<(), Throw> {
        let args = &mut w[args as usize..args as usize + argc as usize];
        let r = (self.isolate.registry.function(i).run)(self, args);
        for v in args {
            clear(v);
        }
        self.report_diags();
        w[dst as usize] = r?;
        Ok(())
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
    fn missing_args(&self, func: &Function, passed: usize) -> Throw {
        let (file, line) = self.location();
        let declared = func.params.iter().filter(|p| !p.variadic).count();
        let exactly = if func.required == declared { "exactly" } else { "at least" };
        let name = String::from_utf8_lossy(func.name.as_bytes());
        Throw::Error {
            class: "ArgumentCountError",
            message: format!(
                "Too few arguments to function {name}(), {passed} passed in {file} on line {line} and {exactly} {} expected",
                func.required
            ),
            file: Rc::from(&*func.file),
            line: func.line,
        }
    }

    /// The cell of global `$name`: the main frame's variable if it has one.
    /// `main` is the main frame's window (it starts at register 0).
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
        self.frames.last().and_then(|f| f.extra.as_deref()).unwrap_or(&[])
    }
}


impl Context<'_> {
    /// The constant `name`: the script's own, else an extension's.
    fn constant_value(&self, name: &[u8]) -> Option<Value> {
        match self.constants.get(name) {
            Some(v) => Some(v.clone()),
            None => self.isolate.registry.constant(name).map(|c| c.to_value()),
        }
    }
}

impl Ctx for Context<'_> {
    fn echo(&mut self, bytes: &[u8]) {
        self.out.write(bytes);
    }

    fn ob_start(&mut self) {
        self.out.start_buffer();
    }

    fn ob_end_clean(&mut self) -> bool {
        self.out.end_clean()
    }

    fn warn(&mut self, message: &str) {
        Context::warn(self, message);
    }

    fn constant(&self, name: &[u8]) -> Option<Value> {
        self.constant_value(name)
    }

    fn define(&mut self, name: &[u8], value: Value) -> bool {
        if self.constant_value(name).is_some() {
            return false;
        }
        self.constants.insert(name.to_vec(), value);
        true
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

/// Arguments past the parameters, moved out for func_get_args() (a
/// variadic parameter collects them instead).
#[cold]
fn take_extra(regs: &mut [Value], func: &Function, base: usize, nparams: usize, argc: usize) -> Option<Box<[Value]>> {
    if func.params.last().is_some_and(|p| p.variadic) {
        return None;
    }
    Some(regs[base + nparams..base + argc].iter_mut().map(std::mem::take).collect())
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

/// foreach's next element: its value into `val` (and key into `key`), the
/// position past it into the iterator. False when there is none.
#[inline(never)]
fn iter_next(w: &mut [Value], iter: Reg, val: Reg, key: Reg, has_key: bool) -> bool {
    let pos = match w[iter as usize + 1] {
        Value::Int(p) => p as usize,
        _ => 0,
    };
    let next = match &w[iter as usize] {
        Value::Array(a) => a.entry_at(pos).map(|(p, k, v)| (p, k, v.clone())),
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

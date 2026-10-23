//! The v2 interpreter: one loop over register instructions, all frames in
//! one register file, calls resolved once per call site.

use super::array::{Key, KeyIssue, PhpArray};
use super::builtins::{self, Builtin};
use super::code::{Callee, CastKind, Cmp, Function, Instr, Reg};
use super::ops::{self, BinOp, Diag, OpError};
use super::string::PhpStr;
use super::value::Value;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

/// Something that ends the normal flow: a thrown error, or exit().
#[derive(Debug, Clone)]
pub enum Throw {
    Error { class: &'static str, message: String, file: Rc<str>, line: u32 },
    Exit(i32),
}

struct Frame {
    func: Rc<Function>,
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

/// What `step` leaves the frame loop to do.
enum Next {
    /// Continue this frame at the pc.
    Go(usize),
    /// A call or return changed the current frame.
    Switch,
}

/// Run the common cases of the hottest instructions from `pc` on: those
/// that need only the frame's registers `w` (ints and floats, jumps).
/// Returns the pc of the first instruction it cannot run, for `step`.
///
/// Kept apart from everything else so its state stays in machine registers.
#[inline(never)]
fn fast(w: &mut [Value], code: &[Instr], mut pc: usize) -> usize {
    loop {
        match code[pc] {
            Instr::Jump { target } => pc = target as usize,
            Instr::JumpCmp { cmp, a, b, when, target } => match num_test(cmp, &w[a as usize], &w[b as usize]) {
                Some(t) => pc = if t == when { target as usize } else { pc + 1 },
                None => return pc,
            },
            Instr::Compare { cmp, dst, a, b } => match num_test(cmp, &w[a as usize], &w[b as usize]) {
                Some(t) if cmp != Cmp::Spaceship => {
                    w[dst as usize] = Value::Bool(t);
                    pc += 1;
                }
                _ => return pc,
            },
            Instr::IncVar { var, inc } => match w[var as usize] {
                Value::Int(i) => match if inc { i.checked_add(1) } else { i.checked_sub(1) } {
                    Some(n) => {
                        w[var as usize] = Value::Int(n);
                        pc += 1;
                    }
                    None => return pc,
                },
                _ => return pc,
            },
            Instr::Binary { op, dst, a, b } => {
                let r = match (&w[a as usize], &w[b as usize]) {
                    (Value::Int(x), Value::Int(y)) => match op {
                        BinOp::Add => x.checked_add(*y).map(Value::Int),
                        BinOp::Sub => x.checked_sub(*y).map(Value::Int),
                        BinOp::Mul => x.checked_mul(*y).map(Value::Int),
                        _ => None,
                    },
                    (Value::Float(x), Value::Float(y)) => float_op(op, *x, *y),
                    (Value::Int(x), Value::Float(y)) => float_op(op, *x as f64, *y),
                    (Value::Float(x), Value::Int(y)) => float_op(op, *x, *y as f64),
                    _ => None,
                };
                match r {
                    Some(r) => {
                        w[dst as usize] = r;
                        pc += 1;
                    }
                    None => return pc,
                }
            }
            Instr::Move { dst, src } => match plain(&w[src as usize]) {
                Some(v) => {
                    w[dst as usize] = v;
                    pc += 1;
                }
                None => return pc,
            },
            Instr::Assign { var, src } => match plain(&w[src as usize]) {
                Some(v) => {
                    assign(w, var, v);
                    pc += 1;
                }
                None => return pc,
            },
            Instr::JumpIf { cond, target } => match w[cond as usize] {
                Value::Bool(c) => pc = if c { target as usize } else { pc + 1 },
                _ => return pc,
            },
            Instr::JumpIfNot { cond, target } => match w[cond as usize] {
                Value::Bool(c) => pc = if c { pc + 1 } else { target as usize },
                _ => return pc,
            },
            // $a[int] on an array (or a reference to one) holding a plain
            // value at that key
            Instr::FetchDim { dst, arr, key, .. } => {
                let Value::Int(k) = w[key as usize] else { return pc };
                let v = match &w[arr as usize] {
                    Value::Array(a) => int_elem(a, k),
                    Value::Ref(cell) => match &*cell.borrow() {
                        Value::Array(a) => int_elem(a, k),
                        _ => None,
                    },
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
            _ => return pc,
        }
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

/// Output, with output buffers (ob_start) stacked on top of stdout.
pub struct Output {
    buffers: Vec<Vec<u8>>,
    stdout: std::io::BufWriter<std::io::Stdout>,
}

impl Output {
    fn new() -> Self {
        Output { buffers: Vec::new(), stdout: std::io::BufWriter::with_capacity(64 * 1024, std::io::stdout()) }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        match self.buffers.last_mut() {
            Some(buffer) => buffer.extend_from_slice(bytes),
            None => {
                let _ = self.stdout.write_all(bytes);
            }
        }
    }

    pub fn start_buffer(&mut self) {
        self.buffers.push(Vec::new());
    }

    /// Drop the innermost buffer's contents (ob_end_clean); false if none.
    pub fn end_clean(&mut self) -> bool {
        self.buffers.pop().is_some()
    }

    pub fn flush_all(&mut self) {
        while let Some(buffer) = self.buffers.pop() {
            self.write(&buffer);
        }
        let _ = self.stdout.flush();
    }
}

pub struct Vm {
    /// All frames' registers. While code runs, the interpreter holds the
    /// register file itself (taken out of here) so the hot loop works on a
    /// plain local slice.
    regs: Vec<Value>,
    frames: Vec<Frame>,
    /// Declared functions by lowercase name (only ever grows, so call sites
    /// can cache what they resolve).
    functions: HashMap<Vec<u8>, Rc<Function>>,
    pub(crate) constants: HashMap<Vec<u8>, Value>,
    /// Globals not held in the main frame's registers.
    globals: HashMap<Vec<u8>, Rc<RefCell<Value>>>,
    pub(crate) out: Output,
    diags: Vec<Diag>,
    /// The running frame's pc (Frame.pc is written when it calls out).
    pc: usize,
    /// Reused for builtin arguments, so a builtin call does not allocate.
    args: Vec<Value>,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        Vm {
            regs: Vec::with_capacity(1024),
            frames: Vec::new(),
            functions: HashMap::new(),
            constants: builtins::constants(),
            globals: HashMap::new(),
            out: Output::new(),
            diags: Vec::new(),
            pc: 0,
            args: Vec::new(),
        }
    }

    pub fn declare(&mut self, func: Rc<Function>) -> Result<(), String> {
        let key = func.name.as_bytes().to_ascii_lowercase();
        if self.functions.contains_key(&key) || builtins::lookup(&key).is_some() {
            return Err(format!("Cannot redeclare function {}()", String::from_utf8_lossy(func.name.as_bytes())));
        }
        self.functions.insert(key, func);
        Ok(())
    }

    pub fn function_exists(&self, name: &[u8]) -> bool {
        let key = name.strip_prefix(b"\\").unwrap_or(name).to_ascii_lowercase();
        self.functions.contains_key(&key) || builtins::lookup(&key).is_some()
    }

    /// Run a file's top-level code; returns the exit status.
    pub fn run_main(&mut self, main: Rc<Function>) -> i32 {
        let mut regs = std::mem::take(&mut self.regs);
        self.enter(&mut regs, main, 0, 0, NO_RET);
        let result = self.execute(&mut regs);
        self.regs = regs;
        let status = match result {
            Ok(()) => 0,
            Err(Throw::Exit(status)) => status,
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

    /// Push a frame for `func` whose window starts at `base`, with `argc`
    /// arguments already in its first registers.
    #[inline(never)]
    fn enter(&mut self, regs: &mut Vec<Value>, func: Rc<Function>, base: usize, argc: usize, ret: u32) {
        let need = base + func.nregs as usize;
        if regs.len() < need {
            regs.resize(need, Value::Undef);
        }
        let nparams = func.params.len();
        let extra = if argc > nparams { take_extra(regs, &func, base, nparams, argc) } else { None };
        let consts_at = func.consts_at as usize;
        for r in &mut regs[base + argc.min(nparams)..base + consts_at] {
            *r = Value::Undef;
        }
        regs[base + consts_at..base + consts_at + func.consts.len()].clone_from_slice(&func.consts);
        self.frames.push(Frame { func, pc: 0, base: base as u32, ret, argc: argc as u32, extra });
    }

    fn throw(&self, class: &'static str, message: impl Into<String>) -> Throw {
        let (file, line) = self.location();
        Throw::Error { class, message: message.into(), file, line }
    }

    fn location(&self) -> (Rc<str>, u32) {
        match self.frames.last() {
            Some(f) => (f.func.file.clone(), f.func.lines.get(self.pc.saturating_sub(1)).copied().unwrap_or(0)),
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
    fn report_diags(&mut self) {
        if self.diags.is_empty() {
            return;
        }
        let (file, line) = self.location();
        for d in std::mem::take(&mut self.diags) {
            let (label, message) = match d {
                Diag::Warning(m) => ("Warning", m),
                Diag::Deprecated(m) => ("Deprecated", m),
            };
            self.out.write(format!("\n{label}: {message} in {file} on line {line}\n").as_bytes());
        }
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>) {
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
    fn resolve(&mut self, func: &Function, site: u16) -> Result<Callee, Throw> {
        match func.sites[site as usize].cache.get() {
            Some(c) => Ok(c.clone()),
            None => self.resolve_slow(func, site),
        }
    }

    #[cold]
    #[inline(never)]
    fn resolve_slow(&mut self, func: &Function, site: u16) -> Result<Callee, Throw> {
        let s = &func.sites[site as usize];
        let key = s.name.as_bytes();
        let callee = if let Some(f) = self.functions.get(key) {
            Callee::User(f.clone())
        } else if let Some(i) = builtins::lookup(key) {
            Callee::Builtin(i)
        } else {
            let name = String::from_utf8_lossy(s.display.as_bytes()).into_owned();
            return Err(self.throw("Error", format!("Call to undefined function {name}()")));
        };
        let _ = s.cache.set(callee.clone());
        Ok(callee)
    }

    /// Run frames until the bottom one returns. The outer loop starts (or
    /// resumes) a frame; the inner one alternates `fast`, which runs the
    /// common cases of the hottest instructions, with `step`, which runs
    /// the one instruction `fast` stopped at, until a call or return
    /// switches frames.
    fn execute(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        loop {
            let frame = self.frames.last().expect("a frame");
            let func = frame.func.clone();
            let base = frame.base as usize;
            let argc = frame.argc as usize;
            let mut pc = frame.pc as usize;
            loop {
                pc = fast(&mut regs[base..base + func.nregs as usize], &func.code, pc);
                self.pc = pc + 1;
                match self.step(regs, &func, base, argc, pc)? {
                    Next::Go(next) => pc = next,
                    Next::Switch => break,
                }
            }
            if self.frames.is_empty() {
                return Ok(());
            }
        }
    }

    /// Execute the instruction at `pc` of `func`'s frame (at `base`).
    #[inline(never)]
    fn step(&mut self, regs: &mut Vec<Value>, func: &Function, base: usize, argc: usize, at: usize) -> Result<Next, Throw> {
        let instr = func.code[at];
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
                    reg!(dst) = v;
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
                    reg!(dst) = v;
                }
                Instr::JumpCmp { cmp, a, b, when, target } => {
                    if self.test_regs(w, func, cmp, a, b) == when {
                        pc = target as usize;
                    }
                }
                Instr::IncVar { var, inc } => {
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
                    let pos = match reg!(iter + 1) {
                        Value::Int(p) => p as usize,
                        _ => 0,
                    };
                    let next = match &reg!(iter) {
                        Value::Array(a) => a.entry_at(pos).map(|(p, k, v)| (p, k, v.clone())),
                        _ => None,
                    };
                    match next {
                        Some((p, k, v)) => {
                            reg!(iter + 1) = Value::Int(p as i64 + 1);
                            assign(w, val, v);
                            if has_key {
                                assign(w, key, k.to_value());
                            }
                        }
                        None => pc = end as usize,
                    }
                }
                Instr::SendVar { site, n, arg, var } => {
                    let by_ref = match self.resolve(func, site)? {
                        Callee::User(f) => f.param_by_ref(n as usize),
                        Callee::Builtin(i) => builtins::by_ref(i, n as usize),
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
                    let v = match s.cache.get() {
                        Some(Callee::Constant(v)) => v.clone(),
                        _ => match self.constants.get(s.name.as_bytes()) {
                            Some(v) => {
                                let v = v.clone();
                                let _ = s.cache.set(Callee::Constant(v.clone()));
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
                    let mut bytes = Vec::new();
                    for i in 0..n {
                        let v = self.operand(w, func, start + i as Reg);
                        let s = ops::to_string_lossy(&v, &mut self.diags);
                        bytes.extend_from_slice(s.as_bytes());
                    }
                    self.report_diags();
                    reg!(dst) = Value::Str(PhpStr::new(bytes));
                }
                Instr::BindGlobal { var, name } => {
                    let name = match &func.consts[name as usize] {
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
                    match self.resolve(func, site)? {
                        Callee::User(target) => {
                            if (argc as usize) < target.required {
                                return Err(self.missing_args(&target, argc as usize));
                            }
                            if let Some(caller) = self.frames.last_mut() {
                                caller.pc = pc as u32;
                            }
                            // Missing optional arguments are filled by the callee's defaults.
                            self.enter(regs, target, base + args as usize, argc as usize, (base + dst as usize) as u32);
                            return Ok(Next::Switch);
                        }
                        Callee::Builtin(i) => self.call_builtin(w, i, args, argc, dst)?,
                        Callee::Constant(_) => unreachable!("a call site resolves to a function"),
                    }
                }
                Instr::Return { src } => {
                    let v = self.operand(w, func, src);
                    self.leave(regs, v);
                    return Ok(Next::Switch);
                }

                Instr::ReturnNull => {
                    self.leave(regs, Value::Null);
                    return Ok(Next::Switch);
                }
        }
        Ok(Next::Go(pc))
    }

    /// Call builtin `i` with the `argc` arguments at `args`; result to `dst`.
    #[inline(never)]
    fn call_builtin(&mut self, w: &mut [Value], i: usize, args: Reg, argc: u8, dst: Reg) -> Result<(), Throw> {
        let start = args as usize;
        let mut values = std::mem::take(&mut self.args);
        values.extend(w[start..start + argc as usize].iter_mut().map(std::mem::take));
        let entry: &Builtin = builtins::entry(i);
        let r = (entry.run)(self, &mut values);
        values.clear();
        self.args = values;
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
            file: func.file.clone(),
            line: func.line,
        }
    }

    /// Pop the current frame, putting `value` in the caller's result
    /// register; the caller (if any) resumes at its saved pc.
    #[inline(never)]
    fn leave(&mut self, regs: &mut [Value], value: Value) {
        let frame = self.frames.pop().expect("a frame");
        // Locals die with the frame (before the result lands: the result
        // register may be inside the window).
        let base = frame.base as usize;
        for r in &mut regs[base..base + frame.func.nregs as usize] {
            *r = Value::Undef;
        }
        if frame.ret != NO_RET {
            regs[frame.ret as usize] = value;
        }
        if let Some(caller) = self.frames.last() {
            self.pc = caller.pc as usize;
        }
    }

    /// The cell of global `$name`: the main frame's variable if it has one.
    /// `main` is the main frame's window (it starts at register 0).
    fn global_cell(&mut self, main: &mut [Value], name: &[u8]) -> Rc<RefCell<Value>> {
        if let Some(r) = self.frames[0].func.cv_names.iter().position(|n| n.as_bytes() == name) {
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
                let shown = crate::core::value::php_float_to_string(f, crate::core::value::FloatPrecision::Shortest, 'E');
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
    pub(crate) fn extra_args(&self) -> &[Value] {
        self.frames.last().and_then(|f| f.extra.as_deref()).unwrap_or(&[])
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

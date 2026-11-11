//! The interpreter loop: the common cases of the hottest instructions
//! inline, everything else through `Context::slow` (which runs `step`),
//! called from inside the loop so that it never leaves it; and the frame
//! entry and exit the loop shares with `step`.

use super::*;
use crate::vmops::{self, Ctl, Hx};
use p8_bytecode::{Insn, Op, insn};

// `dispatch!(op, insn, { hand arms })`, written by p8-vmgen from ops.def:
// the loop's match, its hand arms then one arm per generated opcode.
include!(concat!(env!("OUT_DIR"), "/dispatch.rs"));

impl Context<'_> {
    /// Run frames until the bottom one returns. The loop itself runs the
    /// common cases of the hottest instructions: those that need only the
    /// frame's registers (ints and floats, jumps), plus calls to a user
    /// function the call site already resolved and returns of a value,
    /// which switch frames here, and frameless builtin calls, which keep
    /// the frame and its window. Every other instruction, and every case
    /// these do not cover, runs through `slow` (`step`), a call from inside
    /// the loop; the loop goes on with the pc it gives back.
    ///
    /// It reads the function's 8-byte instructions (`Function::insns`):
    /// their `u8` register operands index the frame's 256-register window
    /// with no bounds check. An `Op::Slow` one is the `Instr` at the same
    /// pc, `step`'s.
    #[inline(never)]
    pub(crate) fn execute(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        let mut pc = self.frames.last().expect("a frame").pc as usize;
        // Fiber::throw(): the exception is raised where the fiber went on.
        if let Some(e) = self.fiber_throw.take() {
            pc = self.unwind(regs, Throw::Object(e), pc.saturating_sub(1))?;
        }
        // Each round of this loop is one frame's run (the frame's code and
        // window are read once); the inner loop goes back here whenever
        // the frame changes or `slow` ran (it may have moved the registers).
        'frame: loop {
            let mut top = self.frames.last().expect("a frame");
            let mut clo = &*top.func;
            let base = top.base as usize;
            let mut code = &clo.code.insns[..];
            // A loop's backward branch is where an interrupt is noticed:
            // `step` then raises it.
            macro_rules! back {
                ($target:expr) => {
                    $target as usize <= pc && self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT
                };
            }
            // The instruction at `pc` is `slow`'s (`step`'s): run it there,
            // then start over with the frame as it left it.
            macro_rules! slow {
                () => {{
                    match self.slow(regs, pc)? {
                        Flow::Go(next) => pc = next,
                        Flow::Done => return Ok(()),
                    }
                    continue 'frame;
                }};
            }
            // A write freed an object with a destructor: it runs before
            // the next instruction.
            macro_rules! pending {
                () => {{
                    pc = self.after_errors(regs, pc, pc.saturating_sub(1))?;
                    continue 'frame;
                }};
            }
            // The same for an instruction that jumped: `$next` is where it
            // goes on, `$at` the instruction itself.
            macro_rules! pending_at {
                ($next:expr, $at:expr) => {{
                    pc = self.after_errors(regs, $next, $at)?;
                    continue 'frame;
                }};
            }
            // The straight-line instruction at `pc` is done; `$released`
            // says whether its write released memory (`put`), which is
            // when a destructor may be due (it runs at this instruction's
            // pc, as PHP runs it where the value went). A plain overwrite
            // tests nothing.
            macro_rules! done {
                ($released:expr) => {{
                    let released: bool = $released;
                    pc += 1;
                    if released && self.ids.has_pending() {
                        pending!();
                    }
                }};
            }
            // A register operand (a byte: in the window by its type).
            macro_rules! rx {
                ($e:expr) => {
                    usize::from($e)
                };
            }
            // A register computed from one (an argument after the first):
            // `lower` keeps such blocks in the window, so the mask is the
            // identity, and the index needs no check.
            macro_rules! rk {
                ($e:expr) => {
                    ($e) & (WINDOW - 1)
                };
            }
            // The register file is kept a window ahead of every frame
            // (`enter`), so the frame's fixed window always exists.
            if regs.len() < base + WINDOW {
                regs.resize(base + WINDOW, Value::UNDEF);
            }
            let w: &mut [Value; WINDOW] = (&mut regs[base..base + WINDOW]).try_into().expect("a window's worth of registers");
            // A constant operand (an `Op::*C` form): the closure's constant
            // (`Function::consts`, thawed once), read in place as PHP reads a
            // CONST operand from the op_array. `lower` keeps the index in
            // range; the check is the slice's.
            macro_rules! cst {
                ($k:expr) => {
                    &clo.consts[$k as usize]
                };
            }
            // A register operand of a two-operand arm, or a constant one, for
            // the helpers that take either (`Src`).
            macro_rules! reg_src {
                ($r:expr) => {
                    Src::Reg(Reg::from($r))
                };
            }
            macro_rules! cst_src {
                ($k:expr) => {
                    Src::Const(cst!($k))
                };
            }
            // `dst = a op b`: two ints inline, the rest (floats, references,
            // `.` on strings) through `arith`; what may warn or throw is
            // `step`'s. `$b` is the right operand as a `Src` (a register, or
            // a constant for the `C` forms).
            macro_rules! arith {
                ($op:expr, $dst:expr, $a:expr, $b:expr, $int:expr) => {{
                    let b: Src = $b;
                    let released = if let (Some(x), Some(y)) = (w[rx!($a)].as_int(), src(&w[..], b).as_int())
                        && let Some(n) = $int(x, y)
                    {
                        put_var(&mut w[rx!($dst)], Value::int(n))
                    } else if arith(&mut w[..], $op, Reg::from($dst), Reg::from($a), b) {
                        true
                    } else {
                        slow!()
                    };
                    done!(released);
                }};
            }
            // `var op= src`: two ints inline, the rest (floats, references,
            // `.=` on strings) through `assign_op`; `$b` a `Src` as `arith!`'s.
            macro_rules! assign_op {
                ($op:expr, $var:expr, $b:expr) => {{
                    let b: Src = $b;
                    let released = if let (Some(x), Some(y)) = (w[rx!($var)].as_int(), src(&w[..], b).as_int())
                        && let Some(n) = int_op($op, x, y)
                    {
                        put(&mut w[rx!($var)], Value::int(n))
                    } else if assign_op(&mut w[..], $op, Reg::from($var), b) {
                        true
                    } else {
                        slow!()
                    };
                    done!(released);
                }};
            }
            // `dst = a cmp b` (`<=>` is `step`'s): two ints inline, the rest
            // through `test`; `$y` the right operand's value.
            macro_rules! compare {
                ($cmp:expr, $dst:expr, $a:expr, $y:expr) => {{
                    let cmp: Cmp = $cmp;
                    let (x, y): (&Value, &Value) = (&w[rx!($a)], $y);
                    let t = if let (Some(x), Some(y)) = (x.as_int(), y.as_int())
                        && cmp != Cmp::Spaceship
                    {
                        int_test(cmp, x, y)
                    } else if cmp == Cmp::Spaceship {
                        slow!()
                    } else {
                        match test(cmp, x, y) {
                            Some(t) => t,
                            None => slow!(),
                        }
                    };
                    done!(put_var(&mut w[rx!($dst)], Value::bool(t)));
                }};
            }
            // An array literal's elements pushed or set (an int or string
            // key; a value defined and read through a reference; anything
            // else, a next key occupied included, is `step`'s); the value a
            // `Src` (taken when it is a temporary read for the last time).
            macro_rules! array_push {
                ($arr:expr, $val:expr, $take:expr) => {{
                    let val: Src = $val;
                    let Some(arr) = w[rx!($arr)].as_array() else { slow!() };
                    if arr.next_key().is_none() {
                        slow!();
                    }
                    let Some(v) = taken(&mut w[..], val, $take) else { slow!() };
                    let _ = w[rx!($arr)].array_mut().expect("checked above").push(v);
                    // (A taken temporary that held a reference let go of it.)
                    done!(true);
                }};
            }
            macro_rules! array_set {
                ($arr:expr, $key:expr, $val:expr, $take:expr) => {{
                    let (key, val): (Src, Src) = ($key, $val);
                    if !w[rx!($arr)].is_array() {
                        slow!();
                    }
                    let key = src(&w[..], key);
                    let k = if let Some(k) = key.as_int() {
                        Key::Int(k)
                    } else if let Some(s) = key.cloned_str() {
                        Key::from_string(s)
                    } else {
                        slow!()
                    };
                    let Some(v) = taken(&mut w[..], val, $take) else { slow!() };
                    // (A key written twice in one literal replaces the first value.)
                    w[rx!($arr)].array_mut().expect("checked above").insert(k, v);
                    done!(true);
                }};
            }
            // A fused step and test (`IncJumpLt` and co.) on ints (through a
            // reference too): step, then jump or go past the test. A non-int
            // bound leaves the test to the next instruction; a non-int
            // variable (or an overflow) leaves both to `step`. `$b` is the
            // bound's value.
            macro_rules! step_jump {
                ($var:expr, $b:expr, $target:expr, $step:expr, $holds:expr) => {{
                    let r = &mut w[rx!($var)];
                    // Written in place: no drop of the old value to compile.
                    let n = if let Some(i) = r.as_int_mut() {
                        match $step(*i, 1) {
                            Some(n) => {
                                *i = n;
                                n
                            }
                            None => slow!(),
                        }
                    } else if let Some(cell) = r.as_reference() {
                        match step_ref(cell, $step) {
                            Some(n) => n,
                            None => slow!(),
                        }
                    } else {
                        slow!()
                    };
                    match int_of($b) {
                        Some(b) if $holds(n, b) => {
                            if back!($target) {
                                // The step is done: the test (next) sees the interrupt.
                                pc += 1;
                                slow!();
                            }
                            pc = $target as usize;
                        }
                        Some(_) => pc += 2,
                        None => pc += 1,
                    }
                }};
            }
            // A frameless call (PHP's ZEND_FRAMELESS_ICALL) of builtin `$i`
            // with the `$n` arguments at window register `$start`, the
            // result to `$dst`, for the call instruction at `$at`: the
            // builtin borrows the registers in place, and the frame and its
            // window stay as they are (the loop's view of the frame is read
            // again from the frame stack, which the builtin cannot change).
            // `$temps`: the arguments are the call's own registers, consumed
            // by it; else they are variables, left as they are. A fault,
            // a diagnostic or a destructor due goes out of line.
            macro_rules! frameless {
                ($i:expr, $run:expr, $start:expr, $n:expr, $dst:expr, $temps:expr, $at:expr) => {{
                    let at: usize = $at;
                    self.pc = at + 1;
                    let r = self.frameless_run($i, $run, &mut w[$start..$start + $n]);
                    match r {
                        Some(v) if self.diags.is_empty() && self.pending_errors.is_empty() => {
                            if $temps {
                                for r in &mut w[$start..$start + $n] {
                                    let _ = clear(r);
                                }
                            }
                            // (A tag test, not the out-of-line drop an
                            // assignment calls.)
                            let _ = put(&mut w[rx!($dst)], v);
                            pc = at + 1;
                            if self.ids.has_pending() {
                                pending_at!(pc, at);
                            }
                            top = self.frames.last().expect("a frame");
                            clo = &*top.func;
                            code = &clo.code.insns[..];
                        }
                        r => {
                            pc = self.frameless_tail(regs, at, $i, r, (base + $start, $n, base + rx!($dst), $temps))?;
                            continue 'frame;
                        }
                    }
                }};
            }
            // A generated instruction's arm (`dispatch.rs`): the handler's
            // view of the frame, built in the arm (all of it registers once
            // the handler is inlined) ...
            macro_rules! hx {
                () => {
                    Hx { w: &mut *w, clo, top, frames: &self.frames, reinit: !self.reinit.is_empty() }
                };
            }
            // A return of `$v` from the top frame (not the script's main
            // frame, not a generator's): to the calling frame; from the
            // bottom frame of this run of the loop (a callback a builtin
            // called), out of the loop, as `slow` leaves it, with the
            // destructors the return made due run first.
            macro_rules! ret {
                ($v:expr) => {{
                    let value: Value = $v;
                    if self.frames.len() == self.frames.floor + 1 {
                        self.pc = pc + 1;
                        leave(&mut self.frames, regs, value);
                        if self.ids.has_pending() {
                            self.run_destructors(regs)?;
                        }
                        return Ok(());
                    }
                    leave(&mut self.frames, regs, value);
                    pc = self.frames.last().expect("the caller's frame").pc as usize;
                    if self.ids.has_pending() {
                        pc = self.after_errors(regs, pc, pc.saturating_sub(1))?;
                    }
                    continue 'frame;
                }};
            }
            // ... its cold method, run out of line as `step` would run the
            // instruction (with nothing of `step`'s own dispatch), then the
            // frame as it left it ...
            macro_rules! cold {
                ($m:ident) => {{
                    match self.cold(regs, pc, Self::$m)? {
                        Flow::Go(next) => pc = next,
                        Flow::Done => return Ok(()),
                    }
                    continue 'frame;
                }};
            }
            // ... and what the handler said to do next.
            macro_rules! gen_arm {
                ($m:ident, $call:expr) => {{
                    match $call {
                        Ctl::Next(released) => done!(released),
                        Ctl::Skip(released) => {
                            let at = pc;
                            pc += 2;
                            if released && self.ids.has_pending() {
                                pending_at!(pc, at);
                            }
                        }
                        Ctl::Jump(target) => {
                            if back!(target) {
                                cold!($m);
                            }
                            pc = target as usize;
                        }
                        Ctl::Return(value) => ret!(value),
                        Ctl::Cold => cold!($m),
                    }
                }};
            }
            loop {
                let insn: &Insn = &code[pc];
                let op = insn.op;
                // An arm's operands, read in the arm: read before the match,
                // they would all be loaded (and kept) on every dispatch.
                macro_rules! ops {
                    ($a:ident, $b:ident, $c:ident, $imm:ident) => {
                        // (Each arm names all four; it reads the ones it uses.)
                        #[allow(unused_variables)]
                        let ($a, $b, $c, $imm) = (insn.a, insn.b, insn.c, insn.imm);
                    };
                }
                // `C::f()` the site's cache has for this class and scope: a
                // PHP method's frame pushed here, after the variables
                // `$vars` names (`Function::arg_vars`) are sent (`send_vars`).
                // Anything else (a property hook's `parent::$p::get()`, a
                // native method, __callStatic (`remember_static` caches no
                // trampoline), the first call) is `step`'s.
                macro_rules! static_call {
                    ($vars:expr) => {{
                        ops!(a, b, c, imm);
                        let (dst, args, n, method, class) = (a, b, rx!(c), (imm & 0xffff) as usize, (imm >> 16) as u16);
                        let Some(m) = self.cached_static_call(clo, class, method) else { slow!() };
                        // (Taken apart: the frame takes its parts, no copy.)
                        let classes::MethodFrame { func: classes::Code::User(target), this, scope, called, closure, .. } = m else { slow!() };
                        if n < target.code.required || target.code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                            slow!();
                        }
                        let vars: Option<u16> = $vars;
                        if let Some(vars) = vars {
                            // (A destructor the sends made due: `step` sends
                            // them again, the same values, and runs it.)
                            match send_vars(&mut w[..], &clo.code.arg_vars[usize::from(vars)], rx!(args), &target.code) {
                                Some(released) if !(released && self.ids.has_pending()) => {}
                                _ => slow!(),
                            }
                        }
                        let frame = self.frames.call(Some(&target), pc + 1, base + args as usize, n, (base + dst as usize) as u32);
                        frame.this = this;
                        frame.scope = scope;
                        frame.called = called;
                        frame.owner = closure;
                        enter(&mut self.frames, regs);
                        pc = 0;
                        continue 'frame;
                    }};
                }
                // A foreach step over an array (`$key`: the key too). An
                // object (a generator, an Iterator) is `step`'s. (One arm
                // per opcode, so that no arm reads the opcode again: kept
                // for that, it would hold a register through every arm.)
                macro_rules! iter_next {
                    ($key:expr) => {{
                        ops!(a, b, c, imm);
                        if !w[rx!(a)].is_array() {
                            slow!();
                        }
                        let next = if iter_next(&mut w[..], Reg::from(a), Reg::from(b), Reg::from(c), $key) { pc + 1 } else { imm as usize };
                        if self.ids.has_pending() {
                            pending_at!(next, pc);
                        }
                        pc = next;
                    }};
                }
                // `!x` (`$not`) and a condition made a value, as `truth`
                // decides (NAN and an undefined variable warn: `step`).
                macro_rules! truth {
                    ($not:expr) => {{
                        ops!(a, b, c, imm);
                        let v = &w[rx!(b)];
                        let t = match v.as_bool() {
                            Some(b) => b,
                            None => match truth(v) {
                                Some(t) => t,
                                None => slow!(),
                            },
                        };
                        done!(put(&mut w[rx!(a)], Value::bool(t != $not)));
                    }};
                }
                // `dispatch!`: one match of these hand arms and the generated
                // instructions' (ops.def, vmops.rs), so that every opcode is
                // one jump table away.
                dispatch!(op, insn, {
                    Op::Slow => slow!(),
                    // Function entry, when every passed argument is plainly of
                    // its declared type (`verify_args` would accept each as it
                    // is); a conversion, a class check or a variadic: `step`.
                    Op::VerifyArgs => {
                        if !args_plain(clo, &w[..], top.argc as usize) {
                            slow!();
                        }
                        pc += 1;
                    }
                    // A return value plainly of the declared type (anything
                    // else, a generator's included: `step`).
                    Op::VerifyReturn => match &clo.code.ret_type {
                        Some(ty) if clo.code.is_generator || !ClassMemo::quick(clo.type_memo.last(), ty, &w[rx!(insn.a)]) => slow!(),
                        _ => pc += 1,
                    },
                    // A builtin PHP compiles to an opcode (strlen, count, the
                    // type tests, array_key_exists): the plain case here, once
                    // the site resolved to the builtin (the first run, and a
                    // disabled function, are `step`'s); the arguments are
                    // consumed as a call's are. Anything else is the call.
                    // (The kinds are numbered as `insn::INTRINSICS`.)
                    Op::Intrinsic => {
                        ops!(a, b, c, imm);
                        let (dst, args, argc) = (a, rx!(b), c);
                        if !matches!(clo.sites[(imm & 0xffff) as usize].get(), Some(Callee::Builtin(_))) {
                            slow!();
                        }
                        let x = &w[args];
                        let r = match imm >> 16 {
                            0 => match x.as_str() {
                                Some(s) => Value::int(s.len() as i64),
                                None => slow!(),
                            },
                            1 => match x.as_array() {
                                Some(arr) => Value::int(arr.len() as i64),
                                None => slow!(),
                            },
                            9 => {
                                let Some(arr) = w[rk!(args + 1)].as_array() else { slow!() };
                                if let Some(k) = x.as_int() {
                                    Value::bool(arr.contains_key(&Key::Int(k)))
                                } else if let Some(s) = x.as_str() {
                                    Value::bool(arr.get_str(s).is_some())
                                } else {
                                    slow!()
                                }
                            }
                            _ if x.is_undef() || x.is_reference() => slow!(),
                            2 => Value::bool(x.is_null()),
                            3 => Value::bool(x.is_bool()),
                            4 => Value::bool(x.is_int()),
                            5 => Value::bool(x.is_float()),
                            6 => Value::bool(x.is_str()),
                            7 => Value::bool(x.is_array()),
                            8 => Value::bool(x.as_object().is_some_and(|o| o.class.flags & p8_runtime::object::CLASS_RESOURCE == 0)),
                            _ => slow!(),
                        };
                        // The arguments consumed may have been an object's last
                        // holder (`count([new D])`): a destructor due runs here.
                        let mut released = false;
                        for i in 0..usize::from(argc) {
                            released |= clear(&mut w[rk!(args + i)]);
                        }
                        released |= put(&mut w[rx!(dst)], r);
                        done!(released);
                    }
                    // `$o->m($v)`: an argument of the user method the call
                    // site's cache names for this class and scope, copied
                    // when it takes it by value, the variable made a
                    // reference when it takes it by reference (a miss, an
                    // undefined variable: `step`, which looks the method up
                    // by name).
                    Op::SendVarMethod => {
                        ops!(a, b, c, imm);
                        let (obj, arg, var, method, n) = (a, b, c, (imm & 0xffff) as usize, (imm >> 16) as usize);
                        let Some(o) = w[rx!(obj)].as_object() else { slow!() };
                        let (class, scope) = (Rc::as_ptr(&o.class) as usize, top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize));
                        let by_ref = clo.methods[method].borrow().as_ref().and_then(|s| match &s.code {
                            // (Not counted when the next frame slot runs it.)
                            MethodCode::User(code) if s.class == class && s.scope == scope => {
                                self.frames.site_code(code).map(|t| self.frames.next_code(t.as_ref()).code.param_by_ref(n))
                            }
                            _ => None,
                        });
                        match by_ref {
                            Some(false) => {
                                let Some(v) = copied(&w[rx!(var)]) else { slow!() };
                                done!(put(&mut w[rx!(arg)], v));
                            }
                            Some(true) => {
                                let cell = make_ref(&mut w[..], Reg::from(var));
                                done!(put(&mut w[rx!(arg)], Value::reference(cell)));
                            }
                            None => slow!(),
                        }
                    }
                    Op::Jump => {
                        let imm = insn.imm;
                        if back!(imm) {
                            slow!();
                        }
                        pc = imm as usize;
                    }
                    // A comparison (`<=>` is `step`'s): two ints inline, the
                    // rest through `test`.
                    Op::Compare => {
                        ops!(a, b, c, imm);
                        compare!(insn::cmp(imm), a, b, &w[rx!(c)]);
                    }
                    Op::CompareC => {
                        ops!(a, b, c, imm);
                        compare!(insn::cmp(imm), a, b, cst!(imm >> 8));
                    }
                    Op::IncVar => {
                        let r = &mut w[rx!(insn.a)];
                        if let Some(i) = r.as_int_mut()
                            && *i != i64::MAX
                        {
                            *i += 1;
                            pc += 1;
                        } else if let Some(cell) = r.as_reference()
                            && step_ref(cell, i64::checked_add).is_some()
                        {
                            pc += 1;
                        } else {
                            slow!();
                        }
                    }
                    Op::IncJumpLt => step_jump!(insn.a, &w[rx!(insn.b)], insn.imm, i64::checked_add, |n, b| n < b),
                    Op::IncJumpLe => step_jump!(insn.a, &w[rx!(insn.b)], insn.imm, i64::checked_add, |n, b| n <= b),
                    Op::DecJumpGt => step_jump!(insn.a, &w[rx!(insn.b)], insn.imm, i64::checked_sub, |n, b| n > b),
                    Op::DecJumpGe => step_jump!(insn.a, &w[rx!(insn.b)], insn.imm, i64::checked_sub, |n, b| n >= b),
                    // The same against a literal bound (`$i < 10`).
                    Op::IncJumpLtC => step_jump!(insn.a, cst!(insn::wide(insn)), insn.imm, i64::checked_add, |n, b| n < b),
                    Op::IncJumpLeC => step_jump!(insn.a, cst!(insn::wide(insn)), insn.imm, i64::checked_add, |n, b| n <= b),
                    Op::DecJumpGtC => step_jump!(insn.a, cst!(insn::wide(insn)), insn.imm, i64::checked_sub, |n, b| n > b),
                    Op::DecJumpGeC => step_jump!(insn.a, cst!(insn::wide(insn)), insn.imm, i64::checked_sub, |n, b| n >= b),
                    Op::DecVar => {
                        let r = &mut w[rx!(insn.a)];
                        if let Some(i) = r.as_int_mut()
                            && *i != i64::MIN
                        {
                            *i -= 1;
                            pc += 1;
                        } else if let Some(cell) = r.as_reference()
                            && step_ref(cell, i64::checked_sub).is_some()
                        {
                            pc += 1;
                        } else {
                            slow!();
                        }
                    }
                    Op::Binary => {
                        ops!(a, b, c, imm);
                        if !arith(&mut w[..], insn::binop(imm), Reg::from(a), Reg::from(b), reg_src!(c)) {
                            slow!();
                        }
                        done!(true);
                    }
                    // `b op K`, or `K op b` (CONST_LEFT: `"prefix" . $s`).
                    Op::BinaryC => {
                        ops!(a, b, c, imm);
                        let done = if imm & insn::CONST_LEFT != 0 {
                            arith_left(&mut w[..], insn::binop(imm), Reg::from(a), cst!(imm >> 8), Reg::from(b))
                        } else {
                            arith(&mut w[..], insn::binop(imm), Reg::from(a), Reg::from(b), cst_src!(imm >> 8))
                        };
                        if !done {
                            slow!();
                        }
                        done!(true);
                    }
                    Op::Add => arith!(BinOp::Add, insn.a, insn.b, reg_src!(insn.c), i64::checked_add),
                    Op::Sub => arith!(BinOp::Sub, insn.a, insn.b, reg_src!(insn.c), i64::checked_sub),
                    Op::Mul => arith!(BinOp::Mul, insn.a, insn.b, reg_src!(insn.c), i64::checked_mul),
                    Op::AddC => arith!(BinOp::Add, insn.a, insn.b, cst_src!(insn.imm), i64::checked_add),
                    Op::SubC => arith!(BinOp::Sub, insn.a, insn.b, cst_src!(insn.imm), i64::checked_sub),
                    Op::MulC => arith!(BinOp::Mul, insn.a, insn.b, cst_src!(insn.imm), i64::checked_mul),
                    Op::ConcatN => {
                        ops!(a, b, c, imm);
                        if !concat_n(&mut w[..], Reg::from(a), Reg::from(b), c) {
                            slow!();
                        }
                        done!(true);
                    }
                    // $a[int] op= number, $a an array holding a number at
                    // that key
                    Op::AssignDimOp => {
                        ops!(a, b, c, imm);
                        if !dim_op(&mut w[..], insn::binop(imm >> 8), Reg::from(a), Reg::from(b), Reg::from(c), Reg::from(imm as u8)) {
                            slow!();
                        }
                        done!(true);
                    }
                    // The loop variable's old value may have been an object's
                    // last holder: a destructor due runs before the body, at
                    // this instruction.
                    Op::IterNext => iter_next!(false),
                    Op::IterNextKey => iter_next!(true),
                    // $var op= number, $var holding a number (two ints inline,
                    // the rest through `assign_op`); $str .= string
                    Op::AssignOp => {
                        ops!(a, b, c, imm);
                        assign_op!(insn::binop(imm), a, reg_src!(b));
                    }
                    // `$s .= 'literal'`, `$n += 1`.
                    Op::AssignOpC => {
                        ops!(a, b, c, imm);
                        assign_op!(insn::binop(imm), a, cst_src!(imm >> 8));
                    }
                    // $i++ / ++$i (and --) whose value is used, on an int
                    Op::IncDec => {
                        ops!(a, b, c, imm);
                        let Some(slot) = w[rx!(a)].as_int_mut() else { slow!() };
                        let i = *slot;
                        let Some(n) = (if imm & insn::INC != 0 { i.checked_add(1) } else { i.checked_sub(1) }) else { slow!() };
                        *slot = n;
                        done!(put(&mut w[rx!(b)], Value::int(if imm & insn::POST != 0 { i } else { n })));
                    }
                    Op::LoadNull => {
                        done!(put(&mut w[rx!(insn.a)], Value::NULL));
                    }
                    Op::Not => truth!(true),
                    Op::ToBool => truth!(false),
                    Op::IssetVar => {
                        ops!(a, b, c, imm);
                        let set = is_set(&w[rx!(b)]);
                        done!(put(&mut w[rx!(a)], Value::bool(set)));
                    }
                    // `a ?? b`: a set and not null goes to dst and jumps.
                    Op::JumpIfSet => {
                        ops!(a, b, c, imm);
                        let s = &w[rx!(a)];
                        let v = if s.is_null_or_undef() {
                            None
                        } else if let Some(cell) = s.as_reference() {
                            read_ref(cell).filter(|v| !v.is_null())
                        } else {
                            Some(s.clone())
                        };
                        match v {
                            Some(v) => {
                                if put(&mut w[rx!(b)], v) && self.ids.has_pending() {
                                    pending_at!(imm as usize, pc);
                                }
                                pc = imm as usize;
                            }
                            None => pc += 1,
                        }
                    }
                    // An array literal: the empty array, then its elements
                    // pushed or set (an int or string key; a value defined and
                    // read through a reference; anything else, a next key
                    // occupied included, is `step`'s).
                    Op::NewArray => {
                        ops!(a, b, c, imm);
                        let arr = Value::array(PhpArray::list_with_capacity(imm as usize));
                        done!(put(&mut w[rx!(a)], arr));
                    }
                    Op::ArrayPush => {
                        ops!(a, b, c, imm);
                        array_push!(a, reg_src!(b), imm & insn::TAKE != 0);
                    }
                    Op::ArrayPushC => {
                        ops!(a, b, c, imm);
                        array_push!(a, cst_src!(imm), false);
                    }
                    Op::ArraySet => {
                        ops!(a, b, c, imm);
                        array_set!(a, reg_src!(b), reg_src!(c), imm & insn::TAKE != 0);
                    }
                    // `['key' => 1]`: the key, the value or both literals.
                    Op::ArraySetC => {
                        ops!(a, b, c, imm);
                        let key = if imm & insn::KEY_CONST != 0 { cst_src!(imm & 0x3fff) } else { reg_src!(b) };
                        let val = if imm & insn::VAL_CONST != 0 { cst_src!(imm >> 14 & 0x3fff) } else { reg_src!(c) };
                        array_set!(a, key, val, imm & insn::TAKE != 0);
                    }
                    // A constant the site resolved (a deprecated one is never
                    // cached: it warns on every fetch).
                    Op::FetchConst => {
                        ops!(a, b, c, imm);
                        let Some(Callee::Constant(v)) = clo.sites[imm as usize].get() else { slow!() };
                        let v = v.clone();
                        done!(put(&mut w[rx!(a)], v));
                    }
                    // foreach over an array (a defined, non-reference
                    // register): the iterator is the array and position 0;
                    // an empty one jumps past the loop. (An object, a
                    // reference, anything that warns: `step`.)
                    Op::IterInit => {
                        ops!(a, b, c, imm);
                        let Some(arr) = w[rx!(a)].as_array() else { slow!() };
                        if arr.is_empty() {
                            pc = imm as usize;
                        } else {
                            let v = w[rx!(a)].clone();
                            let released = put(&mut w[rx!(b)], v) | put(&mut w[rk!(rx!(b) + 1)], Value::int(0));
                            done!(released);
                        }
                    }
                    // An argument a user function or builtin the call site
                    // resolved to takes by value: a copy (by reference, an
                    // unresolved site, an undefined variable: `step`).
                    Op::SendVar => {
                        ops!(a, b, c, imm);
                        let n = (imm >> 16) as usize;
                        let by_value = match clo.sites[(imm & 0xffff) as usize].get() {
                            Some(&Callee::User(f)) => !self.codes[f as usize].param_by_ref(n),
                            Some(&Callee::Builtin(i)) => !self.isolate.registry.by_ref(i, n),
                            Some(Callee::Constant(_)) => true,
                            None => false,
                        };
                        if !by_value {
                            slow!();
                        }
                        let Some(v) = copied(&w[rx!(b)]) else { slow!() };
                        done!(put(&mut w[rx!(a)], v));
                    }
                    // A call of a user function the call site resolved, with
                    // its required arguments, and no interrupt pending: its
                    // frame pushed here. A builtin it resolved: frameless when
                    // it can be (the frame and window kept), else called out of
                    // line. Anything else (a miss, a generator) is `step`'s.
                    Op::Call => {
                        ops!(a, b, c, imm);
                        let (args, n, dst) = (a, b, c);
                        match clo.sites[imm as usize].get() {
                            Some(&Callee::User(target)) => {
                                let target = &self.closures[target as usize];
                                if (n as usize) < target.code.required || target.code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                                    slow!();
                                }
                                self.frames.call(Some(target), pc + 1, base + args as usize, n as usize, (base + dst as usize) as u32);
                                enter(&mut self.frames, regs);
                                pc = 0;
                                continue 'frame;
                            }
                            Some(&Callee::Builtin(i)) => {
                                // Frameless when the builtin can be and its
                                // arguments need no conversion (and no header
                                // callback can run before output).
                                let (start, n) = (rx!(args), rx!(n));
                                let call = self.isolate.registry.call(i);
                                if let Some(run) = call.frameless
                                    && call.plain(&w[start..start + n])
                                    && (!self.header_callback_set || self.out.started)
                                {
                                    frameless!(i, run, start, n, dst, true, pc);
                                } else {
                                    pc = self.builtin_call(regs, pc, base, i, (Reg::from(args), n as u8, Reg::from(dst)))?;
                                    continue 'frame;
                                }
                            }
                            _ => slow!(),
                        }
                    }
                    // `f($x)` (`Op::CallRegs`): a builtin on the borrowed ABI
                    // reads its arguments from the variables (or constants)
                    // themselves, framelessly, and the Moves into the
                    // argument registers are not run. Anything else runs the
                    // first Move as `Op::Move` does, and goes on with the next.
                    Op::CallRegs => {
                        ops!(a, b, c, imm);
                        let (start, n) = (rx!(a), rx!(b));
                        if let Some(&Callee::Builtin(i)) = clo.sites[(imm & 0xffff) as usize].get() {
                            let call = self.isolate.registry.call(i);
                            if imm & insn::ARG_CONST != 0 {
                                // `f('literal')`: the one argument is the
                                // constant, lent from the closure's table (a
                                // copy of it: the builtin takes the slice
                                // mutably, as the moved ABI's do).
                                if let Some(run) = call.frameless
                                    && matches!(run, Imp::Lent(_))
                                    && call.plain(std::slice::from_ref(cst!(start)))
                                    && (!self.header_callback_set || self.out.started)
                                {
                                    let mut arg = [cst!(start).clone()];
                                    self.pc = pc + 2;
                                    let r = self.frameless_run(i, run, &mut arg);
                                    match r {
                                        Some(v) if self.diags.is_empty() && self.pending_errors.is_empty() => {
                                            let _ = put(&mut w[rx!(c)], v);
                                            pc += 2;
                                            if self.ids.has_pending() {
                                                pending_at!(pc, pc - 1);
                                            }
                                            top = self.frames.last().expect("a frame");
                                            clo = &*top.func;
                                            code = &clo.code.insns[..];
                                        }
                                        r => {
                                            // The argument register as the
                                            // checked path would have it.
                                            let at = rk!((imm >> 16) as usize);
                                            let _ = put(&mut w[at], arg[0].clone());
                                            pc = self.frameless_tail(regs, pc + 1, i, r, (base + at, 1, base + rx!(c), true))?;
                                            continue 'frame;
                                        }
                                    }
                                    continue;
                                }
                            } else if let Some(run) = call.frameless
                                && matches!(run, Imp::Lent(_))
                                && call.plain(&w[start..start + n])
                                && (!self.header_callback_set || self.out.started)
                            {
                                frameless!(i, run, start, n, c, false, pc + n);
                                continue;
                            }
                        }
                        if imm & insn::ARG_CONST != 0 {
                            let v = cst!(start).clone();
                            done!(put(&mut w[rk!((imm >> 16) as usize)], v));
                        } else {
                            let Some(v) = copied(&w[start]) else { slow!() };
                            done!(put(&mut w[rk!((imm >> 16) as usize)], v));
                            if self.ids.has_pending() {
                                pending!();
                            }
                        }
                    }
                    // A user method the call site's cache names for this class
                    // and scope, on an object, with its required arguments, and
                    // no interrupt pending: anything else is `method_call`'s
                    // (out of line) or `step`'s.
                    Op::MethodCall => {
                        ops!(a, b, c, imm);
                        let (dst, obj, args, method, n) = (a, b, c, (imm & 0xffff) as usize, (imm >> 16) as usize);
                        let Some(o) = w[rx!(obj)].as_object() else { slow!() };
                        let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                        let found = {
                            let site = clo.methods[method].borrow();
                            match site.as_ref() {
                                // (The code not counted when the frame slot
                                // the call reuses runs it already.)
                                Some(s) if s.class == Rc::as_ptr(&o.class) as usize && s.scope == scope && !s.is_static => match &s.code {
                                    MethodCode::User(code) => self.frames.site_code(code).zip(s.declaring.upgrade()),
                                    MethodCode::Native(..) => None,
                                },
                                _ => None,
                            }
                        };
                        // A native method, a class the site has not seen:
                        // the instruction whole (`method_call`), with none
                        // of `step`'s dispatch.
                        let Some((target, declaring)) = found else { cold!(method_call) };
                        let code = &self.frames.next_code(target.as_ref()).code;
                        if n < code.required || code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                            slow!();
                        }
                        let this = Rc::clone(o);
                        // A temporary (`(new A)->f()`) is released as it is used:
                        // the call's `$this` holds the object.
                        if is_temp(&clo.code, Reg::from(obj)) {
                            clear(&mut w[rx!(obj)]);
                        }
                        // (No `called`: it is `$this`'s class, `called_class`.)
                        let frame = self.frames.call(target.as_ref(), pc + 1, base + args as usize, n, (base + dst as usize) as u32);
                        frame.this = Some(this);
                        frame.scope = Some(declaring);
                        enter(&mut self.frames, regs);
                        pc = 0;
                        continue 'frame;
                    }
                    // `new C` of a class the site's cache has, whose objects
                    // need only their slots (`make_plain`): anything else
                    // (a Throwable, a constructor this scope may not call:
                    // the error) is `step`'s.
                    Op::New => {
                        ops!(a, b, c, imm);
                        let Some(info) = Self::cached_new(clo, imm as u16) else { slow!() };
                        let Some(object) = self.make_plain(&info) else { slow!() };
                        // Not destructed unless constructed (an argument may throw first).
                        object.destructed.set(true);
                        self.last_new = Some(info);
                        done!(put(&mut w[rx!(a)], Value::object(object)));
                    }
                    // The constructor `new` found: a PHP one's frame pushed
                    // here, on the arguments where they are; none, the
                    // arguments released. Anything else is `step`'s.
                    Op::Construct => {
                        ops!(a, b, c, imm);
                        let (obj, args, n) = (a, b, rx!(c));
                        let Some(object) = w[rx!(obj)].as_object() else { slow!() };
                        // Its class's NewInfo: the one its `new` left, else the
                        // kept one (a `new` in the arguments took `last_new`).
                        let info = match self.last_new.as_ref() {
                            Some(i) if Rc::ptr_eq(&i.class, &object.class) => i,
                            _ => match self.new_infos.get(&(Rc::as_ptr(&object.class) as usize)) {
                                Some(i) => i,
                                None => slow!(),
                            },
                        };
                        match &info.ctor {
                            Some(crate::instances::Ctor { func: classes::Code::User(f), declaring, .. })
                                if !f.code.is_generator && n >= f.code.required && self.stop.load(AtomicOrdering::Relaxed) == NO_INTERRUPT =>
                            {
                                let (f, declaring) = (Rc::clone(f), Rc::clone(declaring));
                                let object = Rc::clone(object);
                                self.last_new = None;
                                object.destructed.set(false);
                                // (No `called`: it is the object's class, `called_class`.)
                                let frame = self.frames.call(Some(&f), pc + 1, base + args as usize, n, NO_RET);
                                frame.this = Some(object);
                                frame.scope = Some(declaring);
                                enter(&mut self.frames, regs);
                                pc = 0;
                                continue 'frame;
                            }
                            None => {
                                object.destructed.set(false);
                                self.last_new = None;
                                let start = rx!(args);
                                let mut released = false;
                                for r in &mut w[start..start + n] {
                                    released |= clear(r);
                                }
                                done!(released);
                            }
                            Some(_) => slow!(),
                        }
                    }
                    // `new C(...$args)`: a PHP constructor's frame pushed
                    // here, the array's values (no string key: positions
                    // only) placed after the frame's registers as `step`
                    // places them (`place_args`: each read through a
                    // reference). Named arguments, no constructor, a class
                    // not seen by `new`, anything else: `step`.
                    Op::ConstructSpread => {
                        ops!(a, b, c, imm);
                        let Some(object) = w[rx!(a)].as_object() else { slow!() };
                        let info = match self.last_new.as_ref() {
                            Some(i) if Rc::ptr_eq(&i.class, &object.class) => i,
                            _ => match self.new_infos.get(&(Rc::as_ptr(&object.class) as usize)) {
                                Some(i) => i,
                                None => slow!(),
                            },
                        };
                        let Some(crate::instances::Ctor { func: classes::Code::User(f), declaring, .. }) = &info.ctor else { slow!() };
                        let Some(list) = w[rx!(b)].as_array() else { slow!() };
                        let n = list.len();
                        if f.code.is_generator || n < f.code.required || n > usize::from(u8::MAX) || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT || !spread_plain(list) {
                            slow!();
                        }
                        let (f, declaring, object, list) = (Rc::clone(f), Rc::clone(declaring), Rc::clone(object), w[rx!(b)].clone());
                        object.destructed.set(false);
                        let at = base + clo.code.nregs as usize;
                        place_spread(regs, at, list.as_array().expect("an array"));
                        drop(list);
                        // (No `called`: it is the object's class, `called_class`.)
                        let frame = self.frames.call(Some(&f), pc + 1, at, n, NO_RET);
                        frame.this = Some(object);
                        frame.scope = Some(declaring);
                        enter(&mut self.frames, regs);
                        pc = 0;
                        continue 'frame;
                    }
                    // `C::f()` the site's cache has for this class and scope:
                    // a PHP method's frame pushed here. Anything else (a
                    // property hook's `parent::$p::get()`, a native method,
                    // __callStatic (`remember_static` caches no trampoline),
                    // the first call) is `step`'s.
                    Op::StaticCall => static_call!(None),
                    // The same, with the variables the call sends itself
                    // (`C::f($x)`: the method decides by-reference, so the
                    // compiler sends none): each made a reference where the
                    // method takes one so, else copied (an undefined one,
                    // the warning: `step`).
                    Op::StaticCallVars => {
                        let Instr::StaticCall { vars, .. } = clo.code.code[pc] else { slow!() };
                        static_call!(Some(vars))
                    }
                    // A closure (`function () {..}`, `fn () =>`) of this
                    // code's lambda, bound as `make_closure` binds it; its
                    // captures follow.
                    Op::MakeClosure => {
                        ops!(a, b, c, imm);
                        let lambda = Arc::clone(&clo.code.lambdas[(imm & 0xffff) as usize]);
                        let object = self.make_closure(&lambda, imm >> 16 != 0);
                        // (The context was borrowed whole: the frame's view
                        // is read again, as after a frameless call.)
                        top = self.frames.last().expect("a frame");
                        clo = &*top.func;
                        code = &clo.code.insns[..];
                        done!(put(&mut w[rx!(a)], Value::object(object)));
                    }
                    // `...$rest`: its own argument (if passed) and those
                    // past the parameters, as a list (named arguments no
                    // parameter takes: `step`).
                    Op::CollectVariadic => {
                        if self.named_extra.is_some() {
                            slow!();
                        }
                        let var = rx!(insn.a);
                        let list = collect_variadic(&mut w[var], top.argc as usize > var, top.args());
                        done!(put(&mut w[var], Value::array(list)));
                    }
                    // `[...$src]` of an array (`unpack_array`; a Traversable,
                    // a reference among the elements: `step`).
                    Op::ArrayUnpack => {
                        ops!(a, b, c, imm);
                        let from = w[rx!(b)].clone();
                        let Some(list) = from.as_array() else { slow!() };
                        if !unpack_array(&mut w[rx!(a)], list) {
                            slow!();
                        }
                        pc += 1;
                    }
                    // `$o->p[k0].. = v` on a declared property the name's
                    // cache entry answers (`assign_prop_path`); anything
                    // else is `step`'s.
                    Op::AssignPropDim => {
                        ops!(a, b, c, imm);
                        let o = if imm & insn::PROP_PATH_THIS != 0 {
                            let Some(o) = &top.this else { slow!() };
                            o
                        } else {
                            let Some(o) = w[rx!(a)].as_object() else { slow!() };
                            o
                        };
                        let Some(site) = clo.props.get((imm & 0xffff) as usize) else { slow!() };
                        // (Its own handle: the window is written while it is used.)
                        let o = Rc::clone(o);
                        let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                        let unused = imm & insn::PATH_UNUSED != 0;
                        let dst = rx!((imm >> 16) as u8);
                        let path = Path { keys: rx!(b), n: (imm >> 24 & 0xf) as usize, append: imm & insn::PATH_APPEND != 0 };
                        let written = assign_prop_path(&mut w[..], &o, site.get(), scope, path, rx!(c), (!unused).then_some(dst));
                        drop(o);
                        let Some(released) = written else { slow!() };
                        if unused {
                            // (What the skipped Free does: a stale value there goes now.)
                            let released = released | clear(&mut w[dst]);
                            let at = pc;
                            pc += 2;
                            if released && self.ids.has_pending() {
                                pending_at!(pc, at);
                            }
                        } else {
                            done!(released);
                        }
                    }
                    // `C::m($v)`: an argument of the method the call site's
                    // cache names for this class and scope, copied when it
                    // takes it by value, the variable made a reference when
                    // it takes it by reference (a class not resolved yet, a
                    // miss, an undefined variable: `step`).
                    Op::SendVarStatic => {
                        ops!(a, b, c, imm);
                        let (arg, var, n, method, class) = (a, b, usize::from(c), (imm & 0xffff) as usize, (imm >> 16) as u16);
                        let by_ref = self.static_target(clo, class).and_then(|t| self.cached_takes_ref(&clo.methods[method], &t, n));
                        match by_ref {
                            Some(false) => {
                                let Some(v) = copied(&w[rx!(var)]) else { slow!() };
                                done!(put(&mut w[rx!(arg)], v));
                            }
                            Some(true) => {
                                let cell = make_ref(&mut w[..], Reg::from(var));
                                done!(put(&mut w[rx!(arg)], Value::reference(cell)));
                            }
                            None => slow!(),
                        }
                    }
                    // `global $name`: the cell at the global's index the
                    // name's cache entry holds, while the main frame runs
                    // the code it was found for and the entry there has the
                    // name (ZEND_BIND_GLOBAL's cache slot and key test). A
                    // miss, a global not set (`step` makes it null), a main
                    // script's variable, a fiber running: `step`.
                    Op::BindGlobal => {
                        ops!(a, b, c, imm);
                        let Some(sites) = clo.globals.get() else { slow!() };
                        let site = sites[imm as usize].get();
                        if !self.fibers.is_empty() || site.main != Arc::as_ptr(&self.frames.slots[0].func.code) as usize {
                            slow!();
                        }
                        let Some((key, cell)) = self.globals.get_index(site.index) else { slow!() };
                        if cst!(imm).as_str().is_none_or(|n| n.as_bytes() != key.as_slice()) || cell.borrow().is_undef() {
                            slow!();
                        }
                        let cell = Rc::clone(cell);
                        done!(put(&mut w[rx!(a)], Value::reference(cell)));
                    }
                    // `$a[k0][k1].. = v` through arrays (`assign_path`);
                    // anything else on the way is `step`'s.
                    Op::AssignDimPath => {
                        ops!(a, b, c, imm);
                        let unused = imm & insn::PATH_UNUSED != 0;
                        let dst = rx!((imm >> 8) as u8);
                        let path = Path { keys: rx!(b), n: (imm & 0xff) as usize, append: imm & insn::PATH_APPEND != 0 };
                        let Some(released) = assign_path(&mut w[..], rx!(a), path, rx!(c), (!unused).then_some(dst)) else { slow!() };
                        if unused {
                            // (What the skipped Free does: a stale value there goes now.)
                            let released = released | clear(&mut w[dst]);
                            let at = pc;
                            pc += 2;
                            if released && self.ids.has_pending() {
                                pending_at!(pc, at);
                            }
                        } else {
                            done!(released);
                        }
                    }
                    // `unset($a[k0]..)` through arrays (`unset_dim`);
                    // anything else on the way is `step`'s.
                    Op::UnsetDim => {
                        ops!(a, b, c, imm);
                        let Some(released) = unset_dim(&mut w[..], rx!(a), rx!(b), imm as usize) else { slow!() };
                        done!(released);
                    }
                    // `$a[k0]..++` on an int through arrays (`dim_inc_dec`);
                    // anything else is `step`'s.
                    Op::DimIncDec => {
                        ops!(a, b, c, imm);
                        let Some(v) = dim_inc_dec(&mut w[..], rx!(a), rx!(b), imm) else { slow!() };
                        done!(put(&mut w[rx!(c)], v));
                    }
                    // `(int)`, `(float)`, `(string)`, `(bool)` of a value
                    // that converts with nothing to report (`cast`).
                    Op::Cast => {
                        ops!(a, b, c, imm);
                        let Some(v) = cast(&w[rx!(b)], imm) else { slow!() };
                        done!(put(&mut w[rx!(a)], v));
                    }
                    // `$o->p++` on an int in the declared slot the name's
                    // cache entry answers (`prop_inc_dec`); anything else is
                    // `step`'s.
                    Op::PropIncDec => {
                        ops!(a, b, c, imm);
                        let o = if imm & insn::PROP_PATH_THIS != 0 {
                            let Some(o) = &top.this else { slow!() };
                            o
                        } else {
                            let Some(o) = w[rx!(a)].as_object() else { slow!() };
                            o
                        };
                        let Some(site) = clo.props.get((imm & 0xffff) as usize) else { slow!() };
                        let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                        let Some(v) = prop_inc_dec(o, site.get(), scope, imm) else { slow!() };
                        done!(put(&mut w[rx!(b)], v));
                    }
                    // Whether `$o->m(..)` takes argument c by reference, when
                    // the call site's cache knows the method for this class
                    // and scope (`cached_takes_ref`); else `step`'s lookup.
                    Op::MethodArgByRef => {
                        ops!(a, b, c, imm);
                        let Some(o) = w[rx!(b)].as_object() else { slow!() };
                        let Some(by_ref) = self.cached_takes_ref(&clo.methods[imm as usize], &o.class, usize::from(c)) else { slow!() };
                        done!(put(&mut w[rx!(a)], Value::bool(by_ref)));
                    }
                    // `return;` (`ret!`; the main frame's and a generator's:
                    // `step`).
                    Op::ReturnNull => {
                        if self.frames.len() == self.frames.floor + 1 && (self.frames.floor == 0 || clo.code.is_generator) {
                            slow!();
                        }
                        ret!(Value::NULL);
                    }
                })
            }
        }
    }
}

/// The fixed window: every frame's first this many registers, which the
/// loop's `u8` operands index with no bounds check (`Context::execute`).
/// The register file is kept this far ahead of every frame's base.
pub(crate) const WINDOW: usize = 256;

const _: () = assert!(WINDOW == u8::MAX as usize + 1, "a u8 operand indexes the window");

/// What `slow` leaves the loop to do: go on at a pc, or the run is over
/// (the bottom frame returned, or a fiber suspended).
pub(crate) enum Flow {
    Go(usize),
    Done,
}

/// Whether the first `argc` registers of `w` are each plainly of their
/// parameter's declared type (`TypeDecl::quick`, an object's class test
/// through the closure's memo), so `verify_args` has nothing to check or
/// convert. A reference is tested by its value (`verify_args` checks
/// that); a typed variadic by each element of its list (CollectVariadic
/// made it), as ZEND_RECV_VARIADIC checks them.
#[inline]
pub(crate) fn args_plain(clo: &Closure, w: &[Value], argc: usize) -> bool {
    clo.code.params.iter().zip(w).enumerate().all(|(i, (p, v))| match &p.ty {
        None => true,
        Some(ty) if p.variadic => variadic_plain(ty, v),
        Some(_) if i >= argc || v.is_undef() => true,
        Some(ty) => plain_arg(clo.type_memo.get(i), ty, v),
    })
}

/// The variables a call sends itself (`vars`, its `Function::arg_vars`
/// entry) into its argument registers from `args`, for `target`: a
/// variable is made a reference where it takes the parameter by
/// reference, else copied (`send_vars`). None, with nothing sent, when a
/// copied one is undefined (the warning: `step`); else whether a write
/// released memory.
#[inline(never)]
fn send_vars(w: &mut [Value], vars: &[(u8, Reg)], args: usize, target: &Function) -> Option<bool> {
    let defined = |v: &Value| !v.is_undef() && v.as_reference().is_none_or(|c| !c.borrow().is_undef());
    if vars.iter().any(|&(n, var)| !target.param_by_ref(usize::from(n)) && !defined(&w[usize::from(var)])) {
        return None;
    }
    let mut released = false;
    for &(n, var) in vars {
        let v = if target.param_by_ref(usize::from(n)) { Value::reference(make_ref(w, var)) } else { copied(&w[usize::from(var)]).expect("defined") };
        released |= put(&mut w[(args + usize::from(n)) & (WINDOW - 1)], v);
    }
    Some(released)
}

/// A variadic parameter's list (`CollectVariadic`): its own argument
/// (moved out of `own`, when `passed`), then the arguments past the
/// parameters, copied; made once, at its size.
#[inline(never)]
fn collect_variadic(own: &mut Value, passed: bool, extra: &[Value]) -> PhpArray {
    let mut list = p8_runtime::array::values_buffer(usize::from(passed) + extra.len());
    if passed {
        list.push(std::mem::replace(own, Value::UNDEF));
    }
    list.extend(extra.iter().cloned());
    PhpArray::from_list(list)
}

/// `[...$from]` into the array `to` (`unpack_into`): string keys kept,
/// int keys appended. False, with nothing done, when `to` is no array, an
/// element is a reference (whether it stays one is `step`'s: a reference
/// held elsewhere too does) or the appends could run out of keys (the
/// error).
#[inline(never)]
fn unpack_array(to: &mut Value, from: &PhpArray) -> bool {
    if !to.is_array() || from.values().any(Value::is_reference) {
        return false;
    }
    let to = to.array_mut().expect("an array");
    if to.next_key().and_then(|k| k.checked_add(from.len() as i64)).is_none() {
        return false;
    }
    to.reserve(from.len());
    for (k, v) in from.iter() {
        match k {
            Key::Str(s) => to.insert(Key::Str(s), v.clone()),
            Key::Int(_) => to.push(v.clone()).expect("room checked above"),
        }
    }
    true
}

/// Whether `new C(...$list)` passes `list`'s values by position alone (no
/// string key: `place_args` then reads each through its reference).
#[inline(never)]
fn spread_plain(list: &PhpArray) -> bool {
    list.iter().all(|(k, _)| matches!(k, Key::Int(_)))
}

/// The values of a spread's list as arguments from register `at`, as
/// `place_args` gives them (read through references), the register file
/// grown to hold them.
#[inline(never)]
fn place_spread(regs: &mut Vec<Value>, at: usize, list: &PhpArray) {
    if regs.len() < at + list.len() + 1 {
        regs.resize(at + list.len() + 1, Value::UNDEF);
    }
    for (r, v) in regs[at..].iter_mut().zip(list.values()) {
        *r = v.deref();
    }
}

/// The keys of a path write (`Op::AssignDimPath`, `Op::AssignPropDim`):
/// the `n` registers from `keys`, then `[]` when `append`.
#[derive(Clone, Copy)]
pub(crate) struct Path {
    pub(crate) keys: usize,
    pub(crate) n: usize,
    pub(crate) append: bool,
}

/// `$var[k0]..[kn-1] = $val` (or `..[] = $val`: `Op::AssignDimPath`),
/// `$var` an array, each key an int or a string: each level separated
/// (copy-on-write) and written in place, a missing or null one made an
/// empty array, as `update_path` walks it; the value (and, with a `dst`,
/// the result) a copy. None, with nothing written, when a key is of
/// another type (or is `$var` itself), the value is undefined, a level is
/// anything but an array or null (a reference, an object, a string,
/// false), or the next key to append at is taken: `step`'s, which may
/// only find the levels before it separated. Else whether a write
/// released memory.
#[inline(never)]
fn assign_path(w: &mut [Value], var: usize, p: Path, val: usize, dst: Option<usize>) -> Option<bool> {
    let path = p.keys..p.keys + p.n;
    if path.contains(&var) || !w[var].is_array() || !w[path.clone()].iter().all(|k| k.is_int() || k.is_str()) {
        return None;
    }
    let v = copied(&w[val])?;
    // The root out of its register while it is written (nothing else
    // reads it meanwhile), then back.
    let mut root = std::mem::take(&mut w[var]);
    let written = write_path(&mut root, &w[path], p.append, v.clone());
    w[var] = root;
    let released = written?;
    Some(match dst {
        Some(dst) => released | put(&mut w[dst], v),
        None => released,
    })
}

/// `$o->p[k0]..[kn-1] = $val` (or `..[] = $val`: `Op::AssignPropDim`),
/// each key an int or a string: the declared slot
/// the name's cache entry `site` holds for this class and `scope`
/// (`cached_slot`: visible, the object not lazy), of a class with no
/// hooked property, the property neither readonly nor of a narrower set
/// visibility, holding an array (or null, when it is untyped), walked as
/// `assign_path` walks a variable (`update_slot_dim`'s write). None, with
/// nothing written but levels separated, for anything else (`step`'s).
#[inline(never)]
fn assign_prop_path(w: &mut [Value], o: &Rc<Object>, site: PropSite, scope: usize, path: Path, val: usize, dst: Option<usize>) -> Option<bool> {
    let slot = cached_slot(site, o, scope)?;
    let p = &o.class.properties[slot];
    if o.class.flags & p8_runtime::object::CLASS_HOOKS != 0 || p.readonly || p.set_visibility.is_some() {
        return None;
    }
    let keys = &w[path.keys..path.keys + path.n];
    if !keys.iter().all(|k| k.is_int() || k.is_str()) {
        return None;
    }
    let v = copied(&w[val])?;
    let cell = &o.slots()[slot];
    let mut root = cell.replace(Value::UNDEF);
    let written = if root.is_array() || (root.is_null() && p.ty.is_none()) { write_path(&mut root, keys, path.append, v.clone()) } else { None };
    p8_runtime::object::put_slot(cell, root);
    let released = written?;
    Some(match dst {
        Some(dst) => released | put(&mut w[dst], v),
        None => released,
    })
}

/// `assign_path`'s walk below the root: every key a level when `append`
/// (the value pushed onto the last), else all but the last.
fn write_path(root: &mut Value, keys: &[Value], append: bool, v: Value) -> Option<bool> {
    let key = |k: &Value| match k.as_int() {
        Some(i) => Key::Int(i),
        None => Key::from_string(k.cloned_str().expect("a string key")),
    };
    let (last, levels) = if append { (None, keys) } else { keys.split_last().map(|(l, rest)| (Some(l), rest))? };
    let mut cur = root;
    for k in levels {
        if cur.is_null() {
            *cur = Value::array(PhpArray::new());
        }
        cur = cur.array_mut()?.slot_mut(&key(k));
    }
    if cur.is_null() {
        *cur = Value::array(PhpArray::new());
    }
    let arr = cur.array_mut()?;
    let Some(last) = last else {
        // (A taken next key: the error, `step`'s.)
        arr.push(v).ok()?;
        return Some(false);
    };
    let key = key(last);
    match arr.get_mut(&key) {
        Some(e) if e.is_reference() => None,
        Some(e) => Some(put(e, v)),
        None => {
            arr.insert(key, v);
            Some(false)
        }
    }
}

/// An int or string key (`assign_path`'s, `unset_dim`'s) as the array
/// keys it.
fn path_key(k: &Value) -> Key {
    match k.as_int() {
        Some(i) => Key::Int(i),
        None => Key::from_string(k.cloned_str().expect("a string key")),
    }
}

/// The root of a path through arrays (`unset_dim`, `dim_inc_dec`): the
/// array in register `var` or in the reference it holds, given to `f`
/// with the keys, the `n` registers from `keys`, each an int or a string
/// (not `var` itself). None when the root is not an array or a key is of
/// another type.
#[inline(always)]
fn on_path<R>(w: &mut [Value], var: usize, keys: usize, n: usize, f: impl FnOnce(&mut Value, &[Value]) -> Option<R>) -> Option<R> {
    let path = keys..keys + n;
    if path.contains(&var) || !w[path.clone()].iter().all(|k| k.is_int() || k.is_str()) {
        return None;
    }
    if let Some(cell) = w[var].as_reference() {
        // (The keys are no references: the cell is not theirs.)
        let cell = Rc::clone(cell);
        let mut root = cell.borrow_mut();
        return if root.is_array() { f(&mut root, &w[path]) } else { None };
    }
    if !w[var].is_array() {
        return None;
    }
    // The root out of its register while it changes, then back.
    let mut root = std::mem::take(&mut w[var]);
    let r = f(&mut root, &w[path]);
    w[var] = root;
    r
}

/// `unset($var[k0]..[kn-1])` (`Op::UnsetDim`), `$var` an array (or a
/// reference to one), each key an int or a string: as `unset_path` walks
/// it, each level that has the key separated (copy-on-write) and gone
/// into, the element at the last key removed; a missing key anywhere is
/// nothing to do. None, with nothing removed, when a key is of another
/// type or a level on the way is anything but an array (a reference,
/// null, an object, a string): `step`'s, which may find the levels before
/// it separated. Else whether an element was removed (its value let go).
#[inline(never)]
fn unset_dim(w: &mut [Value], var: usize, keys: usize, n: usize) -> Option<bool> {
    on_path(w, var, keys, n, |root, keys| {
        let (last, levels) = keys.split_last()?;
        let mut cur = root;
        for k in levels {
            let k = path_key(k);
            if !cur.as_array()?.contains_key(&k) {
                return Some(false);
            }
            cur = cur.array_mut()?.get_mut(&k)?;
            if !cur.is_array() {
                return None;
            }
        }
        let k = path_key(last);
        if !cur.as_array()?.contains_key(&k) {
            return Some(false);
        }
        Some(cur.array_mut()?.remove(&k).is_some())
    })
}

/// `$var[k0]..[kn-1]++` (and `--`, `++$var[..]`: `Op::DimIncDec`, imm's
/// DIM_INC and DIM_POST), `$var` an array (or a reference to one), each
/// key an int or a string: each level an array that has the key,
/// separated (copy-on-write), as `incdec_path` walks it, the element at
/// the last an int stepped in place with no overflow. The value of the
/// expression (the old one for a post step). None for anything else (a
/// missing key: the warning; a reference, a float, a string, null:
/// `step`'s, which may find the levels before it separated).
#[inline(never)]
fn dim_inc_dec(w: &mut [Value], var: usize, keys: usize, imm: u32) -> Option<Value> {
    on_path(w, var, keys, (imm & 0xff) as usize, |root, keys| {
        let mut cur = root;
        for k in keys {
            cur = cur.array_mut()?.get_mut(&path_key(k))?;
        }
        let slot = cur.as_int_mut()?;
        let old = *slot;
        let new = if imm & insn::DIM_INC != 0 { old.checked_add(1) } else { old.checked_sub(1) }?;
        *slot = new;
        Some(Value::int(if imm & insn::DIM_POST != 0 { old } else { new }))
    })
}

/// `(int)`, `(float)`, `(string)`, `(bool)` (`Op::Cast`, imm the kind) of
/// `v` (a reference's value): null, a bool, an int, a float, a string,
/// and an array but to a string, converted as `step` converts them, when
/// that reports nothing. None for anything else: an undefined variable,
/// an object (its cast handler, `__toString`, the warning), a float out
/// of an int's range, NAN to a bool, an array to a string.
#[inline(never)]
fn cast(v: &Value, kind: u32) -> Option<Value> {
    if let Some(cell) = v.as_reference() {
        return cast(&cell.borrow(), kind);
    }
    if !(v.is_null() || v.as_bool().is_some() || v.is_int() || v.is_float() || v.is_str() || v.is_array()) {
        return None;
    }
    match kind {
        insn::CAST_INT => {
            if let Some(f) = v.as_float()
                && ops::float_to_int_cast_diag(f).is_some()
            {
                return None;
            }
            Some(Value::int(v.to_int()))
        }
        insn::CAST_FLOAT => Some(Value::float(v.to_float())),
        insn::CAST_BOOL => (!v.is_nan()).then(|| Value::bool(v.to_bool())),
        _ => {
            if v.is_str() {
                return Some(v.clone());
            }
            let mut diags = ops::Diags::new();
            let s = ops::to_string_lossy(v, &mut diags);
            diags.is_empty().then(|| Value::str(s))
        }
    }
}

/// `$o->p++` (and `--`, `++$o->p`: `Op::PropIncDec`, imm's PROP_INC and
/// PROP_POST): the declared slot the name's cache entry `site` holds for
/// this class and `scope` (`cached_slot`: visible, the object not lazy),
/// of a class with no hooked property, the property neither readonly nor
/// of a narrower set visibility, holding an int (not a reference) that
/// steps with no overflow to a value of its type: written in place. The
/// value of the expression. None for anything else (`step`'s: magic,
/// hooks, a float, null, a string, the errors).
#[inline(never)]
fn prop_inc_dec(o: &Rc<Object>, site: PropSite, scope: usize, imm: u32) -> Option<Value> {
    let slot = cached_slot(site, o, scope)?;
    let p = &o.class.properties[slot];
    if o.class.flags & p8_runtime::object::CLASS_HOOKS != 0 || p.readonly || p.set_visibility.is_some() {
        return None;
    }
    let cell = &o.slots()[slot];
    let old = cell.peek(Value::as_int)?;
    let new = if imm & insn::PROP_INC != 0 { old.checked_add(1) } else { old.checked_sub(1) }?;
    let new = Value::int(new);
    if p.ty_decl.as_ref().is_some_and(|t| !t.quick(&new)) {
        return None;
    }
    cell.assign(new.clone());
    Some(if imm & insn::PROP_POST != 0 { Value::int(old) } else { new })
}

/// A typed variadic's list (`CollectVariadic`'s), each element plainly of
/// the type (out of line: not on a plain call's path).
#[cold]
#[inline(never)]
fn variadic_plain(ty: &p8_runtime::types::TypeDecl, v: &Value) -> bool {
    v.as_array().is_none_or(|list| list.values().all(|v| plain_arg(None, ty, v)))
}

/// One argument plainly of its type (`ClassMemo::quick`), a reference's
/// value tested in its place.
#[inline(always)]
fn plain_arg(memo: Option<&ClassMemo>, ty: &p8_runtime::types::TypeDecl, v: &Value) -> bool {
    match v.as_reference() {
        Some(cell) => quick_ref(memo, ty, cell),
        None => ClassMemo::quick(memo, ty, v),
    }
}

#[cold]
#[inline(never)]
fn quick_ref(memo: Option<&ClassMemo>, ty: &p8_runtime::types::TypeDecl, cell: &RefCell<Value>) -> bool {
    ClassMemo::quick(memo, ty, &cell.borrow())
}

/// Set up the registers of the frame just pushed: its window starts at
/// its base, with its arguments already in the first registers.
#[inline(always)]
pub(crate) fn enter(frames: &mut Frames, regs: &mut Vec<Value>) {
    let top = frames.len - 1;
    let (slots, spare) = (&mut frames.slots, &mut frames.spare);
    let frame = &mut slots[top];
    let (func, base, argc) = (&frame.func, frame.base as usize, frame.argc as usize);
    let code = &*func.code;
    // A window's worth at least: the loop's fixed window (WINDOW) must exist.
    let need = base + usize::from(code.nregs).max(WINDOW);
    if regs.len() < need {
        regs.resize(need, Value::UNDEF);
    }
    let nparams = code.params.len();
    if argc > nparams {
        frame.extra = take_extra(spare, regs, base, nparams, argc);
    }
    // The variables not passed are undefined (PHP initialises its CVs).
    // Constants are read in place (`const_index`): none is copied in.
    for r in &mut regs[base + argc.min(nparams)..base + code.ncv as usize] {
        clear(r);
    }
    // Arguments not passed: their constant defaults.
    if argc < nparams && !func.defaults.is_empty() {
        fill_defaults(&func.defaults, &mut regs[base..base + nparams], argc);
    }
}

/// The value `return` hands back from register `src` of `frame` (at
/// `base`); None when it is undefined (`step` warns) or a constant
/// operand (`step` reads it in place). The frame ends with the return,
/// so the value is moved out of its register (no copy, and no drop of
/// the register's copy at `leave`) unless something still reads the
/// register: a reference's target, or a variable of a frame whose
/// variables are not its own (the global scope's, or a file's included in
/// a function, given back at `leave`).
#[inline(always)]
pub(crate) fn returned(regs: &mut [Value], frame: &Frame, base: usize, src: Reg) -> Option<Value> {
    if const_index(src).is_some() {
        return None;
    }
    let code = &frame.func.code;
    let at = src as usize;
    let temp = at >= code.ncv as usize;
    let own_var = !temp && !frame.global && frame.extra.is_none();
    let r = &mut regs[base + at];
    if r.is_undef() {
        None
    } else if let Some(cell) = r.as_reference() {
        let v = cell.borrow().clone();
        // A temporary lets go of the reference (none is left at exit).
        if temp {
            drop(std::mem::take(r));
        }
        Some(v)
    } else if temp || own_var {
        Some(std::mem::take(r))
    } else {
        Some(r.clone())
    }
}

/// The constant defaults (`Closure::defaults`) of the parameters from
/// `from` on whose registers (`params`, the frame's first) are empty.
#[inline(never)]
pub(crate) fn fill_defaults(defaults: &[Option<Value>], params: &mut [Value], from: usize) {
    for (r, d) in params.iter_mut().zip(defaults.iter()).skip(from) {
        if let Some(d) = d
            && r.is_undef()
        {
            *r = d.clone();
        }
    }
}

/// Whether an ini value turns a flag on ("1", "on", "yes", "true").
pub(crate) fn ini_on(v: &Value) -> bool {
    match v.as_str() {
        Some(s) => matches!(s.as_bytes().to_ascii_lowercase().as_slice(), b"1" | b"on" | b"true" | b"yes"),
        None => v.to_bool(),
    }
}

/// Pop the current frame, putting `value` in the caller's result
/// register; the caller (if any) resumes at its saved pc.
#[inline(always)]
pub(crate) fn leave(frames: &mut Frames, regs: &mut [Value], value: Value) {
    let n = frames.len - 1;
    // A frame with an Extra (arguments past its parameters, variables
    // named at run time, an included file's shared scope): one test.
    if frames.slots[n].extra.is_some() {
        give_back_shared(frames, regs);
        if let Some(extra) = frames.slots[n].extra.take() {
            crate::frames::recycle_extra(&mut frames.spare, extra);
        }
    }
    frames.len = n;
    let frame = &mut frames.slots[n];
    // Locals die with the frame (before the result lands: the result
    // register may be inside the window).
    let base = frame.base as usize;
    let code = &frame.func.code;
    // The variables, and the temporaries unless the code frees each
    // where it dies (`frees_temps`), as PHP's are: then they are empty.
    let end = base + if code.frees_temps { code.ncv as usize } else { usize::from(code.nregs) };
    for r in &mut regs[base..end] {
        clear(r);
    }
    // `$this` goes after the arguments, as in PHP (it decides which
    // object id is free first).
    if frame.scope.is_some() {
        frame.this = None;
        frame.scope = None;
        frame.called = None;
    }
    // The Closure it was called as (a generator's frame gave its
    // Generator back before it got here).
    if frame.owner.is_some() {
        frame.owner = None;
    }
    if frame.ret != NO_RET {
        // (The Return arm tests pending after `leave`.)
        let _ = put(&mut regs[frame.ret as usize], value);
    }
}

/// Whether a value holds nothing to release (a scalar, null, undefined):
/// overwriting it is a store. (The scalars are listed, so any value
/// holding memory is released.)
#[inline(always)]
pub(crate) fn is_plain(v: &Value) -> bool {
    v.is_plain()
}

/// Write `v` over a register's value: the old value's tag is tested
/// here, and only one holding memory is released (PHP's
/// zval_ptr_dtor_nogc: `Z_REFCOUNTED`, then `GC_DELREF`): its count's
/// decrement inline, the free out of line for the last handle only
/// (`Value::release`).
///
/// Returns whether the release freed the last handle of something that
/// may hold an object with a destructor, which the loop runs before the
/// next instruction (`done!`), at this instruction's pc, as PHP runs it
/// where the value went. A plain old value, or a decrement that freed
/// nothing, cannot have, so that test is on the freeing path only.
#[inline(always)]
#[must_use = "a released value may leave a destructor due: test `ids.has_pending()`"]
pub(crate) fn put(slot: &mut Value, v: Value) -> bool {
    // The tag is tested in the slot, so the plain case reads one byte
    // and stores; only the other case reads the old value out, for `release`.
    if is_plain(slot) {
        std::mem::forget(std::mem::replace(slot, v));
        false
    } else {
        std::mem::replace(slot, v).release()
    }
}

/// `put` into a variable: through the reference it holds, if it is one.
/// The plain test comes first (a plain value is never a reference), so
/// the common store is the one tag compare `put` makes.
#[inline(always)]
#[must_use = "a released value may leave a destructor due: test `ids.has_pending()`"]
pub(crate) fn put_var(slot: &mut Value, v: Value) -> bool {
    if is_plain(slot) {
        std::mem::forget(std::mem::replace(slot, v));
        return false;
    }
    match slot.as_reference() {
        Some(cell) => write_ref(cell, v),
        None => std::mem::replace(slot, v).release(),
    }
}

/// Write through a reference. Out of line, and cold to the loop so that
/// the plain store is each arm's straight path: common (`global $x`),
/// not hot. (The cold helpers below are all the inner loop's branches
/// off its inline cases.)
#[cold]
#[inline(never)]
pub(crate) fn write_ref(cell: &RefCell<Value>, v: Value) -> bool {
    put(&mut cell.borrow_mut(), v)
}

/// A reference's value, for a read: None when it is undefined (a global
/// another file unset: `step` warns).
#[cold]
#[inline(never)]
pub(crate) fn read_ref(cell: &RefCell<Value>) -> Option<Value> {
    let v = cell.borrow();
    if v.is_undef() { None } else { Some(v.clone()) }
}

/// Make a register undefined, releasing what it held.
#[inline(always)]
pub(crate) fn clear(r: &mut Value) -> bool {
    put(r, Value::UNDEF)
}

/// The right operand of a two-operand arm: a register of the window, or a
/// constant read in place (the `Op::*C` forms), for the helpers that
/// serve both.
#[derive(Clone, Copy)]
pub(crate) enum Src<'c> {
    Reg(Reg),
    Const(&'c Value),
}

/// The operand's value.
#[inline(always)]
pub(crate) fn src<'w, 'c: 'w>(w: &'w [Value], s: Src<'c>) -> &'w Value {
    match s {
        Src::Reg(r) => &w[usize::from(r)],
        Src::Const(c) => c,
    }
}

/// An operand to store (`operand_take`): moved out of its register when
/// `take` (a temporary read for the last time), else copied; a reference's
/// value is read through it (the taken register lets go of the
/// reference); a constant is copied. None when undefined (`step` warns).
#[inline(always)]
fn taken(w: &mut [Value], r: Src, take: bool) -> Option<Value> {
    let r = match r {
        Src::Reg(r) => r,
        Src::Const(c) => return Some(c.clone()),
    };
    let slot = &w[usize::from(r)];
    if slot.is_undef() {
        return None;
    }
    let v = match slot.as_reference() {
        Some(cell) => read_ref(cell)?,
        None if take => return Some(std::mem::take(&mut w[usize::from(r)])),
        None => return Some(slot.clone()),
    };
    if take {
        clear(&mut w[usize::from(r)]);
    }
    Some(v)
}

/// isset's test of an element: there and not null (through a reference).
#[inline(always)]
pub(crate) fn is_set(v: &Value) -> bool {
    if v.is_null_or_undef() {
        false
    } else if let Some(cell) = v.as_reference() {
        !cell.borrow().is_null_or_undef()
    } else {
        true
    }
}

/// An int, through a reference.
#[inline(always)]
fn int_of(v: &Value) -> Option<i64> {
    if let Some(i) = v.as_int() {
        Some(i)
    } else if let Some(cell) = v.as_reference() {
        int_in(cell)
    } else {
        None
    }
}

#[cold]
#[inline(never)]
fn int_in(cell: &RefCell<Value>) -> Option<i64> {
    cell.borrow().as_int()
}

/// A number (an int or a float), through a reference.
#[inline(always)]
fn num_of(v: &Value) -> Option<Value> {
    if v.is_number() {
        Some(v.clone())
    } else if let Some(cell) = v.as_reference() {
        let n = cell.borrow();
        n.is_number().then(|| n.clone())
    } else {
        None
    }
}

/// `++`/`--` of an int through a reference: the new value (None: not an
/// int there, or it overflows: `step`).
#[cold]
#[inline(never)]
fn step_ref(cell: &RefCell<Value>, step: fn(i64, i64) -> Option<i64>) -> Option<i64> {
    let mut v = cell.borrow_mut();
    let i = v.as_int_mut()?;
    let n = step(*i, 1)?;
    *i = n;
    Some(n)
}

/// `a cmp b` for anything but two ints: through references, numbers as
/// `num_test`, and every other pair of defined values as `step` compares
/// them (`test_values`: nothing there warns). None for an undefined
/// variable (`step` warns) or `<=>`.
#[cold]
#[inline(never)]
pub(crate) fn test(cmp: Cmp, x: &Value, y: &Value) -> Option<bool> {
    if cmp == Cmp::Spaceship {
        return None;
    }
    if x.is_undef() || y.is_undef() {
        None
    } else if x.is_reference() || y.is_reference() {
        let (x, y) = (copied(x)?, copied(y)?);
        Some(num_test(cmp, &x, &y).unwrap_or_else(|| test_values(cmp, &x, &y)))
    } else {
        Some(num_test(cmp, x, y).unwrap_or_else(|| test_values(cmp, x, y)))
    }
}

/// `dst = a op b` beyond two ints: floats, operands through references,
/// `.` on strings. False (nothing done) when `step` must run it: a
/// warning, an error, a conversion with side effects, an overflow.
#[cold]
#[inline(never)]
fn arith(w: &mut [Value], op: BinOp, dst: Reg, a: Reg, b: Src) -> bool {
    match num_pair(op, &w[usize::from(a)], src(w, b)) {
        Some(r) => {
            // (Reported as released whatever the old value: the arm tests pending.)
            let _ = put_var(&mut w[usize::from(dst)], r);
            true
        }
        None => op == BinOp::Concat && concat(w, dst, a, b),
    }
}

/// `arith` with the literal on the left (`K op b`: BinaryC's CONST_LEFT).
#[cold]
#[inline(never)]
fn arith_left(w: &mut [Value], op: BinOp, dst: Reg, x: &Value, b: Reg) -> bool {
    match num_pair(op, x, &w[usize::from(b)]) {
        Some(r) => {
            let _ = put_var(&mut w[usize::from(dst)], r);
            true
        }
        None => op == BinOp::Concat && concat_left(w, dst, x, b),
    }
}

/// `x op y` on two numbers, each read through a reference if it is one.
#[inline(always)]
fn num_pair(op: BinOp, x: &Value, y: &Value) -> Option<Value> {
    if x.is_reference() || y.is_reference() {
        match (num_of(x), num_of(y)) {
            (Some(x), Some(y)) => num_op(op, &x, &y),
            _ => None,
        }
    } else {
        num_op(op, x, y)
    }
}

/// `var op= src` beyond two ints: numbers (through references), `.=` on
/// strings. False when `step` must run it.
#[cold]
#[inline(never)]
fn assign_op(w: &mut [Value], op: BinOp, var: Reg, b: Src) -> bool {
    match num_pair(op, &w[usize::from(var)], src(w, b)) {
        Some(r) => {
            let _ = put_var(&mut w[usize::from(var)], r);
            true
        }
        None => op == BinOp::Concat && append_str(w, var, b),
    }
}

/// A condition that is not a bool: what i_zend_is_true says, when
/// nothing can warn (NAN, an undefined variable: None, `step`'s).
#[cold]
#[inline(never)]
pub(crate) fn truth(v: &Value) -> Option<bool> {
    fn of(v: &Value) -> Option<bool> {
        if v.is_undef() || v.as_float().is_some_and(f64::is_nan) { None } else { Some(v.to_bool()) }
    }
    match v.as_reference() {
        Some(cell) => of(&cell.borrow()),
        None => of(v),
    }
}

/// `$a[k]` beyond an array and an int key: a string key, an array held
/// through a reference; None (`step`: a warning, an object, a string's
/// offset) otherwise. The element must not be a reference.
#[cold]
#[inline(never)]
pub(crate) fn elem(w: &[Value], arr: Reg, key: Src) -> Option<Value> {
    let at = |a: &PhpArray| {
        let key = src(w, key);
        if let Some(k) = key.as_int() {
            a.get_int(k).cloned()
        } else if let Some(s) = key.as_str() {
            a.get_str(s).cloned()
        } else {
            None
        }
    };
    let arr = &w[usize::from(arr)];
    let v = if let Some(a) = arr.as_array() {
        at(a)
    } else if let Some(cell) = arr.as_reference() {
        cell.borrow().as_array().and_then(at)
    } else {
        None
    };
    v.filter(|v| !v.is_reference())
}

/// isset($a[k]) beyond an array and an int key: a string key, an array
/// held through a reference; None (`step`) otherwise.
#[cold]
#[inline(never)]
pub(crate) fn isset_elem(w: &[Value], var: Reg, keys: Src) -> Option<bool> {
    let at = |a: &PhpArray| {
        let key = src(w, keys);
        if let Some(k) = key.as_int() {
            Some(a.get_int(k).is_some_and(is_set))
        } else {
            key.as_str().map(|s| a.get_str(s).is_some_and(is_set))
        }
    };
    let var = &w[usize::from(var)];
    if let Some(a) = var.as_array() {
        at(a)
    } else if let Some(cell) = var.as_reference() {
        cell.borrow().as_array().and_then(at)
    } else {
        None
    }
}

/// `$a[k] = v` for a plain `v` into the array `slot` holds (false when it
/// holds none), unless the element is a reference (false).
#[inline(always)]
pub(crate) fn set_int_elem(slot: &mut Value, k: i64, v: &Value) -> bool {
    match slot.as_array() {
        Some(a) if !a.get_int(k).is_some_and(Value::is_reference) => {}
        _ => return false,
    }
    let a = slot.array_mut().expect("checked above");
    match a.get_int_mut(k) {
        Some(e) => *e = v.clone(),
        None => a.insert(Key::Int(k), v.clone()),
    }
    true
}

/// A copy of a register's value, as `step` makes one (strings and arrays
/// are shared until written; a reference's value is read through it),
/// unless it is undefined (a warning: `step`'s).
#[inline(always)]
pub(crate) fn copied(v: &Value) -> Option<Value> {
    if v.is_undef() {
        return None;
    }
    match v.as_reference() {
        Some(cell) => read_ref(cell),
        None => Some(v.clone()),
    }
}

/// A copy of a value that holds nothing reference-counted (and is defined).
#[inline(always)]
pub(crate) fn plain(v: &Value) -> Option<Value> {
    match v.kind() {
        Kind::Null | Kind::Bool | Kind::Int | Kind::Float => Some(v.clone()),
        _ => None,
    }
}

/// `x cmp y` for two numbers, when that is a plain numeric comparison
/// (not NAN, and not === between an int and a float).
#[inline(always)]
pub(crate) fn num_test(cmp: Cmp, x: &Value, y: &Value) -> Option<bool> {
    if let (Some(x), Some(y)) = (x.as_int(), y.as_int()) {
        return Some(int_test(cmp, x, y));
    }
    let (x, y) = match (x.as_float(), y.as_float()) {
        (Some(x), Some(y)) => (x, y),
        _ if matches!(cmp, Cmp::Identical | Cmp::NotIdentical) => return None,
        (None, Some(y)) => (x.as_int()? as f64, y),
        (Some(x), None) => (x, y.as_int()? as f64),
        (None, None) => return None,
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

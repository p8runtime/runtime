//! The interpreter loop: the common cases of the hottest instructions
//! inline, everything else through `Context::slow` (which runs `step`),
//! called from inside the loop so that it never leaves it; and the frame
//! entry and exit the loop shares with `step`.

use super::*;
use crate::vmops::{self, Ctl, Hx};
use p8_bytecode::insn::kinds;
use p8_bytecode::{Insn, Op, insn};

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
                    Hx { w: &mut *w, clo, top, reinit: !self.reinit.is_empty() }
                };
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
                        Ctl::Cold => cold!($m),
                    }
                }};
            }
            // (`dispatch.rs`'s last arm: a hand opcode, matched before it.)
            macro_rules! hand_arm {
                () => {
                    slow!()
                };
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
                match op {
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
                    // `$o->m($v)`: an argument the method the call site's cache
                    // names for this class and scope takes by value is copied
                    // (by reference, a miss, or an undefined or reference
                    // variable: `step`, which looks the method up by name).
                    Op::SendVarMethod => {
                        ops!(a, b, c, imm);
                        let (obj, arg, var, method, n) = (a, b, c, (imm & 0xffff) as usize, (imm >> 16) as usize);
                        let Some(o) = w[rx!(obj)].as_object() else { slow!() };
                        let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                        let by_value = clo.methods[method].borrow().as_ref().is_some_and(|s| {
                            s.class == Rc::as_ptr(&o.class) as usize
                                && s.scope == scope
                                && matches!(&s.code, MethodCode::User(code) if code.upgrade().is_some_and(|c| !c.code.param_by_ref(n)))
                        });
                        if !by_value {
                            slow!();
                        }
                        let Some(v) = copied(&w[rx!(var)]) else { slow!() };
                        done!(put(&mut w[rx!(arg)], v));
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
                    Op::IterNext | Op::IterNextKey => {
                        ops!(a, b, c, imm);
                        // An object (a generator, an Iterator) is `step`'s.
                        if !w[rx!(a)].is_array() {
                            slow!();
                        }
                        let next = if iter_next(&mut w[..], Reg::from(a), Reg::from(b), Reg::from(c), op == Op::IterNextKey) { pc + 1 } else { imm as usize };
                        if self.ids.has_pending() {
                            pending_at!(next, pc);
                        }
                        pc = next;
                    }
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
                    // `!x` and a condition made a value: as `truth` decides
                    // (NAN and an undefined variable warn: `step`).
                    Op::Not | Op::ToBool => {
                        ops!(a, b, c, imm);
                        let v = &w[rx!(b)];
                        let t = match v.as_bool() {
                            Some(b) => b,
                            None => match truth(v) {
                                Some(t) => t,
                                None => slow!(),
                            },
                        };
                        done!(put(&mut w[rx!(a)], Value::bool(t != (op == Op::Not))));
                    }
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
                    // `C::NAME` the site's cache holds (filled by `step`).
                    Op::ClassConst => {
                        ops!(a, b, c, imm);
                        let v = match clo.class_consts.get((imm & 0xffff) as usize).and_then(OnceCell::get) {
                            Some((class, v)) if u32::from(*class) == imm >> 16 => v.clone(),
                            _ => slow!(),
                        };
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
                                if let Some(caller) = self.frames.last_mut() {
                                    caller.pc = (pc + 1) as u32;
                                }
                                self.frames.push(target, base + args as usize, n as usize, (base + dst as usize) as u32);
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
                    // no interrupt pending: anything else is `step`'s.
                    Op::MethodCall => {
                        ops!(a, b, c, imm);
                        let (dst, obj, args, method, n) = (a, b, c, (imm & 0xffff) as usize, (imm >> 16) as usize);
                        let Some(o) = w[rx!(obj)].as_object() else { slow!() };
                        let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                        let found = {
                            let site = clo.methods[method].borrow();
                            match site.as_ref() {
                                Some(s) if s.class == Rc::as_ptr(&o.class) as usize && s.scope == scope && !s.is_static => match &s.code {
                                    MethodCode::User(code) => code.upgrade().zip(s.declaring.upgrade()),
                                    MethodCode::Native(..) => None,
                                },
                                _ => None,
                            }
                        };
                        let Some((target, declaring)) = found else { slow!() };
                        if n < target.code.required || target.code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                            slow!();
                        }
                        let this = Rc::clone(o);
                        // A temporary (`(new A)->f()`) is released as it is used:
                        // the call's `$this` holds the object.
                        if is_temp(&clo.code, Reg::from(obj)) {
                            clear(&mut w[rx!(obj)]);
                        }
                        if let Some(caller) = self.frames.last_mut() {
                            caller.pc = (pc + 1) as u32;
                        }
                        self.frames.push(&target, base + args as usize, n, (base + dst as usize) as u32);
                        let frame = self.frames.last_mut().expect("the frame just pushed");
                        frame.called = Some(Rc::clone(&this.class));
                        frame.this = Some(this);
                        frame.scope = Some(declaring);
                        enter(&mut self.frames, regs);
                        pc = 0;
                        continue 'frame;
                    }
                    // `new C` of a class the site's cache has, whose objects
                    // need only their slots (`make_plain`): anything else
                    // (a Throwable, a constructor to check) is `step`'s.
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
                                let (f, declaring, class) = (Rc::clone(f), Rc::clone(declaring), Rc::clone(&info.class));
                                let object = Rc::clone(object);
                                self.last_new = None;
                                object.destructed.set(false);
                                if let Some(caller) = self.frames.last_mut() {
                                    caller.pc = (pc + 1) as u32;
                                }
                                self.frames.push(&f, base + args as usize, n, NO_RET);
                                let frame = self.frames.last_mut().expect("the frame just pushed");
                                frame.this = Some(object);
                                frame.scope = Some(declaring);
                                frame.called = Some(class);
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
                    // `C::f()` the site's cache has for this class and scope:
                    // a PHP method's frame pushed here. Anything else (a
                    // property hook's `parent::$p::get()`, a native method,
                    // __callStatic (`remember_static` caches no trampoline),
                    // the first call) is `step`'s.
                    Op::StaticCall => {
                        ops!(a, b, c, imm);
                        let (dst, args, n, method, class) = (a, b, rx!(c), (imm & 0xffff) as usize, (imm >> 16) as u16);
                        let Some(m) = self.cached_static_call(clo, class, method) else { slow!() };
                        let classes::Code::User(target) = &m.func else { slow!() };
                        if n < target.code.required || target.code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                            slow!();
                        }
                        let target = Rc::clone(target);
                        if let Some(caller) = self.frames.last_mut() {
                            caller.pc = (pc + 1) as u32;
                        }
                        self.frames.push(&target, base + args as usize, n, (base + dst as usize) as u32);
                        let frame = self.frames.last_mut().expect("the frame just pushed");
                        frame.this = m.this;
                        frame.scope = m.scope;
                        frame.called = m.called;
                        frame.owner = m.closure;
                        enter(&mut self.frames, regs);
                        pc = 0;
                        continue 'frame;
                    }
                    // A return to the calling frame. The main frame's return
                    // ends the run, and the bottom frame of this `execute`
                    // returns to the builtin that called it: `step` does those.
                    Op::Return => {
                        if self.frames.len() == self.frames.floor + 1 {
                            slow!();
                        }
                        let Some(value) = returned(regs, top, base, Reg::from(insn.a)) else { slow!() };
                        leave(&mut self.frames, regs, value);
                        pc = self.frames.last().expect("the caller's frame").pc as usize;
                        if self.ids.has_pending() {
                            pc = self.after_errors(regs, pc, pc.saturating_sub(1))?;
                        }
                        continue 'frame;
                    }
                    // `return 'literal'`, `return true`.
                    Op::ReturnC => {
                        if self.frames.len() == self.frames.floor + 1 {
                            slow!();
                        }
                        let value = cst!(insn.imm).clone();
                        leave(&mut self.frames, regs, value);
                        pc = self.frames.last().expect("the caller's frame").pc as usize;
                        if self.ids.has_pending() {
                            pc = self.after_errors(regs, pc, pc.saturating_sub(1))?;
                        }
                        continue 'frame;
                    }
                    Op::ReturnNull => {
                        if self.frames.len() == self.frames.floor + 1 {
                            slow!();
                        }
                        leave(&mut self.frames, regs, Value::NULL);
                        pc = self.frames.last().expect("the caller's frame").pc as usize;
                        if self.ids.has_pending() {
                            pc = self.after_errors(regs, pc, pc.saturating_sub(1))?;
                        }
                        continue 'frame;
                    }
                    // The generated instructions (ops.def, vmops.rs): one arm
                    // per specialisation, written by p8-vmgen.
                    _ => include!(concat!(env!("OUT_DIR"), "/dispatch.rs")),
                }
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
/// convert. A typed variadic is left to it.
#[inline]
pub(crate) fn args_plain(clo: &Closure, w: &[Value], argc: usize) -> bool {
    clo.code.params.iter().zip(w).enumerate().all(|(i, (p, v))| match &p.ty {
        None => true,
        Some(_) if p.variadic => false,
        Some(_) if i >= argc || v.is_undef() => true,
        Some(ty) => ClassMemo::quick(clo.type_memo.get(i), ty, v),
    })
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
    give_back_shared(frames, regs);
    frames.len -= 1;
    let (slots, spare) = (&mut frames.slots, &mut frames.spare);
    let frame = &mut slots[frames.len];
    if let Some(extra) = frame.extra.take() {
        crate::frames::recycle_extra(spare, extra);
    }
    // Locals die with the frame (before the result lands: the result
    // register may be inside the window).
    let base = frame.base as usize;
    let code = &frame.func.code;
    // The variables. The temporaries are empty when the code frees each
    // where it dies (`frees_temps`), as PHP's are.
    let temps = base + code.ncv as usize;
    for r in &mut regs[base..temps] {
        clear(r);
    }
    if !code.frees_temps {
        for r in &mut regs[temps..base + usize::from(code.nregs)] {
            clear(r);
        }
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

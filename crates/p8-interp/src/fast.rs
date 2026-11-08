//! The interpreter loop: the common cases of the hottest instructions
//! inline, everything else through `Context::slow` (which runs `step`),
//! called from inside the loop so that it never leaves it; and the frame
//! entry and exit the loop shares with `step`.

use super::*;
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
            // The loop carries as few values as it can, so that they stay in
            // registers (x86-64 has six callee-saved ones, and nearly every
            // arm calls out): `pc`, the window `w`, the code slice `code`,
            // `self` and `regs`. The frame itself and its closure are read
            // from the frame stack where an arm needs them (`top!`, `clo!`,
            // `base!`: a load or two, not a value kept live across every
            // arm, which is what used to be reloaded from the stack).
            macro_rules! top {
                () => {
                    self.frames.top()
                };
            }
            macro_rules! clo {
                () => {
                    &*top!().func
                };
            }
            macro_rules! base {
                () => {
                    top!().base as usize
                };
            }
            let mut code = &clo!().code.insns[..];
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
            // An out-of-line arm (`arms.rs`) ran the instruction: go on as
            // it says. (The code slice is read again when the frame is kept:
            // it is borrowed from the frame stack, which the arm had.)
            macro_rules! arm {
                ($e:expr) => {
                    match $e {
                        Arm::Go(next) => {
                            pc = next;
                            code = &clo!().code.insns[..];
                        }
                        Arm::Pending(next) => pending_at!(next, pc),
                        Arm::Switch(next) => {
                            pc = next;
                            continue 'frame;
                        }
                        Arm::Slow => slow!(),
                        Arm::Fault => return Err(self.arm_throw.take().expect("the arm's throw")),
                    }
                };
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
            // The register file is kept a window ahead of every frame
            // (`enter`), so the frame's fixed window always exists.
            macro_rules! window {
                () => {{
                    let base = base!();
                    if regs.len() < base + WINDOW {
                        regs.resize(base + WINDOW, Value::UNDEF);
                    }
                    let w: &mut [Value; WINDOW] = (&mut regs[base..base + WINDOW]).try_into().expect("a window's worth of registers");
                    w
                }};
            }
            let mut w = window!();
            // `arm!` for an arm that had the whole register file (a call,
            // which may keep the frame: a frameless builtin's); the window is
            // read again too.
            macro_rules! arm_regs {
                ($e:expr) => {
                    match $e {
                        Arm::Go(next) => {
                            pc = next;
                            code = &clo!().code.insns[..];
                            w = window!();
                        }
                        Arm::Pending(next) => pending_at!(next, pc),
                        Arm::Switch(next) => {
                            pc = next;
                            continue 'frame;
                        }
                        Arm::Slow => slow!(),
                        Arm::Fault => return Err(self.arm_throw.take().expect("the arm's throw")),
                    }
                };
            }
            // A compare-and-branch: two ints inline, anything else (floats,
            // references, strings, null) through `test`; an undefined
            // variable (its warning) is `step`'s.
            macro_rules! jump_if {
                ($cmp:expr, $when:expr, $a:expr, $b:expr, $target:expr) => {{
                    let t = if let (Some(x), Some(y)) = (w[rx!($a)].as_int(), w[rx!($b)].as_int()) {
                        int_test($cmp, x, y)
                    } else {
                        match test(&w[..], $cmp, Reg::from($a), Reg::from($b)) {
                            Some(t) => t,
                            None => slow!(),
                        }
                    };
                    if t == $when {
                        if back!($target) {
                            slow!();
                        }
                        pc = $target as usize;
                    } else {
                        pc += 1;
                    }
                }};
            }
            // `dst = a op b`: two ints inline, the rest (floats, references,
            // `.` on strings) through `arith`; what may warn or throw is `step`'s.
            macro_rules! arith {
                ($op:expr, $dst:expr, $a:expr, $b:expr, $int:expr) => {{
                    let released = if let (Some(x), Some(y)) = (w[rx!($a)].as_int(), w[rx!($b)].as_int())
                        && let Some(n) = $int(x, y)
                    {
                        put_var(&mut w[rx!($dst)], Value::int(n))
                    } else if arith(&mut w[..], $op, Reg::from($dst), Reg::from($a), Reg::from($b)) {
                        true
                    } else {
                        slow!()
                    };
                    done!(released);
                }};
            }
            // A fused step and test (`IncJumpLt` and co.) on ints (through a
            // reference too): step, then jump or go past the test. A non-int
            // bound leaves the test to the next instruction; a non-int
            // variable (or an overflow) leaves both to `step`.
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
                    match int_of(&w[rx!($b)]) {
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
                    // `$this`, in a method that has one (else `step` raises).
                    Op::This => match &top!().this {
                        Some(this) => {
                            let this = Value::object(Rc::clone(this));
                            done!(put(&mut w[rx!(insn.a)], this));
                        }
                        None => slow!(),
                    },
                    // A property read the name's cache entry answers (anything
                    // else, `step`: a miss, an unset slot, a reference operand).
                    Op::FetchProp | Op::FetchPropThis => {
                        ops!(a, b, c, imm);
                        let Some(site) = clo!().props.get(imm as usize) else { slow!() };
                        // FetchPropThis: the method's own `$this`, from the frame.
                        let o = if op == Op::FetchPropThis {
                            let Some(o) = &top!().this else { slow!() };
                            o
                        } else {
                            let Some(o) = w[rx!(b)].as_object() else { slow!() };
                            o
                        };
                        let scope = top!().scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                        let Some(v) = cached_prop(site.get(), o, scope) else { slow!() };
                        done!(put(&mut w[rx!(a)], v));
                    }
                    // `$o->name = v` the name's cache entry answers, on a
                    // property with nothing to check (`plain_slot`): the slots
                    // borrowed once, nothing cloned but the value, and that
                    // once when the result is freed next (a statement's is).
                    // An unset slot of a class with __set and its kin, and
                    // anything `plain_slot` declines, is `step`'s.
                    Op::AssignProp | Op::AssignPropThis => arm!(self.arm_assign_prop(w, pc, *insn)),
                    // Function entry, when every passed argument is plainly of
                    // its declared type (`verify_args` would accept each as it
                    // is); a conversion, a class check or a variadic: `step`.
                    Op::VerifyArgs => arm!(self.arm_verify_args(w, pc)),
                    // A return value plainly of the declared type (anything
                    // else, a generator's included: `step`).
                    Op::VerifyReturn => match &clo!().code.ret_type {
                        Some(ty) if clo!().code.is_generator || !ClassMemo::quick(clo!().type_memo.last(), ty, &w[rx!(insn.a)]) => slow!(),
                        _ => pc += 1,
                    },
                    // A builtin PHP compiles to an opcode (strlen, count, the
                    // type tests, array_key_exists): the plain case here, once
                    // the site resolved to the builtin (the first run, and a
                    // disabled function, are `step`'s); the arguments are
                    // consumed as a call's are. Anything else is the call.
                    // (The kinds are numbered as `insn::INTRINSICS`.)
                    Op::Intrinsic => arm!(self.arm_intrinsic(w, pc, *insn)),
                    // `$o->m($v)`: an argument the method the call site's cache
                    // names for this class and scope takes by value is copied
                    // (by reference, a miss, or an undefined or reference
                    // variable: `step`, which looks the method up by name).
                    Op::SendVarMethod => arm!(self.arm_send_var_method(w, pc, *insn)),
                    Op::Jump => {
                        let imm = insn.imm;
                        if back!(imm) {
                            slow!();
                        }
                        pc = imm as usize;
                    }
                    Op::JumpLt => jump_if!(Cmp::Lt, true, insn.a, insn.b, insn.imm),
                    Op::JumpNotLt => jump_if!(Cmp::Lt, false, insn.a, insn.b, insn.imm),
                    Op::JumpLe => jump_if!(Cmp::Le, true, insn.a, insn.b, insn.imm),
                    Op::JumpNotLe => jump_if!(Cmp::Le, false, insn.a, insn.b, insn.imm),
                    Op::JumpEq => jump_if!(Cmp::Eq, true, insn.a, insn.b, insn.imm),
                    Op::JumpNotEq => jump_if!(Cmp::Eq, false, insn.a, insn.b, insn.imm),
                    Op::JumpIdentical => jump_if!(Cmp::Identical, true, insn.a, insn.b, insn.imm),
                    Op::JumpNotIdentical => jump_if!(Cmp::Identical, false, insn.a, insn.b, insn.imm),
                    // A comparison (`<=>` is `step`'s): two ints inline, the
                    // rest through `test`.
                    Op::Compare => {
                        ops!(a, b, c, imm);
                        let cmp = insn::cmp(imm);
                        let t = if let (Some(x), Some(y)) = (w[rx!(b)].as_int(), w[rx!(c)].as_int())
                            && cmp != Cmp::Spaceship
                        {
                            int_test(cmp, x, y)
                        } else if cmp == Cmp::Spaceship {
                            slow!()
                        } else {
                            match test(&w[..], cmp, Reg::from(b), Reg::from(c)) {
                                Some(t) => t,
                                None => slow!(),
                            }
                        };
                        done!(put_var(&mut w[rx!(a)], Value::bool(t)));
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
                    Op::IncJumpLt => step_jump!(insn.a, insn.b, insn.imm, i64::checked_add, |n, b| n < b),
                    Op::IncJumpLe => step_jump!(insn.a, insn.b, insn.imm, i64::checked_add, |n, b| n <= b),
                    Op::DecJumpGt => step_jump!(insn.a, insn.b, insn.imm, i64::checked_sub, |n, b| n > b),
                    Op::DecJumpGe => step_jump!(insn.a, insn.b, insn.imm, i64::checked_sub, |n, b| n >= b),
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
                    Op::Binary => arm!(self.arm_binary(w, pc, *insn)),
                    Op::Add => arith!(BinOp::Add, insn.a, insn.b, insn.c, i64::checked_add),
                    Op::Sub => arith!(BinOp::Sub, insn.a, insn.b, insn.c, i64::checked_sub),
                    Op::Mul => arith!(BinOp::Mul, insn.a, insn.b, insn.c, i64::checked_mul),
                    Op::ConcatN => arm!(self.arm_concat_n(w, pc, *insn)),
                    // A copy of a register (a reference's value read through
                    // it; undefined: `step` warns). A write that freed an object
                    // with a destructor stops here: it runs before the next
                    // instruction.
                    Op::Move => {
                        ops!(a, b, c, imm);
                        let Some(v) = copied(&w[rx!(b)]) else { slow!() };
                        done!(put(&mut w[rx!(a)], v));
                        if self.ids.has_pending() {
                            pending!();
                        }
                    }
                    Op::Assign => {
                        ops!(a, b, c, imm);
                        let Some(v) = copied(&w[rx!(b)]) else { slow!() };
                        done!(put_var(&mut w[rx!(a)], v));
                    }
                    Op::AssignTake => {
                        ops!(a, b, c, imm);
                        let (var, src) = (a, b);
                        let taken = std::mem::replace(&mut w[rx!(src)], Value::UNDEF);
                        // Undefined: `step`'s (the warning).
                        if taken.is_undef() {
                            slow!();
                        }
                        let released = if let Some(cell) = taken.as_reference() {
                            // A reference's value, and the temporary lets go of
                            // the reference (which may have been its last holder).
                            match read_ref(cell) {
                                Some(v) => {
                                    let _ = put_var(&mut w[rx!(var)], v);
                                    drop(taken);
                                    true
                                }
                                None => {
                                    w[rx!(src)] = taken;
                                    slow!();
                                }
                            }
                        } else {
                            put_var(&mut w[rx!(var)], taken)
                        };
                        done!(released);
                    }
                    Op::Free => {
                        done!(clear(&mut w[rx!(insn.a)]));
                    }
                    // A condition: a bool inline, anything else through `truth`
                    // (what may warn, NAN and an undefined variable, is `step`'s).
                    Op::JumpIf | Op::JumpIfNot => {
                        ops!(a, b, c, imm);
                        let v = &w[rx!(a)];
                        let c = match v.as_bool() {
                            Some(c) => c,
                            None => match truth(v) {
                                Some(c) => c,
                                None => slow!(),
                            },
                        };
                        if c == (op == Op::JumpIf) {
                            if back!(imm) {
                                slow!();
                            }
                            pc = imm as usize;
                        } else {
                            pc += 1;
                        }
                    }
                    // $a[k] on an array (or a reference to one) holding a plain
                    // value at that key: an int key inline, a string key (and
                    // a reference) through `elem`.
                    Op::FetchDim => {
                        ops!(a, b, c, imm);
                        let v = if let (Some(arr), Some(k)) = (w[rx!(b)].as_array(), w[rx!(c)].as_int()) {
                            match arr.get_int(k) {
                                Some(v) if !v.is_reference() => v.clone(),
                                _ => slow!(),
                            }
                        } else {
                            match elem(&w[..], Reg::from(b), Reg::from(c)) {
                                Some(v) => v,
                                None => slow!(),
                            }
                        };
                        done!(put(&mut w[rx!(a)], v));
                        if self.ids.has_pending() {
                            pending!();
                        }
                    }
                    // isset($a[k]) on an array (or a reference to one), k an int
                    // or a string; the rest (an object, a string's offset, a
                    // key of another type) is `step`'s.
                    Op::IssetDim => {
                        ops!(a, b, c, imm);
                        let set = if let (Some(arr), Some(k)) = (w[rx!(b)].as_array(), w[rx!(c)].as_int()) {
                            arr.get_int(k).is_some_and(is_set)
                        } else {
                            match isset_elem(&w[..], Reg::from(b), Reg::from(c)) {
                                Some(set) => set,
                                None => slow!(),
                            }
                        };
                        done!(put(&mut w[rx!(a)], Value::bool(set)));
                    }
                    // $a[k] = v, $a an array (or a reference to one): a string
                    // key with any defined, non-reference value; an int key
                    // with a plain value. The element replaced may have been
                    // an object's last holder: tested as for any write.
                    Op::AssignDim => arm!(self.arm_assign_dim(w, pc, *insn)),
                    // $a[int] op= number, $a an array holding a number at
                    // that key
                    Op::AssignDimOp => arm!(self.arm_assign_dim_op(w, pc, *insn)),
                    // The loop variable's old value may have been an object's
                    // last holder: a destructor due runs before the body, at
                    // this instruction.
                    Op::IterNext | Op::IterNextKey => arm!(self.arm_iter_next(w, pc, *insn)),
                    // $var op= number, $var holding a number (two ints inline,
                    // the rest through `assign_op`); $str .= string
                    Op::AssignOp => {
                        ops!(a, b, c, imm);
                        let op = insn::binop(imm);
                        let released = if let (Some(x), Some(y)) = (w[rx!(a)].as_int(), w[rx!(b)].as_int())
                            && let Some(n) = int_op(op, x, y)
                        {
                            put(&mut w[rx!(a)], Value::int(n))
                        } else if assign_op(&mut w[..], op, Reg::from(a), Reg::from(b)) {
                            true
                        } else {
                            slow!()
                        };
                        done!(released);
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
                    Op::ArrayPush => arm!(self.arm_array_push(w, pc, *insn)),
                    Op::ArraySet => arm!(self.arm_array_set(w, pc, *insn)),
                    // A constant the site resolved (a deprecated one is never
                    // cached: it warns on every fetch).
                    Op::FetchConst => {
                        ops!(a, b, c, imm);
                        let Some(Callee::Constant(v)) = clo!().sites[imm as usize].get() else { slow!() };
                        let v = v.clone();
                        done!(put(&mut w[rx!(a)], v));
                    }
                    // foreach over an array (a defined, non-reference
                    // register): the iterator is the array and position 0;
                    // an empty one jumps past the loop. (An object, a
                    // reference, anything that warns: `step`.)
                    Op::IterInit => arm!(self.arm_iter_init(w, pc, *insn)),
                    // An argument a user function or builtin the call site
                    // resolved to takes by value: a copy (by reference, an
                    // unresolved site, an undefined variable: `step`).
                    Op::SendVar => arm!(self.arm_send_var(w, pc, *insn)),
                    // `C::NAME` the site's cache holds (filled by `step`).
                    Op::ClassConst => {
                        ops!(a, b, c, imm);
                        let v = match clo!().class_consts.get((imm & 0xffff) as usize).and_then(OnceCell::get) {
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
                    Op::Call => arm_regs!(self.arm_call(regs, pc, *insn)),
                    // `f($x)` (`Op::CallRegs`): a builtin on the borrowed ABI
                    // reads its arguments from the variables (or constants)
                    // themselves, framelessly, and the Moves into the
                    // argument registers are not run. Anything else runs the
                    // first Move as `Op::Move` does, and goes on with the next.
                    Op::CallRegs => arm_regs!(self.arm_call_regs(regs, pc, *insn)),
                    // A user method the call site's cache names for this class
                    // and scope, on an object, with its required arguments, and
                    // no interrupt pending: anything else is `step`'s.
                    Op::MethodCall => arm_regs!(self.arm_method_call(regs, pc, *insn)),
                    // `new C` of a class the site's cache has, whose objects
                    // need only their slots (`make_plain`): anything else
                    // (a Throwable, a constructor to check) is `step`'s.
                    Op::New => arm!(self.arm_new(w, pc, *insn)),
                    // The constructor `new` found: a PHP one's frame pushed
                    // here, on the arguments where they are; none, the
                    // arguments released. Anything else is `step`'s.
                    Op::Construct => arm_regs!(self.arm_construct(regs, pc, *insn)),
                    // `C::f()` the site's cache has for this class and scope:
                    // a PHP method's frame pushed here. Anything else is `step`'s.
                    Op::StaticCall => arm_regs!(self.arm_static_call(regs, pc, *insn)),
                    // A return to the calling frame. The main frame's return
                    // ends the run, and the bottom frame of this `execute`
                    // returns to the builtin that called it: `step` does those.
                    Op::Return => arm_regs!(self.arm_return(regs, *insn, false)),
                    Op::ReturnNull => arm_regs!(self.arm_return(regs, *insn, true)),
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

/// What an out-of-line arm (`arms.rs`) leaves the loop to do. Two words,
/// so that it comes back in registers: a `Result<_, Throw>` came back
/// through memory, four loads per call and return.
pub(crate) enum Arm {
    /// Go on at this pc, the frame as it was.
    Go(usize),
    /// Go on at this pc, once the destructors due have run (raised at the
    /// instruction the arm ran).
    Pending(usize),
    /// The frame changed (a call, a return): go on at this pc of the frame
    /// now on top.
    Switch(usize),
    /// The instruction is `step`'s.
    Slow,
    /// The arm raised: the Throw waits in `Context::arm_throw`.
    Fault,
}

const _: () = assert!(std::mem::size_of::<Arm>() == 16, "an arm's result comes back in two registers");

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
    let consts_at = code.consts_at as usize;
    for r in &mut regs[base + argc.min(nparams)..base + consts_at] {
        clear(r);
    }
    // Arguments not passed: their constant defaults.
    if argc < nparams && !func.defaults.is_empty() {
        fill_defaults(&func.defaults, &mut regs[base..base + nparams], argc);
    }
    // Only the constants read as values (names stay in the constants).
    let values = code.value_consts as usize;
    // A register still holding the very constant (the same function called
    // again at this base: `leave` leaves the constants in place) is kept.
    for (r, c) in regs[base + consts_at..base + consts_at + values].iter_mut().zip(func.consts.iter()) {
        if !r.same_handle(c) {
            enter_const(r, c);
        }
    }
}

/// A constant copied into its register, the register's old value released.
#[inline(never)]
fn enter_const(r: &mut Value, c: &Value) {
    let _ = clear(r);
    // `r` is undefined now: nothing to drop.
    std::mem::forget(std::mem::replace(r, c.clone()));
}

/// The value `return` hands back from register `src` of `frame` (at
/// `base`); None when it is undefined (`step` warns). The frame ends with
/// the return, so the value is moved out of its register (no copy, and
/// no drop of the register's copy at `leave`) unless something still
/// reads the register: a constant (a name's stays for every call), a reference's
/// target, or a variable of a frame whose variables are not its own (the
/// global scope's, or a file's included in a function, given back at
/// `leave`).
#[inline(always)]
pub(crate) fn returned(regs: &mut [Value], frame: &Frame, base: usize, src: Reg) -> Option<Value> {
    let code = &frame.func.code;
    let at = src as usize;
    let temp = at >= code.consts_at as usize + frame.func.consts.len();
    let own_var = at < code.consts_at as usize && !frame.global && frame.extra.is_none();
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
    // The variables; the constants' registers are left as they are (the
    // next frame there overwrites them). The temporaries are empty when the
    // code frees each where it dies (`frees_temps`), as PHP's are.
    let consts = base + code.consts_at as usize..base + code.consts_at as usize + code.value_consts as usize;
    for r in &mut regs[base..consts.start] {
        clear(r);
    }
    if !code.frees_temps {
        for r in &mut regs[consts.end..base + usize::from(code.nregs)] {
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

/// An operand to store (`operand_take`): moved out of its register when
/// `take` (a temporary read for the last time), else copied; a reference's
/// value is read through it (the taken register lets go of the
/// reference). None when undefined (`step` warns).
#[inline(always)]
pub(crate) fn taken(w: &mut [Value], r: Reg, take: bool) -> Option<Value> {
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
fn is_set(v: &Value) -> bool {
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
fn test(w: &[Value], cmp: Cmp, a: Reg, b: Reg) -> Option<bool> {
    if cmp == Cmp::Spaceship {
        return None;
    }
    let (x, y) = (&w[usize::from(a)], &w[usize::from(b)]);
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
pub(crate) fn arith(w: &mut [Value], op: BinOp, dst: Reg, a: Reg, b: Reg) -> bool {
    let (x, y) = (&w[usize::from(a)], &w[usize::from(b)]);
    let r = if x.is_reference() || y.is_reference() {
        match (num_of(x), num_of(y)) {
            (Some(x), Some(y)) => num_op(op, &x, &y),
            _ => None,
        }
    } else {
        num_op(op, x, y)
    };
    match r {
        Some(r) => {
            // (Reported as released whatever the old value: the arm tests pending.)
            let _ = put_var(&mut w[usize::from(dst)], r);
            true
        }
        None => op == BinOp::Concat && concat(w, dst, a, b),
    }
}

/// `var op= src` beyond two ints: numbers (through references), `.=` on
/// strings. False when `step` must run it.
#[cold]
#[inline(never)]
fn assign_op(w: &mut [Value], op: BinOp, var: Reg, src: Reg) -> bool {
    let (x, y) = (&w[usize::from(var)], &w[usize::from(src)]);
    let r = if x.is_reference() || y.is_reference() {
        match (num_of(x), num_of(y)) {
            (Some(x), Some(y)) => num_op(op, &x, &y),
            _ => None,
        }
    } else {
        num_op(op, x, y)
    };
    match r {
        Some(r) => {
            let _ = put_var(&mut w[usize::from(var)], r);
            true
        }
        None => op == BinOp::Concat && append_str(w, var, src),
    }
}

/// A condition that is not a bool: what i_zend_is_true says, when
/// nothing can warn (NAN, an undefined variable: None, `step`'s).
#[cold]
#[inline(never)]
fn truth(v: &Value) -> Option<bool> {
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
fn elem(w: &[Value], arr: Reg, key: Reg) -> Option<Value> {
    let at = |a: &PhpArray| {
        let key = &w[usize::from(key)];
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
fn isset_elem(w: &[Value], var: Reg, keys: Reg) -> Option<bool> {
    let at = |a: &PhpArray| {
        let key = &w[usize::from(keys)];
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

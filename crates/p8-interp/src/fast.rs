//! The interpreter loop: the common cases of the hottest instructions
//! inline, everything else through `Context::slow` (which runs `step`),
//! called from inside the loop so that it never leaves it; and the frame
//! entry and exit the loop shares with `step`.

use super::*;

impl Context<'_> {
    /// Run frames until the bottom one returns. The loop itself runs the
    /// common cases of the hottest instructions: those that need only the
    /// frame's registers (ints and floats, jumps), plus calls to a user
    /// function the call site already resolved and returns of a value,
    /// which switch frames here. Every other instruction, and every case
    /// these do not cover, runs through `slow` (`step`), a call from inside
    /// the loop; the loop goes on with the pc it gives back.
    ///
    /// The inner loop touches nothing but the code, the registers and the
    /// two flags, so that its state stays in machine registers.
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
            let top = self.frames.last().expect("a frame");
            let (clo, base) = (&*top.func, top.base as usize);
            let code = &clo.code.code[..];
            let (stop, ids) = (&self.stop, &self.ids);
            // A loop's backward branch is where an interrupt is noticed:
            // `step` then raises it.
            let back = |target: u32, pc: usize| target as usize <= pc && stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT;
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
                    if released && ids.has_pending() {
                        pending!();
                    }
                }};
            }
        // A compare-and-branch on two numbers (anything else goes to `step`).
        // A compare-and-branch: two ints inline, anything else (floats,
        // references, strings, null) through `test`; an undefined
        // variable (its warning) is `step`'s.
        macro_rules! jump_if {
            ($w:ident, $cmp:expr, $when:expr, $a:expr, $b:expr, $target:expr) => {{
                let t = if let (Some(x), Some(y)) = ($w[rx!($a as usize)].as_int(), $w[rx!($b as usize)].as_int()) {
                    int_test($cmp, x, y)
                } else {
                    match test(&$w[..],$cmp, $a, $b) {
                        Some(t) => t,
                        None => slow!(),
                    }
                };
                if t == $when {
                    if back($target, pc) {
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
            ($w:ident, $op:expr, $dst:expr, $a:expr, $b:expr, $int:expr) => {{
                let released = if let (Some(x), Some(y)) = ($w[rx!($a as usize)].as_int(), $w[rx!($b as usize)].as_int())
                    && let Some(n) = $int(x, y)
                {
                    put_var(&mut $w[rx!($dst as usize)], Value::int(n))
                } else if arith(&mut $w[..], $op, $dst, $a, $b) {
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
            ($w:ident, $var:expr, $b:expr, $target:expr, $step:expr, $holds:expr) => {{
                let r = &mut $w[rx!($var as usize)];
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
                match int_of(&$w[rx!($b as usize)]) {
                    Some(b) if $holds(n, b) => {
                        if back($target, pc) {
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
        // The inner loop, over the frame's window `$w`: a `[Value; 256]`
        // for a function with at most 256 registers (every register
        // operand is then in range by construction, so `rx!` masks the
        // index and the compiler emits no bounds check), else the
        // registers as a slice, checked. One text, two instantiations.
        macro_rules! inner {
            ($w:ident) => {
        loop {
            match code[pc] {
                // `$this`, in a method that has one (else `step` raises).
                Instr::This { dst } => match &top.this {
                    Some(this) => {
                        let this = Value::object(Rc::clone(this));
                        done!(put(&mut $w[rx!(dst as usize)], this));
                    }
                    None => slow!(),
                },
                // A property read the name's cache entry answers (anything
                // else, `step`: a miss, an unset slot, a reference operand).
                Instr::FetchProp { dst, obj, name, .. } => {
                    let Some(site) = (name as usize).checked_sub(clo.code.consts_at as usize).and_then(|i| clo.props.get(i)) else { slow!() };
                    // THIS: the method's own `$this`, from the frame.
                    let o = if obj == p8_bytecode::THIS {
                        let Some(o) = &top.this else { slow!() };
                        o
                    } else {
                        let Some(o) = $w[rx!(obj as usize)].as_object() else { slow!() };
                        o
                    };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let Some(v) = cached_prop(site.get(), o, scope) else { slow!() };
                    done!(put(&mut $w[rx!(dst as usize)], v));
                }
                // `$o->name = v` the name's cache entry answers, on a
                // property with nothing to check (`plain_slot`): the slots
                // borrowed once, nothing cloned but the value, and that
                // once when the result is freed next (a statement's is).
                // An unset slot of a class with __set and its kin, and
                // anything `plain_slot` declines, is `step`'s.
                Instr::AssignProp { obj, name, val, dst, take } => {
                    let Some(site) = (name as usize).checked_sub(clo.code.consts_at as usize).and_then(|i| clo.props.get(i)) else { slow!() };
                    // THIS: the method's own `$this`, from the frame.
                    let o = if obj == p8_bytecode::THIS {
                        let Some(o) = &top.this else { slow!() };
                        o
                    } else {
                        let Some(o) = $w[rx!(obj as usize)].as_object() else { slow!() };
                        o
                    };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let Some(slot) = plain_slot(site.get(), o, scope, &$w[rx!(val as usize)]) else { slow!() };
                    let unused = matches!(code.get(pc + 1), Some(Instr::Free { reg }) if *reg == dst);
                    let written = match o.slots().get(slot) {
                        Some(s) if o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0 || !p8_runtime::object::Slot::peek(s, Value::is_undef) => {
                            // The slot's value out, written as a register
                            // is, back in. (Pending is tested below,
                            // whatever this released.)
                            let mut cur = s.replace(Value::UNDEF);
                            let _ = put_var(&mut cur, $w[rx!(val as usize)].clone());
                            p8_runtime::object::put_slot(s, cur);
                            true
                        }
                        _ => false,
                    };
                    if !written {
                        slow!();
                    }
                    let at = pc;
                    if unused {
                        pc += 2;
                    } else {
                        let v = $w[rx!(val as usize)].clone();
                        let _ = put(&mut $w[rx!(dst as usize)], v);
                        pc += 1;
                    }
                    if take {
                        let _ = clear(&mut $w[rx!(val as usize)]);
                    }
                    // (The slot's old value may have been the last holder.)
                    if ids.has_pending() {
                        pending_at!(pc, at);
                    }
                }
                // Function entry, when every passed argument is plainly of
                // its declared type (`verify_args` would accept each as it
                // is); a conversion, a class check or a variadic: `step`.
                Instr::Rare { op: Rare::VerifyArgs, .. } => {
                    if !args_plain(&clo.code.params, &$w[..],top.argc as usize) {
                        slow!();
                    }
                    pc += 1;
                }
                // A return value plainly of the declared type (anything
                // else, a generator's included: `step`).
                Instr::Rare { op: Rare::VerifyReturn, a, .. } => match &clo.code.ret_type {
                    Some(ty) if clo.code.is_generator || !ty.quick(&$w[rx!(a as usize)]) => slow!(),
                    _ => pc += 1,
                },
                // A builtin PHP compiles to an opcode (strlen, count, the
                // type tests, array_key_exists): the plain case here, once
                // the site resolved to the builtin (the first run, and a
                // disabled function, are `step`'s); the arguments are
                // consumed as a call's are. Anything else is the call.
                Instr::Rare { op, a: dst, b: args, c: argc, class: site } if op.is_intrinsic() => {
                    if !matches!(clo.sites[usize::from(site)].get(), Some(Callee::Builtin(_))) {
                        slow!();
                    }
                    let x = &$w[rx!(args as usize)];
                    let r = match op {
                        Rare::Strlen => match x.as_str() {
                            Some(s) => Value::int(s.len() as i64),
                            None => slow!(),
                        },
                        Rare::Count => match x.as_array() {
                            Some(arr) => Value::int(arr.len() as i64),
                            None => slow!(),
                        },
                        Rare::ArrayKeyExists => {
                            let Some(arr) = $w[rx!(args as usize + 1)].as_array() else { slow!() };
                            if let Some(k) = x.as_int() {
                                Value::bool(arr.contains_key(&Key::Int(k)))
                            } else if let Some(s) = x.as_str() {
                                Value::bool(arr.get_str(s).is_some())
                            } else {
                                slow!()
                            }
                        }
                        _ if x.is_undef() || x.is_reference() => slow!(),
                        Rare::IsNull => Value::bool(x.is_null()),
                        Rare::IsBool => Value::bool(x.is_bool()),
                        Rare::IsInt => Value::bool(x.is_int()),
                        Rare::IsFloat => Value::bool(x.is_float()),
                        Rare::IsString => Value::bool(x.is_str()),
                        Rare::IsArray => Value::bool(x.is_array()),
                        Rare::IsObject => Value::bool(x.as_object().is_some_and(|o| o.class.flags & p8_runtime::object::CLASS_RESOURCE == 0)),
                        _ => slow!(),
                    };
                    // The arguments consumed may have been an object's last
                    // holder (`count([new D])`): a destructor due runs here.
                    let mut released = false;
                    for i in 0..usize::from(argc) {
                        released |= clear(&mut $w[rx!(args as usize + i)]);
                    }
                    released |= put(&mut $w[rx!(dst as usize)], r);
                    done!(released);
                }
                // `$o->m($v)`: an argument the method the call site's cache
                // names for this class and scope takes by value is copied
                // (by reference, a miss, or an undefined or reference
                // variable: `step`, which looks the method up by name).
                Instr::SendVarMethod { obj, method, n, arg, var } => {
                    let Some(o) = $w[rx!(obj as usize)].as_object() else { slow!() };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let by_value = clo.methods[usize::from(method)].borrow().as_ref().is_some_and(|s| {
                        s.class == Rc::as_ptr(&o.class) as usize
                            && s.scope == scope
                            && matches!(&s.code, MethodCode::User(code) if code.upgrade().is_some_and(|c| !c.code.param_by_ref(n as usize)))
                    });
                    if !by_value {
                        slow!();
                    }
                    let Some(v) = copied(&$w[rx!(var as usize)]) else { slow!() };
                    done!(put(&mut $w[rx!(arg as usize)], v));
                }
                Instr::Jump { target } => {
                    if back(target, pc) {
                        slow!();
                    }
                    pc = target as usize;
                }
                Instr::JumpLt { a, b, target } => jump_if!($w,Cmp::Lt, true, a, b, target),
                Instr::JumpNotLt { a, b, target } => jump_if!($w,Cmp::Lt, false, a, b, target),
                Instr::JumpLe { a, b, target } => jump_if!($w,Cmp::Le, true, a, b, target),
                Instr::JumpNotLe { a, b, target } => jump_if!($w,Cmp::Le, false, a, b, target),
                Instr::JumpEq { a, b, target } => jump_if!($w,Cmp::Eq, true, a, b, target),
                Instr::JumpNotEq { a, b, target } => jump_if!($w,Cmp::Eq, false, a, b, target),
                Instr::JumpIdentical { a, b, target } => jump_if!($w,Cmp::Identical, true, a, b, target),
                Instr::JumpNotIdentical { a, b, target } => jump_if!($w,Cmp::Identical, false, a, b, target),
                // A comparison (`<=>` is `step`'s): two ints inline, the
                // rest through `test`.
                Instr::Compare { cmp, dst, a, b } => {
                    let t = if let (Some(x), Some(y)) = ($w[rx!(a as usize)].as_int(), $w[rx!(b as usize)].as_int())
                        && cmp != Cmp::Spaceship
                    {
                        int_test(cmp, x, y)
                    } else if cmp == Cmp::Spaceship {
                        slow!()
                    } else {
                        match test(&$w[..],cmp, a, b) {
                            Some(t) => t,
                            None => slow!(),
                        }
                    };
                    done!(put_var(&mut $w[rx!(dst as usize)], Value::bool(t)));
                }
                Instr::IncVar { var } => {
                    let r = &mut $w[rx!(var as usize)];
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
                Instr::IncJumpLt { var, b, target } => step_jump!($w,var, b, target, i64::checked_add, |n, b| n < b),
                Instr::IncJumpLe { var, b, target } => step_jump!($w,var, b, target, i64::checked_add, |n, b| n <= b),
                Instr::DecJumpGt { var, b, target } => step_jump!($w,var, b, target, i64::checked_sub, |n, b| n > b),
                Instr::DecJumpGe { var, b, target } => step_jump!($w,var, b, target, i64::checked_sub, |n, b| n >= b),
                Instr::DecVar { var } => {
                    let r = &mut $w[rx!(var as usize)];
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
                Instr::Binary { op, dst, a, b } => {
                    if !arith(&mut $w[..], op, dst, a, b) {
                        slow!();
                    }
                    done!(true);
                }
                Instr::Add { dst, a, b } => arith!($w,BinOp::Add, dst, a, b, i64::checked_add),
                Instr::Sub { dst, a, b } => arith!($w,BinOp::Sub, dst, a, b, i64::checked_sub),
                Instr::Mul { dst, a, b } => arith!($w,BinOp::Mul, dst, a, b, i64::checked_mul),
                Instr::ConcatN { dst, start, n } => {
                    if !concat_n(&mut $w[..], dst, start, n) {
                        slow!();
                    }
                    done!(true);
                }
                // A copy of a register (a reference's value read through
                // it; undefined: `step` warns). A write that freed an object
                // with a destructor stops here: it runs before the next
                // instruction.
                Instr::Move { dst, src } => {
                    let Some(v) = copied(&$w[rx!(src as usize)]) else { slow!() };
                    done!(put(&mut $w[rx!(dst as usize)], v));
                    if ids.has_pending() {
                        pending!();
                    }
                }
                Instr::Assign { var, src } => {
                    let Some(v) = copied(&$w[rx!(src as usize)]) else { slow!() };
                    done!(put_var(&mut $w[rx!(var as usize)], v));
                }
                Instr::AssignTake { var, src } => {
                    let taken = std::mem::replace(&mut $w[rx!(src as usize)], Value::UNDEF);
                    // Undefined: `step`'s (the warning).
                    if taken.is_undef() {
                        slow!();
                    }
                    let released = if let Some(cell) = taken.as_reference() {
                        // A reference's value, and the temporary lets go of
                        // the reference (which may have been its last holder).
                        match read_ref(cell) {
                            Some(v) => {
                                let _ = put_var(&mut $w[rx!(var as usize)], v);
                                drop(taken);
                                true
                            }
                            None => {
                                $w[rx!(src as usize)] = taken;
                                slow!();
                            }
                        }
                    } else {
                        put_var(&mut $w[rx!(var as usize)], taken)
                    };
                    done!(released);
                }
                Instr::Free { reg } => {
                    done!(clear(&mut $w[rx!(reg as usize)]));
                }
                // A condition: a bool inline, anything else through `truth`
                // (what may warn, NAN and an undefined variable, is `step`'s).
                Instr::JumpIf { cond, target } => {
                    let v = &$w[rx!(cond as usize)];
                    let c = match v.as_bool() {
                        Some(c) => c,
                        None => match truth(v) {
                            Some(c) => c,
                            None => slow!(),
                        },
                    };
                    if c {
                        if back(target, pc) {
                            slow!();
                        }
                        pc = target as usize;
                    } else {
                        pc += 1;
                    }
                }
                Instr::JumpIfNot { cond, target } => {
                    let v = &$w[rx!(cond as usize)];
                    let c = match v.as_bool() {
                        Some(c) => c,
                        None => match truth(v) {
                            Some(c) => c,
                            None => slow!(),
                        },
                    };
                    if c {
                        pc += 1;
                    } else {
                        if back(target, pc) {
                            slow!();
                        }
                        pc = target as usize;
                    }
                }
                // $a[k] on an array (or a reference to one) holding a plain
                // value at that key: an int key inline, a string key (and
                // a reference) through `elem`.
                Instr::FetchDim { dst, arr, key, .. } => {
                    let v = if let (Some(a), Some(k)) = ($w[rx!(arr as usize)].as_array(), $w[rx!(key as usize)].as_int()) {
                        match a.get_int(k) {
                            Some(v) if !v.is_reference() => v.clone(),
                            _ => slow!(),
                        }
                    } else {
                        match elem(&$w[..], arr, key) {
                            Some(v) => v,
                            None => slow!(),
                        }
                    };
                    done!(put(&mut $w[rx!(dst as usize)], v));
                    if ids.has_pending() {
                        pending!();
                    }
                }
                // isset($a[k]) on an array (or a reference to one), k an int
                // or a string; the rest (an object, a string's offset, a
                // key of another type) is `step`'s.
                Instr::IssetDim { dst, var, keys, n: 1 } => {
                    let set = if let (Some(a), Some(k)) = ($w[rx!(var as usize)].as_array(), $w[rx!(keys as usize)].as_int()) {
                        a.get_int(k).is_some_and(is_set)
                    } else {
                        match isset_elem(&$w[..],var, keys) {
                            Some(set) => set,
                            None => slow!(),
                        }
                    };
                    done!(put(&mut $w[rx!(dst as usize)], Value::bool(set)));
                }
                // $a[int] = int|float|bool|null, $a an array (or a reference to one)
                // $a[k] = v, $a an array (or a reference to one): a string
                // key with any defined, non-reference value; an int key
                // with a plain value. The element replaced may have been
                // an object's last holder: tested as for any write.
                Instr::AssignDim { var, keys, n: 1, append: false, val, dst } => {
                    if $w[rx!(keys as usize)].is_str() {
                        if !set_str_elem(&mut $w[..], var, keys, val, dst) {
                            slow!();
                        }
                    } else {
                        let (Some(k), Some(v)) = ($w[rx!(keys as usize)].as_int(), plain(&$w[rx!(val as usize)])) else { slow!() };
                        let slot = &mut $w[rx!(var as usize)];
                        let done = if slot.is_array() {
                            set_int_elem(slot, k, &v)
                        } else if let Some(cell) = slot.as_reference() {
                            set_int_elem(&mut cell.borrow_mut(), k, &v)
                        } else {
                            false
                        };
                        if !done {
                            slow!();
                        }
                        let _ = put(&mut $w[rx!(dst as usize)], v);
                    }
                    done!(true);
                }
                // $a[int] op= number, $a an array holding a number at
                // that key
                Instr::AssignDimOp { op, var, keys, n: 1, val, dst } => {
                    if !dim_op(&mut $w[..], op, var, keys, val, dst) {
                        slow!();
                    }
                    done!(true);
                }
                // The loop variable's old value may have been an object's
                // last holder: a destructor due runs before the body, at
                // this instruction.
                Instr::IterNext { iter, val, key, has_key, end } => {
                    // An object (a generator, an Iterator) is `step`'s.
                    if !$w[rx!(iter as usize)].is_array() {
                        slow!();
                    }
                    let next = if iter_next(&mut $w[..], iter, val, key, has_key) { pc + 1 } else { end as usize };
                    if ids.has_pending() {
                        pending_at!(next, pc);
                    }
                    pc = next;
                }
                // $var op= number, $var holding a number (two ints inline,
                // the rest through `assign_op`); $str .= string
                Instr::AssignOp { op, var, src } => {
                    let released = if let (Some(x), Some(y)) = ($w[rx!(var as usize)].as_int(), $w[rx!(src as usize)].as_int())
                        && let Some(n) = int_op(op, x, y)
                    {
                        put(&mut $w[rx!(var as usize)], Value::int(n))
                    } else if assign_op(&mut $w[..], op, var, src) {
                        true
                    } else {
                        slow!()
                    };
                    done!(released);
                }
                // $i++ / ++$i (and --) whose value is used, on an int
                Instr::IncDec { var, dst, inc, post } => {
                    let Some(slot) = $w[rx!(var as usize)].as_int_mut() else { slow!() };
                    let i = *slot;
                    let Some(n) = (if inc { i.checked_add(1) } else { i.checked_sub(1) }) else { slow!() };
                    *slot = n;
                    done!(put(&mut $w[rx!(dst as usize)], Value::int(if post { i } else { n })));
                }
                Instr::LoadNull { dst } => {
                    done!(put(&mut $w[rx!(dst as usize)], Value::NULL));
                }
                // `!x` and a condition made a value: as `truth` decides
                // (NAN and an undefined variable warn: `step`).
                Instr::Not { dst, src } => {
                    let v = &$w[rx!(src as usize)];
                    let t = match v.as_bool() {
                        Some(b) => b,
                        None => match truth(v) {
                            Some(t) => t,
                            None => slow!(),
                        },
                    };
                    done!(put(&mut $w[rx!(dst as usize)], Value::bool(!t)));
                }
                Instr::ToBool { dst, src } => {
                    let v = &$w[rx!(src as usize)];
                    let t = match v.as_bool() {
                        Some(b) => b,
                        None => match truth(v) {
                            Some(t) => t,
                            None => slow!(),
                        },
                    };
                    done!(put(&mut $w[rx!(dst as usize)], Value::bool(t)));
                }
                Instr::IssetVar { dst, var } => {
                    let set = is_set(&$w[rx!(var as usize)]);
                    done!(put(&mut $w[rx!(dst as usize)], Value::bool(set)));
                }
                // `a ?? b`: a set and not null goes to dst and jumps.
                Instr::JumpIfSet { src, dst, target } => {
                    let s = &$w[rx!(src as usize)];
                    let v = if s.is_null_or_undef() {
                        None
                    } else if let Some(cell) = s.as_reference() {
                        read_ref(cell).filter(|v| !v.is_null())
                    } else {
                        Some(s.clone())
                    };
                    match v {
                        Some(v) => {
                            if put(&mut $w[rx!(dst as usize)], v) && ids.has_pending() {
                                pending_at!(target as usize, pc);
                            }
                            pc = target as usize;
                        }
                        None => pc += 1,
                    }
                }
                // An array literal: the empty array, then its elements
                // pushed or set (an int or string key; a value defined and
                // read through a reference; anything else, a next key
                // occupied included, is `step`'s).
                Instr::NewArray { dst, n } => {
                    let a = Value::array(PhpArray::list_with_capacity(n as usize));
                    done!(put(&mut $w[rx!(dst as usize)], a));
                }
                Instr::ArrayPush { arr, val, take } => {
                    let Some(a) = $w[rx!(arr as usize)].as_array() else { slow!() };
                    if a.next_key().is_none() {
                        slow!();
                    }
                    let Some(v) = taken(&mut $w[..],val, take) else { slow!() };
                    let _ = $w[rx!(arr as usize)].array_mut().expect("checked above").push(v);
                    // (A taken temporary that held a reference let go of it.)
                    done!(true);
                }
                Instr::ArraySet { arr, key, val, take } => {
                    if !$w[rx!(arr as usize)].is_array() {
                        slow!();
                    }
                    let key = &$w[rx!(key as usize)];
                    let k = if let Some(k) = key.as_int() {
                        Key::Int(k)
                    } else if let Some(s) = key.cloned_str() {
                        Key::from_string(s)
                    } else {
                        slow!()
                    };
                    let Some(v) = taken(&mut $w[..],val, take) else { slow!() };
                    // (A key written twice in one literal replaces the first value.)
                    $w[rx!(arr as usize)].array_mut().expect("checked above").insert(k, v);
                    done!(true);
                }
                // A constant the site resolved (a deprecated one is never
                // cached: it warns on every fetch).
                Instr::FetchConst { site, dst } => {
                    let Some(Callee::Constant(v)) = clo.sites[usize::from(site)].get() else { slow!() };
                    let v = v.clone();
                    done!(put(&mut $w[rx!(dst as usize)], v));
                }
                // Every other instruction (listed, so dispatch needs no range check).
                Instr::AssignDim { .. }
                | Instr::AssignDimOp { .. }
                | Instr::Neg { .. }
                | Instr::BitNot { .. }
                | Instr::Cast { .. }
                | Instr::IssetDim { .. }
                | Instr::UnsetVar { .. }
                | Instr::UnsetDim { .. }
                | Instr::IterInit { .. }
                | Instr::SendVar { .. }
                | Instr::SendVarStatic { .. }
                | Instr::MethodArgByRef { .. }
                | Instr::StaticArgByRef { .. }
                | Instr::FuncArgByRef { .. }
                | Instr::Echo { .. }
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
                | Instr::AssignPropOp { .. }
                | Instr::PropIncDec { .. }
                | Instr::AssignPropDim { .. }
                | Instr::IssetProp { .. }
                | Instr::UnsetProp { .. }
                | Instr::StaticCall { .. }
                | Instr::ClassConst { .. }
                | Instr::FetchStaticProp { .. }
                | Instr::AssignStaticProp { .. }
                | Instr::AssignStaticPropOp { .. }
                | Instr::InstanceOf { .. }
                | Instr::Clone { .. }
                | Instr::Throw { .. }
                | Instr::CatchIf { .. } => slow!(),
                // A call of a user function the call site resolved, with
                // its required arguments, and no interrupt pending: its
                // frame pushed here. A builtin it resolved: called out of
                // line. Anything else (a miss, a generator) is `step`'s.
                Instr::Call { site, base: args, argc: n, dst } => match clo.sites[usize::from(site)].get() {
                    Some(&Callee::User(target)) => {
                        let target = &self.closures[target as usize];
                        if (n as usize) < target.code.required || target.code.is_generator || stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
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
                        // Frameless when the builtin is on the borrowed ABI
                        // and its arguments need no conversion (and no
                        // header callback can run before output).
                        let call = self.isolate.registry.call(i);
                        let start = base + args as usize;
                        if let (Some(run), Some(sig)) = (call.frameless, call.sig)
                            && p8_runtime::args::plain_call(sig, call.counts, call.objects_ok, &regs[start..start + n as usize])
                            && !(self.header_callback_set && !self.out.started)
                        {
                            pc = self.frameless_call(regs, pc, i, run, (start, n as usize, base + dst as usize))?;
                            continue 'frame;
                        }
                        pc = self.builtin_call(regs, pc, base, i, (args, n, dst))?;
                        continue 'frame;
                    }
                    _ => slow!(),
                },
                // A user method the call site's cache names for this class
                // and scope, on an object, with its required arguments, and
                // no interrupt pending: anything else is `step`'s.
                Instr::MethodCall { dst, obj, method, base: args, argc: n } => {
                    let Some(o) = $w[rx!(obj as usize)].as_object() else { slow!() };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let found = {
                        let site = clo.methods[usize::from(method)].borrow();
                        match site.as_ref() {
                            Some(s) if s.class == Rc::as_ptr(&o.class) as usize && s.scope == scope && !s.is_static => match &s.code {
                                MethodCode::User(code) => code.upgrade().zip(s.declaring.upgrade()),
                                MethodCode::Native(..) => None,
                            },
                            _ => None,
                        }
                    };
                    let Some((target, declaring)) = found else { slow!() };
                    if (n as usize) < target.code.required || target.code.is_generator || stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                        slow!();
                    }
                    let this = Rc::clone(o);
                    // A temporary (`(new A)->f()`) is released as it is used:
                    // the call's `$this` holds the object.
                    if is_temp(&clo.code, obj) {
                        clear(&mut $w[rx!(obj as usize)]);
                    }
                    if let Some(caller) = self.frames.last_mut() {
                        caller.pc = (pc + 1) as u32;
                    }
                    self.frames.push(&target, base + args as usize, n as usize, (base + dst as usize) as u32);
                    let frame = self.frames.last_mut().expect("the frame just pushed");
                    frame.called = Some(Rc::clone(&this.class));
                    frame.this = Some(this);
                    frame.scope = Some(declaring);
                    enter(&mut self.frames, regs);
                    pc = 0;
                    continue 'frame;
                }
                // A return to the calling frame. The main frame's return
                // ends the run, and the bottom frame of this `execute`
                // returns to the builtin that called it: `step` does those.
                Instr::Return { src } => {
                    if self.frames.len() == self.frames.floor + 1 {
                        slow!();
                    }
                    let Some(value) = returned(regs, top, base, src) else { slow!() };
                    leave(&mut self.frames, regs, value);
                    pc = self.frames.last().expect("the caller's frame").pc as usize;
                    if self.ids.has_pending() {
                        pc = self.after_errors(regs, pc, pc.saturating_sub(1))?;
                    }
                    continue 'frame;
                }
                Instr::ReturnNull => {
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
            }
        }
            };
        }
        let nregs = usize::from(clo.code.nregs);
        if nregs <= WINDOW {
            // The register file is kept a window ahead of every frame
            // (`enter`), so the frame's fixed window always exists.
            if regs.len() < base + WINDOW {
                regs.resize(base + WINDOW, Value::UNDEF);
            }
            let w: &mut [Value; WINDOW] = (&mut regs[base..base + WINDOW]).try_into().expect("a window's worth of registers");
            macro_rules! rx {
                ($e:expr) => {
                    ($e) & (WINDOW - 1)
                };
            }
            inner!(w);
        } else {
            let w = &mut regs[base..base + nregs];
            macro_rules! rx {
                ($e:expr) => {
                    $e
                };
            }
            inner!(w);
        }
        }
    }
}

/// The fixed window: a function with up to this many registers runs with
/// no register bounds checks (`Context::execute`). The register file is
/// kept this far ahead of every frame's base.
pub(crate) const WINDOW: usize = 256;

/// What `slow` leaves the loop to do: go on at a pc, or the run is over
/// (the bottom frame returned, or a fiber suspended).
pub(crate) enum Flow {
    Go(usize),
    Done,
}

/// Whether the first `argc` registers of `w` are each plainly of their
/// parameter's declared type (`TypeDecl::quick`), so `verify_args` has
/// nothing to check or convert. A typed variadic is left to it.
#[inline]
pub(crate) fn args_plain(params: &[p8_bytecode::Param], w: &[Value], argc: usize) -> bool {
    params.iter().zip(w).enumerate().all(|(i, (p, v))| match &p.ty {
        None => true,
        Some(_) if p.variadic => false,
        Some(_) if i >= argc || v.is_undef() => true,
        Some(ty) => ty.quick(v),
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
    for (r, c) in regs[base + consts_at..base + consts_at + values].iter_mut().zip(func.consts.iter()) {
        clear(r);
        // `r` is undefined now: nothing to drop.
        std::mem::forget(std::mem::replace(r, c.clone()));
    }
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
        Some(cell.borrow().clone())
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
    // Variables and temporaries; the constants' registers are left as
    // they are (the next frame there overwrites them).
    let consts = base + code.consts_at as usize..base + code.consts_at as usize + code.value_consts as usize;
    for r in &mut regs[base..consts.start] {
        clear(r);
    }
    for r in &mut regs[consts.end..base + usize::from(code.nregs)] {
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

/// Release what a value holds (a string, an array, a reference, an
/// object): the one call an overwrite makes, and only for those.
#[cold]
#[inline(never)]
pub(crate) fn release(v: Value) {
    drop(v);
}

/// Write `v` over a register's value: the old value's tag is tested
/// here, and only one holding memory is released, out of line (PHP's
/// zval_ptr_dtor_nogc: `Z_REFCOUNTED`, then a call).
///
/// Returns whether the old value held memory: the release may have freed
/// the last holder of an object with a destructor, which the loop runs
/// before the next instruction (`done!`), at this instruction's pc, as
/// PHP runs it where the value went. A plain old value cannot have, so
/// that test is on the release path only.
#[inline(always)]
#[must_use = "a released value may leave a destructor due: test `ids.has_pending()`"]
pub(crate) fn put(slot: &mut Value, v: Value) -> bool {
    // The tag is tested in the slot, so the plain case reads one byte
    // and stores; only the other case reads the old value out, for `release`.
    if is_plain(slot) {
        std::mem::forget(std::mem::replace(slot, v));
        false
    } else {
        release(std::mem::replace(slot, v));
        true
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
        None => {
            release(std::mem::replace(slot, v));
            true
        }
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
fn taken(w: &mut [Value], r: Reg, take: bool) -> Option<Value> {
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
fn arith(w: &mut [Value], op: BinOp, dst: Reg, a: Reg, b: Reg) -> bool {
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

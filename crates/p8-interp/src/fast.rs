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
            let w = &mut regs[base..base + clo.code.nregs as usize];
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
        // A compare-and-branch on two numbers (anything else goes to `step`).
        // A compare-and-branch: two ints inline, anything else (floats,
        // references, strings, null) through `test`; an undefined
        // variable (its warning) is `step`'s.
        macro_rules! jump_if {
            ($cmp:expr, $when:expr, $a:expr, $b:expr, $target:expr) => {{
                let t = match (&w[$a as usize], &w[$b as usize]) {
                    (Value::Int(x), Value::Int(y)) => int_test($cmp, *x, *y),
                    _ => match test(w, $cmp, $a, $b) {
                        Some(t) => t,
                        None => slow!(),
                    },
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
            ($op:expr, $dst:expr, $a:expr, $b:expr, $int:expr) => {{
                if let (Value::Int(x), Value::Int(y)) = (&w[$a as usize], &w[$b as usize])
                    && let Some(n) = $int(*x, *y)
                {
                    put_var(&mut w[$dst as usize], Value::Int(n));
                } else if !arith(w, $op, $dst, $a, $b) {
                    slow!();
                }
                pc += 1;
            }};
        }
        // A fused step and test (`IncJumpLt` and co.) on ints (through a
        // reference too): step, then jump or go past the test. A non-int
        // bound leaves the test to the next instruction; a non-int
        // variable (or an overflow) leaves both to `step`.
        macro_rules! step_jump {
            ($var:expr, $b:expr, $target:expr, $step:expr, $holds:expr) => {{
                let n = match &mut w[$var as usize] {
                    // Written in place: no drop of the old value to compile.
                    Value::Int(i) => match $step(*i, 1) {
                        Some(n) => {
                            *i = n;
                            n
                        }
                        None => slow!(),
                    },
                    Value::Ref(cell) => match step_ref(cell, $step) {
                        Some(n) => n,
                        None => slow!(),
                    },
                    _ => slow!(),
                };
                match int_of(&w[$b as usize]) {
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
        loop {
            match code[pc] {
                // `$this`, in a method that has one (else `step` raises).
                Instr::This { dst } => match &top.this {
                    Some(this) => {
                        let this = Value::Object(Rc::clone(this));
                        put(&mut w[dst as usize], this);
                        pc += 1;
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
                        let Value::Object(o) = &w[obj as usize] else { slow!() };
                        o
                    };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let Some(v) = cached_prop(site.get(), o, scope) else { slow!() };
                    put(&mut w[dst as usize], v);
                    pc += 1;
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
                        let Value::Object(o) = &w[obj as usize] else { slow!() };
                        o
                    };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let Some(slot) = plain_slot(site.get(), o, scope, &w[val as usize]) else { slow!() };
                    let unused = matches!(code.get(pc + 1), Some(Instr::Free { reg }) if *reg == dst);
                    let written = {
                        let mut slots = o.slots.borrow_mut();
                        match slots.get_mut(slot) {
                            Some(s) if !(matches!(s, Value::Undef) && o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS != 0) => {
                                put_var(s, w[val as usize].clone());
                                true
                            }
                            _ => false,
                        }
                    };
                    if !written {
                        slow!();
                    }
                    if unused {
                        pc += 2;
                    } else {
                        let v = w[val as usize].clone();
                        put(&mut w[dst as usize], v);
                        pc += 1;
                    }
                    if take {
                        clear(&mut w[val as usize]);
                    }
                    if ids.has_pending() {
                        pending!();
                    }
                }
                // Function entry, when every passed argument is plainly of
                // its declared type (`verify_args` would accept each as it
                // is); a conversion, a class check or a variadic: `step`.
                Instr::Rare { op: Rare::VerifyArgs, .. } => {
                    if !args_plain(&clo.code.params, w, top.argc as usize) {
                        slow!();
                    }
                    pc += 1;
                }
                // A return value plainly of the declared type (anything
                // else, a generator's included: `step`).
                Instr::Rare { op: Rare::VerifyReturn, a, .. } => match &clo.code.ret_type {
                    Some(ty) if clo.code.is_generator || !ty.quick(&w[a as usize]) => slow!(),
                    _ => pc += 1,
                },
                // `$o->m($v)`: an argument the method the call site's cache
                // names for this class and scope takes by value is copied
                // (by reference, a miss, or an undefined or reference
                // variable: `step`, which looks the method up by name).
                Instr::SendVarMethod { obj, method, n, arg, var } => {
                    let Value::Object(o) = &w[obj as usize] else { slow!() };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let by_value = clo.methods[method as usize].borrow().as_ref().is_some_and(|s| {
                        s.class == Rc::as_ptr(&o.class) as usize
                            && s.scope == scope
                            && matches!(&s.code, MethodCode::User(code) if code.upgrade().is_some_and(|c| !c.code.param_by_ref(n as usize)))
                    });
                    if !by_value {
                        slow!();
                    }
                    let Some(v) = copied(&w[var as usize]) else { slow!() };
                    put(&mut w[arg as usize], v);
                    pc += 1;
                    if ids.has_pending() {
                        pending!();
                    }
                }
                Instr::Jump { target } => {
                    if back(target, pc) {
                        slow!();
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
                // A comparison (`<=>` is `step`'s): two ints inline, the
                // rest through `test`.
                Instr::Compare { cmp, dst, a, b } => {
                    let t = match (&w[a as usize], &w[b as usize]) {
                        (Value::Int(x), Value::Int(y)) if cmp != Cmp::Spaceship => int_test(cmp, *x, *y),
                        _ if cmp == Cmp::Spaceship => slow!(),
                        _ => match test(w, cmp, a, b) {
                            Some(t) => t,
                            None => slow!(),
                        },
                    };
                    put_var(&mut w[dst as usize], Value::Bool(t));
                    pc += 1;
                }
                Instr::IncVar { var } => match &mut w[var as usize] {
                    Value::Int(i) if *i != i64::MAX => {
                        *i += 1;
                        pc += 1;
                    }
                    Value::Ref(cell) if step_ref(cell, i64::checked_add).is_some() => pc += 1,
                    _ => slow!(),
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
                    Value::Ref(cell) if step_ref(cell, i64::checked_sub).is_some() => pc += 1,
                    _ => slow!(),
                },
                Instr::Binary { op, dst, a, b } => {
                    if !arith(w, op, dst, a, b) {
                        slow!();
                    }
                    pc += 1;
                }
                Instr::Add { dst, a, b } => arith!(BinOp::Add, dst, a, b, i64::checked_add),
                Instr::Sub { dst, a, b } => arith!(BinOp::Sub, dst, a, b, i64::checked_sub),
                Instr::Mul { dst, a, b } => arith!(BinOp::Mul, dst, a, b, i64::checked_mul),
                Instr::ConcatN { dst, start, n } => {
                    if !concat_n(w, dst, start, n) {
                        slow!();
                    }
                    pc += 1;
                }
                // A copy of a register (a reference's value read through
                // it; undefined: `step` warns). A write that freed an object
                // with a destructor stops here: it runs before the next
                // instruction.
                Instr::Move { dst, src } => {
                    let v = match &w[src as usize] {
                        Value::Undef => slow!(),
                        Value::Ref(cell) => match read_ref(cell) {
                            Some(v) => v,
                            None => slow!(),
                        },
                        v => v.clone(),
                    };
                    put(&mut w[dst as usize], v);
                    pc += 1;
                    if ids.has_pending() {
                        pending!();
                    }
                }
                Instr::Assign { var, src } => {
                    let v = match &w[src as usize] {
                        Value::Undef => slow!(),
                        Value::Ref(cell) => match read_ref(cell) {
                            Some(v) => v,
                            None => slow!(),
                        },
                        v => v.clone(),
                    };
                    put_var(&mut w[var as usize], v);
                    pc += 1;
                    if ids.has_pending() {
                        pending!();
                    }
                }
                Instr::AssignTake { var, src } => {
                    match std::mem::replace(&mut w[src as usize], Value::Undef) {
                        // Undefined: `step`'s (the warning).
                        Value::Undef => slow!(),
                        // A reference's value, and the temporary lets go of
                        // the reference.
                        Value::Ref(cell) => match read_ref(&cell) {
                            Some(v) => put_var(&mut w[var as usize], v),
                            None => {
                                w[src as usize] = Value::Ref(cell);
                                slow!();
                            }
                        },
                        v => put_var(&mut w[var as usize], v),
                    }
                    pc += 1;
                    if ids.has_pending() {
                        pending!();
                    }
                }
                Instr::Free { reg } => {
                    clear(&mut w[reg as usize]);
                    pc += 1;
                    if ids.has_pending() {
                        pending!();
                    }
                }
                // A condition: a bool inline, anything else through `truth`
                // (what may warn, NAN and an undefined variable, is `step`'s).
                Instr::JumpIf { cond, target } => {
                    let c = match &w[cond as usize] {
                        Value::Bool(c) => *c,
                        v => match truth(v) {
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
                    let c = match &w[cond as usize] {
                        Value::Bool(c) => *c,
                        v => match truth(v) {
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
                    let v = match (&w[arr as usize], &w[key as usize]) {
                        (Value::Array(a), Value::Int(k)) => match a.get_int(*k) {
                            Some(Value::Ref(_)) | None => slow!(),
                            Some(v) => v.clone(),
                        },
                        _ => match elem(w, arr, key) {
                            Some(v) => v,
                            None => slow!(),
                        },
                    };
                    put(&mut w[dst as usize], v);
                    pc += 1;
                    if ids.has_pending() {
                        pending!();
                    }
                }
                // isset($a[k]) on an array (or a reference to one), k an int
                // or a string; the rest (an object, a string's offset, a
                // key of another type) is `step`'s.
                Instr::IssetDim { dst, var, keys, n: 1 } => {
                    let set = match (&w[var as usize], &w[keys as usize]) {
                        (Value::Array(a), Value::Int(k)) => a.get_int(*k).is_some_and(is_set),
                        _ => match isset_elem(w, var, keys) {
                            Some(set) => set,
                            None => slow!(),
                        },
                    };
                    put(&mut w[dst as usize], Value::Bool(set));
                    pc += 1;
                }
                // $a[int] = int|float|bool|null, $a an array (or a reference to one)
                Instr::AssignDim { var, keys, n: 1, append: false, val, dst } => {
                    if let Value::Str(_) = w[keys as usize] {
                        if !set_str_elem(w, var, keys, val, dst) {
                            slow!();
                        }
                        pc += 1;
                        continue;
                    }
                    let (Value::Int(k), Some(v)) = (&w[keys as usize], plain(&w[val as usize])) else { slow!() };
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
                        slow!();
                    }
                    put(&mut w[dst as usize], v);
                    pc += 1;
                    if ids.has_pending() {
                        pending!();
                    }
                }
                // $a[int] op= number, $a an array holding a number at
                // that key
                Instr::AssignDimOp { op, var, keys, n: 1, val, dst } => {
                    if !dim_op(w, op, var, keys, val, dst) {
                        slow!();
                    }
                    pc += 1;
                }
                Instr::IterNext { iter, val, key, has_key, end } => {
                    // An object (a generator, an Iterator) is `step`'s.
                    if !matches!(w[iter as usize], Value::Array(_)) {
                        slow!();
                    }
                    pc = if iter_next(w, iter, val, key, has_key) { pc + 1 } else { end as usize };
                }
                // $var op= number, $var holding a number (two ints inline,
                // the rest through `assign_op`); $str .= string
                Instr::AssignOp { op, var, src } => {
                    if let (Value::Int(x), Value::Int(y)) = (&w[var as usize], &w[src as usize])
                        && let Some(n) = int_op(op, *x, *y)
                    {
                        put(&mut w[var as usize], Value::Int(n));
                    } else if !assign_op(w, op, var, src) {
                        slow!();
                    }
                    pc += 1;
                }
                // $i++ / ++$i (and --) whose value is used, on an int
                Instr::IncDec { var, dst, inc, post } => {
                    let Value::Int(slot) = &mut w[var as usize] else { slow!() };
                    let i = *slot;
                    let Some(n) = (if inc { i.checked_add(1) } else { i.checked_sub(1) }) else { slow!() };
                    *slot = n;
                    put(&mut w[dst as usize], Value::Int(if post { i } else { n }));
                    pc += 1;
                }
                Instr::LoadNull { dst } => {
                    put(&mut w[dst as usize], Value::Null);
                    pc += 1;
                }
                // `!x` and a condition made a value: as `truth` decides
                // (NAN and an undefined variable warn: `step`).
                Instr::Not { dst, src } => {
                    let t = match &w[src as usize] {
                        Value::Bool(b) => *b,
                        v => match truth(v) {
                            Some(t) => t,
                            None => slow!(),
                        },
                    };
                    put(&mut w[dst as usize], Value::Bool(!t));
                    pc += 1;
                }
                Instr::ToBool { dst, src } => {
                    let t = match &w[src as usize] {
                        Value::Bool(b) => *b,
                        v => match truth(v) {
                            Some(t) => t,
                            None => slow!(),
                        },
                    };
                    put(&mut w[dst as usize], Value::Bool(t));
                    pc += 1;
                }
                Instr::IssetVar { dst, var } => {
                    let set = is_set(&w[var as usize]);
                    put(&mut w[dst as usize], Value::Bool(set));
                    pc += 1;
                }
                // `a ?? b`: a set and not null goes to dst and jumps.
                Instr::JumpIfSet { src, dst, target } => {
                    let v = match &w[src as usize] {
                        Value::Undef | Value::Null => None,
                        Value::Ref(cell) => match read_ref(cell) {
                            Some(Value::Null) | None => None,
                            v => v,
                        },
                        v => Some(v.clone()),
                    };
                    match v {
                        Some(v) => {
                            put(&mut w[dst as usize], v);
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
                    let a = Value::Array(Rc::new(PhpArray::list_with_capacity(n as usize)));
                    put(&mut w[dst as usize], a);
                    pc += 1;
                }
                Instr::ArrayPush { arr, val, take } => {
                    let Value::Array(a) = &w[arr as usize] else { slow!() };
                    if a.next_key().is_none() {
                        slow!();
                    }
                    let Some(v) = taken(w, val, take) else { slow!() };
                    let Value::Array(a) = &mut w[arr as usize] else { unreachable!("checked above") };
                    let _ = Rc::make_mut(a).push(v);
                    pc += 1;
                }
                Instr::ArraySet { arr, key, val, take } => {
                    let k = match (&w[arr as usize], &w[key as usize]) {
                        (Value::Array(_), Value::Int(k)) => Key::Int(*k),
                        (Value::Array(_), Value::Str(s)) => Key::from_string(s.clone()),
                        _ => slow!(),
                    };
                    let Some(v) = taken(w, val, take) else { slow!() };
                    let Value::Array(a) = &mut w[arr as usize] else { unreachable!("checked above") };
                    Rc::make_mut(a).insert(k, v);
                    pc += 1;
                }
                // A constant the site resolved (a deprecated one is never
                // cached: it warns on every fetch).
                Instr::FetchConst { site, dst } => {
                    let Some(Callee::Constant(v)) = clo.sites[site as usize].get() else { slow!() };
                    let v = v.clone();
                    put(&mut w[dst as usize], v);
                    pc += 1;
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
                Instr::Call { site, base: args, argc: n, dst } => match clo.sites[site as usize].get() {
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
                        pc = self.builtin_call(regs, pc, base, i, args, n, dst)?;
                        continue 'frame;
                    }
                    _ => slow!(),
                },
                // A user method the call site's cache names for this class
                // and scope, on an object, with its required arguments, and
                // no interrupt pending: anything else is `step`'s.
                Instr::MethodCall { dst, obj, method, base: args, argc: n } => {
                    let Value::Object(o) = &w[obj as usize] else { slow!() };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let found = {
                        let site = clo.methods[method as usize].borrow();
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
                        clear(&mut w[obj as usize]);
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
                    leave(&mut self.frames, regs, Value::Null);
                    pc = self.frames.last().expect("the caller's frame").pc as usize;
                    if self.ids.has_pending() {
                        pc = self.after_errors(regs, pc, pc.saturating_sub(1))?;
                    }
                    continue 'frame;
                }
            }
        }
        }
    }
}

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
        Some(_) if i >= argc || matches!(v, Value::Undef) => true,
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
    let need = base + code.nregs as usize;
    if regs.len() < need {
        regs.resize(need, Value::Undef);
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
    match r {
        Value::Undef => None,
        Value::Ref(cell) => Some(cell.borrow().clone()),
        _ if temp || own_var => Some(std::mem::take(r)),
        v => Some(v.clone()),
    }
}

/// The constant defaults (`Closure::defaults`) of the parameters from
/// `from` on whose registers (`params`, the frame's first) are empty.
#[inline(never)]
pub(crate) fn fill_defaults(defaults: &[Option<Value>], params: &mut [Value], from: usize) {
    for (r, d) in params.iter_mut().zip(defaults.iter()).skip(from) {
        if let Some(d) = d
            && matches!(r, Value::Undef)
        {
            *r = d.clone();
        }
    }
}

/// Whether an ini value turns a flag on ("1", "on", "yes", "true").
pub(crate) fn ini_on(v: &Value) -> bool {
    match v {
        Value::Str(s) => matches!(s.as_bytes().to_ascii_lowercase().as_slice(), b"1" | b"on" | b"true" | b"yes"),
        v => v.to_bool(),
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
    // The Closure it was called as (a generator's frame gave its
    // Generator back before it got here).
    if frame.owner.is_some() {
        frame.owner = None;
    }
    if frame.ret != NO_RET {
        regs[frame.ret as usize] = value;
    }
}

/// Whether a value holds nothing to release (a scalar, null, undefined):
/// overwriting it is a store. (The scalars are listed, so any value
/// holding memory is released.)
#[inline(always)]
pub(crate) fn is_plain(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Undef)
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
#[inline(always)]
pub(crate) fn put(slot: &mut Value, v: Value) {
    let old = std::mem::replace(slot, v);
    if is_plain(&old) {
        std::mem::forget(old);
    } else {
        release(old);
    }
}

/// `put` into a variable: through the reference it holds, if it is one.
#[inline(always)]
pub(crate) fn put_var(slot: &mut Value, v: Value) {
    match slot {
        Value::Ref(cell) => write_ref(cell, v),
        _ => put(slot, v),
    }
}

/// Write through a reference. Out of line: common (`global $x`), not hot.
#[inline(never)]
pub(crate) fn write_ref(cell: &RefCell<Value>, v: Value) {
    put(&mut cell.borrow_mut(), v);
}

/// A reference's value, for a read: None when it is undefined (a global
/// another file unset: `step` warns).
#[inline(never)]
pub(crate) fn read_ref(cell: &RefCell<Value>) -> Option<Value> {
    match &*cell.borrow() {
        Value::Undef => None,
        v => Some(v.clone()),
    }
}

/// Make a register undefined, releasing what it held.
#[inline(always)]
pub(crate) fn clear(r: &mut Value) {
    put(r, Value::Undef);
}

/// An operand to store (`operand_take`): moved out of its register when
/// `take` (a temporary read for the last time), else copied; a reference's
/// value is read through it (the taken register lets go of the
/// reference). None when undefined (`step` warns).
#[inline(always)]
fn taken(w: &mut [Value], r: Reg, take: bool) -> Option<Value> {
    let v = match &w[r as usize] {
        Value::Undef => return None,
        Value::Ref(cell) => read_ref(cell)?,
        _ if take => return Some(std::mem::take(&mut w[r as usize])),
        v => return Some(v.clone()),
    };
    if take {
        clear(&mut w[r as usize]);
    }
    Some(v)
}

/// isset's test of an element: there and not null (through a reference).
#[inline(always)]
fn is_set(v: &Value) -> bool {
    match v {
        Value::Null | Value::Undef => false,
        Value::Ref(cell) => !matches!(*cell.borrow(), Value::Null | Value::Undef),
        _ => true,
    }
}

/// An int, through a reference.
#[inline(always)]
fn int_of(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Ref(cell) => int_in(cell),
        _ => None,
    }
}

#[inline(never)]
fn int_in(cell: &RefCell<Value>) -> Option<i64> {
    match *cell.borrow() {
        Value::Int(i) => Some(i),
        _ => None,
    }
}

/// A number (an int or a float), through a reference.
#[inline(always)]
fn num_of(v: &Value) -> Option<Value> {
    match v {
        Value::Int(_) | Value::Float(_) => Some(v.clone()),
        Value::Ref(cell) => match &*cell.borrow() {
            n @ (Value::Int(_) | Value::Float(_)) => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// `++`/`--` of an int through a reference: the new value (None: not an
/// int there, or it overflows: `step`).
#[inline(never)]
fn step_ref(cell: &RefCell<Value>, step: fn(i64, i64) -> Option<i64>) -> Option<i64> {
    let mut v = cell.borrow_mut();
    let Value::Int(i) = &mut *v else { return None };
    let n = step(*i, 1)?;
    *i = n;
    Some(n)
}

/// `a cmp b` for anything but two ints: through references, numbers as
/// `num_test`, and every other pair of defined values as `step` compares
/// them (`test_values`: nothing there warns). None for an undefined
/// variable (`step` warns) or `<=>`.
#[inline(never)]
fn test(w: &[Value], cmp: Cmp, a: Reg, b: Reg) -> Option<bool> {
    if cmp == Cmp::Spaceship {
        return None;
    }
    let (x, y) = (&w[a as usize], &w[b as usize]);
    match (x, y) {
        (Value::Undef, _) | (_, Value::Undef) => None,
        (Value::Ref(_), _) | (_, Value::Ref(_)) => {
            let (x, y) = (deref_defined(x)?, deref_defined(y)?);
            Some(num_test(cmp, &x, &y).unwrap_or_else(|| test_values(cmp, &x, &y)))
        }
        _ => Some(num_test(cmp, x, y).unwrap_or_else(|| test_values(cmp, x, y))),
    }
}

/// A value read through its reference (if it is one); None when undefined.
#[inline(always)]
fn deref_defined(v: &Value) -> Option<Value> {
    match v {
        Value::Ref(cell) => read_ref(cell),
        Value::Undef => None,
        v => Some(v.clone()),
    }
}

/// `dst = a op b` beyond two ints: floats, operands through references,
/// `.` on strings. False (nothing done) when `step` must run it: a
/// warning, an error, a conversion with side effects, an overflow.
#[inline(never)]
fn arith(w: &mut [Value], op: BinOp, dst: Reg, a: Reg, b: Reg) -> bool {
    let (x, y) = (&w[a as usize], &w[b as usize]);
    let r = match (x, y) {
        (Value::Ref(_), _) | (_, Value::Ref(_)) => match (num_of(x), num_of(y)) {
            (Some(x), Some(y)) => num_op(op, &x, &y),
            _ => None,
        },
        _ => num_op(op, x, y),
    };
    match r {
        Some(r) => {
            put_var(&mut w[dst as usize], r);
            true
        }
        None => op == BinOp::Concat && concat(w, dst, a, b),
    }
}

/// `var op= src` beyond two ints: numbers (through references), `.=` on
/// strings. False when `step` must run it.
#[inline(never)]
fn assign_op(w: &mut [Value], op: BinOp, var: Reg, src: Reg) -> bool {
    let (x, y) = (&w[var as usize], &w[src as usize]);
    let r = match (x, y) {
        (Value::Ref(_), _) | (_, Value::Ref(_)) => match (num_of(x), num_of(y)) {
            (Some(x), Some(y)) => num_op(op, &x, &y),
            _ => None,
        },
        _ => num_op(op, x, y),
    };
    match r {
        Some(r) => {
            put_var(&mut w[var as usize], r);
            true
        }
        None => op == BinOp::Concat && append_str(w, var, src),
    }
}

/// A condition that is not a bool: what i_zend_is_true says, when
/// nothing can warn (NAN, an undefined variable: None, `step`'s).
#[inline(never)]
fn truth(v: &Value) -> Option<bool> {
    match v {
        Value::Undef => None,
        Value::Float(f) if f.is_nan() => None,
        Value::Ref(cell) => match &*cell.borrow() {
            Value::Undef => None,
            Value::Float(f) if f.is_nan() => None,
            v => Some(v.to_bool()),
        },
        v => Some(v.to_bool()),
    }
}

/// `$a[k]` beyond an array and an int key: a string key, an array held
/// through a reference; None (`step`: a warning, an object, a string's
/// offset) otherwise. The element must not be a reference.
#[inline(never)]
fn elem(w: &[Value], arr: Reg, key: Reg) -> Option<Value> {
    let at = |a: &PhpArray| match &w[key as usize] {
        Value::Int(k) => a.get_int(*k).cloned(),
        Value::Str(s) => a.get(&Key::from_string(s.clone())).cloned(),
        _ => None,
    };
    let v = match &w[arr as usize] {
        Value::Array(a) => at(a),
        Value::Ref(cell) => match &*cell.borrow() {
            Value::Array(a) => at(a),
            _ => None,
        },
        _ => None,
    };
    match v {
        Some(Value::Ref(_)) | None => None,
        v => v,
    }
}

/// isset($a[k]) beyond an array and an int key: a string key, an array
/// held through a reference; None (`step`) otherwise.
#[inline(never)]
fn isset_elem(w: &[Value], var: Reg, keys: Reg) -> Option<bool> {
    let at = |a: &PhpArray| match &w[keys as usize] {
        Value::Int(k) => Some(a.get_int(*k).is_some_and(is_set)),
        Value::Str(s) => Some(a.get(&Key::from_string(s.clone())).is_some_and(is_set)),
        _ => None,
    };
    match &w[var as usize] {
        Value::Array(a) => at(a),
        Value::Ref(cell) => match &*cell.borrow() {
            Value::Array(a) => at(a),
            _ => None,
        },
        _ => None,
    }
}

/// `$a[k] = v` for a plain `v`, unless the element is a reference (false).
#[inline(always)]
pub(crate) fn set_int_elem(a: &mut Rc<PhpArray>, k: i64, v: &Value) -> bool {
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
/// are shared until written; a reference's value is read through it),
/// unless it is undefined (a warning: `step`'s).
#[inline(always)]
pub(crate) fn copied(v: &Value) -> Option<Value> {
    match v {
        Value::Undef => None,
        Value::Ref(cell) => read_ref(cell),
        v => Some(v.clone()),
    }
}

/// A copy of a value that holds nothing reference-counted.
#[inline(always)]
pub(crate) fn plain(v: &Value) -> Option<Value> {
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
pub(crate) fn num_test(cmp: Cmp, x: &Value, y: &Value) -> Option<bool> {
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

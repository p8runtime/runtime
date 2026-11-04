//! The common cases of the hottest instructions (`quick`), and the frame
//! entry and exit they share with the handlers.

use super::*;

impl Context<'_> {
    /// Run `instr` (at `pc` of the top frame: `clo`'s, its registers at
    /// `base`) when it is one of the common cases: those that need only the
    /// frame's registers (ints and floats, jumps), plus calls to a user
    /// function the call site already resolved and returns of a value,
    /// which switch frames here. None: its handler runs it in full.
    ///
    /// Inlined into each instruction's handler, which keeps only its case.
    #[inline(always)]
    pub(crate) fn quick(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, pc: usize, instr: Instr) -> Option<Next> {
        let stop = &*self.stop;
        let go = |next: usize| Some(Next::Go(next));
        let w = &mut regs[base..base + clo.code.nregs as usize];
        match instr {
            // `$this`, in a method that has one.
            Instr::This { dst } => {
                let this = Rc::clone(self.frames.last()?.this.as_ref()?);
                w[dst as usize] = Value::Object(this);
                go(pc + 1)
            }
            // A property read the name's cache entry answers (a miss, an
            // unset slot or a reference operand: the handler's).
            Instr::FetchProp { dst, obj, name, .. } => {
                let site = (name as usize).checked_sub(clo.code.consts_at as usize).and_then(|i| clo.props.get(i))?;
                let scope = self.frames.last()?.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                let Value::Object(o) = &w[obj as usize] else { return None };
                let v = cached_prop(site.get(), o, scope)?;
                w[dst as usize] = v;
                go(pc + 1)
            }
            // Function entry, when every passed argument is plainly of its
            // declared type (`verify_args` would accept each as it is).
            Instr::Rare { op: Rare::VerifyArgs, .. } => {
                let argc = self.frames.last()?.argc as usize;
                if !args_plain(&clo.code.params, w, argc) {
                    return None;
                }
                go(pc + 1)
            }
            // A return value plainly of the declared type (a generator's
            // is the handler's).
            Instr::Rare { op: Rare::VerifyReturn, a, .. } => match &clo.code.ret_type {
                Some(ty) if clo.code.is_generator || !ty.quick(&w[a as usize]) => None,
                _ => go(pc + 1),
            },
            // `$o->m($v)`: an argument the method the call site's cache
            // names for this class and scope takes by value is copied.
            Instr::SendVarMethod { obj, method, n, arg, var } => {
                let scope = self.frames.last()?.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                let Value::Object(o) = &w[obj as usize] else { return None };
                let by_value = clo.methods[method as usize].borrow().as_ref().is_some_and(|s| {
                    s.class == Rc::as_ptr(&o.class) as usize
                        && s.scope == scope
                        && matches!(&s.code, MethodCode::User(code) if code.upgrade().is_some_and(|c| !c.code.param_by_ref(n as usize)))
                });
                if !by_value {
                    return None;
                }
                let v = copied(&w[var as usize])?;
                w[arg as usize] = v;
                go(pc + 1)
            }
            // The register-only cases the loop itself may run (`hot`).
            i if is_hot(&i) => hot(w, pc, instr, stop).map(Next::Go),
            Instr::ConcatN { dst, start, n } => {
                if !concat_n(w, dst, start, n) {
                    return None;
                }
                go(pc + 1)
            }
            // A user function the call site resolved, with its required
            // arguments, and no interrupt pending.
            Instr::Call { site, base: args, argc: n, dst } => {
                let Some(&Callee::User(target)) = clo.sites[site as usize].get() else { return None };
                let target = &self.closures[target as usize];
                if (n as usize) < target.code.required || target.code.is_generator || stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                    return None;
                }
                if let Some(caller) = self.frames.last_mut() {
                    caller.pc = (pc + 1) as u32;
                }
                self.frames.push(target, base + args as usize, n as usize, (base + dst as usize) as u32);
                enter(&mut self.frames, regs);
                Some(Next::Switch)
            }
            // A user method the call site's cache names for this class and
            // scope, on an object, with its required arguments, and no
            // interrupt pending.
            Instr::MethodCall { dst, obj, method, base: args, argc: n } => {
                let Value::Object(o) = &regs[base + obj as usize] else { return None };
                let scope = self.frames.last()?.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                let (target, declaring) = {
                    let site = clo.methods[method as usize].borrow();
                    let s = site.as_ref()?;
                    if s.class != Rc::as_ptr(&o.class) as usize || s.scope != scope || s.is_static {
                        return None;
                    }
                    let MethodCode::User(code) = &s.code else { return None };
                    (code.upgrade()?, s.declaring.upgrade()?)
                };
                if (n as usize) < target.code.required || target.code.is_generator || stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                    return None;
                }
                let this = Rc::clone(o);
                // A temporary (`(new A)->f()`) is released as it is used:
                // the call's `$this` holds the object.
                if is_temp(&clo.code, obj) {
                    clear(&mut regs[base + obj as usize]);
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
                Some(Next::Switch)
            }
            // The bottom frame of this `execute` returns in the handler.
            Instr::Return { .. } | Instr::ReturnNull if self.frames.len() == self.frames.floor + 1 => None,
            Instr::Return { src } => {
                let value = match &regs[base + src as usize] {
                    Value::Undef => return None,
                    Value::Ref(cell) => cell.borrow().clone(),
                    v => v.clone(),
                };
                leave(&mut self.frames, regs, value);
                Some(Next::Switch)
            }
            Instr::ReturnNull => {
                leave(&mut self.frames, regs, Value::Null);
                Some(Next::Switch)
            }
            _ => None,
        }
    }
}

/// The common cases that need only the frame's registers `w`: jumps,
/// compare-and-branch and arithmetic on numbers, increments, copies.
/// The next pc, or None (the instruction's handler runs it). A loop's
/// backward branch is where an interrupt is noticed: the handler then
/// raises it. `quick` runs these, and the loop itself may too.
#[inline(always)]
pub(crate) fn hot(w: &mut [Value], pc: usize, instr: Instr, stop: &AtomicU64) -> Option<usize> {
    let back = |target: u32| target as usize <= pc && stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT;
    // A compare-and-branch on two numbers.
    macro_rules! jump_if {
        ($cmp:expr, $when:expr, $a:expr, $b:expr, $target:expr) => {
            match num_test($cmp, &w[$a as usize], &w[$b as usize])? {
                t if t == $when => {
                    if back($target) {
                        return None;
                    }
                    Some($target as usize)
                }
                _ => Some(pc + 1),
            }
        };
    }
    // `dst = a op b` on numbers (and `.` on strings).
    macro_rules! arith {
        ($op:expr, $dst:expr, $a:expr, $b:expr) => {
            match num_op($op, &w[$a as usize], &w[$b as usize]) {
                // A variable holding a reference is written through it.
                Some(_) if matches!(w[$dst as usize], Value::Ref(_)) => None,
                Some(r) => {
                    w[$dst as usize] = r;
                    Some(pc + 1)
                }
                None if $op == BinOp::Concat && concat(w, $dst, $a, $b) => Some(pc + 1),
                None => None,
            }
        };
    }
    // A fused step and test (`IncJumpLt` and co.) on ints: step, then
    // jump or go past the test. A non-int bound leaves the test to the
    // next instruction; a non-int variable leaves both to the handler.
    macro_rules! step_jump {
        ($var:expr, $b:expr, $target:expr, $step:expr, $holds:expr) => {{
            // Written in place: no drop of the old value to compile.
            let Value::Int(i) = &mut w[$var as usize] else { return None };
            let n = $step(*i, 1)?;
            *i = n;
            match w[$b as usize] {
                Value::Int(b) if $holds(n, b) => {
                    if back($target) {
                        // The step is done: the test (next) sees the interrupt.
                        return Some(pc + 1);
                    }
                    Some($target as usize)
                }
                Value::Int(_) => Some(pc + 2),
                _ => Some(pc + 1),
            }
        }};
    }
    match instr {
        Instr::Jump { target } => {
            if back(target) {
                return None;
            }
            Some(target as usize)
        }
        Instr::JumpLt { a, b, target } => jump_if!(Cmp::Lt, true, a, b, target),
        Instr::JumpNotLt { a, b, target } => jump_if!(Cmp::Lt, false, a, b, target),
        Instr::JumpLe { a, b, target } => jump_if!(Cmp::Le, true, a, b, target),
        Instr::JumpNotLe { a, b, target } => jump_if!(Cmp::Le, false, a, b, target),
        Instr::JumpEq { a, b, target } => jump_if!(Cmp::Eq, true, a, b, target),
        Instr::JumpNotEq { a, b, target } => jump_if!(Cmp::Eq, false, a, b, target),
        Instr::JumpIdentical { a, b, target } => jump_if!(Cmp::Identical, true, a, b, target),
        Instr::JumpNotIdentical { a, b, target } => jump_if!(Cmp::Identical, false, a, b, target),
        Instr::JumpIf { cond, target } => match w[cond as usize] {
            Value::Bool(true) if back(target) => None,
            Value::Bool(c) => Some(if c { target as usize } else { pc + 1 }),
            _ => None,
        },
        Instr::JumpIfNot { cond, target } => match w[cond as usize] {
            Value::Bool(false) if back(target) => None,
            Value::Bool(c) => Some(if c { pc + 1 } else { target as usize }),
            _ => None,
        },
        Instr::IncVar { var } => match &mut w[var as usize] {
            Value::Int(i) if *i != i64::MAX => {
                *i += 1;
                Some(pc + 1)
            }
            _ => None,
        },
        Instr::DecVar { var } => match &mut w[var as usize] {
            Value::Int(i) if *i != i64::MIN => {
                *i -= 1;
                Some(pc + 1)
            }
            _ => None,
        },
        Instr::IncJumpLt { var, b, target } => step_jump!(var, b, target, i64::checked_add, |n, b| n < b),
        Instr::IncJumpLe { var, b, target } => step_jump!(var, b, target, i64::checked_add, |n, b| n <= b),
        Instr::DecJumpGt { var, b, target } => step_jump!(var, b, target, i64::checked_sub, |n, b| n > b),
        Instr::DecJumpGe { var, b, target } => step_jump!(var, b, target, i64::checked_sub, |n, b| n >= b),
        Instr::Binary { op, dst, a, b } => arith!(op, dst, a, b),
        Instr::Add { dst, a, b } => arith!(BinOp::Add, dst, a, b),
        Instr::Sub { dst, a, b } => arith!(BinOp::Sub, dst, a, b),
        Instr::Mul { dst, a, b } => arith!(BinOp::Mul, dst, a, b),
        // A write that frees an object with a destructor: the loop runs
        // it before the next instruction.
        Instr::Move { dst, src } => {
            let v = copied(&w[src as usize])?;
            w[dst as usize] = v;
            Some(pc + 1)
        }
        Instr::Assign { var, src } => {
            let v = copied(&w[src as usize])?;
            assign(w, var, v);
            Some(pc + 1)
        }
        Instr::Compare { cmp, dst, a, b } => match num_test(cmp, &w[a as usize], &w[b as usize]) {
            Some(t) if cmp != Cmp::Spaceship && !matches!(w[dst as usize], Value::Ref(_)) => {
                w[dst as usize] = Value::Bool(t);
                Some(pc + 1)
            }
            _ => None,
        },
        // $var op= number, $var holding a number
        Instr::AssignOp { op, var, src } => match num_op(op, &w[var as usize], &w[src as usize]) {
            Some(r) => {
                w[var as usize] = r;
                Some(pc + 1)
            }
            // $str .= string
            None if op == BinOp::Concat && append_str(w, var, src) => Some(pc + 1),
            None => None,
        },
        Instr::AssignTake { var, src } => match std::mem::replace(&mut w[src as usize], Value::Undef) {
            // Through a reference, or undefined: the handler's.
            v @ (Value::Ref(_) | Value::Undef) => {
                w[src as usize] = v;
                None
            }
            v => {
                assign(w, var, v);
                Some(pc + 1)
            }
        },
        Instr::Free { reg } => {
            clear(&mut w[reg as usize]);
            Some(pc + 1)
        }
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
            }?;
            w[dst as usize] = v;
            Some(pc + 1)
        }
        // $a[int] = int|float|bool|null, $a an array (or a reference to one)
        Instr::AssignDim { var, keys, n: 1, append: false, val, dst } => {
            if let Value::Str(_) = w[keys as usize] {
                if !set_str_elem(w, var, keys, val, dst) {
                    return None;
                }
                return Some(pc + 1);
            }
            let (Value::Int(k), Some(v)) = (&w[keys as usize], plain(&w[val as usize])) else { return None };
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
                return None;
            }
            w[dst as usize] = v;
            Some(pc + 1)
        }
        // $a[int] op= number, $a an array holding a number at that key
        Instr::AssignDimOp { op, var, keys, n: 1, val, dst } => {
            if !dim_op(w, op, var, keys, val, dst) {
                return None;
            }
            Some(pc + 1)
        }
        // An object (a generator, an Iterator) is the handler's.
        Instr::IterNext { iter, val, key, has_key, end } => {
            if !matches!(w[iter as usize], Value::Array(_)) {
                return None;
            }
            Some(if iter_next(w, iter, val, key, has_key) { pc + 1 } else { end as usize })
        }
        // $i++ / ++$i (and --) whose value is used, on an int
        Instr::IncDec { var, dst, inc, post } => {
            let Value::Int(slot) = &mut w[var as usize] else { return None };
            let i = *slot;
            let n = if inc { i.checked_add(1) } else { i.checked_sub(1) }?;
            *slot = n;
            w[dst as usize] = Value::Int(if post { i } else { n });
            Some(pc + 1)
        }
        _ => None,
    }
}

/// Whether `instr` is one of `hot`'s.
#[inline(always)]
pub(crate) fn is_hot(instr: &Instr) -> bool {
    matches!(
        instr,
        Instr::Jump { .. }
            | Instr::JumpLt { .. }
            | Instr::JumpNotLt { .. }
            | Instr::JumpLe { .. }
            | Instr::JumpNotLe { .. }
            | Instr::JumpEq { .. }
            | Instr::JumpNotEq { .. }
            | Instr::JumpIdentical { .. }
            | Instr::JumpNotIdentical { .. }
            | Instr::JumpIf { .. }
            | Instr::JumpIfNot { .. }
            | Instr::IncVar { .. }
            | Instr::DecVar { .. }
            | Instr::IncJumpLt { .. }
            | Instr::IncJumpLe { .. }
            | Instr::DecJumpGt { .. }
            | Instr::DecJumpGe { .. }
            | Instr::Binary { .. }
            | Instr::Add { .. }
            | Instr::Sub { .. }
            | Instr::Mul { .. }
            | Instr::Move { .. }
            | Instr::Assign { .. }
            | Instr::Compare { .. }
            | Instr::AssignOp { .. }
            | Instr::AssignTake { .. }
            | Instr::Free { .. }
            | Instr::FetchDim { .. }
            | Instr::AssignDim { n: 1, append: false, .. }
            | Instr::AssignDimOp { n: 1, .. }
            | Instr::IterNext { .. }
            | Instr::IncDec { .. }
    )
}

/// Run `hot`'s cases from `pc` of a frame (`code`, its registers `w`) on,
/// until an instruction it leaves to its handler, or a write that freed
/// an object with a destructor (the loop runs it next): that pc. A small
/// function of its own, so its state stays in machine registers.
#[inline(never)]
pub(crate) fn hot_run(w: &mut [Value], code: &[Instr], mut pc: usize, stop: &AtomicU64, ids: &Ids) -> usize {
    loop {
        let instr = code[pc];
        match hot(w, pc, instr, stop) {
            Some(next) => pc = next,
            None => return pc,
        }
        if ids.has_pending() {
            return pc;
        }
    }
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
    // Only the constants read as values (names stay in the constants).
    let values = code.value_consts as usize;
    for (r, c) in regs[base + consts_at..base + consts_at + values].iter_mut().zip(func.consts.iter()) {
        clear(r);
        // `r` is undefined now: nothing to drop.
        std::mem::forget(std::mem::replace(r, c.clone()));
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
    let frame = frames.pop();
    frame.extra = None;
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
    if frame.ret != NO_RET {
        regs[frame.ret as usize] = value;
    }
}

/// Make a register undefined, releasing what it held. Scalars hold
/// nothing to release, so they are only re-tagged.
#[inline(always)]
pub(crate) fn clear(r: &mut Value) {
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
pub(crate) fn int_elem(a: &PhpArray, k: i64) -> Option<Value> {
    match a.get_int(k) {
        Some(Value::Ref(_)) | None => None,
        Some(v) => Some(v.clone()),
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

/// A copy of a register's value, as the handlers make one (strings and
/// arrays are shared until written), unless it is undefined (a warning)
/// or a reference (read through its cell).
#[inline(always)]
pub(crate) fn copied(v: &Value) -> Option<Value> {
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

//! The fast tier: the instruction loop for the common cases, and the
//! frame entry and exit it shares with `step`.

use super::*;

/// Run the common cases of the hottest instructions, from `pc` of the top
/// frame on: those that need only the frame's registers (ints and floats,
/// jumps), plus calls to a user function the call site already resolved
/// and returns of a value, which switch frames here. Returns the pc (in
/// the top frame as it is then) of the first instruction it cannot run,
/// for `step`.
///
/// Kept apart from everything else so its state stays in machine registers.
#[inline(never)]
pub(crate) fn fast(regs: &mut Vec<Value>, frames: &mut Frames, closures: &[Rc<Closure>], mut pc: usize, stop: &AtomicU64, ids: &Ids) -> usize {
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
                // `$this`, in a method that has one (else `step` raises).
                Instr::This { dst } => match &top.this {
                    Some(this) => {
                        w[dst as usize] = Value::Object(Rc::clone(this));
                        pc += 1;
                    }
                    None => return pc,
                },
                // A property read the name's cache entry answers (anything
                // else, `step`: a miss, an unset slot, a reference operand).
                Instr::FetchProp { dst, obj, name, .. } => {
                    let site = (name as usize).checked_sub(clo.code.consts_at as usize).and_then(|i| clo.props.get(i));
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let hit = match (site, &w[obj as usize]) {
                        (Some(site), Value::Object(o)) => cached_prop(site.get(), o, scope),
                        _ => None,
                    };
                    let Some(v) = hit else { return pc };
                    w[dst as usize] = v;
                    pc += 1;
                }
                // Function entry, when every passed argument is plainly of
                // its declared type (`verify_args` would accept each as it
                // is); a conversion, a class check or a variadic: `step`.
                Instr::Rare { op: Rare::VerifyArgs, .. } => {
                    if !args_plain(&clo.code.params, w, top.argc as usize) {
                        return pc;
                    }
                    pc += 1;
                }
                // A return value plainly of the declared type (anything
                // else, a generator's included: `step`).
                Instr::Rare { op: Rare::VerifyReturn, a, .. } => match &clo.code.ret_type {
                    Some(ty) if clo.code.is_generator || !ty.quick(&w[a as usize]) => return pc,
                    _ => pc += 1,
                },
                // `$o->m($v)`: an argument the method the call site's cache
                // names for this class and scope takes by value is copied
                // (by reference, a miss, or an undefined or reference
                // variable: `step`, which looks the method up by name).
                Instr::SendVarMethod { obj, method, n, arg, var } => {
                    let Value::Object(o) = &w[obj as usize] else { return pc };
                    let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                    let by_value = clo.methods[method as usize].borrow().as_ref().is_some_and(|s| {
                        s.class == Rc::as_ptr(&o.class) as usize
                            && s.scope == scope
                            && matches!(&s.code, MethodCode::User(code) if code.upgrade().is_some_and(|c| !c.code.param_by_ref(n as usize)))
                    });
                    if !by_value {
                        return pc;
                    }
                    let Some(v) = copied(&w[var as usize]) else { return pc };
                    w[arg as usize] = v;
                    pc += 1;
                    if ids.has_pending() {
                        return pc;
                    }
                }
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
                | Instr::AssignProp { .. }
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
                | Instr::CatchIf { .. } => return pc,
                // Calls and returns leave this loop, so the loop itself
                // keeps only the code and the registers live.
                Instr::Call { site, base: args, argc: n, dst } => break Switch::Call(site, args, n, dst),
                Instr::MethodCall { dst, obj, method, base: args, argc: n } => break Switch::Method(dst, obj, method, args, n),
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
            Switch::Method(dst, obj, method, args, n) => {
                // A user method the call site's cache names for this class
                // and scope, on an object, with its required arguments, and
                // no interrupt pending: anything else is `step`'s.
                let Value::Object(o) = &regs[base + obj as usize] else { return pc };
                let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
                let (target, declaring) = {
                    let site = clo.methods[method as usize].borrow();
                    let Some(s) = site.as_ref() else { return pc };
                    if s.class != Rc::as_ptr(&o.class) as usize || s.scope != scope || s.is_static {
                        return pc;
                    }
                    let MethodCode::User(code) = &s.code else { return pc };
                    let (Some(target), Some(declaring)) = (code.upgrade(), s.declaring.upgrade()) else { return pc };
                    (target, declaring)
                };
                if (n as usize) < target.code.required || target.code.is_generator || stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                    return pc;
                }
                let this = Rc::clone(o);
                // A temporary (`(new A)->f()`) is released as it is used:
                // the call's `$this` holds the object.
                if is_temp(&clo.code, obj) {
                    clear(&mut regs[base + obj as usize]);
                }
                if let Some(caller) = frames.last_mut() {
                    caller.pc = (pc + 1) as u32;
                }
                frames.push(&target, base + args as usize, n as usize, (base + dst as usize) as u32);
                let frame = frames.last_mut().expect("the frame just pushed");
                frame.called = Some(Rc::clone(&this.class));
                frame.this = Some(this);
                frame.scope = Some(declaring);
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
pub(crate) enum Switch {
    Call(u16, Reg, u8, Reg),
    /// A method call: dst, obj, method (cache index), args, argc.
    Method(Reg, Reg, u16, Reg, u8),
    Return(Reg),
    ReturnNull,
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
    for (r, c) in regs[base + consts_at..base + consts_at + func.consts.len()].iter_mut().zip(&func.consts) {
        // A constant's register still holding this very constant (the
        // function called again from the same place: a loop) keeps it:
        // no drop of the old one, no copy of the new.
        if same_constant(r, c) {
            continue;
        }
        clear(r);
        // `r` is undefined now: nothing to drop.
        std::mem::forget(std::mem::replace(r, c.clone()));
    }
}

/// Whether register `r` holds constant `c` itself (the same string or
/// array, not only an equal one).
#[inline(always)]
fn same_constant(r: &Value, c: &Value) -> bool {
    match (r, c) {
        (Value::Str(a), Value::Str(b)) => PhpStr::ptr_eq(a, b),
        (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
        _ => false,
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
    // Rarely any: no drop call unless there is.
    if frame.extra.is_some() {
        frame.extra = None;
    }
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

/// A copy of a register's value, as `step` makes one (strings and arrays
/// are shared until written), unless it is undefined (a warning) or a
/// reference (read through its cell).
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

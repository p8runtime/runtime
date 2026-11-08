//! The interpreter loop's out-of-line arms: the instructions whose common
//! case needs more than the registers and a few loads (calls and returns,
//! which switch frames; property and element writes; the array literals;
//! the frameless builtins). Each is a function the loop calls with the
//! instruction, and goes on as its `Arm` says.
//!
//! They are out of line so that the loop itself stays small: every value
//! an arm keeps live across a call competes with the loop's own (`pc`, the
//! window, the code) for the six callee-saved registers, and with these
//! arms inlined the loop reloaded its own values from the stack on every
//! instruction (#352). The straight-line arms stay in `execute`.

use super::fast::{Arm, WINDOW, args_plain, arith, clear, copied, enter, leave, put, put_var, returned, set_int_elem, taken};
use super::*;
use p8_bytecode::{Insn, Op, insn};

/// A register operand (a byte: in the window by its type).
#[inline(always)]
fn rx(r: u8) -> usize {
    usize::from(r)
}

/// A register computed from one (an argument after the first): `lower`
/// keeps such blocks in the window, so the mask is the identity.
#[inline(always)]
fn rk(r: usize) -> usize {
    r & (WINDOW - 1)
}

/// The frame's window of registers (`enter` keeps the register file a
/// window ahead of every frame, so it exists).
#[inline(always)]
fn window(regs: &mut [Value], base: usize) -> &mut [Value; WINDOW] {
    (&mut regs[base..base + WINDOW]).try_into().expect("a window's worth of registers")
}

/// `done!`: the straight-line instruction at `pc` is done; `released`
/// says whether its write may have left a destructor due.
#[inline(always)]
fn done(ids: &Ids, pc: usize, released: bool) -> Arm {
    if released && ids.has_pending() { Arm::Pending(pc + 1) } else { Arm::Go(pc + 1) }
}

impl Context<'_> {
    /// The arm raised `t`: parked for the loop to return (`Arm::Fault`).
    #[cold]
    #[inline(never)]
    fn fault(&mut self, t: Throw) -> Arm {
        self.arm_throw = Some(t);
        Arm::Fault
    }

    /// `$o->name = v` the name's cache entry answers, on a property with
    /// nothing to check (`plain_slot`): the slots borrowed once, nothing
    /// cloned but the value, and that once when the result is freed next
    /// (a statement's is). An unset slot of a class with __set and its
    /// kin, and anything `plain_slot` declines, is `step`'s.
    #[inline(never)]
    pub(crate) fn arm_assign_prop(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (a, val, dst, imm) = (insn.a, insn.b, insn.c, insn.imm);
        let top = self.frames.top();
        let clo = &*top.func;
        let code = &clo.code.insns[..];
        let Some(site) = clo.props.get((imm & !insn::TAKE) as usize) else { return Arm::Slow };
        // AssignPropThis: the method's own `$this`, from the frame.
        let o = if insn.op == Op::AssignPropThis {
            let Some(o) = &top.this else { return Arm::Slow };
            o
        } else {
            let Some(o) = w[rx(a)].as_object() else { return Arm::Slow };
            o
        };
        let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
        let Some((slot, plain)) = plain_slot(site.get(), o, (scope, top.scope.as_deref()), &w[rx(val)], !self.reinit.is_empty()) else { return Arm::Slow };
        let unused = matches!(code.get(pc + 1), Some(next) if next.op == Op::Free && next.a == dst);
        let written = match o.slots().get(slot) {
            Some(s)
                if (o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0 && plain == Plain::Any)
                    || {
                        let unset = p8_runtime::object::Slot::peek(s, Value::is_undef);
                        if plain == Plain::Unset { unset && o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0 } else { !unset }
                    } =>
            {
                // The slot's value out, written as a register is, back
                // in. (Pending is tested below, whatever this released.)
                let mut cur = s.replace(Value::UNDEF);
                let _ = put_var(&mut cur, w[rx(val)].clone());
                p8_runtime::object::put_slot(s, cur);
                true
            }
            _ => false,
        };
        if !written {
            return Arm::Slow;
        }
        let next = if unused {
            pc + 2
        } else {
            let v = w[rx(val)].clone();
            let _ = put(&mut w[rx(dst)], v);
            pc + 1
        };
        if imm & insn::TAKE != 0 {
            let _ = clear(&mut w[rx(val)]);
        }
        // (The slot's old value may have been the last holder.)
        if self.ids.has_pending() {
            return Arm::Pending(next);
        }
        Arm::Go(next)
    }

    /// Function entry, when every passed argument is plainly of its
    /// declared type (`verify_args` would accept each as it is); a
    /// conversion, a class check or a variadic: `step`.
    #[inline(never)]
    pub(crate) fn arm_verify_args(&mut self, w: &mut [Value; WINDOW], pc: usize) -> Arm {
        let top = self.frames.top();
        if !args_plain(&top.func, &w[..], top.argc as usize) {
            return Arm::Slow;
        }
        Arm::Go(pc + 1)
    }

    /// A builtin PHP compiles to an opcode (strlen, count, the type tests,
    /// array_key_exists): the plain case here, once the site resolved to
    /// the builtin (the first run, and a disabled function, are `step`'s);
    /// the arguments are consumed as a call's are. Anything else is the
    /// call. (The kinds are numbered as `insn::INTRINSICS`.)
    #[inline(never)]
    pub(crate) fn arm_intrinsic(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (dst, args, argc, imm) = (insn.a, rx(insn.b), insn.c, insn.imm);
        let clo = &*self.frames.top().func;
        if !matches!(clo.sites[(imm & 0xffff) as usize].get(), Some(Callee::Builtin(_))) {
            return Arm::Slow;
        }
        let x = &w[args];
        let r = match imm >> 16 {
            0 => match x.as_str() {
                Some(s) => Value::int(s.len() as i64),
                None => return Arm::Slow,
            },
            1 => match x.as_array() {
                Some(arr) => Value::int(arr.len() as i64),
                None => return Arm::Slow,
            },
            9 => {
                let Some(arr) = w[rk(args + 1)].as_array() else { return Arm::Slow };
                if let Some(k) = x.as_int() {
                    Value::bool(arr.contains_key(&Key::Int(k)))
                } else if let Some(s) = x.as_str() {
                    Value::bool(arr.get_str(s).is_some())
                } else {
                    return Arm::Slow;
                }
            }
            _ if x.is_undef() || x.is_reference() => return Arm::Slow,
            2 => Value::bool(x.is_null()),
            3 => Value::bool(x.is_bool()),
            4 => Value::bool(x.is_int()),
            5 => Value::bool(x.is_float()),
            6 => Value::bool(x.is_str()),
            7 => Value::bool(x.is_array()),
            8 => Value::bool(x.as_object().is_some_and(|o| o.class.flags & p8_runtime::object::CLASS_RESOURCE == 0)),
            _ => return Arm::Slow,
        };
        // The arguments consumed may have been an object's last holder
        // (`count([new D])`): a destructor due runs here.
        let mut released = false;
        for i in 0..usize::from(argc) {
            released |= clear(&mut w[rk(args + i)]);
        }
        released |= put(&mut w[rx(dst)], r);
        done(&self.ids, pc, released)
    }

    /// `$o->m($v)`: an argument the method the call site's cache names for
    /// this class and scope takes by value is copied (by reference, a
    /// miss, or an undefined or reference variable: `step`, which looks
    /// the method up by name).
    #[inline(never)]
    pub(crate) fn arm_send_var_method(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (obj, arg, var, method, n) = (insn.a, insn.b, insn.c, (insn.imm & 0xffff) as usize, (insn.imm >> 16) as usize);
        let Some(o) = w[rx(obj)].as_object() else { return Arm::Slow };
        let top = self.frames.top();
        let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
        let by_value = top.func.methods[method].borrow().as_ref().is_some_and(|s| {
            s.class == Rc::as_ptr(&o.class) as usize
                && s.scope == scope
                && matches!(&s.code, MethodCode::User(code) if code.upgrade().is_some_and(|c| !c.code.param_by_ref(n)))
        });
        if !by_value {
            return Arm::Slow;
        }
        let Some(v) = copied(&w[rx(var)]) else { return Arm::Slow };
        done(&self.ids, pc, put(&mut w[rx(arg)], v))
    }

    /// An argument a user function or builtin the call site resolved to
    /// takes by value: a copy (by reference, an unresolved site, an
    /// undefined variable: `step`).
    #[inline(never)]
    pub(crate) fn arm_send_var(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (a, b, imm) = (insn.a, insn.b, insn.imm);
        let n = (imm >> 16) as usize;
        let clo = &*self.frames.top().func;
        let by_value = match clo.sites[(imm & 0xffff) as usize].get() {
            Some(&Callee::User(f)) => !self.codes[f as usize].param_by_ref(n),
            Some(&Callee::Builtin(i)) => !self.isolate.registry.by_ref(i, n),
            Some(Callee::Constant(_)) => true,
            None => false,
        };
        if !by_value {
            return Arm::Slow;
        }
        let Some(v) = copied(&w[rx(b)]) else { return Arm::Slow };
        done(&self.ids, pc, put(&mut w[rx(a)], v))
    }

    /// `dst = a op b` for the operators without an int fast path.
    #[inline(never)]
    pub(crate) fn arm_binary(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        if !arith(&mut w[..], insn::binop(insn.imm), Reg::from(insn.a), Reg::from(insn.b), Reg::from(insn.c)) {
            return Arm::Slow;
        }
        done(&self.ids, pc, true)
    }

    #[inline(never)]
    pub(crate) fn arm_concat_n(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        if !concat_n(&mut w[..], Reg::from(insn.a), Reg::from(insn.b), insn.c) {
            return Arm::Slow;
        }
        done(&self.ids, pc, true)
    }

    /// $a[k] = v, $a an array (or a reference to one): a string key with
    /// any defined, non-reference value; an int key with a plain value.
    /// The element replaced may have been an object's last holder: tested
    /// as for any write.
    #[inline(never)]
    pub(crate) fn arm_assign_dim(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (var, keys, val, dst) = (insn.a, insn.b, insn.c, insn.imm as u8);
        if w[rx(keys)].is_str() {
            if !set_str_elem(&mut w[..], Reg::from(var), Reg::from(keys), Reg::from(val), Reg::from(dst)) {
                return Arm::Slow;
            }
        } else {
            let (Some(k), Some(v)) = (w[rx(keys)].as_int(), plain(&w[rx(val)])) else { return Arm::Slow };
            let slot = &mut w[rx(var)];
            let done = if slot.is_array() {
                set_int_elem(slot, k, &v)
            } else if let Some(cell) = slot.as_reference() {
                set_int_elem(&mut cell.borrow_mut(), k, &v)
            } else {
                false
            };
            if !done {
                return Arm::Slow;
            }
            let _ = put(&mut w[rx(dst)], v);
        }
        done(&self.ids, pc, true)
    }

    /// $a[int] op= number, $a an array holding a number at that key
    #[inline(never)]
    pub(crate) fn arm_assign_dim_op(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let imm = insn.imm;
        if !dim_op(&mut w[..], insn::binop(imm >> 8), Reg::from(insn.a), Reg::from(insn.b), Reg::from(insn.c), Reg::from(imm as u8)) {
            return Arm::Slow;
        }
        done(&self.ids, pc, true)
    }

    /// The loop variable's old value may have been an object's last
    /// holder: a destructor due runs before the body, at this instruction.
    #[inline(never)]
    pub(crate) fn arm_iter_next(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (a, b, c, imm) = (insn.a, insn.b, insn.c, insn.imm);
        // An object (a generator, an Iterator) is `step`'s.
        if !w[rx(a)].is_array() {
            return Arm::Slow;
        }
        let next = if iter_next(&mut w[..], Reg::from(a), Reg::from(b), Reg::from(c), insn.op == Op::IterNextKey) { pc + 1 } else { imm as usize };
        if self.ids.has_pending() {
            return Arm::Pending(next);
        }
        Arm::Go(next)
    }

    /// foreach over an array (a defined, non-reference register): the
    /// iterator is the array and position 0; an empty one jumps past the
    /// loop. (An object, a reference, anything that warns: `step`.)
    #[inline(never)]
    pub(crate) fn arm_iter_init(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (a, b, imm) = (insn.a, insn.b, insn.imm);
        let Some(arr) = w[rx(a)].as_array() else { return Arm::Slow };
        if arr.is_empty() {
            return Arm::Go(imm as usize);
        }
        let v = w[rx(a)].clone();
        let released = put(&mut w[rx(b)], v) | put(&mut w[rk(rx(b) + 1)], Value::int(0));
        done(&self.ids, pc, released)
    }

    /// An array literal's elements pushed or set (an int or string key; a
    /// value defined and read through a reference; anything else, a next
    /// key occupied included, is `step`'s).
    #[inline(never)]
    pub(crate) fn arm_array_push(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (a, b, imm) = (insn.a, insn.b, insn.imm);
        let Some(arr) = w[rx(a)].as_array() else { return Arm::Slow };
        if arr.next_key().is_none() {
            return Arm::Slow;
        }
        let Some(v) = taken(&mut w[..], Reg::from(b), imm & insn::TAKE != 0) else { return Arm::Slow };
        let _ = w[rx(a)].array_mut().expect("checked above").push(v);
        // (A taken temporary that held a reference let go of it.)
        done(&self.ids, pc, true)
    }

    #[inline(never)]
    pub(crate) fn arm_array_set(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let (a, b, c, imm) = (insn.a, insn.b, insn.c, insn.imm);
        if !w[rx(a)].is_array() {
            return Arm::Slow;
        }
        let key = &w[rx(b)];
        let k = if let Some(k) = key.as_int() {
            Key::Int(k)
        } else if let Some(s) = key.cloned_str() {
            Key::from_string(s)
        } else {
            return Arm::Slow;
        };
        let Some(v) = taken(&mut w[..], Reg::from(c), imm & insn::TAKE != 0) else { return Arm::Slow };
        // (A key written twice in one literal replaces the first value.)
        w[rx(a)].array_mut().expect("checked above").insert(k, v);
        done(&self.ids, pc, true)
    }

    /// `new C` of a class the site's cache has, whose objects need only
    /// their slots (`make_plain`): anything else (a Throwable, a
    /// constructor to check) is `step`'s.
    #[inline(never)]
    pub(crate) fn arm_new(&mut self, w: &mut [Value; WINDOW], pc: usize, insn: Insn) -> Arm {
        let Some(info) = Self::cached_new(&self.frames.top().func, insn.imm as u16) else { return Arm::Slow };
        let Some(object) = self.make_plain(&info) else { return Arm::Slow };
        // Not destructed unless constructed (an argument may throw first).
        object.destructed.set(true);
        self.last_new = Some(info);
        done(&self.ids, pc, put(&mut w[rx(insn.a)], Value::object(object)))
    }

    /// A frameless call (PHP's ZEND_FRAMELESS_ICALL) of builtin `i` with
    /// the `n` arguments at window register `start`, the result to `dst`,
    /// for the call instruction at `at`: the builtin borrows the registers
    /// in place, and the frame and its window stay as they are (the
    /// builtin cannot change the frame stack). `temps`: the arguments are
    /// the call's own registers, consumed by it; else they are variables,
    /// left as they are. A fault, a diagnostic or a destructor due goes
    /// out of line.
    #[inline(always)]
    fn frameless(&mut self, regs: &mut Vec<Value>, i: usize, run: Imp, (start, n, dst, temps): (usize, usize, usize, bool), at: usize) -> Arm {
        self.pc = at + 1;
        let base = self.frames.top().base as usize;
        let w = window(regs, base);
        let r = self.frameless_run(i, run, &mut w[start..start + n]);
        match r {
            Some(v) if self.diags.is_empty() && self.pending_errors.is_empty() => {
                if temps {
                    for r in &mut w[start..start + n] {
                        let _ = clear(r);
                    }
                }
                // (A tag test, not the out-of-line drop an assignment calls.)
                let _ = put(&mut w[dst], v);
                if self.ids.has_pending() {
                    return Arm::Pending(at + 1);
                }
                Arm::Go(at + 1)
            }
            r => match self.frameless_tail(regs, at, i, r, (base + start, n, base + dst, temps)) {
                Ok(next) => Arm::Switch(next),
                Err(t) => self.fault(t),
            },
        }
    }

    /// A call of a user function the call site resolved, with its required
    /// arguments, and no interrupt pending: its frame pushed here. A
    /// builtin it resolved: frameless when it can be (the frame and window
    /// kept), else called out of line. Anything else (a miss, a generator)
    /// is `step`'s.
    #[inline(never)]
    pub(crate) fn arm_call(&mut self, regs: &mut Vec<Value>, pc: usize, insn: Insn) -> Arm {
        let (args, n, dst, imm) = (insn.a, insn.b, insn.c, insn.imm);
        let top = self.frames.top();
        let base = top.base as usize;
        match top.func.sites[imm as usize].get() {
            Some(&Callee::User(target)) => {
                let target = &self.closures[target as usize];
                if (n as usize) < target.code.required || target.code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                    return Arm::Slow;
                }
                if let Some(caller) = self.frames.last_mut() {
                    caller.pc = (pc + 1) as u32;
                }
                self.frames.push(target, base + args as usize, n as usize, (base + dst as usize) as u32);
                enter(&mut self.frames, regs);
                Arm::Switch(0)
            }
            Some(&Callee::Builtin(i)) => {
                // Frameless when the builtin can be and its arguments need
                // no conversion (and no header callback can run before
                // output).
                let (start, n) = (rx(args), rx(n));
                let call = self.isolate.registry.call(i);
                if let Some(run) = call.frameless
                    && call.plain(&regs[base + start..base + start + n])
                    && (!self.header_callback_set || self.out.started)
                {
                    self.frameless(regs, i, run, (start, n, rx(dst), true), pc)
                } else {
                    match self.builtin_call(regs, pc, base, i, (Reg::from(args), n as u8, Reg::from(dst))) {
                        Ok(next) => Arm::Switch(next),
                        Err(t) => self.fault(t),
                    }
                }
            }
            _ => Arm::Slow,
        }
    }

    /// `f($x)` (`Op::CallRegs`): a builtin on the borrowed ABI reads its
    /// arguments from the variables (or constants) themselves, framelessly,
    /// and the Moves into the argument registers are not run. Anything
    /// else runs the first Move as `Op::Move` does, and goes on with the
    /// next.
    #[inline(never)]
    pub(crate) fn arm_call_regs(&mut self, regs: &mut Vec<Value>, pc: usize, insn: Insn) -> Arm {
        let (start, n, c, imm) = (rx(insn.a), rx(insn.b), insn.c, insn.imm);
        let top = self.frames.top();
        let base = top.base as usize;
        if let Some(&Callee::Builtin(i)) = top.func.sites[(imm & 0xffff) as usize].get() {
            let call = self.isolate.registry.call(i);
            if let Some(run) = call.frameless
                && matches!(run, Imp::Lent(_))
                && call.plain(&regs[base + start..base + start + n])
                && (!self.header_callback_set || self.out.started)
            {
                return self.frameless(regs, i, run, (start, n, rx(c), false), pc + n);
            }
        }
        let w = window(regs, base);
        let Some(v) = copied(&w[start]) else { return Arm::Slow };
        done(&self.ids, pc, put(&mut w[rk((imm >> 16) as usize)], v))
    }

    /// A user method the call site's cache names for this class and scope,
    /// on an object, with its required arguments, and no interrupt
    /// pending: anything else is `step`'s.
    #[inline(never)]
    pub(crate) fn arm_method_call(&mut self, regs: &mut Vec<Value>, pc: usize, insn: Insn) -> Arm {
        let (dst, obj, args, method, n) = (insn.a, insn.b, insn.c, (insn.imm & 0xffff) as usize, (insn.imm >> 16) as usize);
        let top = self.frames.top();
        let base = top.base as usize;
        let clo = &*top.func;
        let w = window(regs, base);
        let Some(o) = w[rx(obj)].as_object() else { return Arm::Slow };
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
        let Some((target, declaring)) = found else { return Arm::Slow };
        if n < target.code.required || target.code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
            return Arm::Slow;
        }
        let this = Rc::clone(o);
        // A temporary (`(new A)->f()`) is released as it is used: the
        // call's `$this` holds the object.
        if is_temp(&clo.code, Reg::from(obj)) {
            clear(&mut w[rx(obj)]);
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
        Arm::Switch(0)
    }

    /// The constructor `new` found: a PHP one's frame pushed here, on the
    /// arguments where they are; none, the arguments released. Anything
    /// else is `step`'s.
    #[inline(never)]
    pub(crate) fn arm_construct(&mut self, regs: &mut Vec<Value>, pc: usize, insn: Insn) -> Arm {
        let (obj, args, n) = (insn.a, insn.b, rx(insn.c));
        let base = self.frames.top().base as usize;
        let w = window(regs, base);
        let Some(object) = w[rx(obj)].as_object() else { return Arm::Slow };
        // Its class's NewInfo: the one its `new` left, else the kept one
        // (a `new` in the arguments took `last_new`).
        let info = match self.last_new.as_ref() {
            Some(i) if Rc::ptr_eq(&i.class, &object.class) => i,
            _ => match self.new_infos.get(&(Rc::as_ptr(&object.class) as usize)) {
                Some(i) => i,
                None => return Arm::Slow,
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
                Arm::Switch(0)
            }
            None => {
                object.destructed.set(false);
                self.last_new = None;
                let start = rx(args);
                let mut released = false;
                for r in &mut w[start..start + n] {
                    released |= clear(r);
                }
                done(&self.ids, pc, released)
            }
            Some(_) => Arm::Slow,
        }
    }

    /// `C::f()` the site's cache has for this class and scope: a PHP
    /// method's frame pushed here. Anything else (a property hook's
    /// `parent::$p::get()`, a native method, __callStatic
    /// (`remember_static` caches no trampoline), the first call) is
    /// `step`'s.
    #[inline(never)]
    pub(crate) fn arm_static_call(&mut self, regs: &mut Vec<Value>, pc: usize, insn: Insn) -> Arm {
        let (dst, args, n, method, class) = (insn.a, insn.b, rx(insn.c), (insn.imm & 0xffff) as usize, (insn.imm >> 16) as u16);
        let top = self.frames.top();
        let base = top.base as usize;
        let Some(m) = self.cached_static_call(&top.func, class, method) else { return Arm::Slow };
        let classes::Code::User(target) = &m.func else { return Arm::Slow };
        if n < target.code.required || target.code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
            return Arm::Slow;
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
        Arm::Switch(0)
    }

    /// A return to the calling frame (`null`: `return;`). The main frame's
    /// return ends the run, and the bottom frame of this `execute` returns
    /// to the builtin that called it: `step` does those.
    #[inline(never)]
    pub(crate) fn arm_return(&mut self, regs: &mut Vec<Value>, insn: Insn, null: bool) -> Arm {
        if self.frames.len() == self.frames.floor + 1 {
            return Arm::Slow;
        }
        let value = if null {
            Value::NULL
        } else {
            let top = self.frames.top();
            let Some(value) = returned(regs, top, top.base as usize, Reg::from(insn.a)) else { return Arm::Slow };
            value
        };
        leave(&mut self.frames, regs, value);
        let pc = self.frames.top().pc as usize;
        if self.ids.has_pending() {
            return match self.after_errors(regs, pc, pc.saturating_sub(1)) {
                Ok(next) => Arm::Switch(next),
                Err(t) => self.fault(t),
            };
        }
        Arm::Switch(pc)
    }
}

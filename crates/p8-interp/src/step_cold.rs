//! The instructions `step` hands on: the rarely run ones, and the ones
//! with big locals (messages, argument lists, references, includes).
//! Kept out of `step` so that its stack frame, set up on every one of
//! its many entries, holds only what the frequent instructions need.

use super::*;

impl Context<'_> {
    /// Execute `instr` (the instruction at `at` of `clo`'s frame at
    /// `base`), one `step` does not run itself; as `step` does.
    #[inline(never)]
    pub(crate) fn step_cold(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, argc: usize, at: usize, instr: Instr) -> Result<Next, Throw> {
        let insn_at = at;
        let func = &*clo.code;
        let mut pc = at + 1;
        let in_main = base == 0;
        let (below, w) = regs.split_at_mut(base);
        let w = &mut w[..func.nregs as usize];
        macro_rules! reg {
            ($r:expr) => {
                w[$r as usize]
            };
        }
        match instr {
                Instr::MatchError { src } => {
                    let v = self.operand(w, func, src);
                    return Err(self.throw("UnhandledMatchError", format!("Unhandled match case {}", match_case(&v))));
                }
                Instr::Exit { src, has } => {
                    let v = self.operand(w, func, src);
                    let status = match (has, &v) {
                        (true, Value::Int(i)) => *i as i32,
                        (true, other) => {
                            let s = ops::to_string_lossy(other, &mut self.diags);
                            self.write_out(s.as_bytes());
                            0
                        }
                        (false, _) => 0,
                    };
                    return Err(Throw::Exit(status));
                }
                Instr::DeclareClass { class } => {
                    let def = Arc::clone(&func.decls[class as usize]);
                    self.declare_class(&def)?;
                }
                Instr::DeclareFunction { func: i } => {
                    let declared = Arc::clone(&func.lambdas[i as usize]);
                    if let Err(message) = self.declare(Arc::clone(&declared)) {
                        return Err(Throw::Fatal { message, file: Rc::from(&*declared.file), line: declared.line });
                    }
                }
                Instr::MakeClosure { dst, func: i, is_static } => {
                    let object = self.make_closure(&func.lambdas[i as usize], is_static);
                    reg!(dst) = Value::Object(object);
                }
                Instr::Capture { closure, var, by_ref, quiet } => {
                    let v = if by_ref {
                        Value::Ref(make_ref(w, var))
                    } else if quiet {
                        reg!(var).deref()
                    } else {
                        self.operand(w, func, var)
                    };
                    if let Value::Object(o) = &reg!(closure)
                        && let Some(data) = callable::closure_data(o)
                    {
                        data.captured.borrow_mut().push(v);
                    }
                }
                Instr::Rare { op, a, b, c, class } => match op {
                    Rare::PropRef => {
                            let (dst, obj, name) = (a, b, c);
                        let target = self.operand(w, func, obj);
                        let name = reg!(name).clone();
                        if let Some(lo) = self.lazy_pending(&target, &name) {
                            self.lazy_init(regs, &lo)?;
                            return Ok(Next::Go(insn_at));
                        }
                        let target = self.lazy_forward(&target, &name).unwrap_or(target);
                        let cell = self.prop_ref(&target, &name, class != 0)?;
                        reg!(dst) = Value::Ref(cell);
                    }
                    Rare::AssignPropRef => {
                            let (obj, name, src) = (a, b, c);
                        let target = self.operand(w, func, obj);
                        let name = reg!(name).clone();
                        let cell = make_ref(w, src);
                        if let Some(lo) = self.lazy_pending(&target, &name) {
                            self.lazy_init(regs, &lo)?;
                            return Ok(Next::Go(insn_at));
                        }
                        let target = self.lazy_forward(&target, &name).unwrap_or(target);
                        self.prop_bind(&target, &name, cell)?;
                    }
                    Rare::StaticRef => {
                            let (dst, name) = (a, b);
                        if let Some(c) = self.lazy_class(func, class, w)? {
                            self.resolve_class(regs, &c)?;
                            return Ok(Next::Go(at));
                        }
                        let name = reg!(name).clone();
                        let cell = self.static_cell(func, class, &name, w)?;
                        let inner = match &*cell.borrow() {
                            Value::Ref(r) => Some(Rc::clone(r)),
                            _ => None,
                        };
                        let r = match inner {
                            Some(r) => r,
                            None => {
                                let v = match std::mem::take(&mut *cell.borrow_mut()) {
                                    Value::Undef => Value::Null,
                                    v => v,
                                };
                                let r = Rc::new(RefCell::new(v));
                                *cell.borrow_mut() = Value::Ref(Rc::clone(&r));
                                r
                            }
                        };
                        reg!(dst) = Value::Ref(r);
                    }
                    Rare::AssignStaticRef => {
                            let (name, src) = (a, b);
                        if let Some(c) = self.lazy_class(func, class, w)? {
                            self.resolve_class(regs, &c)?;
                            return Ok(Next::Go(at));
                        }
                        let name = reg!(name).clone();
                        let cell = self.static_cell(func, class, &name, w)?;
                        let r = make_ref(w, src);
                        *cell.borrow_mut() = Value::Ref(r);
                    }
                    Rare::WantRef => self.ref_wanted.push(base + a as usize),
                    Rare::BindRefResult => {
                            let (var, src) = (a, b);
                        let at_reg = base + src as usize;
                        self.ref_wanted.retain(|&r| r != at_reg);
                        match reg!(src).clone() {
                            Value::Ref(cell) => reg!(var) = Value::Ref(cell),
                            v => {
                                self.notice("Only variables should be assigned by reference");
                                assign(w, var, v);
                            }
                        }
                    }
                    Rare::ArrayPushRef => {
                        let cell = make_ref(w, b);
                        if let Value::Array(arr) = &mut reg!(a) {
                            let _ = Rc::make_mut(arr).push(Value::Ref(cell));
                        }
                    }
                    Rare::ArraySetRef => {
                        let key = self.key(w, func, b)?;
                        let cell = make_ref(w, c);
                        if let Value::Array(arr) = &mut reg!(a) {
                            Rc::make_mut(arr).insert(key, Value::Ref(cell));
                        }
                    }
                    Rare::ToObject => {
                        let v = self.operand(w, func, b);
                        reg!(a) = self.object_cast(v);
                    }
                    Rare::VerifyArgs => self.verify_args(regs, base, argc)?,
                    Rare::VerifyReturn => {
                        if let Some(ty) = func.ret_type.as_ref().filter(|_| !func.is_generator) {
                            let v = self.operand(w, func, a);
                            match self.check_type(ty, &v, func.strict) {
                                Ok(Some(converted)) => reg!(a) = converted,
                                Ok(None) => {}
                                Err(()) => {
                                    if let Some(s) = self.stringable(regs, ty, &v, func.strict)? {
                                        regs[base + a as usize] = s;
                                        return Ok(Next::Go(pc));
                                    }
                                    // b: 0 a value returned, else none (`return;`, the end).
                                    let given = if b != 0 { "none".into() } else { p8_runtime::args::value_name(&v) };
                                    let name = self.function_display();
                                    return Err(self.throw("TypeError", format!("{name}(): Return value must be of type {}, {given} returned", ty.shown_resolved())));
                                }
                            }
                        }
                    }
                    Rare::VarRef => {
                        let name = self.var_name(w, func, b);
                        let global = c == 1 || self.frames.last().is_some_and(|f| f.global);
                        let cell = if global && !in_main { self.with_main(below, |cx, main| cx.global_cell(main, &name)) } else { self.var_cell(w, in_main, &name) };
                        reg!(a) = Value::Ref(cell);
                    }
                    Rare::VarGet => {
                        let name = self.var_name(w, func, b);
                        let global = c & 2 != 0;
                        let scope = global || self.frames.last().is_some_and(|f| f.global);
                        let v = if scope && !in_main { self.with_main(below, |cx, main| cx.global_value(main, &name)) } else { self.var_value(w, in_main, &name) };
                        match v {
                            Some(v) => reg!(a) = v,
                            None => {
                                if c & 1 == 0 {
                                    let what = if global { "global variable" } else { "variable" };
                                    self.warn(format!("Undefined {what} ${}", String::from_utf8_lossy(&name)));
                                }
                                reg!(a) = Value::Null;
                            }
                        }
                    }
                    Rare::VarUnset => {
                        let name = self.var_name(w, func, a);
                        let at_global = self.frames.last().is_some_and(|f| f.global);
                        if at_global && !in_main {
                            // A file included at global scope: the global is
                            // emptied, and this file's binding to it goes.
                            self.with_main(below, |cx, main| cx.global_clear(main, &name));
                            if let Some(r) = func.cv_names.iter().position(|n| n.as_bytes() == name.as_slice()) {
                                w[r] = Value::Undef;
                            }
                        } else if b == 1 && !in_main {
                            self.with_main(below, |cx, main| cx.global_unset(main, &name));
                        } else {
                            self.var_unset(w, in_main, &name);
                        }
                    }
                    Rare::AutoGlobal => {
                        // The name is a constant string: read, not converted.
                        let Value::Str(name) = self.operand(w, func, b) else { unreachable!("AutoGlobal names a constant string") };
                        let main: &mut [Value] = if in_main { w } else { below };
                        reg!(a) = Value::Ref(self.with_main(main, |cx, main| cx.global_slot(main, name.as_bytes())));
                    }
                    Rare::GlobalBind => {
                        let name = self.var_name(w, func, a);
                        let cell = make_ref(w, b);
                        let main: &mut [Value] = if in_main { w } else { below };
                        self.with_main(main, |cx, main| cx.global_bind(main, &name, cell));
                    }
                    Rare::VarBind => {
                        let name = self.var_name(w, func, a);
                        let at_global = in_main || self.frames.last().is_some_and(|f| f.global);
                        if c == 1 {
                            // `global $$name` at global scope changes nothing.
                            if !at_global {
                                let cell = self.with_main(below, |cx, main| cx.global_cell(main, &name));
                                self.bind_local(w, &name, cell);
                            }
                        } else {
                            let cell = make_ref(w, b);
                            if at_global {
                                let main: &mut [Value] = if in_main { w } else { below };
                                self.with_main(main, |cx, main| cx.global_bind(main, &name, cell));
                            } else {
                                self.bind_local(w, &name, cell);
                            }
                        }
                    }
                    Rare::Globals => {
                        let main: &mut [Value] = if in_main { w } else { below };
                        let globals = self.with_main(main, |cx, main| cx.globals_array(main));
                        reg!(a) = Value::Array(Rc::new(globals));
                    }
                    Rare::IssetStatic => {
                        let set = self.isset_static(func, class, &reg!(b).clone(), w)?;
                        reg!(a) = Value::Bool(set);
                    }
                    Rare::ThrowError => {
                        let message = reg!(a).to_php_string().unwrap_or_else(PhpStr::empty);
                        return Err(self.throw("Error", String::from_utf8_lossy(message.as_bytes()).into_owned()));
                    }
                    Rare::CallableArgByRef => {
                        let callee = reg!(b).clone();
                        let retry = self.autoload_retry.clone();
                        let by_ref = match self.resolve_callable(&callee) {
                            Ok(t) if class == u16::MAX => {
                                let name = reg!(c).to_php_string().unwrap_or_else(PhpStr::empty);
                                self.named_takes_ref(&t, name.as_bytes())
                            }
                            Ok(t) => self.takes_ref(&t, class as usize),
                            // A class not loaded yet: the autoloaders run and the
                            // question is asked again (not answered "no", which
                            // left the class unloaded for the call itself).
                            Err(t @ Throw::Autoload(_)) => return Err(t),
                            // Still not there after they ran: the call itself is
                            // told so too, not running them again.
                            Err(_) => {
                                self.autoload_retry = retry;
                                false
                            }
                        };
                        reg!(a) = Value::Bool(by_ref);
                    }
                    Rare::ReturnRef => {
                            let src = a;
                        let ret = self.frames.last().map_or(NO_RET, |f| f.ret);
                        let wanted = ret != NO_RET && self.ref_wanted.contains(&(ret as usize));
                        if wanted {
                            self.ref_wanted.retain(|&r| r != ret as usize);
                        }
                        let v = if wanted { Value::Ref(make_ref(w, src)) } else { self.operand(w, func, src) };
                        if let Some(g) = self.frames.last_mut().and_then(|f| f.generator.take()) {
                            self.finish(&g, v);
                            leave(&mut self.frames, regs, Value::Null);
                            return Ok(Next::Switch);
                        }
                        leave(&mut self.frames, regs, v);
                        return Ok(Next::Switch);
                    }
                },
                Instr::BindRef { var, src } => {
                    if var != src {
                        let cell = make_ref(w, src);
                        reg!(var) = Value::Ref(cell);
                    }
                }
                // No keys and no `[]`: the root itself (`&$GLOBALS['x']` is the
                // global's reference).
                Instr::RefDim { var, root, n: 0, append: false, .. } => {
                    let cell = make_ref(w, root);
                    reg!(var) = Value::Ref(cell);
                }
                Instr::RefDim { var, root, keys, n, append } => {
                    let path = self.key_path(w, func, keys, n)?;
                    let cell = make_ref(w, root);
                    let mut value = std::mem::take(&mut *cell.borrow_mut());
                    let r = self.dim_ref(&mut value, &path, append);
                    self.key_path_done(path);
                    *cell.borrow_mut() = value;
                    reg!(var) = Value::Ref(r?);
                }
                Instr::DimSetRef { root, keys, n, append, src } => {
                    let path = self.key_path(w, func, keys, n)?;
                    let target = make_ref(w, src);
                    let cell = make_ref(w, root);
                    let mut value = std::mem::take(&mut *cell.borrow_mut());
                    let r = self.dim_set_ref(&mut value, &path, append, target);
                    self.key_path_done(path);
                    *cell.borrow_mut() = value;
                    r?;
                }
                Instr::IterInitRef { src, iter, end } => {
                    let cell = make_ref(w, src);
                    let object = match &*cell.borrow() {
                        Value::Object(o) => Some(Rc::clone(o)),
                        _ => None,
                    };
                    // A plain object: its properties the code can see, each
                    // made a reference as the loop reaches it.
                    if let Some(mut o) = object {
                        // An IteratorAggregate: the iterator it gives (the call
                        // back into PHP needs the registers, so they are
                        // taken afresh after it).
                        while o.class.is_a(b"iteratoraggregate") && !o.class.is_a(b"arrayobject") {
                            match self.call_named(regs, &o, b"getIterator")? {
                                Value::Object(it) if it.class.is_a(b"traversable") => o = it,
                                _ => return Err(self.throw("Error", "An iterator cannot be used with foreach by reference")),
                            }
                        }
                        let w = &mut regs[base..base + func.nregs as usize];
                        // ArrayObject / ArrayIterator: their stored array, by reference.
                        let storage = ["arrayiterator", "arrayobject"]
                            .iter()
                            .find(|c| o.class.is_a(c.as_bytes()))
                            .and_then(|c| o.class.slot(b"storage", Some(c.as_bytes())));
                        if let Some(i) = storage
                            && matches!(o.slots.borrow()[i].deref(), Value::Array(_))
                        {
                            let cell = refs::make_ref(&mut o.slots.borrow_mut()[i]);
                            let empty = matches!(&*cell.borrow(), Value::Array(a) if a.is_empty());
                            if empty {
                                pc = end as usize;
                            } else {
                                w[iter as usize] = Value::Int(0);
                                w[iter as usize + 1] = Value::Ref(cell);
                            }
                            return Ok(Next::Go(pc));
                        }
                        if o.class.is_a(b"traversable") {
                            return Err(self.throw("Error", "An iterator cannot be used with foreach by reference"));
                        }
                        // A plain object: its properties the code can see, each
                        // made a reference as the loop reaches it.
                        let names: Vec<Value> = self.visible_props(&o).iter().map(|(k, _)| k.to_value()).collect();
                        if names.is_empty() {
                            pc = end as usize;
                        } else {
                            w[iter as usize] = Value::Int(0);
                            w[iter as usize + 1] = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(o), Value::Array(Rc::new(PhpArray::from_list(names)))])));
                        }
                        return Ok(Next::Go(pc));
                    }
                    let state = match &*cell.borrow() {
                        Value::Array(a) => Ok(!a.is_empty()),
                        other => Err(other.type_name()),
                    };
                    match state {
                        Ok(true) => {
                            reg!(iter) = Value::Int(0);
                            reg!(iter + 1) = Value::Ref(cell);
                        }
                        Ok(false) => pc = end as usize,
                        Err(ty) => {
                            self.warn(format!("foreach() argument must be of type array|object, {ty} given"));
                            pc = end as usize;
                        }
                    }
                }
                Instr::IterNextRef { iter, val, key, has_key, end } => {
                    let pos = match reg!(iter) {
                        Value::Int(p) => p as usize,
                        _ => 0,
                    };
                    let next = match &reg!(iter + 1) {
                        Value::Ref(cell) => refs::next_ref(cell, pos),
                        Value::Array(state) => self.next_prop_ref(state, pos),
                        _ => None,
                    };
                    match next {
                        Some((p, k, cell)) => {
                            reg!(iter) = Value::Int(p as i64 + 1);
                            reg!(val) = Value::Ref(cell);
                            if has_key {
                                assign(w, key, k.into_value());
                            }
                        }
                        None => pc = end as usize,
                    }
                }
                Instr::FromCallable { dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = self.closure_from_callable(&v)?;
                }
                Instr::CallValue { dst, callee, base: args, argc } => {
                    let target = self.operand(w, func, callee);
                    return self.call_value(regs, &target, base + args as usize, argc, (base + dst as usize) as u32, pc);
                }
                Instr::Silence { saved } => {
                    reg!(saved) = Value::Int(self.error_level);
                    // E_ERROR | E_CORE_ERROR | E_COMPILE_ERROR | E_USER_ERROR | E_RECOVERABLE_ERROR | E_PARSE
                    self.error_level &= 4437;
                }
                Instr::Unsilence { saved } => {
                    // A level the silenced code set stays (as in PHP),
                    // unless it only narrowed it to the silenced one.
                    if let Value::Int(level) = reg!(saved)
                        && self.error_level == level & 4437
                    {
                        self.error_level = level;
                    }
                }
                Instr::Include { dst, src, kind } => {
                    let v = self.operand(w, func, src);
                    // The registers are lent: a user-space wrapper's methods
                    // may serve the file.
                    self.isolate.regs = std::mem::take(regs);
                    let loaded = self.load_include(&v, kind, &func.file);
                    *regs = std::mem::take(&mut self.isolate.regs);
                    let Some(program) = loaded? else {
                        regs[base + dst as usize] = Value::Bool(kind.once());
                        return Ok(Next::Go(pc));
                    };
                    let program = match program {
                        Ok(p) => p,
                        Err(()) => {
                            regs[base + dst as usize] = Value::Bool(false);
                            return Ok(Next::Go(pc));
                        }
                    };
                    return Ok(self.run_in_scope(regs, base, func, &program.main, dst, pc));
                }
                Instr::Eval { dst, src } => {
                    let code = self.operand(w, func, src);
                    let program = self.load_eval(&code)?;
                    return Ok(self.run_in_scope(regs, base, func, &program.main, dst, pc));
                }
                Instr::CallSpread { dst, callee, args } => {
                    let target = self.operand(w, func, callee);
                    let list: Vec<Value> = match self.operand(w, func, args) {
                        // Named arguments: placed by the target's parameters.
                        Value::Array(a) if a.iter().any(|(k, _)| matches!(k, p8_runtime::array::Key::Str(_))) => {
                            match self.resolve_callable(&target)? {
                                // __call / __callStatic take the named ones
                                // in their array, by name.
                                callable::Target::Frame(mut m) if m.trampoline.is_some() => {
                                    let name = m.trampoline.take().unwrap_or_else(PhpStr::empty);
                                    let mut packed = PhpArray::new();
                                    for (k, v) in a.iter() {
                                        packed.insert(k.clone(), v.deref());
                                    }
                                    let at = base + func.nregs as usize;
                                    if regs.len() < at + 3 {
                                        regs.resize(at + 3, Value::Undef);
                                    }
                                    regs[at] = Value::Str(name);
                                    regs[at + 1] = Value::Array(Rc::new(packed));
                                    return self.call_method(regs, m, at, 2, (base + dst as usize) as u32, pc);
                                }
                                t => self.place_args(&t, &a)?,
                            }
                        }
                        // References (the arguments compiled as ones) kept
                        // for the parameters that take them.
                        Value::Array(a) if a.values().any(|v| matches!(v, Value::Ref(_))) => {
                            let t = self.resolve_callable(&target)?;
                            a.values().enumerate().map(|(n, v)| if matches!(v, Value::Ref(_)) && self.takes_ref(&t, n) { v.clone() } else { v.deref() }).collect()
                        }
                        Value::Array(a) => a.values().map(Value::deref).collect(),
                        _ => Vec::new(),
                    };
                    let at = base + func.nregs as usize;
                    if regs.len() < at + list.len() + 1 {
                        regs.resize(at + list.len() + 1, Value::Undef);
                    }
                    let argc = list.len() as u8;
                    for (i, v) in list.into_iter().enumerate() {
                        regs[at + i] = v;
                    }
                    return self.call_value(regs, &target, at, argc, (base + dst as usize) as u32, pc);
                }
                Instr::ArrayUnpack { arr, src } => {
                    let v = self.operand(w, func, src);
                    let items = match v {
                        // A reference held elsewhere too stays one (as PHP's
                        // unpacking does): a call takes it by reference.
                        Value::Array(a) => a
                            .iter()
                            .map(|(k, v)| {
                                let v = match v {
                                    Value::Ref(cell) if Rc::strong_count(cell) > 1 => v.clone(),
                                    other => other.deref(),
                                };
                                (k.into_value(), v)
                            })
                            .collect(),
                        Value::Object(ref o) if o.class.is_a(b"traversable") => {
                            let items = self.iterate_all(regs, &v)?;
                            let w = &mut regs[base..base + func.nregs as usize];
                            return self.unpack_into(w, arr, items).map(|()| Next::Go(pc));
                        }
                        _ => return Err(self.throw("Error", "Only arrays and Traversables can be unpacked")),
                    };
                    self.unpack_into(w, arr, items)?;
                }
                Instr::ConstructSpread { obj, args } => {
                    let Value::Object(object) = reg!(obj).clone() else { unreachable!("ConstructSpread follows New") };
                    object.destructed.set(false);
                    let arr = self.operand(w, func, args);
                    let Some(m) = self.constructor(&object) else { return Ok(Next::Go(pc)) };
                    let target = callable::Target::Frame(m);
                    let list = match &arr {
                        Value::Array(a) => self.place_args(&target, a)?,
                        _ => Vec::new(),
                    };
                    let callable::Target::Frame(m) = target else { unreachable!("a frame") };
                    let at = base + func.nregs as usize;
                    if regs.len() < at + list.len() + 1 {
                        regs.resize(at + list.len() + 1, Value::Undef);
                    }
                    let argc = list.len() as u8;
                    for (i, v) in list.into_iter().enumerate() {
                        regs[at + i] = v;
                    }
                    return self.call_method(regs, m, at, argc, NO_RET, pc);
                }
                Instr::IssetProp { dst, obj, name } => {
                    let target = reg!(obj).deref();
                    let name = reg!(name).clone();
                    if let Some(hook) = self.prop_hook(&target, &name, HOOK_GET) {
                        let v = self.run_nested(regs, hook?, &mut [])?;
                        regs[base + dst as usize] = Value::Bool(!matches!(v.deref(), Value::Null | Value::Undef));
                        return Ok(Next::Go(pc));
                    }
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Isset) {
                        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        let v = self.run_magic(regs, found, &mut [pname])?;
                        regs[base + dst as usize] = Value::Bool(v.to_bool());
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    reg!(dst) = Value::Bool(self.isset_prop(&target, &name)?);
                }
                Instr::UnsetProp { obj, name } => {
                    let target = reg!(obj).deref();
                    let name = reg!(name).clone();
                    if let (Value::Object(o), Some(n)) = (&target, name.to_php_string())
                        && let Some(p) = self.hooked_property(o, n.as_bytes())
                    {
                        let class = String::from_utf8_lossy(p.declaring_name.as_bytes()).into_owned();
                        return Err(self.throw("Error", format!("Cannot unset hooked property {class}::${}", String::from_utf8_lossy(n.as_bytes()))));
                    }
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Unset) {
                        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        self.run_magic(regs, found, &mut [pname])?;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    self.unset_prop(&target, &name)?;
                }
                Instr::FetchStaticProp { dst, class, name } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = reg!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let v = match cell.borrow().deref() {
                        Value::Undef => Value::Null,
                        v => v,
                    };
                    reg!(dst) = v;
                }
                Instr::AssignStaticProp { class, name, val, dst } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = reg!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let v = self.operand(w, func, val);
                    store(&cell, v.clone());
                    reg!(dst) = v;
                }
                Instr::AssignStaticPropOp { op, class, name, val, dst } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = reg!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let y = self.operand(w, func, val);
                    let x = cell.borrow().deref();
                    let r = ops::binary(op, &x, &y, &mut self.diags);
                    self.report_diags();
                    let v = r.map_err(|e| self.op_error(e))?;
                    store(&cell, v.clone());
                    reg!(dst) = v;
                }
                Instr::Clone { dst, src } => {
                    let v = self.operand(w, func, src);
                    // A lazy object is initialized before it is cloned.
                    if let Value::Object(o) = &v
                        && o.is_uninit_lazy()
                    {
                        let o = Rc::clone(o);
                        self.lazy_init(regs, &o)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let (copy, hook) = self.clone_object(&v)?;
                    reg!(dst) = Value::Object(copy);
                    if let Some(m) = hook {
                        // __clone runs in a window past this frame's.
                        return self.call_method(regs, m, base + func.nregs as usize, 0, NO_RET, pc);
                    }
                }
                Instr::Throw { src } => {
                    let v = self.operand(w, func, src);
                    return Err(match v {
                        Value::Object(o) if o.class.is_a(b"throwable") => Throw::Object(o),
                        // A force-closed fiber's exit, rethrown past a catch or finally.
                        Value::Object(o) if self.fiber_exit.as_ref().is_some_and(|m| Rc::ptr_eq(m, &o)) => Throw::Object(o),
                        _ => self.throw("Error", "Can only throw objects"),
                    });
                }
                Instr::CatchIf { src, class, target } => {
                    if self.instance_of(func, &reg!(src), class, w) {
                        pc = target as usize;
                    }
                }
                Instr::Yield { dst, value, key, has_key } => {
                    let v = self.operand(w, func, value);
                    let k = if has_key { Some(self.operand(w, func, key)) } else { None };
                    return Ok(self.suspend(regs, v, k, pc, Some(dst)));
                }
                Instr::YieldFrom { dst, src } => {
                    match self.yield_from(regs, base, src, at)? {
                        Ok(next) => return Ok(next),
                        Err(result) => regs[base + dst as usize] = result,
                    }
                }
                _ => unreachable!("step runs this instruction itself"),
        }
        Ok(Next::Go(pc))
    }
}

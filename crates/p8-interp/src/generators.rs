//! Generators: calling a function that yields makes a Generator holding
//! its frame's registers; resuming one runs that frame (above the running
//! ones, in a nested `execute`) to its next `yield` or its return. Also
//! foreach over objects (generators, Iterator, IteratorAggregate, plain
//! objects' properties).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_generators.c,
//! $PHP_SRC_PATH/Zend/zend_execute.c (ZEND_YIELD, ZEND_YIELD_FROM,
//! ZEND_FE_RESET_R / ZEND_FE_FETCH_R on objects)

use crate::classes::{Code, MethodFrame};
use crate::{Closure, Context, NO_RET, Next};
use p8_bytecode::Reg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::object::{Class, Object};
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

/// What `yield from` is going through.
enum Delegate {
    Array(Rc<PhpArray>, usize),
    Generator(Rc<Object>),
    /// An Iterator (an IteratorAggregate's), and whether it is on its
    /// first element (rewound, not moved on yet).
    Iterator(Rc<Object>, bool),
}

pub(crate) struct GenState {
    func: Rc<Closure>,
    /// The frame's registers while it is suspended.
    regs: Vec<Value>,
    pc: u32,
    argc: u32,
    this: Option<Rc<Object>>,
    scope: Option<Rc<Class>>,
    called: Option<Rc<Class>>,
    current: Value,
    key: Value,
    /// The largest int key yielded (the next auto key is one more).
    largest: i64,
    started: bool,
    /// Resumed past its first yield (rewind() then fails).
    advanced: bool,
    finished: bool,
    running: bool,
    returned: Option<Value>,
    /// Where send()'s value goes when it goes on.
    resume_dst: Option<Reg>,
    delegate: Option<Delegate>,
}

pub(crate) struct Gen(RefCell<GenState>);

pub(crate) fn gen_of(o: &Object) -> Option<&Gen> {
    o.data.as_ref().and_then(|d| d.0.downcast_ref::<Gen>())
}

/// Which Generator method runs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Current,
    Key,
    Next,
    Send,
    Valid,
    Rewind,
    GetReturn,
    Throw,
}

impl Context<'_> {
    /// A call of a generator function: the Generator, its frame's
    /// registers set up from the arguments at `args`.
    pub(crate) fn make_generator(&mut self, regs: &mut [Value], m: MethodFrame, func: Rc<Closure>, args: usize, argc: usize) -> Rc<Object> {
        let code = &func.code;
        let mut window = vec![Value::Undef; code.nregs as usize];
        let nparams = code.params.len();
        for (i, r) in regs[args..args + argc].iter_mut().enumerate() {
            let v = std::mem::replace(r, Value::Undef);
            if i < nparams {
                window[i] = v;
            }
        }
        for (i, c) in func.consts[..code.value_consts as usize].iter().enumerate() {
            window[code.consts_at as usize + i] = c.clone();
        }
        for (r, v) in code.bound.iter().zip(m.captured) {
            window[*r as usize] = v;
        }
        let state = GenState {
            func,
            regs: window,
            pc: 0,
            argc: argc.min(nparams) as u32,
            this: m.this,
            scope: m.scope,
            called: m.called,
            current: Value::Null,
            key: Value::Null,
            largest: -1,
            started: false,
            advanced: false,
            finished: false,
            running: false,
            returned: None,
            resume_dst: None,
            delegate: None,
        };
        let class = Rc::clone(&self.classes[&self.isolate.known.generator]);
        Object::with_data(class, &self.ids, Some(Rc::new(Gen(RefCell::new(state)))))
    }

    /// Run the generator to its next yield (or its end): `sent` is what
    /// the yield it stopped at gives; `throw` is thrown there instead.
    pub(crate) fn resume(&mut self, regs: &mut Vec<Value>, object: &Rc<Object>, sent: Value, throw: Option<Rc<Object>>) -> Result<(), Throw> {
        let state = gen_of(object).expect("a Generator");
        let (func, pc, argc, this, scope, called) = {
            let mut s = state.0.borrow_mut();
            if s.finished {
                return match throw {
                    Some(e) => Err(Throw::Object(e)),
                    None => Ok(()),
                };
            }
            if s.running {
                return Err(self.throw("Error", "Cannot resume an already running generator"));
            }
            s.running = true;
            if s.started {
                s.advanced = true;
            }
            s.started = true;
            (Rc::clone(&s.func), s.pc, s.argc, s.this.clone(), s.scope.clone(), s.called.clone())
        };
        let at = match self.frames.last() {
            Some(top) => top.base as usize + top.func.code.nregs as usize,
            None => 0,
        };
        let nregs = func.code.nregs as usize;
        if regs.len() < at + nregs {
            regs.resize(at + nregs, Value::Undef);
        }
        {
            let mut s = state.0.borrow_mut();
            for (i, v) in std::mem::take(&mut s.regs).into_iter().enumerate() {
                regs[at + i] = v;
            }
            if let Some(d) = s.resume_dst.take() {
                regs[at + d as usize] = sent;
            }
        }
        let (saved_pc, saved_floor) = (self.pc, self.frames.floor);
        let caller_pc = self.frames.last().map(|f| f.pc);
        self.frames.push(&func, at, argc as usize, NO_RET);
        let index = self.frames.len() - 1;
        {
            let frame = self.frames.last_mut().expect("the generator's frame");
            frame.pc = pc;
            frame.this = this;
            frame.scope = scope;
            frame.called = called;
            frame.generator = Some(Rc::clone(object));
        }
        self.frames.floor = index;
        let mut r = Ok(());
        if let Some(e) = throw {
            // Thrown where it stopped, as if by its yield.
            self.pc = pc as usize;
            match self.unwind(regs, Throw::Object(e), (pc as usize).saturating_sub(1)) {
                Ok(target) => self.frames.last_mut().expect("the generator's frame").pc = target as u32,
                Err(t) => r = Err(t),
            }
        }
        if r.is_ok() {
            r = self.execute(regs);
        }
        self.frames.floor = saved_floor;
        self.pc = saved_pc;
        if let (Some(caller), Some(pc)) = (self.frames.last_mut(), caller_pc) {
            caller.pc = pc;
        }
        self.frames.slots[index].generator = None;
        let mut s = state.0.borrow_mut();
        s.running = false;
        if r.is_err() {
            s.finished = true;
            s.current = Value::Null;
            s.key = Value::Null;
            s.regs.clear();
            s.delegate = None;
        }
        r
    }

    /// `yield`: the generator's frame stops at `resume_pc`, keeping its
    /// registers; it gives `value` (under `key`, or the next int key).
    pub(crate) fn suspend(&mut self, regs: &mut [Value], value: Value, key: Option<Value>, resume_pc: usize, dst: Option<Reg>) -> Next {
        let frame = self.frames.pop();
        let generator = frame.generator.take().expect("a generator's frame");
        let base = frame.base as usize;
        let nregs = frame.func.code.nregs as usize;
        frame.this = None;
        frame.scope = None;
        frame.called = None;
        let state = gen_of(&generator).expect("a Generator");
        let mut s = state.0.borrow_mut();
        s.regs = regs[base..base + nregs].iter_mut().map(|r| std::mem::replace(r, Value::Undef)).collect();
        s.pc = resume_pc as u32;
        s.resume_dst = dst;
        s.key = match key {
            Some(k) => {
                if let Value::Int(i) = k
                    && i > s.largest
                {
                    s.largest = i;
                }
                k
            }
            None => {
                s.largest += 1;
                Value::Int(s.largest)
            }
        };
        s.current = value;
        Next::Switch
    }

    /// A generator's frame returns: it is finished.
    pub(crate) fn finish(&mut self, generator: &Rc<Object>, value: Value) {
        let state = gen_of(generator).expect("a Generator");
        let mut s = state.0.borrow_mut();
        s.finished = true;
        s.returned = Some(value);
        s.current = Value::Null;
        s.key = Value::Null;
        s.delegate = None;
    }

    /// `yield from src` at `at`: the next element of what it goes through
    /// (the frame stops, to run this again), or Ok(Some(result)) when
    /// that is done.
    pub(crate) fn yield_from(&mut self, regs: &mut Vec<Value>, base: usize, src: Reg, at: usize) -> Result<Result<Next, Value>, Throw> {
        let generator = self.frames.last().and_then(|f| f.generator.clone()).expect("yield in a generator");
        let state = gen_of(&generator).expect("a Generator");
        let first = state.0.borrow().delegate.is_none();
        if first {
            let delegate = match regs[base + src as usize].deref() {
                Value::Array(a) => Delegate::Array(a, 0),
                Value::Object(o) if gen_of(&o).is_some() => {
                    if Rc::ptr_eq(&o, &generator) {
                        return Err(self.throw("Error", "Impossible to yield from the Generator being currently run"));
                    }
                    Delegate::Generator(o)
                }
                Value::Object(o) if o.class.is_a(b"traversable") => self.iterator_delegate(regs, o)?,
                _ => return Err(self.throw("Error", "Can use \"yield from\" only with arrays and Traversables")),
            };
            state.0.borrow_mut().delegate = Some(delegate);
        }
        let iterator = match &mut state.0.borrow_mut().delegate {
            Some(Delegate::Iterator(o, first)) => Some((Rc::clone(o), std::mem::replace(first, false))),
            _ => None,
        };
        if let Some((it, first)) = iterator {
            let item = self.iter_object_next(regs, &it, first)?;
            let Some((value, key)) = item else {
                state.0.borrow_mut().delegate = None;
                return Ok(Err(Value::Null));
            };
            let largest = state.0.borrow().largest;
            let next = self.suspend(regs, value, Some(key), at, None);
            state.0.borrow_mut().largest = largest;
            return Ok(Ok(next));
        }
        let step = match &mut state.0.borrow_mut().delegate {
            Some(Delegate::Array(a, pos)) => match a.entry_at(*pos) {
                Some((p, k, v)) => {
                    *pos = p + 1;
                    Ok((v.deref(), k.into_value()))
                }
                None => Err(None),
            },
            Some(Delegate::Generator(inner)) => Err(Some(Rc::clone(inner))),
            Some(Delegate::Iterator(..)) | None => unreachable!("set above"),
        };
        let item = match step {
            Ok(item) => Some(item),
            Err(None) => None,
            Err(Some(inner)) => {
                let started = gen_of(&inner).expect("a Generator").0.borrow().started;
                if !started || !first {
                    self.resume(regs, &inner, Value::Null, None)?;
                }
                let s = gen_of(&inner).expect("a Generator").0.borrow();
                if s.finished {
                    let r = s.returned.clone().unwrap_or(Value::Null);
                    drop(s);
                    state.0.borrow_mut().delegate = None;
                    return Ok(Err(r));
                }
                Some((s.current.clone(), s.key.clone()))
            }
        };
        match item {
            Some((value, key)) => {
                // Keys come from what it goes through; the auto key does not move.
                let next = {
                    let largest = state.0.borrow().largest;
                    let n = self.suspend(regs, value, Some(key), at, None);
                    state.0.borrow_mut().largest = largest;
                    n
                };
                Ok(Ok(next))
            }
            None => {
                state.0.borrow_mut().delegate = None;
                Ok(Err(Value::Null))
            }
        }
    }

    /// What `yield from` goes through for a Traversable object: an
    /// IteratorAggregate's iterator (in turn), rewound; a generator as one.
    fn iterator_delegate(&mut self, regs: &mut Vec<Value>, mut object: Rc<Object>) -> Result<Delegate, Throw> {
        loop {
            if gen_of(&object).is_some() {
                return Ok(Delegate::Generator(object));
            }
            if object.class.is_a(b"iterator") {
                self.call_named(regs, &object, b"rewind")?;
                return Ok(Delegate::Iterator(object, true));
            }
            match self.call_named(regs, &object, b"getIterator")? {
                Value::Object(o) if o.class.is_a(b"traversable") => object = o,
                _ => {
                    return Err(self.throw(
                        "Exception",
                        format!(
                            "Objects returned by {}::getIterator() must be traversable or implement interface Iterator",
                            String::from_utf8_lossy(object.class.name.as_bytes())
                        ),
                    ));
                }
            }
        }
    }

    /// Run a not yet started generator to its first yield.
    fn start(&mut self, regs: &mut Vec<Value>, object: &Rc<Object>) -> Result<(), Throw> {
        let started = gen_of(object).expect("a Generator").0.borrow().started;
        if !started {
            self.resume(regs, object, Value::Null, None)?;
        }
        Ok(())
    }

    /// A Generator method.
    pub(crate) fn generator_op(&mut self, regs: &mut Vec<Value>, object: &Rc<Object>, op: Op, arg: Value) -> Result<Value, Throw> {
        fn state(o: &Rc<Object>) -> &Gen {
            gen_of(o).expect("a Generator")
        }
        match op {
            Op::Current | Op::Key | Op::Valid => {
                self.start(regs, object)?;
                let s = state(object).0.borrow();
                Ok(match op {
                    Op::Current => s.current.clone(),
                    Op::Key => s.key.clone(),
                    _ => Value::Bool(!s.finished),
                })
            }
            Op::Next => {
                let started = state(object).0.borrow().started;
                if started {
                    self.resume(regs, object, Value::Null, None)?;
                } else {
                    // PHP's next() on a new generator: to its first yield, then on.
                    self.start(regs, object)?;
                    self.resume(regs, object, Value::Null, None)?;
                }
                Ok(Value::Null)
            }
            Op::Send => {
                self.start(regs, object)?;
                self.resume(regs, object, arg, None)?;
                Ok(state(object).0.borrow().current.clone())
            }
            Op::Rewind => {
                self.start(regs, object)?;
                if state(object).0.borrow().advanced {
                    return Err(self.throw("Exception", "Cannot rewind a generator that was already run"));
                }
                Ok(Value::Null)
            }
            Op::GetReturn => {
                let s = state(object).0.borrow();
                match (&s.returned, s.finished) {
                    (Some(v), true) => Ok(v.clone()),
                    _ => {
                        drop(s);
                        Err(self.throw("Exception", "Cannot get return value of a generator that hasn't returned"))
                    }
                }
            }
            Op::Throw => {
                let Value::Object(e) = arg else { return Ok(Value::Null) };
                let started = state(object).0.borrow().started;
                if !started {
                    // Not started: thrown as if at the start, and not caught there.
                    let mut s = state(object).0.borrow_mut();
                    s.started = true;
                    s.finished = true;
                    drop(s);
                    return Err(Throw::Object(e));
                }
                self.resume(regs, object, Value::Null, Some(e))?;
                Ok(state(object).0.borrow().current.clone())
            }
        }
    }

    /// Call method `name` of an object with no arguments (Iterator's).
    pub(crate) fn call_named(&mut self, regs: &mut Vec<Value>, object: &Rc<Object>, name: &[u8]) -> Result<Value, Throw> {
        let m = self.method_frame(&Value::Object(Rc::clone(object)), &name.to_ascii_lowercase(), name)?;
        if let Code::Native(native, _) = m.func {
            // Lent the registers: it may call PHP (SplHeap's compare()).
            std::mem::swap(&mut self.isolate.regs, regs);
            let r = self.run_native(native, m.this.as_ref(), &mut []);
            std::mem::swap(&mut self.isolate.regs, regs);
            return r;
        }
        self.run_nested(regs, m, &mut [])
    }

    /// foreach over an object: what the iterator register holds, or None
    /// when there is nothing to go through. Generators and Iterators
    /// are gone through as they are; an IteratorAggregate's iterator is;
    /// any other object's visible properties are.
    pub(crate) fn iter_object(&mut self, regs: &mut Vec<Value>, object: Rc<Object>) -> Result<Option<Value>, Throw> {
        // A lazy object is initialized first; a proxy iterates its instance.
        let mut object = if object.lazy.get() != 0 { self.lazy_init(regs, &object)? } else { object };
        loop {
            if gen_of(&object).is_some() {
                self.generator_op(regs, &object, Op::Rewind, Value::Null)?;
                let valid = self.generator_op(regs, &object, Op::Valid, Value::Null)?;
                return Ok(valid.to_bool().then_some(Value::Object(object)));
            }
            if object.class.is_a(b"iterator") {
                self.call_named(regs, &object, b"rewind")?;
                let valid = self.call_named(regs, &object, b"valid")?;
                return Ok(valid.to_bool().then_some(Value::Object(object)));
            }
            if object.class.is_a(b"iteratoraggregate") {
                match self.call_named(regs, &object, b"getIterator")? {
                    Value::Object(o) if o.class.is_a(b"traversable") => {
                        object = o;
                        continue;
                    }
                    _ => {
                        return Err(self.throw(
                            "Exception",
                            format!(
                                "Objects returned by {}::getIterator() must be traversable or implement interface Iterator",
                                String::from_utf8_lossy(object.class.name.as_bytes())
                            ),
                        ));
                    }
                }
            }
            let props = self.visible_props(&object);
            return Ok((!props.is_empty()).then(|| Value::Array(Rc::new(props))));
        }
    }

    /// The next element of an object being gone through (after the first,
    /// it is moved on first): (value, key), or None at the end.
    pub(crate) fn iter_object_next(&mut self, regs: &mut Vec<Value>, object: &Rc<Object>, first: bool) -> Result<Option<(Value, Value)>, Throw> {
        if gen_of(object).is_some() {
            if !first {
                self.generator_op(regs, object, Op::Next, Value::Null)?;
            }
            if !self.generator_op(regs, object, Op::Valid, Value::Null)?.to_bool() {
                return Ok(None);
            }
            let value = self.generator_op(regs, object, Op::Current, Value::Null)?;
            let key = self.generator_op(regs, object, Op::Key, Value::Null)?;
            return Ok(Some((value, key)));
        }
        if !first {
            self.call_named(regs, object, b"next")?;
        }
        if !self.call_named(regs, object, b"valid")?.to_bool() {
            return Ok(None);
        }
        let value = self.call_named(regs, object, b"current")?;
        let key = self.call_named(regs, object, b"key")?;
        Ok(Some((value, key)))
    }

    /// Everything foreach would give for `v`.
    pub(crate) fn iterate_all(&mut self, regs: &mut Vec<Value>, v: &Value) -> Result<Vec<(Value, Value)>, Throw> {
        let it = match v.deref() {
            Value::Array(a) => return Ok(a.iter().map(|(k, v)| (k.into_value(), v.deref())).collect()),
            Value::Object(o) => self.iter_object(regs, o)?,
            _ => None,
        };
        match it {
            None => Ok(Vec::new()),
            Some(Value::Array(a)) => Ok(a.iter().map(|(k, v)| (k.into_value(), v.deref())).collect()),
            Some(Value::Object(o)) => {
                let mut out = Vec::new();
                let mut first = true;
                while let Some((v, k)) = self.iter_object_next(regs, &o, first)? {
                    out.push((k, v));
                    first = false;
                }
                Ok(out)
            }
            Some(_) => Ok(Vec::new()),
        }
    }

    /// foreach by reference over an object (`state`: the object and the names
    /// its visible properties had at the start): the property at position
    /// `pos` or after that still exists, made a reference.
    pub(crate) fn next_prop_ref(&self, state: &PhpArray, pos: usize) -> Option<(usize, Key, crate::refs::Cell)> {
        let Some(Value::Object(o)) = state.get(&Key::Int(0)) else { return None };
        let Some(Value::Array(names)) = state.get(&Key::Int(1)) else { return None };
        let scope = self.scope().0;
        let names: Vec<Value> = names.values().cloned().collect();
        for (p, name) in names.iter().enumerate().skip(pos) {
            let key = match name {
                Value::Int(i) => Key::Int(*i),
                v => Key::from_string(v.to_php_string().unwrap_or_else(p8_runtime::string::PhpStr::empty)),
            };
            let declared = match &key {
                Key::Str(n) => o.class.slot(n.as_bytes(), scope.as_ref().map(|c| c.lower.as_slice())),
                Key::Int(_) => None,
            };
            if let Some(i) = declared
                && !matches!(o.slots.borrow()[i], Value::Undef)
            {
                let cell = crate::refs::make_ref(&mut o.slots.borrow_mut()[i]);
                return Some((p, key, cell));
            }
            if let Some(slot) = o.dynamic.borrow_mut().get_mut(&key) {
                let cell = crate::refs::make_ref(slot);
                return Some((p, key, cell));
            }
        }
        None
    }

    /// An object's properties the running code can see, by name.
    pub(crate) fn visible_props(&self, o: &Object) -> PhpArray {
        let scope = self.scope().0;
        let mut out = PhpArray::new();
        let slots = o.slots.borrow();
        for (i, p) in o.class.properties.iter().enumerate() {
            if matches!(slots[i], Value::Undef) || p.is_internal() {
                continue;
            }
            if o.class.slot(p.name.as_bytes(), scope.as_ref().map(|s| s.lower.as_slice())) != Some(i) {
                continue;
            }
            let declaring = self.class_named(&p.declaring).cloned();
            let visible = match &declaring {
                Some(d) => p8_runtime::object::visible(p.visibility, d, scope.as_deref()),
                None => true,
            };
            if visible {
                out.insert(Key::from_string(p.name.clone()), slots[i].deref());
            }
        }
        for (k, v) in o.dynamic.borrow().iter() {
            out.insert(k, v.deref());
        }
        out
    }
}

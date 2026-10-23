//! Driving a Generator from its methods (current, key, next, send, valid,
//! rewind, throw, getReturn): the generator's frame runs on a nested loop
//! until it yields or returns.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_generators.c - zend_generator_resume,
//! zend_generator_ensure_initialized, ZEND_METHOD(Generator, *)

use crate::core::value::{Handle, Val};
use crate::vm::engine::{VM, VmError};
use crate::vm::frame::{GeneratorData, GeneratorState};
use std::cell::RefCell;
use std::rc::Rc;

impl VM {
    /// The state behind a Generator object.
    pub(crate) fn generator_data(&self, generator: Handle) -> Option<Rc<RefCell<GeneratorData>>> {
        let Val::Object(payload) = &self.arena.get(generator).value else {
            return None;
        };
        let Val::ObjPayload(obj) = &self.arena.get(*payload).value else {
            return None;
        };
        obj.internal
            .clone()?
            .downcast::<RefCell<GeneratorData>>()
            .ok()
    }

    /// Run the generator to its first yield if it has not started.
    pub(crate) fn generator_ensure_started(&mut self, generator: Handle) -> Result<(), VmError> {
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        let created = matches!(data.borrow().state, GeneratorState::Created(_));
        if created {
            self.generator_resume(generator, None, None)?;
        }
        Ok(())
    }

    /// Resume the generator from its current yield: `sent` is what that
    /// yield expression evaluates to, `throw` an exception raised there
    /// instead. Returns once it yields again, returns, or throws.
    pub(crate) fn generator_resume(
        &mut self,
        generator: Handle,
        sent: Option<Handle>,
        throw: Option<Handle>,
    ) -> Result<(), VmError> {
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        let frame = {
            let mut d = data.borrow_mut();
            let frame = match &d.state {
                GeneratorState::Created(f)
                | GeneratorState::Suspended(f)
                | GeneratorState::Delegating(f) => f.clone(),
                GeneratorState::Running => {
                    drop(d);
                    return Err(
                        self.throw_error(b"Error", "Cannot resume an already running generator")
                    );
                }
                GeneratorState::Finished => {
                    drop(d);
                    return match throw {
                        // Throwing into a finished generator throws in the caller.
                        Some(ex) => Err(VmError::Exception(ex)),
                        None => Ok(()),
                    };
                }
            };
            d.state = GeneratorState::Running;
            d.sent_val = sent;
            d.current_val = None;
            d.current_key = None;
            frame
        };

        let mut frame = frame;
        frame.generator = Some(generator);
        frame.stack_base = None;
        let saved_operands = std::mem::take(&mut frame.saved_operands);
        let depth = self.frames.len();
        let saved_return = self.last_return_value.take();
        self.push_frame(frame);
        for h in saved_operands {
            self.operand_stack.push(h);
        }

        let mut result = match throw {
            Some(ex) => {
                if self.handle_exception_at(ex, depth) {
                    self.run_nested(depth)
                } else {
                    Err(VmError::Exception(ex))
                }
            }
            None => self.run_nested(depth),
        };
        self.frames.truncate(depth);
        self.last_return_value = saved_return;

        let mut d = data.borrow_mut();
        if result.is_err() || matches!(d.state, GeneratorState::Running) {
            // An exception escaped the body: the generator is finished.
            d.state = GeneratorState::Finished;
            d.current_val = None;
            d.current_key = None;
            if result.is_ok() {
                result = Err(VmError::RuntimeError("Generator ended unexpectedly".into()));
            }
        }
        result
    }

    /// The value at the current yield (null once finished).
    pub(crate) fn generator_current(&mut self, generator: Handle) -> Result<Handle, VmError> {
        self.generator_ensure_started(generator)?;
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        let current = data.borrow().current_val;
        Ok(current.unwrap_or_else(|| self.arena.alloc(Val::Null)))
    }

    pub(crate) fn generator_key(&mut self, generator: Handle) -> Result<Handle, VmError> {
        self.generator_ensure_started(generator)?;
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        let key = data.borrow().current_key;
        Ok(key.unwrap_or_else(|| self.arena.alloc(Val::Null)))
    }

    pub(crate) fn generator_next(&mut self, generator: Handle) -> Result<(), VmError> {
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        if matches!(data.borrow().state, GeneratorState::Created(_)) {
            self.generator_resume(generator, None, None)?;
        }
        self.generator_resume(generator, None, None)?;
        data.borrow_mut().advanced = true;
        Ok(())
    }

    /// send(): the value becomes the result of the current yield; returns
    /// the next yielded value.
    pub(crate) fn generator_send(&mut self, generator: Handle, value: Handle) -> Result<Handle, VmError> {
        self.generator_ensure_started(generator)?;
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        self.generator_resume(generator, Some(value), None)?;
        data.borrow_mut().advanced = true;
        let current = data.borrow().current_val;
        Ok(current.unwrap_or_else(|| self.arena.alloc(Val::Null)))
    }

    /// throw(): raise the exception at the current yield.
    pub(crate) fn generator_throw(&mut self, generator: Handle, exception: Handle) -> Result<Handle, VmError> {
        self.generator_ensure_started(generator)?;
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        self.generator_resume(generator, None, Some(exception))?;
        let current = data.borrow().current_val;
        Ok(current.unwrap_or_else(|| self.arena.alloc(Val::Null)))
    }

    pub(crate) fn generator_valid(&mut self, generator: Handle) -> Result<bool, VmError> {
        self.generator_ensure_started(generator)?;
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        Ok(!matches!(data.borrow().state, GeneratorState::Finished))
    }

    pub(crate) fn generator_rewind(&mut self, generator: Handle) -> Result<(), VmError> {
        self.generator_ensure_started(generator)?;
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        if data.borrow().advanced {
            return Err(self.throw_error(b"Exception", "Cannot rewind a generator that was already run"));
        }
        Ok(())
    }

    pub(crate) fn generator_get_return(&mut self, generator: Handle) -> Result<Handle, VmError> {
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;
        let d = data.borrow();
        match (&d.state, d.return_val) {
            (GeneratorState::Finished, Some(value)) => Ok(value),
            _ => {
                drop(d);
                Err(self.throw_error(
                    b"Exception",
                    "Cannot get return value of a generator that hasn't returned",
                ))
            }
        }
    }
}

fn not_a_generator() -> VmError {
    VmError::RuntimeError("Not a generator".into())
}

impl VM {
    /// Every (key, value) a Traversable or array produces, as foreach would
    /// see them: arrays, generators, Iterator and IteratorAggregate objects.
    /// Reference: $PHP_SRC_PATH/ext/spl/spl_iterators.c - spl_iterator_apply
    pub(crate) fn traversable_entries(
        &mut self,
        iterable: Handle,
    ) -> Result<Vec<(Handle, Handle)>, VmError> {
        if let Val::Array(arr) = &self.arena.get(iterable).value {
            let entries: Vec<_> = arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect();
            return Ok(entries
                .into_iter()
                .map(|(k, v)| {
                    let key = match k {
                        crate::core::value::ArrayKey::Int(i) => Val::Int(i),
                        crate::core::value::ArrayKey::Str(s) => Val::String(s),
                    };
                    (self.arena.alloc(key), v)
                })
                .collect());
        }
        if self.generator_data(iterable).is_some() {
            self.generator_rewind(iterable)?;
            let mut out = Vec::new();
            while self.generator_valid(iterable)? {
                let key = self.generator_key(iterable)?;
                let value = self.generator_current(iterable)?;
                out.push((key, value));
                self.generator_next(iterable)?;
            }
            return Ok(out);
        }
        let aggregate = self.context.interner.intern(b"IteratorAggregate");
        if self.is_instance_of(iterable, aggregate) {
            let get_iterator = self.context.interner.intern(b"getIterator");
            let inner = self.call_method_simple(iterable, get_iterator)?;
            return self.traversable_entries(inner);
        }
        let [rewind, valid, current, key, next] =
            [&b"rewind"[..], b"valid", b"current", b"key", b"next"].map(|m| self.context.interner.intern(m));
        self.call_method_simple(iterable, rewind)?;
        let mut out = Vec::new();
        loop {
            let ok = self.call_method_simple(iterable, valid)?;
            if !self.arena.get(ok).value.to_bool() {
                break;
            }
            let v = self.call_method_simple(iterable, current)?;
            let k = self.call_method_simple(iterable, key)?;
            out.push((k, v));
            self.call_method_simple(iterable, next)?;
        }
        Ok(out)
    }
}

impl VM {
    /// `yield from <expr>`: each time the generator runs this opcode it
    /// passes on the delegate's next element (suspending with the opcode
    /// re-armed), and once the delegate is exhausted pushes the expression's
    /// result: the inner generator's return value, or null.
    /// Reference: $PHP_SRC_PATH/Zend/zend_vm_def.h - ZEND_YIELD_FROM
    pub(crate) fn exec_yield_from(&mut self) -> Result<(), VmError> {
        use crate::vm::frame::{SubGenState, SubIterator};
        let generator = self
            .frames
            .last()
            .and_then(|f| f.generator)
            .ok_or_else(|| VmError::RuntimeError("Cannot use \"yield from\" outside a generator".into()))?;
        let data = self.generator_data(generator).ok_or_else(not_a_generator)?;

        let existing = data.borrow_mut().sub_iter.take();
        let sub = match existing {
            Some(sub) => sub,
            None => {
                let source = self
                    .operand_stack
                    .pop()
                    .ok_or(VmError::StackUnderflow { operation: "yield from" })?;
                if self.generator_data(source).is_some() {
                    SubIterator::Generator { handle: source, state: SubGenState::Initial }
                } else if matches!(self.arena.get(source).value, Val::Array(_)) {
                    SubIterator::Array { handle: source, index: 0 }
                } else if matches!(self.arena.get(source).value, Val::Object(_)) {
                    // Another Traversable: delegate over a snapshot of it.
                    let mut snapshot = crate::core::value::ArrayData::new();
                    for (k, v) in self.traversable_entries(source)? {
                        let key = self.array_key_from_value(&self.arena.get(k).value.clone())?;
                        snapshot.insert(key, v);
                    }
                    let handle = self.arena.alloc(Val::Array(Rc::new(snapshot)));
                    SubIterator::Array { handle, index: 0 }
                } else {
                    return Err(self.throw_error(
                        b"Error",
                        "Can use \"yield from\" only with arrays and Traversables",
                    ));
                }
            }
        };

        let next = match sub {
            SubIterator::Array { handle, index } => {
                let entry = match &self.arena.get(handle).value {
                    Val::Array(arr) => arr.map.get_index(index).map(|(k, v)| (k.clone(), *v)),
                    _ => None,
                };
                match entry {
                    Some((k, v)) => {
                        let key = match k {
                            crate::core::value::ArrayKey::Int(i) => Val::Int(i),
                            crate::core::value::ArrayKey::Str(s) => Val::String(s),
                        };
                        let key = self.arena.alloc(key);
                        Some((key, v, SubIterator::Array { handle, index: index + 1 }))
                    }
                    None => None,
                }
            }
            SubIterator::Generator { handle, state } => {
                match state {
                    SubGenState::Initial => self.generator_ensure_started(handle)?,
                    _ => {
                        let sent = data.borrow_mut().sent_val.take();
                        match sent {
                            Some(v) if !matches!(self.arena.get(v).value, Val::Null) => {
                                self.generator_send(handle, v)?;
                            }
                            _ => self.generator_next(handle)?,
                        }
                    }
                }
                if self.generator_valid(handle)? {
                    let key = self.generator_key(handle)?;
                    let value = self.generator_current(handle)?;
                    Some((key, value, SubIterator::Generator { handle, state: SubGenState::Yielded }))
                } else {
                    let result = match self.generator_data(handle) {
                        Some(inner) => inner.borrow().return_val,
                        None => None,
                    };
                    let result = result.unwrap_or_else(|| self.arena.alloc(Val::Null));
                    self.operand_stack.push(result);
                    return Ok(());
                }
            }
        };

        match next {
            Some((key, value, sub)) => {
                // Suspend; on resume this opcode runs again.
                let mut frame = self.frames.pop().ok_or(VmError::RuntimeError("No frame".into()))?;
                frame.ip -= 1;
                frame.saved_operands = self.operand_stack.split_off(frame.stack_base.unwrap_or(0));
                let mut d = data.borrow_mut();
                d.current_key = Some(key);
                d.current_val = Some(value);
                d.sub_iter = Some(sub);
                d.state = GeneratorState::Suspended(frame);
            }
            None => {
                let null = self.arena.alloc(Val::Null);
                self.operand_stack.push(null);
            }
        }
        Ok(())
    }
}

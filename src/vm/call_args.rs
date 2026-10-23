//! Spread (`f(...$args)`) and named (`f(x: 1)`) arguments: gathering them
//! at run time, and binding names to the callee's parameters.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_execute.c - zend_handle_named_arg,
//! zend_handle_undef_args; $PHP_SRC_PATH/Zend/zend_vm_def.h - ZEND_SEND_UNPACK

use crate::compiler::chunk::UserFunc;
use crate::core::value::{Handle, Symbol, Val};
use crate::vm::engine::{VM, VmError};
use crate::vm::frame::ArgList;

/// One call's arguments while they are being pushed.
#[derive(Debug, Default)]
pub(crate) struct ArgFrame {
    positional: usize,
    names: Vec<Symbol>,
}

impl VM {
    /// The argument count of the call opcode about to run: EndArgs's, or
    /// the one compiled into the opcode.
    pub(crate) fn call_argc(&mut self, compiled: u8) -> usize {
        match self.next_call_argc.take() {
            Some(n) => n,
            None => {
                // A plain call: no names may linger from an earlier one.
                self.pending_arg_names = None;
                compiled as usize
            }
        }
    }

    pub(crate) fn begin_args(&mut self) {
        self.arg_frames.push(ArgFrame::default());
    }

    pub(crate) fn positional_arg(&mut self) -> Result<(), VmError> {
        let frame = self.arg_frames.last_mut().ok_or_else(no_arg_frame)?;
        if !frame.names.is_empty() {
            return Err(self.throw_error(b"Error", "Cannot use positional argument after named argument"));
        }
        frame.positional += 1;
        Ok(())
    }

    pub(crate) fn named_arg(&mut self, name: Symbol) -> Result<(), VmError> {
        let frame = self.arg_frames.last_mut().ok_or_else(no_arg_frame)?;
        if frame.names.contains(&name) {
            let msg = format!(
                "Named parameter ${} overwrites previous argument",
                self.symbol_to_string(name)
            );
            return Err(self.throw_error(b"Error", &msg));
        }
        frame.names.push(name);
        Ok(())
    }

    /// `...$x`: push each element; string keys become named arguments.
    pub(crate) fn spread_arg(&mut self) -> Result<(), VmError> {
        let source = self
            .operand_stack
            .pop()
            .ok_or(VmError::StackUnderflow { operation: "spread" })?;
        let entries = match &self.arena.get(source).value {
            Val::Array(_) | Val::Object(_) => self.traversable_entries(source)?,
            _ => {
                let msg = format!(
                    "Only arrays and Traversables can be unpacked, {} given",
                    crate::builtins::signature::value_name(self, source)
                );
                return Err(self.throw_error(b"TypeError", &msg));
            }
        };
        for (key, value) in entries {
            let copy = self.arena.alloc(self.arena.get(value).value.clone());
            match self.arena.get(key).value.clone() {
                Val::String(name) => {
                    let sym = self.context.interner.intern(&name);
                    self.operand_stack.push(copy);
                    self.named_arg(sym)?;
                }
                _ => {
                    if self.arg_frames.last().is_some_and(|f| !f.names.is_empty()) {
                        return Err(self.throw_error(
                            b"Error",
                            "Cannot use positional argument after named argument during unpacking",
                        ));
                    }
                    self.operand_stack.push(copy);
                    if let Some(frame) = self.arg_frames.last_mut() {
                        frame.positional += 1;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn end_args(&mut self) -> Result<(), VmError> {
        let frame = self.arg_frames.pop().ok_or_else(no_arg_frame)?;
        self.next_call_argc = Some(frame.positional + frame.names.len());
        self.pending_arg_names = (!frame.names.is_empty()).then_some(frame.names);
        Ok(())
    }

    /// Bind trailing named arguments to `func`'s parameters by name. A
    /// skipped optional parameter gets an Uninitialized marker (RecvInit
    /// then uses its default); names a variadic parameter does not declare
    /// are kept for it, keyed by name.
    pub(crate) fn bind_named_args(
        &mut self,
        func: &UserFunc,
        args: &mut ArgList,
        names: Vec<Symbol>,
        named_extra: &mut Vec<(Symbol, Handle)>,
    ) -> Result<(), VmError> {
        let positional = args.len() - names.len();
        let named: Vec<(Symbol, Handle)> = names.into_iter().zip(args.drain(positional..)).collect();
        for (name, value) in named {
            match func.params.iter().position(|p| p.name == name && !p.is_variadic) {
                Some(idx) => {
                    if idx < args.len() && !matches!(self.arena.get(args[idx]).value, Val::Uninitialized) {
                        let msg = format!(
                            "Named parameter ${} overwrites previous argument",
                            self.symbol_to_string(name)
                        );
                        return Err(self.throw_error(b"Error", &msg));
                    }
                    while args.len() <= idx {
                        args.push(self.arena.alloc(Val::Uninitialized));
                    }
                    args[idx] = value;
                }
                None if func.params.iter().any(|p| p.is_variadic) => named_extra.push((name, value)),
                None => {
                    let msg = format!("Unknown named parameter ${}", self.symbol_to_string(name));
                    return Err(self.throw_error(b"Error", &msg));
                }
            }
        }
        Ok(())
    }
}

fn no_arg_frame() -> VmError {
    VmError::RuntimeError("Argument list not started".into())
}

impl VM {
    /// Bind trailing named arguments of a builtin call by its signature.
    /// A skipped optional parameter gets its default as the stub writes it.
    pub(crate) fn bind_named_builtin_args(
        &mut self,
        sig: &'static crate::builtins::signature::Sig,
        args: &mut ArgList,
        names: Vec<Symbol>,
    ) -> Result<(), VmError> {
        let positional = args.len() - names.len();
        let named: Vec<(Symbol, Handle)> = names.into_iter().zip(args.drain(positional..)).collect();
        let mut slots: Vec<Option<Handle>> = args.iter().map(|&h| Some(h)).collect();
        for (name, value) in named {
            let name_str = self.symbol_to_string(name);
            let Some(idx) = sig.params.iter().position(|p| p.name == name_str && !p.variadic) else {
                let msg = format!("Unknown named parameter ${}", name_str);
                return Err(self.throw_error(b"Error", &msg));
            };
            if slots.get(idx).is_some_and(Option::is_some) {
                let msg = format!("Named parameter ${} overwrites previous argument", name_str);
                return Err(self.throw_error(b"Error", &msg));
            }
            if slots.len() <= idx {
                slots.resize(idx + 1, None);
            }
            slots[idx] = Some(value);
        }
        args.clear();
        for (i, slot) in slots.into_iter().enumerate() {
            let handle = match slot {
                Some(h) => h,
                None => {
                    let param = &sig.params[i];
                    let Some(default) = param.default else {
                        let msg = format!("{}(): Argument #{} (${}) not passed", sig.name, i + 1, param.name);
                        return Err(self.throw_error(b"ArgumentCountError", &msg));
                    };
                    match self.stub_default_value(default) {
                        Some(v) => self.arena.alloc(v),
                        None => {
                            let msg = format!(
                                "{}(): Argument #{} (${}) must be passed explicitly, because the default value is not known",
                                sig.name,
                                i + 1,
                                param.name
                            );
                            return Err(self.throw_error(b"ArgumentCountError", &msg));
                        }
                    }
                }
            };
            args.push(handle);
        }
        Ok(())
    }

    /// The value of a default as a stub writes it: a literal, a constant,
    /// or constants or'ed together. None when it is not known (`UNKNOWN`).
    fn stub_default_value(&self, text: &str) -> Option<Val> {
        let text = text.trim();
        match text {
            "null" => return Some(Val::Null),
            "true" => return Some(Val::Bool(true)),
            "false" => return Some(Val::Bool(false)),
            "[]" => return Some(Val::Array(std::rc::Rc::new(crate::core::value::ArrayData::new()))),
            "UNKNOWN" => return None,
            _ => {}
        }
        if let Ok(i) = text.parse::<i64>() {
            return Some(Val::Int(i));
        }
        if let Ok(f) = text.parse::<f64>() {
            return Some(Val::Float(f));
        }
        if text.len() >= 2
            && (text.starts_with('"') && text.ends_with('"') || text.starts_with('\'') && text.ends_with('\''))
        {
            let inner = &text[1..text.len() - 1];
            let unescaped = inner.replace("\\n", "\n").replace("\\t", "\t").replace("\\\\", "\\");
            return Some(Val::String(std::rc::Rc::new(unescaped.into_bytes())));
        }
        if text.contains('|') {
            let mut acc = 0i64;
            for part in text.split('|') {
                acc |= self.stub_default_value(part)?.to_int();
            }
            return Some(Val::Int(acc));
        }
        let sym = self.context.interner.find(text.trim_start_matches('\\').as_bytes())?;
        self.context.constants.get(&sym).cloned()
    }
}

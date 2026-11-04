//! The engine's side of reflection (Ctx::function_info, invoke,
//! closure_of): declarations of user code and of builtins (their stubs).

use crate::callable::{ClosureFunc, closure_data};
use crate::classes::{Code, MethodFrame};
use crate::Context;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::object::{Class, Object};
use p8_runtime::reflect::{Callee, FnInfo, ParamInfo};
use p8_runtime::string::PhpStr;
use p8_runtime::types::TypeDecl;
use p8_runtime::value::Value;
use std::rc::Rc;
use std::sync::Arc;

/// A stub's type as a TypeDecl.
fn stub_type(mask: u32, classes: &'static [&'static str]) -> Option<TypeDecl> {
    if mask == 0 && classes.is_empty() {
        return None;
    }
    let names: Vec<p8_runtime::frozen::Text> = classes.iter().map(|c| p8_runtime::frozen::Text::from(*c)).collect();
    Some(TypeDecl::new(mask, names.clone(), names, false))
}

impl Context<'_> {
    /// ReflectionFunctionAbstract::getStaticVariables: a closure's `use`
    /// values, then the `static` variables in the order they are declared.
    /// A closure's `use` variables by name, their values now.
    pub(crate) fn used_variables_of(&self, callee: &Callee) -> PhpArray {
        let mut out = PhpArray::new();
        let Callee::Closure(o) = callee else { return out };
        let Some(data) = closure_data(o) else { return out };
        let Some(clo) = data.func.user() else { return out };
        let code = &clo.code;
        for (r, v) in code.bound.iter().zip(data.captured.borrow().iter()) {
            out.insert(Key::from_string(PhpStr::from(code.cv_names[usize::from(*r)].as_bytes())), v.clone());
        }
        out
    }

    pub(crate) fn static_variables_of(&self, callee: &Callee) -> PhpArray {
        let mut out = PhpArray::new();
        // The code (whether or not its closure has been made: reading it
        // makes none), and a Closure object's captured values.
        let (code, captured) = match callee {
            Callee::Function(name) => {
                match self.sym_of(name.as_bytes()).and_then(|key| self.functions.get(&key)) {
                    Some(&i) => (Arc::clone(&self.codes[i as usize]), Vec::new()),
                    None => return out,
                }
            }
            Callee::Method(class, name) => match class.method(&name.as_bytes().to_ascii_lowercase()) {
                Some(m) if m.native.is_none() && m.func != u32::MAX => (Arc::clone(&self.codes[m.func as usize]), Vec::new()),
                _ => return out,
            },
            Callee::Closure(o) => match closure_data(o) {
                Some(data) => match data.func.user() {
                    Some(clo) => (Arc::clone(&clo.code), data.captured.borrow().clone()),
                    None => return out,
                },
                None => return out,
            },
        };
        let code = &code;
        let name = |r: u32| PhpStr::from(code.cv_names[r as usize].as_bytes());
        for (r, v) in code.bound.iter().zip(captured) {
            out.insert(Key::from_string(name(u32::from(*r))), v);
        }
        let statics = self.statics.get(&(Arc::as_ptr(code) as usize)).map(|s| &s.cells);
        let consts = code.consts_at as usize..code.consts_at as usize + code.consts.len();
        for (at, ins) in code.code.iter().enumerate() {
            if let p8_bytecode::Instr::StaticVar { var, index, .. } = ins {
                let v = match statics.and_then(|s| s.get(index)) {
                    Some(cell) => cell.borrow().deref(),
                    // Not run yet: a constant initializer's value, as PHP shows it.
                    None => match (code.code.get(at + 1), code.code.get(at + 2)) {
                        (Some(p8_bytecode::Instr::Assign { var: v, src }), _) if v == var && consts.contains(&(*src as usize)) => {
                            code.consts[*src as usize - consts.start].to_value()
                        }
                        (Some(p8_bytecode::Instr::NewArray { dst, .. }), Some(p8_bytecode::Instr::Assign { var: v, src })) if v == var && src == dst => {
                            Value::Array(Rc::new(PhpArray::new()))
                        }
                        _ => Value::Null,
                    },
                };
                out.insert(Key::from_string(name(u32::from(*var))), v);
            }
        }
        out
    }

    /// A user function's declaration.
    fn user_info(&self, code: &p8_bytecode::Function, class: Option<Rc<Class>>) -> FnInfo {
        FnInfo {
            name: PhpStr::from(code.name.as_bytes()),
            class,
            params: code
                .params
                .iter()
                .map(|p| ParamInfo {
                    name: PhpStr::from(p.name.as_bytes()),
                    by_ref: p.by_ref,
                    variadic: p.variadic,
                    optional: p.has_default || p.variadic,
                    default: p.default.as_ref().map(|d| d.to_value()),
                    default_text: None,
                    default_const: p.default_const.as_ref().map(|c| PhpStr::from(c.as_bytes())),
                    ty: p.ty.clone(),
                    promoted: p.promoted,
                })
                .collect(),
            ret: code.ret_type.clone(),
            tentative_ret: None,
            returns_ref: code.returns_ref,
            user: true,
            file: Some(PhpStr::from(&*code.file)),
            line: code.line,
            is_static: code.is_static,
            is_closure: code.name.as_bytes().starts_with(b"{closure"),
            is_generator: code.is_generator,
            closure_this: None,
            closure_scope: None,
            closure_called: None,
        }
    }

    /// A builtin's declaration, from its stub (`name` or `Class::method`).
    fn stub_info(&self, stub: &str, name: PhpStr, class: Option<Rc<Class>>) -> FnInfo {
        let sig = p8_runtime::stubs::lookup(stub.as_bytes());
        FnInfo {
            name,
            class,
            params: sig
                .map(|s| {
                    s.params
                        .iter()
                        .map(|p| ParamInfo {
                            name: PhpStr::from(p.name),
                            by_ref: p.by_ref,
                            variadic: p.variadic,
                            optional: p.optional(),
                            default: p.default.and_then(|d| self.stub_default(d)),
                            default_text: p.default,
                            default_const: p.default.filter(|d| stub_default_is_constant(d)).map(PhpStr::from),
                            ty: stub_type(p.mask, p.classes),
                            promoted: false,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ret: sig.filter(|s| !s.tentative).and_then(|s| stub_type(s.ret_mask, s.ret_classes)),
            tentative_ret: sig.filter(|s| s.tentative).and_then(|s| stub_type(s.ret_mask, s.ret_classes)),
            returns_ref: sig.is_some_and(|s| s.returns_ref),
            user: false,
            file: None,
            line: 0,
            is_static: sig.is_some_and(|s| s.flags & p8_runtime::stubs::SIG_STATIC != 0),
            is_closure: false,
            is_generator: false,
            closure_this: None,
            closure_scope: None,
            closure_called: None,
        }
    }

    pub(crate) fn info_of(&self, callee: &Callee) -> Option<FnInfo> {
        match callee {
            Callee::Function(name) => {
                let key = self.sym_of(name.as_bytes())?;
                if let Some(&i) = self.functions.get(&key) {
                    return Some(self.user_info(&self.codes[i as usize], None));
                }
                let i = self.builtin(key)?;
                let b = self.isolate.registry.function(i);
                Some(self.stub_info(b.name, PhpStr::from(b.name), None))
            }
            Callee::Method(class, name) => {
                let m = class.method(&name.as_bytes().to_ascii_lowercase())?.clone();
                let declaring = self.class_named(&m.declaring).cloned().unwrap_or_else(|| Rc::clone(class));
                // A builtin interface's abstract method has no code: its stub describes it.
                // Read from its code: describing a method makes no closure.
                let user = m.native.is_none() && m.func != u32::MAX;
                Some(match user {
                    true => self.user_info(&self.codes[m.func as usize], Some(declaring)),
                    false => {
                        let stub = format!("{}::{}", String::from_utf8_lossy(m.declaring_name.as_bytes()), String::from_utf8_lossy(m.name.as_bytes()));
                        let mut info = self.stub_info(&stub, m.name.clone(), Some(declaring));
                        info.is_static = m.is_static;
                        info
                    }
                })
            }
            Callee::Closure(o) => {
                let data = closure_data(o)?;
                let mut info = match &data.func {
                    ClosureFunc::User(clo) => self.user_info(&clo.code, None),
                    ClosureFunc::Builtin(i) => {
                        let b = self.isolate.registry.function(*i);
                        self.stub_info(b.name, PhpStr::from(b.name), None)
                    }
                    ClosureFunc::Method(_, name) => {
                        let class = data.scope.as_ref().map_or_else(PhpStr::empty, |c| c.name.clone());
                        let method = name.as_bytes().rsplit(|&b| b == b':').next().unwrap_or_default();
                        let stub = format!("{}::{}", String::from_utf8_lossy(class.as_bytes()), String::from_utf8_lossy(method));
                        self.stub_info(&stub, PhpStr::from(method), data.scope.clone())
                    }
                };
                info.is_closure = true;
                info.closure_this = data.this.clone();
                info.closure_scope = data.scope.clone();
                info.closure_called = data.called.clone().or_else(|| data.this.as_ref().map(|t| Rc::clone(&t.class))).or_else(|| data.scope.clone());
                Some(info)
            }
        }
    }

    pub(crate) fn invoke_callee(&mut self, callee: &Callee, this: Option<Rc<Object>>, args: &mut [Value]) -> Result<Value, Throw> {
        match callee {
            Callee::Function(name) => self.call_from_builtin(&Value::Str(name.clone()), args),
            Callee::Closure(o) => self.call_from_builtin(&Value::Object(Rc::clone(o)), args),
            Callee::Method(class, name) => {
                let Some(m) = class.method(&name.as_bytes().to_ascii_lowercase()).cloned() else {
                    return Err(self.throw("ReflectionException", format!("Method {}::{}() does not exist", String::from_utf8_lossy(class.name.as_bytes()), String::from_utf8_lossy(name.as_bytes()))));
                };
                let declaring = self.class_named(&m.declaring).cloned().unwrap_or_else(|| Rc::clone(class));
                let called = this.as_ref().map_or_else(|| Rc::clone(class), |o| Rc::clone(&o.class));
                let this = if m.is_static { None } else { this };
                let mut regs = std::mem::take(&mut self.isolate.regs);
                let r = match self.code_of(&m) {
                    Code::User(_) => {
                        let frame = MethodFrame::method(self.code_of(&m), this, declaring, called);
                        self.run_nested(&mut regs, frame, args)
                    }
                    Code::Native(native, _) => {
                        std::mem::swap(&mut self.isolate.regs, &mut regs);
                        let r = self.run_native(native, this.as_ref(), args);
                        std::mem::swap(&mut self.isolate.regs, &mut regs);
                        r
                    }
                };
                self.isolate.regs = regs;
                r
            }
        }
    }

    pub(crate) fn new_instance_of(&mut self, class: &Rc<Class>, args: &mut [Value], construct: bool) -> Result<Rc<Object>, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let made = (|| {
            if self.unresolved(class) {
                self.resolve_class(&mut regs, class)?;
            }
            let (object, ctor) = self.instantiate_as(&|i| regs.get(i).cloned().unwrap_or_default(), Rc::clone(class), construct)?;
            Ok::<_, Throw>((object, ctor.is_some()))
        })();
        self.isolate.regs = regs;
        let (object, has_ctor) = made?;
        if construct && has_ctor {
            object.destructed.set(false);
            self.invoke_callee(&Callee::Method(Rc::clone(class), PhpStr::from("__construct")), Some(Rc::clone(&object)), args)?;
        }
        Ok(object)
    }

    /// Each property slot's starting value, its default computed.
    pub(crate) fn class_defaults_of(&mut self, class: &Rc<Class>) -> Result<Vec<Value>, Throw> {
        if self.unresolved(class) {
            let mut regs = std::mem::take(&mut self.isolate.regs);
            let r = self.resolve_class(&mut regs, class);
            self.isolate.regs = regs;
            r?;
        }
        let mut values: Vec<Value> = class.properties.iter().map(|p| p.default.clone()).collect();
        if let Some(defaults) = self.class_defaults.get(&(Rc::as_ptr(class) as usize)) {
            for (i, v) in defaults.iter() {
                values[*i] = v.clone();
            }
        }
        Ok(values)
    }

    pub(crate) fn closure_of_callee(&mut self, callee: &Callee, this: Option<Rc<Object>>) -> Result<Value, Throw> {
        match callee {
            Callee::Function(name) => self.closure_from_callable(&Value::Str(name.clone())),
            Callee::Closure(o) => Ok(Value::Object(Rc::clone(o))),
            Callee::Method(class, name) => {
                let target = match this {
                    Some(o) => Value::Object(o),
                    None => Value::Str(class.name.clone()),
                };
                let pair = p8_runtime::array::PhpArray::from_list(vec![target, Value::Str(name.clone())]);
                self.closure_from_callable(&Value::Array(Rc::new(pair)))
            }
        }
    }
}

/// Whether a stub's default is a constant's name (`STR_PAD_RIGHT`,
/// `PHP_ROUND_HALF_UP`, `DateTimeInterface::ATOM`).
fn stub_default_is_constant(d: &str) -> bool {
    let first = d.bytes().next().unwrap_or(b'0');
    (first.is_ascii_alphabetic() || first == b'_' || first == b'\\')
        && d.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'\\' | b':'))
        && !matches!(d.to_ascii_lowercase().as_str(), "true" | "false" | "null")
}

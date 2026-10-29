//! The engine's side of reflection (Ctx::function_info, invoke,
//! closure_of): declarations of user code and of builtins (their stubs).

use crate::callable::closure_data;
use crate::classes::{Code, MethodFrame};
use crate::{Closure, Context};
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
    Some(TypeDecl { mask, classes: names.clone(), written: names, intersection: false })
}

impl Context<'_> {
    /// ReflectionFunctionAbstract::getStaticVariables: a closure's `use`
    /// values, then the `static` variables in the order they are declared.
    pub(crate) fn static_variables_of(&self, callee: &Callee) -> PhpArray {
        let mut out = PhpArray::new();
        let (clo, captured) = match callee {
            Callee::Function(name) => {
                let lower = name.as_bytes().strip_prefix(b"\\").unwrap_or(name.as_bytes()).to_ascii_lowercase();
                match self.functions.get(&lower) {
                    Some(&i) => (Rc::clone(&self.closures[i as usize]), Vec::new()),
                    None => return out,
                }
            }
            Callee::Method(class, name) => match class.method(&name.as_bytes().to_ascii_lowercase()).map(|m| self.code_of(m)) {
                Some(Code::User(clo)) => (clo, Vec::new()),
                _ => return out,
            },
            Callee::Closure(o) => match closure_data(o) {
                Some(data) => match &data.func {
                    Ok(clo) => (Rc::clone(clo), data.captured.borrow().clone()),
                    Err(_) => return out,
                },
                None => return out,
            },
        };
        let code = &clo.code;
        let name = |r: u32| PhpStr::from(code.cv_names[r as usize].as_bytes());
        for (r, v) in code.bound.iter().zip(captured) {
            out.insert(Key::from_string(name(u32::from(*r))), v);
        }
        let statics = self.statics.get(&(Arc::as_ptr(code) as usize));
        let consts = code.consts_at as usize..code.consts_at as usize + clo.consts.len();
        for (at, ins) in code.code.iter().enumerate() {
            if let p8_bytecode::Instr::StaticVar { var, index, .. } = ins {
                let v = match statics.and_then(|s| s.get(index)) {
                    Some(cell) => cell.borrow().deref(),
                    // Not run yet: a constant initializer's value, as PHP shows it.
                    None => match (code.code.get(at + 1), code.code.get(at + 2)) {
                        (Some(p8_bytecode::Instr::Assign { var: v, src }), _) if v == var && consts.contains(&(*src as usize)) => {
                            clo.consts[*src as usize - consts.start].clone()
                        }
                        (Some(p8_bytecode::Instr::NewArray { dst }), Some(p8_bytecode::Instr::Assign { var: v, src })) if v == var && src == dst => {
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
    fn user_info(&self, clo: &Closure, class: Option<Rc<Class>>) -> FnInfo {
        let code = &clo.code;
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
                    ty: p.ty.clone(),
                    promoted: p.promoted,
                })
                .collect(),
            ret: code.ret_type.clone(),
            returns_ref: code.returns_ref,
            user: true,
            file: Some(PhpStr::from(&*code.file)),
            line: code.line,
            is_static: code.is_static,
            is_closure: code.name.as_bytes().starts_with(b"{closure"),
            is_generator: code.is_generator,
            closure_this: None,
            closure_scope: None,
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
                            ty: stub_type(p.mask, p.classes),
                            promoted: false,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ret: sig.and_then(|s| stub_type(s.ret_mask, s.ret_classes)),
            returns_ref: sig.is_some_and(|s| s.returns_ref),
            user: false,
            file: None,
            line: 0,
            is_static: sig.is_some_and(|s| s.flags & p8_runtime::stubs::SIG_STATIC != 0),
            is_closure: false,
            is_generator: false,
            closure_this: None,
            closure_scope: None,
        }
    }

    pub(crate) fn info_of(&self, callee: &Callee) -> Option<FnInfo> {
        match callee {
            Callee::Function(name) => {
                let lower = name.as_bytes().strip_prefix(b"\\").unwrap_or(name.as_bytes()).to_ascii_lowercase();
                if let Some(&i) = self.functions.get(&lower) {
                    return Some(self.user_info(&Rc::clone(&self.closures[i as usize]), None));
                }
                let i = self.isolate.registry.lookup(&lower)?;
                let b = self.isolate.registry.function(i);
                Some(self.stub_info(b.name, PhpStr::from(b.name), None))
            }
            Callee::Method(class, name) => {
                let m = class.method(&name.as_bytes().to_ascii_lowercase())?.clone();
                let declaring = self.classes.get(&m.declaring).cloned().unwrap_or_else(|| Rc::clone(class));
                Some(match self.code_of(&m) {
                    Code::User(clo) => self.user_info(&clo, Some(declaring)),
                    Code::Native(..) => {
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
                    Ok(clo) => self.user_info(clo, None),
                    Err(i) => {
                        let b = self.isolate.registry.function(*i);
                        self.stub_info(b.name, PhpStr::from(b.name), None)
                    }
                };
                info.is_closure = true;
                info.closure_this = data.this.clone();
                info.closure_scope = data.scope.clone();
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
                let declaring = self.classes.get(&m.declaring).cloned().unwrap_or_else(|| Rc::clone(class));
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
            let (object, ctor) = self.instantiate(&|i| regs.get(i).cloned().unwrap_or_default(), Rc::clone(class))?;
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

//! What reflection reads of a declaration besides its code: doc comment,
//! attributes (their arguments are constant expressions, computed when
//! asked for, as PHP does) and last line. The compiler keeps them in
//! `p8_bytecode::Meta`; this finds the one a `Subject` names.

use crate::callable::{Target, closure_data};
use crate::classes::{Code, MethodFrame};
use crate::{Closure, Context};
use p8_bytecode::{AttrDef, ClassDef, Function, Meta};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::object::{Class, Object};
use p8_runtime::reflect::{Callee, Meta as Facts, Subject};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;
use std::sync::Arc;

/// A declaration's metadata, and the class its constant expressions run in.
struct Found<'a> {
    meta: &'a Meta,
    attrs: &'a [AttrDef],
    scope: Option<Rc<Class>>,
    /// A class's compiled declaration (its first line).
    def: Option<&'a Arc<ClassDef>>,
}

impl Context<'_> {
    /// The compiled code of a user function or method.
    fn code_for(&self, callee: &Callee) -> Option<Arc<Function>> {
        match callee {
            Callee::Function(name) => {
                let &i = self.functions.get(&self.sym_of(name.as_bytes())?)?;
                Some(Arc::clone(&self.codes[i as usize]))
            }
            Callee::Method(class, name) => {
                let m = class.method(&name.as_bytes().to_ascii_lowercase())?;
                if m.native.is_some() || m.func as usize >= self.codes.len() {
                    return None;
                }
                Some(Arc::clone(&self.codes[m.func as usize]))
            }
            Callee::Closure(o) => closure_data(o)?.func.user().map(|c| Arc::clone(&c.code)),
        }
    }

    /// The class that declares the method `callee` names (the scope its
    /// attributes' constant expressions run in).
    fn scope_of(&self, callee: &Callee) -> Option<Rc<Class>> {
        match callee {
            Callee::Method(class, name) => {
                let m = class.method(&name.as_bytes().to_ascii_lowercase())?;
                self.class_named(&m.declaring).cloned().or_else(|| Some(Rc::clone(class)))
            }
            Callee::Closure(o) => closure_data(o)?.scope.clone(),
            Callee::Function(_) => None,
        }
    }

    /// `pick` applied to the class declaration, then to the traits it uses.
    fn in_def<'a, T>(&'a self, def: &'a ClassDef, pick: &dyn Fn(&'a ClassDef) -> Option<T>) -> Option<T> {
        pick(def).or_else(|| {
            def.traits.iter().find_map(|t| {
                let used = self.def_named(t.as_bytes())?;
                self.in_def(used, pick)
            })
        })
    }

    fn find_meta<'a>(&'a self, subject: &Subject, code: Option<&'a Arc<Function>>) -> Option<Found<'a>> {
        match subject {
            Subject::Class(class) => {
                let def = self.def_named(&class.lower)?;
                Some(Found { meta: &def.meta, attrs: &def.meta.attrs, scope: Some(Rc::clone(class)), def: Some(def) })
            }
            Subject::Function(callee) => {
                let code = code?;
                Some(Found { meta: &code.meta, attrs: &code.meta.attrs, scope: self.scope_of(callee), def: None })
            }
            Subject::Param(callee, pos) => {
                let code = code?;
                let param = code.params.get(*pos)?;
                Some(Found { meta: &code.meta, attrs: &param.attrs, scope: self.scope_of(callee), def: None })
            }
            Subject::Property(class, name) => {
                let n = name.as_bytes();
                let declaring = match class.slot(n, None) {
                    Some(i) => class.properties[i].declaring.clone(),
                    None => class.statics.get(n)?.2.clone(),
                };
                let def = self.def_named(&declaring)?;
                let meta = self.in_def(def, &|d| d.properties.iter().find(|p| p.name.as_bytes() == n).map(|p| &p.meta))?;
                Some(Found { meta, attrs: &meta.attrs, scope: self.class_named(&declaring).cloned(), def: None })
            }
            Subject::Constant(class, name) => {
                let n = name.as_bytes();
                let declaring = class.constants.get(n)?.declaring_name.as_bytes();
                let def = self.def_named(declaring)?;
                let meta = self.in_def(def, &|d| {
                    d.constants
                        .iter()
                        .find(|c| c.name.as_bytes() == n)
                        .map(|c| &c.meta)
                        .or_else(|| d.cases.iter().position(|(c, _)| c.as_bytes() == n).and_then(|i| d.case_meta.get(i)))
                })?;
                Some(Found { meta, attrs: &meta.attrs, scope: self.class_named(declaring).cloned(), def: None })
            }
        }
    }

    fn subject_code(&self, subject: &Subject) -> Option<Arc<Function>> {
        match subject {
            Subject::Function(c) | Subject::Param(c, _) => self.code_for(c),
            _ => None,
        }
    }

    /// A builtin attribute class is itself marked `#[Attribute(flags)]`.
    fn builtin_attribute_flags(subject: &Subject) -> Option<i64> {
        let Subject::Class(class) = subject else { return None };
        Some(match class.lower.as_slice() {
            b"attribute" | b"allowdynamicproperties" => 1,
            b"returntypewillchange" | b"override" => 4,
            b"sensitiveparameter" => 32,
            b"deprecated" => 2 | 4 | 16,
            _ => return None,
        })
    }

    pub(crate) fn meta_facts(&self, subject: &Subject) -> Facts {
        let code = self.subject_code(subject);
        if Self::builtin_attribute_flags(subject).is_some() {
            return Facts { attrs: vec![PhpStr::from("Attribute")], ..Facts::default() };
        }
        let Some(found) = self.find_meta(subject, code.as_ref()) else { return Facts::default() };
        let mut facts = Facts {
            doc: found.meta.doc.as_ref().map(|d| PhpStr::from(d.as_bytes())),
            attrs: found.attrs.iter().map(|a| PhpStr::from(a.name.as_bytes())).collect(),
            file: None,
            line: 0,
            end_line: found.meta.end_line,
            traits: found.def.map(|d| d.traits.iter().map(|t| PhpStr::from(t.as_bytes())).collect()).unwrap_or_default(),
        };
        if let (Some(def), Subject::Class(class)) = (found.def, subject) {
            facts.line = def.line;
            facts.file = self.sym_of(&class.lower).and_then(|key| self.class_origins.get(&key)).map(|(file, _)| PhpStr::from(file.as_bytes()));
        }
        facts
    }

    /// Run a constant expression's code, in `scope`.
    fn eval_code(&mut self, code: &Arc<Function>, scope: Option<Rc<Class>>) -> Result<Value, Throw> {
        let frame = MethodFrame { func: Code::User(Closure::new(Arc::clone(code), self.isolate)), this: None, scope: scope.clone(), called: scope, captured: Vec::new(), trampoline: None };
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = self.run_nested(&mut regs, frame, &mut []);
        self.isolate.regs = regs;
        r
    }

    pub(crate) fn attr_arguments(&mut self, subject: &Subject, index: usize) -> Result<PhpArray, Throw> {
        if let Some(flags) = Self::builtin_attribute_flags(subject) {
            return Ok(PhpArray::from_list(if index == 0 { vec![Value::Int(flags)] } else { Vec::new() }));
        }
        let code = self.subject_code(subject);
        let Some(found) = self.find_meta(subject, code.as_ref()) else { return Ok(PhpArray::new()) };
        let Some(attr) = found.attrs.get(index) else { return Ok(PhpArray::new()) };
        // Cloned: running code needs the context mutably.
        let (args, scope) = (attr.args.clone(), found.scope);
        let mut out = PhpArray::new();
        for arg in &args {
            let value = match &arg.code {
                Some(code) => self.eval_code(code, scope.clone())?,
                None => arg.value.to_value(),
            };
            match &arg.name {
                Some(name) => out.insert(Key::from_string(PhpStr::from(name.as_bytes())), value),
                None => {
                    let _ = out.push(value);
                }
            }
        }
        Ok(out)
    }

    /// `new C(...)` with named arguments placed as a call places them.
    pub(crate) fn new_instance_placed(&mut self, class: &Rc<Class>, args: &PhpArray) -> Result<Rc<Object>, Throw> {
        let object = self.new_instance_of(class, &mut [], false)?;
        if let Some(ctor) = self.constructor(&object) {
            object.destructed.set(false);
            let target = Target::Frame(ctor);
            let mut list = self.place_args(&target, args)?;
            self.invoke_callee(&Callee::Method(Rc::clone(class), PhpStr::from("__construct")), Some(Rc::clone(&object)), &mut list)?;
        } else if let Some((Key::Str(name), _)) = args.iter().find(|(k, _)| matches!(k, Key::Str(_))) {
            return Err(self.throw("Error", format!("Unknown named parameter ${}", String::from_utf8_lossy(name.as_bytes()))));
        }
        Ok(object)
    }
}

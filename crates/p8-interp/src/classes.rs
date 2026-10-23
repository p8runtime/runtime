//! Classes and objects in the interpreter: linking a compiled class for
//! this request, resolving the classes code names, and the object
//! operations the step handler runs, with PHP's errors.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_inheritance.c (linking),
//! $PHP_SRC_PATH/Zend/zend_object_handlers.c (properties, methods),
//! $PHP_SRC_PATH/Zend/zend_execute.c (new, static access)

use crate::{Closure, Context};
use p8_bytecode::{ClassDef, ClassRef, Function};
use p8_runtime::array::Key;
use p8_runtime::ext::Throw;
use p8_runtime::object::{
    CLASS_ABSTRACT, CLASS_FINAL, CLASS_INTERFACE, Class, Constant, Method, NativeMethod, Object, Property, Visibility, visible,
};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// A method call to make: the frame to push.
/// What a method runs: user code, or Rust.
pub(crate) enum Code {
    User(Rc<Closure>),
    Native(NativeMethod),
}

pub(crate) struct MethodFrame {
    pub func: Code,
    pub this: Option<Rc<Object>>,
    pub scope: Rc<Class>,
    pub called: Rc<Class>,
}

fn text(s: &PhpStr) -> String {
    String::from_utf8_lossy(s.as_bytes()).into_owned()
}

impl Context<'_> {
    /// What method `m` runs.
    fn code_of(&self, m: &Method) -> Code {
        match m.native {
            Some(f) => Code::Native(f),
            None => Code::User(Rc::clone(&self.closures[m.func as usize])),
        }
    }

    /// The class the running frame's code belongs to (for visibility and
    /// self), and the called class (for static::).
    pub(crate) fn scope(&self) -> (Option<Rc<Class>>, Option<Rc<Class>>) {
        match self.frames.last() {
            Some(f) => (f.scope.clone(), f.called.clone()),
            None => (None, None),
        }
    }

    /// "global scope" or "scope Foo", as PHP's errors name the caller.
    pub(crate) fn scope_name(&self) -> String {
        match self.scope().0 {
            Some(c) => format!("scope {}", text(&c.name)),
            None => "global scope".into(),
        }
    }

    /// Link the classes whose parent and interfaces are already known,
    /// in order, before the file runs (PHP's early binding); the others
    /// link when their declaration runs.
    pub(crate) fn early_bind(&mut self, defs: &[Arc<ClassDef>]) -> Result<(), Throw> {
        for def in defs {
            let known = |n: &p8_runtime::frozen::Text| self.classes.contains_key(&n.as_bytes().to_ascii_lowercase());
            if def.parent.as_ref().is_none_or(known) && def.interfaces.iter().all(known) {
                let lower = def.name.as_bytes().to_ascii_lowercase();
                if self.classes.contains_key(&lower) {
                    continue;
                }
                self.declare_class(def)?;
            }
        }
        Ok(())
    }

    /// DeclareClass: link a class declaration (once).
    pub(crate) fn declare_class(&mut self, def: &Arc<ClassDef>) -> Result<(), Throw> {
        let key = Arc::as_ptr(def) as usize;
        if self.declared.contains(&key) {
            return Ok(());
        }
        let lower = def.name.as_bytes().to_ascii_lowercase();
        if self.classes.contains_key(&lower) {
            return Err(self.fatal_at(format!("Cannot declare class {}, because the name is already in use", text_of(&def.name)), def.line));
        }
        let class = self.link(def)?;
        self.classes.insert(lower, class);
        self.declared.insert(key);
        Ok(())
    }

    fn fatal_at(&self, message: String, line: u32) -> Throw {
        let (file, _) = self.location();
        Throw::Fatal { message, file, line }
    }

    fn lookup_class(&self, name: &p8_runtime::frozen::Text) -> Result<Rc<Class>, Throw> {
        match self.classes.get(&name.as_bytes().to_ascii_lowercase()) {
            Some(c) => Ok(Rc::clone(c)),
            None => Err(self.throw("Error", format!("Class \"{}\" not found", text_of(name)))),
        }
    }

    /// Build this request's class from its compiled declaration.
    fn link(&mut self, def: &ClassDef) -> Result<Rc<Class>, Throw> {
        let name = text_of(&def.name);
        let lower = def.name.as_bytes().to_ascii_lowercase();
        let parent = match &def.parent {
            None => None,
            Some(p) => {
                let parent = self.lookup_class(p)?;
                if parent.flags & CLASS_INTERFACE != 0 {
                    return Err(self.fatal_at(format!("Class {name} cannot extend interface {}", text(&parent.name)), def.line));
                }
                if parent.flags & CLASS_FINAL != 0 {
                    return Err(self.fatal_at(format!("Class {name} cannot extend final class {}", text(&parent.name)), def.line));
                }
                Some(parent)
            }
        };
        let mut interfaces: Vec<Vec<u8>> = parent.as_ref().map(|p| p.interfaces.clone()).unwrap_or_default();
        let mut methods: HashMap<Vec<u8>, Method> = parent.as_ref().map(|p| p.methods.clone()).unwrap_or_default();
        let mut constants: HashMap<Vec<u8>, Constant> = parent.as_ref().map(|p| p.constants.clone()).unwrap_or_default();
        for i in &def.interfaces {
            let iface = self.lookup_class(i)?;
            if iface.flags & CLASS_INTERFACE == 0 {
                let what = if def.flags & CLASS_INTERFACE != 0 { "extend" } else { "implement" };
                return Err(self.fatal_at(format!("{name} cannot {what} {} - it is not an interface", text(&iface.name)), def.line));
            }
            for x in std::iter::once(iface.lower.clone()).chain(iface.interfaces.iter().cloned()) {
                if !interfaces.contains(&x) {
                    interfaces.push(x);
                }
            }
            for (k, m) in &iface.methods {
                methods.entry(k.clone()).or_insert_with(|| m.clone());
            }
            for (k, c) in &iface.constants {
                constants.entry(k.clone()).or_insert_with(|| c.clone());
            }
        }
        // Properties: the parent's slots first; an own declaration
        // replaces a visible parent one, and sits beside a private one.
        let mut properties: Vec<Property> = parent.as_ref().map(|p| p.properties.clone()).unwrap_or_default();
        let mut statics = parent.as_ref().map(|p| p.statics.clone()).unwrap_or_default();
        for p in &def.properties {
            let pname = p.name.as_bytes().to_vec();
            let default = p.default.as_ref().map_or(Value::Undef, |d| d.to_value());
            if p.is_static {
                statics.insert(pname, (Rc::new(RefCell::new(default)), p.visibility, lower.clone()));
                continue;
            }
            let prop = Property {
                name: PhpStr::from(p.name.as_bytes()),
                default,
                visibility: p.visibility,
                declaring: lower.clone(),
                declaring_name: PhpStr::from(def.name.as_bytes()),
                readonly: p.readonly,
                ty: p.ty.as_ref().map(|t| PhpStr::from(t.as_bytes())),
            };
            match properties.iter().position(|q| q.name.as_bytes() == pname.as_slice() && q.visibility != Visibility::Private) {
                Some(at) => {
                    let inherited = &properties[at];
                    if visibility_rank(p.visibility) > visibility_rank(inherited.visibility) {
                        return Err(self.fatal_at(
                            format!(
                                "Access level to {name}::${} must be {} (as in class {}){}",
                                text_of(&p.name),
                                inherited.visibility.word(),
                                text(&inherited.declaring_name),
                                if inherited.visibility == Visibility::Protected { " or weaker" } else { "" }
                            ),
                            def.line,
                        ));
                    }
                    properties[at] = prop;
                }
                None => properties.push(prop),
            }
        }
        let mut slots: HashMap<Vec<u8>, Vec<usize>> = HashMap::new();
        for (i, p) in properties.iter().enumerate() {
            slots.entry(p.name.as_bytes().to_vec()).or_default().push(i);
        }
        for c in &def.constants {
            constants.insert(
                c.name.as_bytes().to_vec(),
                Constant { value: c.value.to_value(), visibility: c.visibility, declaring_name: PhpStr::from(def.name.as_bytes()) },
            );
        }
        for m in &def.methods {
            let mlower = m.func.name.as_bytes().to_ascii_lowercase();
            if let Some(old) = methods.get(&mlower)
                && old.is_final
                && !old.is_abstract
            {
                return Err(self.fatal_at(
                    format!("Cannot override final method {}::{}()", text(&old.declaring_name), text(&old.name)),
                    m.func.line,
                ));
            }
            let func = self.add_closure(Arc::clone(&m.func));
            methods.insert(
                mlower,
                Method {
                    name: PhpStr::from(m.func.name.as_bytes()),
                    func,
                    native: None,
                    visibility: m.visibility,
                    is_static: m.is_static,
                    is_abstract: m.is_abstract,
                    is_final: m.is_final,
                    declaring: lower.clone(),
                    declaring_name: PhpStr::from(def.name.as_bytes()),
                },
            );
        }
        if def.flags & (CLASS_ABSTRACT | CLASS_INTERFACE) == 0 {
            let mut missing: Vec<String> =
                methods.values().filter(|m| m.is_abstract).map(|m| format!("{}::{}", text(&m.declaring_name), text(&m.name))).collect();
            if !missing.is_empty() {
                missing.sort();
                let n = missing.len();
                let listed = missing.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                let more = if n > 3 { ", ..." } else { "" };
                return Err(self.fatal_at(
                    format!(
                        "Class {name} contains {n} abstract method{} and must therefore be declared abstract or implement the remaining methods ({listed}{more})",
                        if n == 1 { "" } else { "s" }
                    ),
                    def.line,
                ));
            }
        }
        Ok(Rc::new(Class {
            name: PhpStr::from(def.name.as_bytes()),
            lower,
            flags: def.flags,
            parent,
            interfaces,
            properties,
            slots,
            methods,
            constants,
            statics,
        }))
    }

    /// The class a code operand names.
    pub(crate) fn class_of(&self, func: &Function, i: u16) -> Result<Rc<Class>, Throw> {
        match &func.classes[i as usize] {
            ClassRef::Named(lower, display) | ClassRef::Forward(lower, display) => match self.classes.get(lower.as_bytes()) {
                Some(c) => Ok(Rc::clone(c)),
                None => Err(self.throw("Error", format!("Class \"{}\" not found", text_of(display)))),
            },
            ClassRef::Static => match self.scope().1 {
                Some(c) => Ok(c),
                None => Err(self.throw("Error", "Cannot use \"static\" when no class scope is active")),
            },
        }
    }

    /// `new C(args)`: the object, and its constructor's frame (if any).
    pub(crate) fn instantiate(&mut self, regs: &dyn Fn(usize) -> Value, class: Rc<Class>) -> Result<(Rc<Object>, Option<MethodFrame>), Throw> {
        if class.flags & CLASS_INTERFACE != 0 {
            return Err(self.throw("Error", format!("Cannot instantiate interface {}", text(&class.name))));
        }
        if class.flags & CLASS_ABSTRACT != 0 {
            return Err(self.throw("Error", format!("Cannot instantiate abstract class {}", text(&class.name))));
        }
        let object = Object::new(Rc::clone(&class), &self.ids);
        if class.is_a(b"throwable") {
            // Where it was made, and the calls that led there.
            let (file, line) = self.location();
            let root: &[u8] = if class.is_a(b"exception") { b"exception" } else { b"error" };
            class.write(&object, b"file", root, Value::Str(PhpStr::from(&*file)));
            class.write(&object, b"line", root, Value::Int(i64::from(line)));
            let trace = self.trace(regs);
            class.write(&object, b"trace", root, Value::Array(Rc::new(trace)));
        }
        let Some(ctor) = class.method(b"__construct").cloned() else { return Ok((object, None)) };
        let declaring = self.declaring(&class, &ctor.declaring);
        if !visible(ctor.visibility, &declaring, self.scope().0.as_deref()) {
            return Err(self.throw(
                "Error",
                format!("Call to {} {}::__construct() from {}", ctor.visibility.word(), text(&class.name), self.scope_name()),
            ));
        }
        let func = self.code_of(&ctor);
        Ok((Rc::clone(&object), Some(MethodFrame { func, this: Some(object), scope: declaring, called: class })))
    }

    /// The class in `class`'s chain that declares a member (as linked).
    fn declaring(&self, class: &Rc<Class>, lower: &[u8]) -> Rc<Class> {
        let mut c = Some(class);
        while let Some(k) = c {
            if k.lower == lower {
                return Rc::clone(k);
            }
            c = k.parent.as_ref();
        }
        self.classes.get(lower).cloned().unwrap_or_else(|| Rc::clone(class))
    }

    /// `$obj->method(args)`: the frame to push.
    pub(crate) fn method_frame(&mut self, target: &Value, lower: &[u8], display: &[u8]) -> Result<MethodFrame, Throw> {
        let object = match target {
            Value::Object(o) => Rc::clone(o),
            Value::Ref(cell) => return self.method_frame(&cell.borrow().clone(), lower, display),
            other => {
                return Err(self.throw(
                    "Error",
                    format!("Call to a member function {}() on {}", String::from_utf8_lossy(display), other.type_name()),
                ));
            }
        };
        let class = Rc::clone(&object.class);
        let Some(method) = class.method(lower).cloned() else {
            return Err(self.throw(
                "Error",
                format!("Call to undefined method {}::{}()", text(&class.name), String::from_utf8_lossy(display)),
            ));
        };
        let declaring = self.declaring(&class, &method.declaring);
        if !visible(method.visibility, &declaring, self.scope().0.as_deref()) {
            return Err(self.throw(
                "Error",
                format!("Call to {} method {}::{}() from {}", method.visibility.word(), text(&class.name), text(&method.name), self.scope_name()),
            ));
        }
        if method.is_abstract {
            return Err(self.throw("Error", format!("Cannot call abstract method {}::{}()", text(&method.declaring_name), text(&method.name))));
        }
        let func = self.code_of(&method);
        let this = (!method.is_static).then_some(object);
        Ok(MethodFrame { func, this, scope: declaring, called: class })
    }

    /// `C::method(args)`: the frame to push. A non-static method gets the
    /// caller's `$this` when it is an instance of C; `self::` and
    /// `parent::` keep the caller's called class.
    pub(crate) fn static_frame(&mut self, func: &Function, class: u16, lower: &[u8], display: &[u8]) -> Result<MethodFrame, Throw> {
        let target = self.class_of(func, class)?;
        let Some(method) = target.method(lower).cloned() else {
            return Err(self.throw(
                "Error",
                format!("Call to undefined method {}::{}()", text(&target.name), String::from_utf8_lossy(display)),
            ));
        };
        let declaring = self.declaring(&target, &method.declaring);
        let (scope, called) = self.scope();
        if !visible(method.visibility, &declaring, scope.as_deref()) {
            return Err(self.throw(
                "Error",
                format!("Call to {} method {}::{}() from {}", method.visibility.word(), text(&target.name), text(&method.name), self.scope_name()),
            ));
        }
        if method.is_abstract {
            return Err(self.throw("Error", format!("Cannot call abstract method {}::{}()", text(&method.declaring_name), text(&method.name))));
        }
        let this = self.frames.last().and_then(|f| f.this.clone());
        let forward = matches!(func.classes[class as usize], ClassRef::Forward(..) | ClassRef::Static);
        let fframe = self.code_of(&method);
        if method.is_static {
            let called = match called {
                Some(c) if forward && c.is_a(&target.lower) => c,
                _ => target,
            };
            return Ok(MethodFrame { func: fframe, this: None, scope: declaring, called });
        }
        match this {
            Some(this) if this.class.is_a(&target.lower) => {
                let called = Rc::clone(&this.class);
                Ok(MethodFrame { func: fframe, this: Some(this), scope: declaring, called })
            }
            _ => Err(self.throw(
                "Error",
                format!("Non-static method {}::{}() cannot be called statically", text(&method.declaring_name), text(&method.name)),
            )),
        }
    }

    /// The declared slot (or None: dynamic) of property `name` on
    /// `object`, after the visibility check for code in the current scope.
    fn prop_slot(&self, object: &Object, name: &[u8]) -> Result<Option<(usize, Property)>, Throw> {
        let scope = self.scope().0;
        let Some(slot) = object.class.slot(name, scope.as_ref().map(|s| s.lower.as_slice())) else { return Ok(None) };
        let prop = object.class.properties[slot].clone();
        let declaring = self.declaring(&object.class, &prop.declaring);
        if !visible(prop.visibility, &declaring, scope.as_deref()) {
            // A parent's private property is invisible here: as if absent.
            if prop.visibility == Visibility::Private && scope.as_ref().is_some_and(|s| s.lower != prop.declaring) {
                return Ok(None);
            }
            return Err(self.throw(
                "Error",
                format!("Cannot access {} property {}::${}", prop.visibility.word(), text(&object.class.name), String::from_utf8_lossy(name)),
            ));
        }
        Ok(Some((slot, prop)))
    }

    /// `$obj->name`
    pub(crate) fn fetch_prop(&mut self, target: &Value, name: &Value, quiet: bool) -> Result<Value, Throw> {
        let name = self.prop_name(name)?;
        let object = match target {
            Value::Object(o) => Rc::clone(o),
            Value::Ref(cell) => return self.fetch_prop(&cell.borrow().clone(), &Value::Str(name), quiet),
            other => {
                if !quiet {
                    self.warn(format!("Attempt to read property \"{}\" on {}", text(&name), other.type_name()));
                }
                return Ok(Value::Null);
            }
        };
        match self.prop_slot(&object, name.as_bytes())? {
            Some((slot, prop)) => {
                let value = object.slots.borrow()[slot].clone();
                match value {
                    Value::Undef if quiet => Ok(Value::Null),
                    Value::Undef if prop.ty.is_some() => Err(self.throw(
                        "Error",
                        format!("Typed property {}::${} must not be accessed before initialization", text(&prop.declaring_name), text(&name)),
                    )),
                    Value::Undef => {
                        self.warn(format!("Undefined property: {}::${}", text(&object.class.name), text(&name)));
                        Ok(Value::Null)
                    }
                    Value::Ref(cell) => Ok(cell.borrow().clone()),
                    v => Ok(v),
                }
            }
            None => {
                let key = Key::from_string(name.clone());
                let found = object.dynamic.borrow().get(&key).map(Value::deref);
                match found {
                    Some(v) => Ok(v),
                    None if quiet => Ok(Value::Null),
                    None => {
                        self.warn(format!("Undefined property: {}::${}", text(&object.class.name), text(&name)));
                        Ok(Value::Null)
                    }
                }
            }
        }
    }

    fn prop_name(&self, name: &Value) -> Result<PhpStr, Throw> {
        match name.deref() {
            Value::Str(s) => Ok(s),
            v @ (Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null) => Ok(v.to_php_string().unwrap_or_else(PhpStr::empty)),
            other => Err(self.throw("Error", format!("Cannot access property of type {}", other.type_name()))),
        }
    }

    fn target_object(&self, target: &Value, name: &PhpStr, verb: &str) -> Result<Rc<Object>, Throw> {
        match target {
            Value::Object(o) => Ok(Rc::clone(o)),
            Value::Ref(cell) => self.target_object(&cell.borrow(), name, verb),
            other => Err(self.throw("Error", format!("Attempt to {verb} property \"{}\" on {}", text(name), other.type_name()))),
        }
    }

    /// Write `value` to property `name` of `object` (checks visibility,
    /// readonly, and that a dynamic property may be created).
    pub(crate) fn write_prop(&mut self, target: &Value, name: &Value, value: Value) -> Result<(), Throw> {
        let name = self.prop_name(name)?;
        let object = self.target_object(target, &name, "assign")?;
        match self.prop_slot(&object, name.as_bytes())? {
            Some((slot, prop)) => {
                if prop.readonly {
                    let initialized = !matches!(object.slots.borrow()[slot], Value::Undef);
                    let scope = self.scope().0;
                    if initialized {
                        return Err(self.throw("Error", format!("Cannot modify readonly property {}::${}", text(&object.class.name), text(&name))));
                    }
                    if scope.as_ref().is_none_or(|s| s.lower != prop.declaring) {
                        return Err(self.throw(
                            "Error",
                            format!("Cannot initialize readonly property {}::${} from {}", text(&object.class.name), text(&name), self.scope_name()),
                        ));
                    }
                }
                let mut slots = object.slots.borrow_mut();
                match &slots[slot] {
                    Value::Ref(cell) => *cell.borrow_mut() = value,
                    _ => slots[slot] = value,
                }
            }
            None => {
                let key = Key::from_string(name.clone());
                let exists = object.dynamic.borrow().contains_key(&key);
                if !exists && object.class.lower != b"stdclass" {
                    self.diags.push(p8_runtime::ops::Diag::Deprecated(format!(
                        "Creation of dynamic property {}::${} is deprecated",
                        text(&object.class.name),
                        text(&name)
                    )));
                    self.report_diags();
                }
                object.dynamic.borrow_mut().insert(key, value);
            }
        }
        Ok(())
    }

    /// The value a property holds right now, for a read-modify-write
    /// (`$o->p += 1`): what fetch_prop gives, without its warnings for a
    /// missing property becoming errors.
    pub(crate) fn read_for_update(&mut self, target: &Value, name: &Value) -> Result<Value, Throw> {
        self.fetch_prop(target, name, false)
    }

    /// `$obj->name[k1]..[kn] (op)= val`: the property's value is taken
    /// out, updated in place (no copy of an array only it holds), and put
    /// back.
    pub(crate) fn update_prop_dim(
        &mut self,
        target: &Value,
        name: &Value,
        path: &[Key],
        append: bool,
        val: Value,
        op: Option<p8_runtime::ops::BinOp>,
    ) -> Result<Value, Throw> {
        let pname = self.prop_name(name)?;
        let object = self.target_object(target, &pname, "assign")?;
        let slot = self.prop_slot(&object, pname.as_bytes())?;
        let mut root = match &slot {
            Some((i, _)) => {
                let mut slots = object.slots.borrow_mut();
                match &mut slots[*i] {
                    Value::Ref(cell) => std::mem::take(&mut *cell.borrow_mut()),
                    v => std::mem::take(v),
                }
            }
            None => {
                let key = Key::from_string(pname.clone());
                object.dynamic.borrow_mut().remove(&key).unwrap_or(Value::Null)
            }
        };
        let result = self.update_path(&mut root, path, append, val, op);
        match &slot {
            Some((i, _)) => {
                let mut slots = object.slots.borrow_mut();
                match &mut slots[*i] {
                    Value::Ref(cell) => *cell.borrow_mut() = root,
                    v => *v = root,
                }
            }
            None => {
                object.dynamic.borrow_mut().insert(Key::from_string(pname), root);
            }
        }
        result
    }

    /// `isset($obj->name)`
    pub(crate) fn isset_prop(&mut self, target: &Value, name: &Value) -> Result<bool, Throw> {
        let Ok(name) = self.prop_name(name) else { return Ok(false) };
        let object = match target {
            Value::Object(o) => Rc::clone(o),
            Value::Ref(cell) => return self.isset_prop(&cell.borrow().clone(), &Value::Str(name)),
            _ => return Ok(false),
        };
        let slot = match self.prop_slot(&object, name.as_bytes()) {
            Ok(s) => s,
            Err(_) => return Ok(false),
        };
        Ok(match slot {
            Some((i, _)) => !matches!(object.slots.borrow()[i].deref(), Value::Null | Value::Undef),
            None => object.dynamic.borrow().get(&Key::from_string(name)).is_some_and(|v| !matches!(v.deref(), Value::Null)),
        })
    }

    /// `unset($obj->name)`
    pub(crate) fn unset_prop(&mut self, target: &Value, name: &Value) -> Result<(), Throw> {
        let name = self.prop_name(name)?;
        let object = match target {
            Value::Object(o) => Rc::clone(o),
            Value::Ref(cell) => return self.unset_prop(&cell.borrow().clone(), &Value::Str(name)),
            _ => return Ok(()),
        };
        match self.prop_slot(&object, name.as_bytes())? {
            Some((i, prop)) => {
                if prop.readonly {
                    return Err(self.throw("Error", format!("Cannot unset readonly property {}::${}", text(&object.class.name), text(&name))));
                }
                object.slots.borrow_mut()[i] = Value::Undef;
            }
            None => {
                object.dynamic.borrow_mut().remove(&Key::from_string(name));
            }
        }
        Ok(())
    }

    /// `C::NAME`
    pub(crate) fn class_constant(&self, func: &Function, class: u16, name: &Value) -> Result<Value, Throw> {
        let class = self.class_of(func, class)?;
        let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
        if name.as_bytes().eq_ignore_ascii_case(b"class") {
            return Ok(Value::Str(class.name.clone()));
        }
        match class.constants.get(name.as_bytes()) {
            Some(c) => {
                let declaring = self.classes.get(&c.declaring_name.as_bytes().to_ascii_lowercase()).cloned().unwrap_or_else(|| Rc::clone(&class));
                if !visible(c.visibility, &declaring, self.scope().0.as_deref()) {
                    return Err(self.throw(
                        "Error",
                        format!("Cannot access {} constant {}::{}", c.visibility.word(), text(&class.name), text(&name)),
                    ));
                }
                Ok(c.value.clone())
            }
            None => Err(self.throw("Error", format!("Undefined constant {}::{}", text(&class.name), text(&name)))),
        }
    }

    /// The cell of static property `C::$name`.
    pub(crate) fn static_cell(&self, func: &Function, class: u16, name: &Value) -> Result<Rc<RefCell<Value>>, Throw> {
        let class = self.class_of(func, class)?;
        let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
        match class.statics.get(name.as_bytes()) {
            Some((cell, visibility, declaring)) => {
                let declaring = self.declaring(&class, declaring);
                if !visible(*visibility, &declaring, self.scope().0.as_deref()) {
                    return Err(self.throw(
                        "Error",
                        format!("Cannot access {} property {}::${}", visibility.word(), text(&class.name), text(&name)),
                    ));
                }
                Ok(Rc::clone(cell))
            }
            None => Err(self.throw("Error", format!("Access to undeclared static property {}::${}", text(&class.name), text(&name)))),
        }
    }

    /// `$x instanceof C` (an unknown class: false).
    pub(crate) fn instance_of(&self, func: &Function, v: &Value, class: u16) -> bool {
        let Value::Object(o) = v.deref() else { return false };
        match &func.classes[class as usize] {
            ClassRef::Named(lower, _) | ClassRef::Forward(lower, _) => o.class.is_a(lower.as_bytes()),
            ClassRef::Static => self.scope().1.is_some_and(|c| o.class.is_a(&c.lower)),
        }
    }

    /// `clone $x`: the copy, and its `__clone`'s frame (if any).
    pub(crate) fn clone_object(&mut self, v: &Value) -> Result<(Rc<Object>, Option<MethodFrame>), Throw> {
        let Value::Object(o) = v.deref() else {
            return Err(self.throw("Error", "__clone method called on non-object"));
        };
        let copy = o.copy();
        let Some(method) = o.class.method(b"__clone").cloned() else { return Ok((copy, None)) };
        let declaring = self.declaring(&o.class, &method.declaring);
        if !visible(method.visibility, &declaring, self.scope().0.as_deref()) {
            return Err(self.throw(
                "Error",
                format!("Call to {} {}::__clone() from {}", method.visibility.word(), text(&o.class.name), self.scope_name()),
            ));
        }
        let func = self.code_of(&method);
        Ok((Rc::clone(&copy), Some(MethodFrame { func, this: Some(copy), scope: declaring, called: Rc::clone(&o.class) })))
    }
}

/// `(array) $object`: initialized properties under PHP's mangled names
/// ("\0A\0p" private to A, "\0*\0p" protected), then dynamic ones.
pub(crate) fn object_to_array(o: &Object) -> p8_runtime::array::PhpArray {
    let mut out = p8_runtime::array::PhpArray::new();
    let slots = o.slots.borrow();
    for (p, v) in o.class.properties.iter().zip(slots.iter()) {
        if matches!(v, Value::Undef) {
            continue;
        }
        let mut name = Vec::new();
        match p.visibility {
            Visibility::Public => {}
            Visibility::Protected => name.extend_from_slice(b"\0*\0"),
            Visibility::Private => {
                name.push(0);
                name.extend_from_slice(p.declaring_name.as_bytes());
                name.push(0);
            }
        }
        name.extend_from_slice(p.name.as_bytes());
        out.insert(Key::from_string(PhpStr::new(name)), v.clone());
    }
    for (k, v) in o.dynamic.borrow().iter() {
        out.insert(k, v.clone());
    }
    out
}

fn text_of(t: &p8_runtime::frozen::Text) -> String {
    String::from_utf8_lossy(t.as_bytes()).into_owned()
}

/// Narrower is larger: public < protected < private.
fn visibility_rank(v: Visibility) -> u8 {
    match v {
        Visibility::Public => 0,
        Visibility::Protected => 1,
        Visibility::Private => 2,
    }
}

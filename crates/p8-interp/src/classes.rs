//! Classes and objects in the interpreter: linking a compiled class for
//! this request, resolving the classes code names, and the object
//! operations the step handler runs, with PHP's errors.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_inheritance.c (linking),
//! $PHP_SRC_PATH/Zend/zend_object_handlers.c (properties, methods),
//! $PHP_SRC_PATH/Zend/zend_execute.c (new, static access)

use crate::{Closure, Context};
use p8_bytecode::{ClassDef, ClassRef, Function};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::object::{
    CLASS_ABSTRACT, CLASS_ENUM, CLASS_FINAL, CLASS_INTERFACE, CLASS_LAZY, CLASS_TRAIT, Class, Constant, Method, Native, Object, Property, Thunk, Visibility,
    visible,
};
use p8_runtime::string::PhpStr;
use p8_runtime::symbol::{Sym, known};
use p8_runtime::value::Value;
use std::cell::RefCell;
use p8_runtime::fast::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// A method call to make: the frame to push.
/// What a method runs: user code, or Rust.
pub(crate) enum Code {
    User(Rc<Closure>),
    /// A builtin method, and its name as declared (for stack traces).
    Native(Native, PhpStr),
}

pub(crate) struct MethodFrame {
    pub func: Code,
    pub this: Option<Rc<Object>>,
    /// The class the code is in (None: a function, or an unbound closure).
    pub scope: Option<Rc<Class>>,
    pub called: Option<Rc<Class>>,
    /// A closure's captured values, for its `bound` registers.
    pub captured: Vec<Value>,
    /// A call of a method that is not there (or not callable from here)
    /// going to __call / __callStatic: its name; the call's arguments are
    /// handed over as (name, [arguments]).
    pub trampoline: Option<PhpStr>,
}

impl MethodFrame {
    pub(crate) fn method(func: Code, this: Option<Rc<Object>>, scope: Rc<Class>, called: Rc<Class>) -> MethodFrame {
        MethodFrame { func, this, scope: Some(scope), called: Some(called), captured: Vec::new(), trampoline: None }
    }
}

/// As PHP's "%s" prints it: up to a NUL (anonymous class names).
/// Whether a call to method `lower` on `o` finds something to run: the
/// method, __call, or an inner object it passes calls to that does.
fn answers(o: &Object, key: Option<Sym>) -> bool {
    method_of(&o.class, key).is_some()
        || o.class.method_sym(known::CALL).is_some()
        || o.class.forward_calls.and_then(|f| f(o)).is_some_and(|inner| answers(&inner, key))
}

/// The method of `class` named `key` (None: a name no code ever named,
/// so no class has such a method).
#[inline]
pub(crate) fn method_of(class: &Class, key: Option<Sym>) -> Option<&Rc<Method>> {
    class.methods.get_sym(key?)
}

fn text(s: &PhpStr) -> String {
    let b = s.as_bytes();
    String::from_utf8_lossy(b.iter().position(|&c| c == 0).map_or(b, |i| &b[..i])).into_owned()
}

impl Context<'_> {
    /// What method `m` runs.
    /// Whether method `lower` of `class` takes argument `n` by reference
    /// (no: a method it lacks, which __call or an error answers; a builtin
    /// method, when its stub signature says so).
    pub(crate) fn method_takes_ref(&self, class: &Class, key: Option<Sym>, n: usize) -> bool {
        match method_of(class, key) {
            Some(m) if m.native.is_none() && m.func != u32::MAX => self.code(m.func).param_by_ref(n),
            // A builtin's method, or a builtin interface's abstract one (no code).
            Some(m) => {
                let stub = format!("{}::{}", text(&m.declaring_name), text(&m.name));
                p8_runtime::stubs::lookup(stub.as_bytes()).is_some_and(|s| s.params.get(n).is_some_and(|p| p.by_ref))
            }
            None => false,
        }
    }

    /// What a method runs: its closure (made now if this is its first
    /// use), or its Rust function.
    pub(crate) fn code_of(&mut self, m: &Method) -> Code {
        match m.native {
            Some(n) => Code::Native(n, m.name.clone()),
            None => Code::User(self.closure_at(m.func)),
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
    pub(crate) fn early_bind(&mut self, defs: &[Arc<ClassDef>], file: Option<Rc<str>>) -> Result<(), Throw> {
        let saved = std::mem::replace(&mut self.binding_file, file);
        let r = self.early_bind_all(defs);
        self.binding_file = saved;
        r
    }

    fn early_bind_all(&mut self, defs: &[Arc<ClassDef>]) -> Result<(), Throw> {
        for def in defs {
            if def.deps.iter().all(|d| self.classes.contains_key(d)) {
                if self.classes.contains_key(&def.sym) {
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
        if self.declared.contains_key(&key) {
            return Ok(());
        }
        let sym = def.sym;
        if let Some(existing) = self.classes.get(&sym) {
            use p8_runtime::object::{CLASS_ENUM, CLASS_INTERFACE, CLASS_TRAIT};
            let kind = match existing.flags {
                f if f & CLASS_INTERFACE != 0 => "interface",
                f if f & CLASS_TRAIT != 0 => "trait",
                f if f & CLASS_ENUM != 0 => "enum",
                _ => "class",
            };
            let name = String::from_utf8_lossy(existing.name.as_bytes()).into_owned();
            let message = match self.class_origins.get(&sym) {
                Some((file, line)) => format!("Cannot redeclare {kind} {name} (previously declared in {file}:{line})"),
                None => format!("Cannot redeclare {kind} {name}"),
            };
            return Err(self.fatal_at(message, def.line));
        }
        let class = self.link(def)?;
        let file = self.binding_file.clone().unwrap_or_else(|| self.location().0);
        self.class_origins.insert(sym, (file, def.line));
        self.defs_by_name.insert(sym, Arc::clone(def));
        self.declared_order.push((class.name.clone(), Rc::clone(&class)));
        self.classes.insert(sym, class);
        self.declared.insert(key, Arc::clone(def));
        Ok(())
    }

    /// A virtual property has a set visibility only with both hooks
    /// (zend_verify_hooked_property; the compiler checks a class with no
    /// parent). A redeclaration without hooks of its own is not virtual.
    fn check_virtual_set_visibility(&self, def: &ClassDef, own_hooks: u8, class: &str, prop: &Property, line: u32) -> Result<(), Throw> {
        use p8_runtime::object::{HOOK_GET, HOOK_SET, HOOK_VIRTUAL};
        if def.parent.is_none() || own_hooks == 0 || prop.set_visibility.is_none() || prop.hooks & HOOK_VIRTUAL == 0 || prop.hooks & (HOOK_GET | HOOK_SET) == HOOK_GET | HOOK_SET {
            return Ok(());
        }
        let what = if prop.hooks & HOOK_GET == 0 { "Write-only" } else { "Read-only" };
        Err(self.fatal_at(format!("{what} virtual property {class}::${} must not specify asymmetric visibility", text(&prop.name)), line))
    }

    fn fatal_at(&self, message: String, line: u32) -> Throw {
        let (file, _) = self.location();
        Throw::Fatal { message, file, line }
    }

    fn lookup_class(&mut self, name: &p8_runtime::frozen::Text) -> Result<Rc<Class>, Throw> {
        self.find_class(name.as_bytes())
    }

    /// Build this request's class from its compiled declaration.
    fn link(&mut self, def: &ClassDef) -> Result<Rc<Class>, Throw> {
        let name = text_of(&def.name);
        let lower = def.name.as_bytes().to_ascii_lowercase();
        // One buffer for the name, shared by everything the class declares.
        let class_name = PhpStr::from(def.name.as_bytes());
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
        // The parent's entries are shared, not copied: only what this class
        // declares, overrides or takes from a trait is made new.
        let declaring: Rc<[u8]> = Rc::from(lower.as_slice());
        let mut interfaces: Vec<Rc<[u8]>> = parent.as_ref().map(|p| p.interfaces.clone()).unwrap_or_default();
        let symbols = Rc::clone(&self.isolate.names);
        let mut methods: p8_runtime::object::Methods = parent.as_ref().map_or_else(|| p8_runtime::object::Methods::new(&symbols), |p| p.methods.clone());
        let mut constants: p8_runtime::object::Constants = parent.as_ref().map(|p| p.constants.clone()).unwrap_or_default();
        for i in &def.interfaces {
            let iface = self.lookup_class(i)?;
            if iface.flags & CLASS_INTERFACE == 0 {
                let what = if def.flags & CLASS_INTERFACE != 0 { "extend" } else { "implement" };
                return Err(self.fatal_at(format!("{name} cannot {what} {} - it is not an interface", text(&iface.name)), def.line));
            }
            if !interfaces.iter().any(|x| **x == *iface.lower) {
                interfaces.push(Rc::from(iface.lower.as_slice()));
            }
            for x in &iface.interfaces {
                if !interfaces.contains(x) {
                    interfaces.push(Rc::clone(x));
                }
            }
            for (k, m) in &iface.methods {
                methods.entry(*k).or_insert_with(|| Rc::clone(m));
            }
            for (k, c) in &iface.constants {
                constants.entry(k.clone()).or_insert_with(|| Rc::clone(c));
            }
        }
        // Properties: the parent's slots first; an own declaration
        // replaces a visible parent one, and sits beside a private one.
        let mut properties: Vec<Property> = parent.as_ref().map(|p| p.properties.clone()).unwrap_or_default();
        // Statics in PHP's order: the class's own as declared, then the
        // parent's it does not redeclare (added below), then its traits'.
        let mut statics: p8_runtime::fast::IndexMap<Vec<u8>, p8_runtime::object::Static> = Default::default();
        let mut static_inits = Vec::new();
        for p in &def.properties {
            let pname = p.name.as_bytes().to_vec();
            let default = p.default.as_ref().map_or(Value::Undef, |d| self.isolate.thaw(d));
            let init = p.code.as_ref().map(|c| Thunk(Rc::new(Arc::clone(c))));
            if p.is_static {
                if let Some(t) = init {
                    static_inits.push((pname.clone(), t));
                }
                statics.insert(pname, (Rc::new(RefCell::new(default)), p.visibility, lower.clone()));
                continue;
            }
            let prop = Property {
                name: PhpStr::from(p.name.as_bytes()),
                default,
                visibility: p.visibility,
                set_visibility: p.set_visibility,
                declaring: lower.clone(),
                declaring_name: class_name.clone(),
                readonly: p.readonly,
                is_final: p.is_final,
                ty: p.ty.as_ref().map(|t| PhpStr::from(t.as_bytes())),
                ty_decl: p.ty_decl.clone().map(Rc::new),
                init,
                hooks: p.hooks,
            };
            match properties.iter().position(|q| q.name.as_bytes() == pname.as_slice() && q.visibility != Visibility::Private) {
                Some(at) => {
                    let inherited = &properties[at];
                    if inherited.is_final {
                        return Err(self.fatal_at(format!("Cannot override final property {}::${}", text(&inherited.declaring_name), text_of(&p.name)), def.line));
                    }
                    // A set visibility no wider than the parent's (its
                    // visibility when it has none); a get-only virtual
                    // property (an abstract one too) has none to keep.
                    use p8_runtime::object::HOOK_VIRTUAL;
                    let hook = |kind: &[u8]| methods.get([b"$", pname.as_slice(), b"::", kind].concat().to_ascii_lowercase().as_slice());
                    let get_only = hook(b"set").is_none() && (inherited.hooks & HOOK_VIRTUAL != 0 || hook(b"get").is_some_and(|m| m.is_abstract));
                    if let Some(set) = p.set_visibility
                        && !get_only
                        && set.rank() > inherited.set_visibility.unwrap_or(inherited.visibility).rank()
                    {
                        let (parent_set, weaker) = match inherited.set_visibility {
                            Some(v) => (format!("{}(set)", v.word()), " or weaker"),
                            None => ("omitted".to_string(), ""),
                        };
                        return Err(self.fatal_at(
                            format!("Set access level of {name}::${} must be {parent_set} (as in class {}){weaker}", text_of(&p.name), text(&inherited.declaring_name)),
                            def.line,
                        ));
                    }
                    if p.visibility.rank() > inherited.visibility.rank() {
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
                    // Hooks are inherited one by one: the redeclaration's
                    // own replace the parent's, the others stay.
                    let mut prop = prop;
                    if inherited.hooks != 0 {
                        use p8_runtime::object::HOOK_VIRTUAL;
                        let virtual_ = inherited.hooks & HOOK_VIRTUAL != 0 && (prop.hooks == 0 || prop.hooks & HOOK_VIRTUAL != 0);
                        prop.hooks = (prop.hooks | inherited.hooks) & !HOOK_VIRTUAL | if virtual_ { HOOK_VIRTUAL } else { 0 };
                    }
                    self.check_virtual_set_visibility(def, p.hooks, &name, &prop, def.line)?;
                    properties[at] = prop;
                }
                None => {
                    self.check_virtual_set_visibility(def, p.hooks, &name, &prop, def.line)?;
                    properties.push(prop);
                }
            }
        }
        // The parent's statics after the class's own (sharing their cells).
        if let Some(p) = &parent {
            for (k, s) in &p.statics {
                if !statics.contains_key(k) {
                    statics.insert(k.clone(), s.clone());
                }
            }
        }
        // Traits: their methods (the class's own win, then theirs over
        // the parent's), properties, statics and constants, as the class's.
        let own: Vec<Sym> = def.methods.iter().map(|m| m.func.sym).collect();
        let mut from_trait: HashMap<Sym, PhpStr> = HashMap::default();
        let mut trait_order: Vec<Sym> = Vec::new();
        let mut trait_properties: Vec<PhpStr> = Vec::new();
        for tname in &def.traits {
            let t = self.lookup_class(tname)?;
            trait_properties.extend(t.property_order.iter().cloned());
            if t.flags & CLASS_TRAIT == 0 {
                return Err(self.fatal_at(format!("{name} cannot use {} - it is not a trait", text(&t.name)), def.line));
            }
            let same = |a: &p8_runtime::frozen::Text, b: &[u8]| a.as_bytes().eq_ignore_ascii_case(b);
            let mut adopt = |c: &mut Self, key: Sym, m: Method| -> Result<(), Throw> {
                if own.contains(&key) {
                    return Ok(());
                }
                if let Some(other) = from_trait.get(&key) {
                    // The same trait method reached twice (a trait used
                    // directly and through another trait): no collision.
                    if methods.get_sym(key).is_some_and(|x| x.native.is_none() && m.native.is_none() && x.func == m.func) {
                        return Ok(());
                    }
                    // An abstract method meets another trait's (abstract or
                    // not) without collision: a concrete one wins.
                    if m.is_abstract {
                        return Ok(());
                    }
                    if methods.get_sym(key).is_some_and(|x| x.is_abstract) {
                        from_trait.insert(key, t.name.clone());
                        methods.insert_sym(key, Rc::new(m));
                        return Ok(());
                    }
                    return Err(c.fatal_at(
                        format!(
                            "Trait method {}::{} has not been applied as {name}::{}, because of collision with {}::{}",
                            text(&t.name),
                            text(&m.name),
                            text(&m.name),
                            text(other),
                            text(&m.name)
                        ),
                        def.line,
                    ));
                }
                if m.is_abstract && methods.get_sym(key).is_some_and(|x| !x.is_abstract) {
                    return Ok(());
                }
                from_trait.insert(key, t.name.clone());
                trait_order.push(key);
                methods.insert_sym(key, Rc::new(m));
                Ok(())
            };
            for (key, m) in &t.methods {
                let rule_for = |r: &&p8_bytecode::TraitRule| same(&r.method, m.name.as_bytes()) && r.trait_name.as_ref().is_none_or(|n| same(n, t.name.as_bytes()));
                let excluded = def
                    .trait_rules
                    .iter()
                    .any(|r| same(&r.method, m.name.as_bytes()) && r.insteadof.iter().any(|x| same(x, t.name.as_bytes())));
                // A trait's method becomes the class's own (declared by it):
                // one new entry for it and one per alias, nothing more.
                let copy = || Method { declaring: Rc::clone(&declaring), declaring_name: class_name.clone(), ..Method::clone(m) };
                // Its aliases first, then the method itself, as
                // zend_traits_copy_functions adds them (get_class_methods
                // and reflection list them so).
                for r in def.trait_rules.iter().filter(rule_for) {
                    if let Some(alias) = &r.alias {
                        let mut aliased = copy();
                        aliased.name = PhpStr::from(alias.as_bytes());
                        if let Some(v) = r.visibility {
                            aliased.visibility = v;
                        }
                        let key = self.intern(alias.as_bytes());
                        adopt(self, key, aliased)?;
                    }
                }
                if !excluded {
                    let mut plain = copy();
                    if let Some(v) = def.trait_rules.iter().filter(rule_for).find(|r| r.alias.is_none()).and_then(|r| r.visibility) {
                        plain.visibility = v;
                    }
                    adopt(self, *key, plain)?;
                }
            }
            for p in &t.properties {
                if !properties.iter().any(|q| q.name == p.name) {
                    let mut p = p.clone();
                    p.declaring = lower.clone();
                    p.declaring_name = class_name.clone();
                    properties.push(p);
                }
            }
            for (k, (cell, visibility, _)) in &t.statics {
                let fresh = Rc::new(RefCell::new(cell.borrow().clone()));
                if !statics.contains_key(k)
                    && let Some((_, init)) = t.static_inits.iter().find(|(n, _)| n == k)
                {
                    static_inits.push((k.clone(), init.clone()));
                }
                statics.entry(k.clone()).or_insert((fresh, *visibility, lower.clone()));
            }
            // A trait's constants are the using class's own (declared by
            // it: its private ones are its to use, and reflection says so).
            for (k, c) in &t.constants {
                constants.entry(k.clone()).or_insert_with(|| {
                    let mut c = p8_runtime::object::Constant::clone(c);
                    c.declaring_name = class_name.clone();
                    Rc::new(c)
                });
            }
        }
        // A class (or interface) with __toString() implements Stringable.
        if (methods.get_sym(known::TO_STRING).is_some() || own.contains(&known::TO_STRING)) && lower.as_slice() != b"stringable" && !interfaces.iter().any(|i| **i == *b"stringable") {
            interfaces.push(Rc::from(&b"stringable"[..]));
        }
        // An enum: its cases are constants (objects made when first used),
        // `name` (and `value`) readonly properties, cases()/from()/tryFrom().
        if def.flags & CLASS_ENUM != 0 {
            for x in std::iter::once(&b"unitenum"[..]).chain(def.enum_backing.is_some().then_some(&b"backedenum"[..])) {
                if !interfaces.iter().any(|i| **i == *x) {
                    interfaces.push(Rc::from(x));
                }
            }
            let prop = |n: &str, ty: Option<&str>| Property {
                name: PhpStr::from(n),
                default: Value::Undef,
                visibility: Visibility::Public,
                set_visibility: Some(Visibility::Protected),
                declaring: lower.clone(),
                declaring_name: class_name.clone(),
                readonly: true,
                is_final: false,
                ty: Some(PhpStr::from(ty.unwrap_or("string"))),
                ty_decl: None,
                init: None,
                hooks: 0,
            };
            properties.insert(0, prop("name", None));
            if let Some(t) = &def.enum_backing {
                properties.insert(1, prop("value", Some(&text_of(t))));
            }
            for (case, value) in &def.cases {
                constants.insert(
                    case.as_bytes().to_vec(),
                    Rc::new(Constant {
                        value: value.as_ref().map_or(Value::Null, |v| self.isolate.thaw(v)),
                        visibility: Visibility::Public,
                        declaring_name: class_name.clone(),
                        is_case: true,
                        init: None,
                    }),
                );
            }
            let natives: &[(&str, p8_runtime::object::NativeStatic)] = if def.enum_backing.is_some() {
                &[("cases", crate::builtin_classes::enum_cases), ("from", crate::builtin_classes::enum_from), ("tryFrom", crate::builtin_classes::enum_try_from)]
            } else {
                &[("cases", crate::builtin_classes::enum_cases)]
            };
            for (n, f) in natives {
                methods.insert(
                    n.to_ascii_lowercase().into_bytes(),
                    Rc::new(Method {
                        name: PhpStr::from(*n),
                        func: u32::MAX,
                        native: Some(p8_runtime::object::Native::Static(*f)),
                        visibility: Visibility::Public,
                        is_static: true,
                        is_abstract: false,
                        is_final: false,
                        declaring: Rc::clone(&declaring),
                        declaring_name: class_name.clone(),
                    }),
                );
            }
        }
        let mut slots = p8_runtime::object::Slots::default();
        for (i, p) in properties.iter().enumerate() {
            slots.entry(p.name.as_bytes().to_vec()).or_default().push(i);
        }
        for c in &def.constants {
            constants.insert(
                c.name.as_bytes().to_vec(),
                Rc::new(Constant {
                    value: self.isolate.thaw(&c.value),
                    visibility: c.visibility,
                    declaring_name: class_name.clone(),
                    is_case: false,
                    init: c.code.as_ref().map(|code| Thunk(Rc::new(Arc::clone(code)))),
                }),
            );
        }
        for m in &def.methods {
            let mlower = m.func.sym;
            if let Some(old) = methods.get_sym(mlower)
                && old.is_final
                && !old.is_abstract
            {
                return Err(self.fatal_at(
                    format!("Cannot override final method {}::{}()", text(&old.declaring_name), text(&old.name)),
                    m.func.line,
                ));
            }
            let func = self.add_code(Arc::clone(&m.func));
            methods.insert_sym(
                mlower,
                Rc::new(Method {
                    name: PhpStr::from(m.func.name.as_bytes()),
                    func,
                    native: None,
                    visibility: m.visibility,
                    is_static: m.is_static,
                    is_abstract: m.is_abstract,
                    is_final: m.is_final,
                    declaring: Rc::clone(&declaring),
                    declaring_name: class_name.clone(),
                }),
            );
        }
        if def.flags & (CLASS_ABSTRACT | CLASS_INTERFACE | CLASS_TRAIT) == 0 {
            // An abstract hook `$x::get` is implemented by a property `x`
            // that can be read (or written, for `$x::set`).
            let implemented = |m: &Method| {
                let n = m.name.as_bytes();
                let Some(rest) = n.strip_prefix(b"$") else { return false };
                let Some(at) = rest.windows(2).position(|w| w == b"::") else { return false };
                let bit = if rest[at + 2..].eq_ignore_ascii_case(b"set") { p8_runtime::object::HOOK_SET } else { p8_runtime::object::HOOK_GET };
                properties.iter().any(|p| p.name.as_bytes() == &rest[..at] && (p.hooks & p8_runtime::object::HOOK_VIRTUAL == 0 || p.hooks & bit != 0))
            };
            let mut missing: Vec<String> = methods
                .values()
                .filter(|m| m.is_abstract && !implemented(m))
                .map(|m| format!("{}::{}", text(&m.declaring_name), text(&m.name)))
                .collect();
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
        // PHP's order: the class's own methods, the inherited ones, then
        // the traits' (get_class_methods, reflection).
        // One pass over the methods, not a removal (which shifts all after
        // it) per own or trait method.
        let mut ordered = p8_runtime::object::Methods::with_capacity(&symbols, methods.len());
        let own_set: p8_runtime::fast::HashSet<Sym> = own.iter().copied().collect();
        let trait_set: p8_runtime::fast::HashSet<Sym> = trait_order.iter().copied().collect();
        let mut own_methods = p8_runtime::fast::HashMap::default();
        let mut trait_methods = p8_runtime::fast::HashMap::default();
        let mut inherited = Vec::with_capacity(methods.len());
        for (k, m) in methods {
            if own_set.contains(&k) {
                own_methods.insert(k, m);
            } else if trait_set.contains(&k) {
                trait_methods.insert(k, m);
            } else {
                inherited.push((k, m));
            }
        }
        for key in &own {
            if let Some(m) = own_methods.remove(key) {
                ordered.insert_sym(*key, m);
            }
        }
        ordered.extend(inherited);
        for key in &trait_order {
            if let Some(m) = trait_methods.remove(key) {
                ordered.insert_sym(*key, m);
            }
        }
        let methods = ordered;
        let native_dtor = parent.as_ref().and_then(|p| p.native_dtor);
        let destructor = if methods.get_sym(known::DESTRUCT).is_some() || native_dtor.is_some() { p8_runtime::object::CLASS_DESTRUCTOR } else { 0 };
        let lazy = properties.iter().any(|p| p.init.is_some())
            || !static_inits.is_empty()
            || constants.values().any(|c| c.init.is_some())
            || parent.as_ref().is_some_and(|p| p.flags & CLASS_LAZY != 0);
        let destructor = destructor | if lazy { CLASS_LAZY } else { 0 };
        let destructor = destructor | if properties.iter().any(|p| p.hooks != 0) { p8_runtime::object::CLASS_HOOKS } else { 0 };
        let destructor = destructor | if [&b"__get"[..], b"__set", b"__isset", b"__unset", b"$*::get", b"$*::set", b"$*::isset", b"$*::unset"].iter().any(|m| methods.contains_key(m)) { p8_runtime::object::CLASS_MAGIC_PROPS } else { 0 };
        let destructor = destructor | parent.as_ref().map_or(0, |p| p.flags & (p8_runtime::object::CLASS_UNCLONEABLE | p8_runtime::object::CLASS_ALLOW_DYNAMIC));
        let make_data = parent.as_ref().and_then(|p| p.make_data);
        let (inherited_props, inherited_bool, inherited_string, inherited_clone) = parent.as_ref().map_or((None, None, None, None), |p| (p.get_properties, p.cast_bool, p.cast_string, p.clone_data));
        let inherited_forward = parent.as_ref().and_then(|p| p.forward_calls);
        let inherited_int = parent.as_ref().and_then(|p| p.cast_int);
        // An enum's `name` (and `value`) come first, as it declares them.
        let enum_props = if def.flags & CLASS_ENUM == 0 {
            &[][..]
        } else if def.enum_backing.is_some() {
            &["name", "value"][..]
        } else {
            &["name"][..]
        };
        let own = enum_props.iter().map(|n| PhpStr::from(*n)).chain(def.properties.iter().map(|p| PhpStr::from(p.name.as_bytes()))).collect();
        let property_order = property_order(own, parent.as_deref(), trait_properties);
        Ok(Rc::new(Class {
            name: class_name,
            lower,
            flags: def.flags | destructor,
            parent,
            interfaces,
            properties,
            slots,
            methods,
            constants,
            statics,
            property_order,
            debug_info: None,
            get_properties: inherited_props,
            cast_bool: inherited_bool,
            cast_string: inherited_string,
            cast_int: inherited_int,
            clone_data: inherited_clone,
            forward_calls: inherited_forward,
            make_data,
            native_dtor,
            enum_cases: def.cases.iter().map(|(n, _)| PhpStr::from(n.as_bytes())).collect(),
            enum_backing: def.enum_backing.as_ref().map(|t| PhpStr::from(t.as_bytes())),
            static_inits,
        }))
    }

    /// The class a code operand names.
    pub(crate) fn class_of(&mut self, func: &Function, i: u16, w: &[Value]) -> Result<Rc<Class>, Throw> {
        match &func.classes[i as usize] {
            ClassRef::Dynamic(r) => match w[*r as usize].deref() {
                Value::Object(o) => Ok(Rc::clone(&o.class)),
                Value::Str(name) => match self.sym_of(name.as_bytes()).and_then(|key| self.classes.get(&key)) {
                    Some(c) => Ok(Rc::clone(c)),
                    None => self.find_class(name.as_bytes()),
                },
                Value::Undef => {
                    self.undefined(func, *r);
                    Err(self.throw("Error", "Class name must be a valid object or a string"))
                }
                _ => Err(self.throw("Error", "Class name must be a valid object or a string")),
            },
            ClassRef::Named(name) | ClassRef::Forward(name) => match self.classes.get(&name.sym) {
                Some(c) => Ok(Rc::clone(c)),
                None => self.find_class(name.display.as_bytes()),
            },
            ClassRef::Scope => match self.scope().0 {
                Some(c) => Ok(c),
                None => Err(self.throw("Error", "Cannot access \"self\" when no class scope is active")),
            },
            ClassRef::ParentScope => match self.scope().0 {
                Some(scope) => match scope.parent.clone() {
                    Some(c) => Ok(c),
                    None => Err(self.throw("Error", "Cannot access \"parent\" when current class scope has no parent")),
                },
                None => Err(self.throw("Error", "Cannot access \"parent\" when no class scope is active")),
            },
            ClassRef::Static => match self.scope().1 {
                Some(c) => Ok(c),
                None => Err(self.throw("Error", "Cannot access \"static\" when no class scope is active")),
            },
        }
    }

    /// A new object of `class`, once its constructor (if any) may be
    /// called from here, and the constructor's frame; with `construct`
    /// false for an object made without its constructor
    /// (newInstanceWithoutConstructor): then the constructor's visibility
    /// does not matter.
    pub(crate) fn instantiate_as(&mut self, regs: &dyn Fn(usize) -> Value, class: Rc<Class>, construct: bool) -> Result<(Rc<Object>, Option<MethodFrame>), Throw> {
        let (info, _) = self.new_info(&class);
        self.make_from(regs, &info, construct, true)
    }

    /// `new C(args)` (and `instantiate_as`) with the class's NewInfo in hand:
    /// the object, its slots copied from the prepared defaults, once its
    /// constructor may be called from here (ZEND_NEW checks that before
    /// the arguments; the Construct after them makes the call), and with
    /// `frame` the constructor's frame.
    pub(crate) fn make_from(&mut self, regs: &dyn Fn(usize) -> Value, info: &crate::instances::NewInfo, construct: bool, frame: bool) -> Result<(Rc<Object>, Option<MethodFrame>), Throw> {
        let class = &info.class;
        if class.flags & (CLASS_INTERFACE | CLASS_ABSTRACT | CLASS_TRAIT | CLASS_ENUM) != 0 {
            return Err(self.cannot_instantiate(class));
        }
        let data = class.make_data.map(|make| make());
        let object = Object::with_slots(Rc::clone(class), &self.ids, data, p8_runtime::object::Props::from_slice(&info.defaults));
        if info.throwable {
            // Where it was made, and the calls that led there.
            let (file, line) = self.location();
            let root: &[u8] = if class.is_a(b"exception") { b"exception" } else { b"error" };
            class.write(&object, b"file", root, Value::Str(PhpStr::from(&*file)));
            class.write(&object, b"line", root, Value::Int(i64::from(line)));
            let trace = self.trace(regs);
            class.write(&object, b"trace", root, Value::Array(Rc::new(trace)));
        }
        let Some(ctor) = &info.ctor else { return Ok((object, None)) };
        if !construct {
            return Ok((object, None));
        }
        // A public constructor may be called from anywhere.
        if ctor.visibility == Visibility::Public && !frame {
            return Ok((object, None));
        }
        if !visible(ctor.visibility, &ctor.declaring, self.frames.last().and_then(|f| f.scope.as_deref())) {
            let t = self.throw("Error", format!("Call to {} {}::__construct() from {}", ctor.visibility.word(), text(&class.name), self.scope_name()));
            // Made while the object still is (PHP numbers the Error after it).
            let e = self.exception(regs, t);
            // Never constructed: never destructed.
            object.destructed.set(true);
            drop(object);
            return Err(e);
        }
        if !frame {
            return Ok((object, None));
        }
        Ok((Rc::clone(&object), Some(ctor.frame(object, class))))
    }

    #[cold]
    fn cannot_instantiate(&self, class: &Class) -> Throw {
        let kind = if class.flags & CLASS_INTERFACE != 0 {
            "interface"
        } else if class.flags & CLASS_ABSTRACT != 0 {
            "abstract class"
        } else if class.flags & CLASS_TRAIT != 0 {
            "trait"
        } else {
            "enum"
        };
        self.throw("Error", format!("Cannot instantiate {kind} {}", text(&class.name)))
    }

    /// The destructor call of an object (if its class has one).
    pub(crate) fn destructor(&mut self, object: &Rc<Object>) -> Option<MethodFrame> {
        let class = Rc::clone(&object.class);
        let dtor = class.method(b"__destruct")?;
        let declaring = self.declaring(&class, &dtor.declaring);
        Some(MethodFrame::method(self.code_of(dtor), Some(Rc::clone(object)), declaring, class))
    }

    /// The constructor call of a new object (its visibility was checked
    /// when the object was made).
    pub(crate) fn constructor(&mut self, object: &Rc<Object>) -> Option<MethodFrame> {
        let (info, _) = self.new_info(&object.class);
        info.ctor.as_ref().map(|c| c.frame(Rc::clone(object), &object.class))
    }

    /// An enum's case object (the same one each time in a request).
    pub(crate) fn enum_case(&mut self, class: &Rc<Class>, case: &[u8]) -> Rc<Object> {
        let key = (class.lower.clone(), case.to_vec());
        if let Some(o) = self.enum_objects.get(&key) {
            return Rc::clone(o);
        }
        let object = Object::new(Rc::clone(class), &self.ids);
        class.write(&object, b"name", &class.lower, Value::Str(PhpStr::from(case)));
        if class.enum_backing.is_some()
            && let Some(c) = class.constants.get(case)
        {
            class.write(&object, b"value", &class.lower, c.value.clone());
        }
        self.enum_objects.insert(key, Rc::clone(&object));
        object
    }

    /// The class in `class`'s chain that declares a member (as linked).
    pub(crate) fn declaring(&self, class: &Rc<Class>, lower: &[u8]) -> Rc<Class> {
        let mut c = Some(class);
        while let Some(k) = c {
            if k.lower == lower {
                return Rc::clone(k);
            }
            c = k.parent.as_ref();
        }
        self.class_named(lower).cloned().unwrap_or_else(|| Rc::clone(class))
    }

    /// The class a method's protected access is checked against: the top
    /// of the classes declaring it (zend_get_function_root_class), so two
    /// subclasses of the class that first declared it may call each other's.
    pub(crate) fn access_class(&mut self, declaring: &Rc<Class>, method: &Method, key: Option<Sym>) -> Rc<Class> {
        let mut root = Rc::clone(declaring);
        if method.visibility != Visibility::Protected {
            return root;
        }
        while let Some(parent) = root.parent.clone() {
            match method_of(&parent, key) {
                Some(m) if m.visibility != Visibility::Private => root = self.declaring(&parent, &m.declaring),
                _ => break,
            }
        }
        root
    }

    /// `$obj->method(args)`: the frame to push.
    pub(crate) fn method_frame(&mut self, target: &Value, key: Option<Sym>, display: &[u8]) -> Result<MethodFrame, Throw> {
        let object = match target {
            Value::Object(o) => Rc::clone(o),
            Value::Ref(cell) => return self.method_frame(&cell.borrow().clone(), key, display),
            other => {
                return Err(self.throw(
                    "Error",
                    format!("Call to a member function {}() on {}", String::from_utf8_lossy(display), other.type_name()),
                ));
            }
        };
        let class = Rc::clone(&object.class);
        let Some(method) = method_of(&class, key).cloned() else {
            if let Some(m) = self.magic_call(&class, Some(Rc::clone(&object)), display, false) {
                return Ok(m);
            }
            // An SPL iterator passes the call on to its inner iterator,
            // when that can take it (else the error names this class).
            if let Some(forward) = class.forward_calls
                && let Some(inner) = forward(&object)
                && answers(&inner, key)
            {
                return self.method_frame(&Value::Object(inner), key, display);
            }
            return Err(self.throw(
                "Error",
                format!("Call to undefined method {}::{}()", text(&class.name), String::from_utf8_lossy(display)),
            ));
        };
        // Code of a class calling its own private method on one of its
        // instances calls that method, even where a subclass declares one
        // of the same name (zend_std_get_method: the subclass's "changed"
        // the private one).
        let method = match self.scope().0 {
            Some(scope) if *method.declaring != *scope.lower && class.is_a(&scope.lower) => match method_of(&scope, key) {
                Some(own) if own.visibility == Visibility::Private && *own.declaring == *scope.lower => Rc::clone(own),
                _ => method,
            },
            _ => method,
        };
        let declaring = self.declaring(&class, &method.declaring);
        if !visible(method.visibility, &self.access_class(&declaring, &method, key), self.scope().0.as_deref()) {
            if let Some(m) = self.magic_call(&class, Some(Rc::clone(&object)), display, false) {
                return Ok(m);
            }
            return Err(self.throw(
                "Error",
                format!("Call to {} method {}::{}() from {}", method.visibility.word(), text(&class.name), String::from_utf8_lossy(display), self.scope_name()),
            ));
        }
        if method.is_abstract {
            return Err(self.throw("Error", format!("Cannot call abstract method {}::{}()", text(&method.declaring_name), text(&method.name))));
        }
        let func = self.code_of(&method);
        let this = (!method.is_static).then_some(object);
        Ok(MethodFrame::method(func, this, declaring, class))
    }

    /// `C::method(args)`: the frame to push. A non-static method gets the
    /// caller's `$this` when it is an instance of C; `self::` and
    /// `parent::` keep the caller's called class.
    pub(crate) fn static_frame(&mut self, func: &Function, class: u16, key: Option<Sym>, display: &[u8], w: &[Value]) -> Result<MethodFrame, Throw> {
        let target = self.class_of(func, class, w)?;
        // Borrowed from the class: a call must not copy the declaration.
        let Some(method) = method_of(&target, key) else {
            if let Some(m) = self.static_magic_call(&target, display) {
                return Ok(m);
            }
            return Err(self.throw(
                "Error",
                format!("Call to undefined method {}::{}()", text(&target.name), String::from_utf8_lossy(display)),
            ));
        };
        let declaring = self.declaring(&target, &method.declaring);
        let access = self.access_class(&declaring, method, key);
        let scope = self.frames.last().and_then(|f| f.scope.as_deref());
        if !visible(method.visibility, &access, scope) {
            if let Some(m) = self.static_magic_call(&target, display) {
                return Ok(m);
            }
            return Err(self.throw(
                "Error",
                format!("Call to {} method {}::{}() from {}", method.visibility.word(), text(&target.name), String::from_utf8_lossy(display), self.scope_name()),
            ));
        }
        if method.is_abstract {
            return Err(self.throw("Error", format!("Cannot call abstract method {}::{}()", text(&method.declaring_name), text(&method.name))));
        }
        let this = self.frames.last().and_then(|f| f.this.clone());
        let forward = matches!(func.classes[class as usize], ClassRef::Forward(..) | ClassRef::Static | ClassRef::Scope | ClassRef::ParentScope);
        let fframe = self.code_of(method);
        if method.is_static {
            let called = match self.frames.last().and_then(|f| f.called.as_ref()) {
                Some(c) if forward && c.is_a(&target.lower) => Rc::clone(c),
                _ => Rc::clone(&target),
            };
            return Ok(MethodFrame::method(fframe, None, declaring, called));
        }
        match this {
            Some(this) if this.class.is_a(&target.lower) => {
                let called = Rc::clone(&this.class);
                Ok(MethodFrame::method(fframe, Some(this), declaring, called))
            }
            _ => Err(self.throw(
                "Error",
                format!("Non-static method {}::{}() cannot be called statically", text(&method.declaring_name), text(&method.name)),
            )),
        }
    }

    /// `C::name()` with no such method callable: __call when the caller's
    /// `$this` is a C and C has it, else __callStatic.
    pub(crate) fn static_magic_call(&mut self, target: &Rc<Class>, display: &[u8]) -> Option<MethodFrame> {
        let this = self.frames.last().and_then(|f| f.this.clone()).filter(|t| t.class.is_a(&target.lower));
        if let Some(this) = this
            && let Some(m) = self.magic_call(target, Some(this), display, false)
        {
            return Some(m);
        }
        self.magic_call(target, None, display, true)
    }

    /// The __call (or, `is_static`, __callStatic) frame for a call of
    /// `display` on `class`, if it has one.
    pub(crate) fn magic_call(&mut self, class: &Rc<Class>, this: Option<Rc<Object>>, display: &[u8], is_static: bool) -> Option<MethodFrame> {
        let method = class.method_sym(if is_static { known::CALL_STATIC } else { known::CALL })?.clone();
        let declaring = self.declaring(class, &method.declaring);
        let called = this.as_ref().map(|t| Rc::clone(&t.class)).unwrap_or_else(|| Rc::clone(class));
        let this = if is_static { None } else { this };
        let mut m = MethodFrame::method(self.code_of(&method), this, declaring, called);
        m.trampoline = Some(PhpStr::from(display));
        Some(m)
    }

    /// The declared slot (or None: dynamic) of property `name` on
    /// `object`, after the visibility check for code in the current scope.
    pub(crate) fn prop_slot<'o>(&self, object: &'o Object, name: &[u8]) -> Result<Option<(usize, &'o Property)>, Throw> {
        let scope = self.scope().0;
        let Some(slot) = object.class.slot(name, scope.as_ref().map(|s| s.lower.as_slice())) else { return Ok(None) };
        // Borrowed from the class: a property read must not copy its
        // declaration (and the declaring name's allocation) each time.
        let prop = &object.class.properties[slot];
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

    /// Whether the running code may write `prop` of `object`, under its
    /// set visibility (zend_asymmetric_property_has_set_access): its
    /// declaring class may; for protected(set), so may a class related
    /// to the one that first declared it.
    pub(crate) fn may_set(&self, object: &Object, prop: &Property) -> bool {
        let Some(set) = prop.set_visibility else { return true };
        let Some(scope) = self.scope().0 else { return false };
        if scope.lower == prop.declaring {
            return true;
        }
        if set != Visibility::Protected {
            return false;
        }
        let mut root = self.declaring(&object.class, &prop.declaring);
        while let Some(parent) = root.parent.clone()
            && parent.properties.iter().any(|q| q.name == prop.name && q.visibility != Visibility::Private)
        {
            root = parent;
        }
        scope.is_a(&root.lower) || root.is_a(&scope.lower)
    }

    /// "Cannot modify private(set) property C::$p from global scope"
    /// (zend_asymmetric_visibility_property_modification_error), naming
    /// the called scope.
    pub(crate) fn set_visibility_error(&self, prop: &Property, operation: &str) -> Throw {
        let visibility = match (prop.set_visibility, prop.readonly) {
            (Some(Visibility::Private), _) => "private(set)",
            (_, true) => "protected(set) readonly",
            _ => "protected(set)",
        };
        let scope = match self.scope().1 {
            Some(c) => format!("scope {}", text(&c.name)),
            None => "global scope".into(),
        };
        self.throw("Error", format!("Cannot {operation} {visibility} property {}::${} from {scope}", text(&prop.declaring_name), text(&prop.name)))
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
    /// `&$obj->name`: the property made a reference (a dynamic one made).
    /// `for_unset`: for `unset($obj->name[k])`, which leaves an
    /// uninitialized property alone.
    pub(crate) fn prop_ref(&mut self, target: &Value, name: &Value, for_unset: bool) -> Result<Rc<RefCell<Value>>, Throw> {
        let name = self.prop_name(name)?;
        let object = self.target_object(target, &name, "modify")?;
        if let Some(p) = self.hooked_property(&object, name.as_bytes()) {
            return Err(self.throw("Error", format!("Indirect modification of {}::${} is not allowed", text(&p.declaring_name), text(&name))));
        }
        match self.prop_slot(&object, name.as_bytes())? {
            Some((slot, prop)) => {
                // An object is handed out as a copy; anything else is not
                // to be had (zend_std_read_property for BP_VAR_W).
                if prop.readonly {
                    match object.slots.borrow()[slot].deref() {
                        v @ Value::Object(_) => return Ok(Rc::new(RefCell::new(v))),
                        Value::Undef if for_unset => return Ok(Rc::new(RefCell::new(Value::Null))),
                        _ => {}
                    }
                    return Err(self.throw("Error", format!("Cannot indirectly modify readonly property {}::${}", text(&prop.declaring_name), text(&name))));
                }
                if prop.set_visibility.is_some() && !self.may_set(&object, prop) {
                    // An object is handed out as a copy (writes through it
                    // change the object, not the property).
                    match object.slots.borrow()[slot].deref() {
                        v @ Value::Object(_) => return Ok(Rc::new(RefCell::new(v))),
                        Value::Undef if for_unset => return Ok(Rc::new(RefCell::new(Value::Null))),
                        _ => {}
                    }
                    return Err(self.set_visibility_error(prop, "indirectly modify"));
                }
                Ok(crate::refs::make_ref(&mut object.slots.borrow_mut()[slot]))
            }
            None if name.as_bytes().first() == Some(&0) => Err(self.throw("Error", "Cannot access property starting with \"\\0\"")),
            None => {
                let key = Key::from_string(name.clone());
                if !object.dynamic.borrow().contains_key(&key) {
                    if object.class.lower != b"stdclass" && object.class.flags & p8_runtime::object::CLASS_ALLOW_DYNAMIC == 0 {
                        self.diags.push(p8_runtime::ops::Diag::Deprecated(format!(
                            "Creation of dynamic property {}::${} is deprecated",
                            text(&object.class.name),
                            text(&name)
                        )));
                        self.report_diags();
                    }
                    object.dynamic.borrow_mut().insert(key.clone(), Value::Null);
                }
                let mut dynamic = object.dynamic.borrow_mut();
                let slot = dynamic.get_mut(&key).expect("just made");
                Ok(crate::refs::make_ref(slot))
            }
        }
    }

    /// `$obj->name = &…`: the property becomes the reference `cell`.
    pub(crate) fn prop_bind(&mut self, target: &Value, name: &Value, cell: Rc<RefCell<Value>>) -> Result<(), Throw> {
        let pname = self.prop_name(name)?;
        let object = self.target_object(target, &pname, "assign")?;
        if let Some(p) = self.hooked_property(&object, pname.as_bytes()) {
            return Err(self.throw("Error", format!("Indirect modification of {}::${} is not allowed", text(&p.declaring_name), text(&pname))));
        }
        match self.prop_slot(&object, pname.as_bytes())? {
            Some((slot, prop)) if prop.set_visibility.is_some() && !self.may_set(&object, prop) => {
                if matches!(object.slots.borrow()[slot].deref(), Value::Object(_)) {
                    return Err(self.throw("Error", "Cannot assign by reference to overloaded object"));
                }
                return Err(self.set_visibility_error(prop, "indirectly modify"));
            }
            Some((slot, _)) => object.slots.borrow_mut()[slot] = Value::Ref(cell),
            None => {
                self.write_prop(target, name, Value::Null)?;
                object.dynamic.borrow_mut().insert(Key::from_string(pname), Value::Ref(cell));
            }
        }
        Ok(())
    }

    pub(crate) fn write_prop(&mut self, target: &Value, name: &Value, value: Value) -> Result<(), Throw> {
        let name = self.prop_name(name)?;
        let object = self.target_object(target, &name, "assign")?;
        match self.prop_slot(&object, name.as_bytes())? {
            Some((slot, prop)) => self.write_slot(&object, slot, prop, &name, value)?,
            // A mangled name ("\0A\0p") is no property's (zend_bad_property_name).
            None if name.as_bytes().first() == Some(&0) => return Err(self.throw("Error", "Cannot access property starting with \"\\0\"")),
            None => {
                let key = Key::from_string(name.clone());
                let exists = object.dynamic.borrow().contains_key(&key);
                if !exists && object.class.lower != b"stdclass" && object.class.flags & p8_runtime::object::CLASS_ALLOW_DYNAMIC == 0 {
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


    /// `write_slot` for the write a property cache answers: stored at once
    /// when there is nothing to check (not readonly, no set visibility,
    /// untyped or already of its type, as zend_assign_to_typed_prop's
    /// ZEND_TYPE_CONTAINS_CODE test); else the full checks.
    #[inline]
    pub(crate) fn write_cached(&mut self, object: &Object, slot: usize, name: &PhpStr, value: Value) -> Result<(), Throw> {
        let prop = &object.class.properties[slot];
        if !prop.readonly && prop.set_visibility.is_none() && prop.ty_decl.as_ref().is_none_or(|t| t.quick(&value)) {
            let mut slots = object.slots.borrow_mut();
            match &mut slots[slot] {
                Value::Ref(cell) => *cell.borrow_mut() = value,
                s => *s = value,
            }
            return Ok(());
        }
        self.write_slot(object, slot, prop, name, value)
    }

    /// Store `value` in declared property `slot` of `object` (`prop`, named
    /// `name`): its readonly and set-visibility rules and its type checked,
    /// as zend_std_write_property does once it has the property's offset.
    pub(crate) fn write_slot(&mut self, object: &Object, slot: usize, prop: &Property, name: &PhpStr, value: Value) -> Result<(), Throw> {
        // Who may initialize a readonly property is its set
        // visibility's business (implicitly protected(set)).
        // (In a clone's __clone or clone()'s updates, once more.)
        if prop.readonly && !matches!(object.slots.borrow()[slot], Value::Undef) && !self.reinitable(object, slot) {
            return Err(self.throw("Error", format!("Cannot modify readonly property {}::${}", text(&prop.declaring_name), text(name))));
        }
        if prop.set_visibility.is_some() && !self.may_set(object, prop) {
            return Err(self.set_visibility_error(prop, "modify"));
        }
        // A typed property: the value checked (and converted) by the
        // writing code's strict_types.
        let value = match &prop.ty_decl {
            Some(ty) => {
                let strict = self.frames.last().is_some_and(|f| f.func.code.strict);
                match self.check_type(ty, &value, strict)? {
                    Ok(Some(converted)) => converted,
                    Ok(None) => value,
                    Err(()) => {
                        return Err(self.throw(
                            "TypeError",
                            format!(
                                "Cannot assign {} to property {}::${} of type {}",
                                p8_runtime::args::value_name(&value),
                                text(&prop.declaring_name),
                                text(name),
                                ty.shown()
                            ),
                        ));
                    }
                }
            }
            None => value,
        };
        if prop.readonly && !self.reinit.is_empty() {
            self.reinit_done(object, slot);
        }
        let mut slots = object.slots.borrow_mut();
        match &slots[slot] {
            Value::Ref(cell) => *cell.borrow_mut() = value,
            _ => slots[slot] = value,
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
        keys: &[Value],
        append: bool,
        val: Value,
        op: Option<p8_runtime::ops::BinOp>,
    ) -> Result<crate::Written, Throw> {
        let pname = self.prop_name(name)?;
        let object = self.target_object(target, &pname, "assign")?;
        let slot = self.prop_slot(&object, pname.as_bytes())?;
        // (Even in __clone: zend_std_read_property for BP_VAR_W.)
        if let Some((_, prop)) = &slot
            && prop.readonly
        {
            return Err(self.throw("Error", format!("Cannot indirectly modify readonly property {}::${}", text(&prop.declaring_name), text(&pname))));
        }
        if let Some((_, prop)) = &slot
            && prop.set_visibility.is_some()
            && !self.may_set(&object, prop)
        {
            return Err(self.set_visibility_error(prop, "indirectly modify"));
        }
        let slot = slot.map(|(i, _)| i);
        self.update_slot_dim(&object, slot, &pname, keys, append, val, op)
    }

    /// `$obj->p[k1]..[kn] = val` into the property's place: declared slot
    /// `slot`, else the dynamic property `pname` (made at the end if it
    /// is not there, as PHP adds it; one that is keeps its place). Taken
    /// out, written, put back (nothing runs PHP code meanwhile).
    #[allow(clippy::too_many_arguments)] // AssignPropDim's operands, decoded once
    pub(crate) fn update_slot_dim(
        &mut self,
        object: &Rc<Object>,
        slot: Option<usize>,
        pname: &PhpStr,
        keys: &[Value],
        append: bool,
        val: Value,
        op: Option<p8_runtime::ops::BinOp>,
    ) -> Result<crate::Written, Throw> {
        fn take(v: &mut Value) -> Value {
            match v {
                Value::Ref(cell) => std::mem::take(&mut *cell.borrow_mut()),
                v => std::mem::take(v),
            }
        }
        fn put(v: &mut Value, root: Value) {
            match v {
                Value::Ref(cell) => *cell.borrow_mut() = root,
                v => *v = root,
            }
        }
        let key = || Key::from_string(pname.clone());
        let mut root = match slot {
            Some(i) => take(&mut object.slots.borrow_mut()[i]),
            None => match take(object.dynamic.borrow_mut().slot_mut(&key())) {
                Value::Undef => Value::Null,
                v => v,
            },
        };
        let result = self.update_path(&mut root, keys, 0, append, val, op);
        match slot {
            Some(i) => put(&mut object.slots.borrow_mut()[i], root),
            None => put(object.dynamic.borrow_mut().slot_mut(&key()), root),
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
                if prop.readonly && !self.reinitable(&object, i) {
                    return Err(self.throw("Error", format!("Cannot unset readonly property {}::${}", text(&object.class.name), text(&name))));
                }
                if prop.set_visibility.is_some() && !self.may_set(&object, prop) {
                    return Err(self.set_visibility_error(prop, "unset"));
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
    pub(crate) fn class_constant(&mut self, func: &Function, class: u16, name: &Value, w: &[Value]) -> Result<Value, Throw> {
        let class = self.class_of(func, class, w)?;
        let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
        if name.as_bytes().eq_ignore_ascii_case(b"class") {
            return Ok(Value::Str(class.name.clone()));
        }
        match class.constants.get(name.as_bytes()) {
            Some(c) => {
                // Only a private or protected constant needs its declaring
                // class (zend_verify_const_access); the class itself when it
                // declared it, else looked up by name.
                if c.visibility != Visibility::Public {
                    let declaring = if c.declaring_name == class.name {
                        Rc::clone(&class)
                    } else {
                        self.class_named(c.declaring_name.as_bytes()).cloned().unwrap_or_else(|| Rc::clone(&class))
                    };
                    if !visible(c.visibility, &declaring, self.frames.last().and_then(|f| f.scope.as_deref())) {
                        return Err(self.throw(
                            "Error",
                            format!("Cannot access {} constant {}::{}", c.visibility.word(), text(&class.name), text(&name)),
                        ));
                    }
                }
                if c.is_case {
                    return Ok(Value::Object(self.enum_case(&class, name.as_bytes())));
                }
                if c.init.is_some() {
                    let key = (c.declaring_name.as_bytes().to_ascii_lowercase(), name.as_bytes().to_vec());
                    return Ok(self.const_cache.get(&key).cloned().unwrap_or(Value::Null));
                }
                Ok(c.value.clone())
            }
            None => Err(self.throw("Error", format!("Undefined constant {}::{}", text(&class.name), text(&name)))),
        }
    }

    /// A class constant whose value is code not yet run: the code, the
    /// class it runs in, and its cache key.
    #[allow(clippy::type_complexity)] // reason: one use, the ClassConst instruction
    pub(crate) fn pending_constant(&mut self, func: &Function, class: u16, name: &Value, w: &[Value]) -> Result<Option<(Thunk, Rc<Class>, (Vec<u8>, Vec<u8>))>, Throw> {
        let class = self.class_of(func, class, w)?;
        let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
        let Some(c) = class.constants.get(name.as_bytes()) else { return Ok(None) };
        let Some(init) = &c.init else { return Ok(None) };
        let key = (c.declaring_name.as_bytes().to_ascii_lowercase(), name.as_bytes().to_vec());
        if self.const_cache.contains_key(&key) {
            return Ok(None);
        }
        let scope = self.class_named(&key.0).cloned().unwrap_or_else(|| Rc::clone(&class));
        Ok(Some((init.clone(), scope, key)))
    }

    /// Compute a class constant from its code (once).
    pub(crate) fn eval_constant(&mut self, regs: &mut Vec<Value>, thunk: &Thunk, scope: Rc<Class>, key: (Vec<u8>, Vec<u8>)) -> Result<(), Throw> {
        if self.const_evaluating.contains(&key) {
            let name = String::from_utf8_lossy(&key.1).into_owned();
            return Err(self.throw("Error", format!("Cannot declare self-referencing constant self::{name}")));
        }
        self.const_evaluating.push(key.clone());
        let r = self.run_thunk(regs, thunk, scope);
        self.const_evaluating.retain(|k| k != &key);
        self.const_cache.insert(key, r?);
        Ok(())
    }

    /// Run a constant expression's code in `scope`.
    fn run_thunk(&mut self, regs: &mut Vec<Value>, thunk: &Thunk, scope: Rc<Class>) -> Result<Value, Throw> {
        let func = thunk.0.downcast_ref::<Arc<Function>>().expect("a constant expression's code");
        let m = MethodFrame { func: Code::User(Closure::new(Arc::clone(func), self.isolate)), this: None, scope: Some(Rc::clone(&scope)), called: Some(scope), captured: Vec::new(), trampoline: None };
        // The caller is at the instruction that needs the value (its line
        // is where an error in the code is reported).
        let (pc, saved) = (self.pc, self.frames.last().map(|f| f.pc));
        if let Some(top) = self.frames.last_mut() {
            top.pc = pc as u32;
        }
        let r = self.run_nested(regs, m, &mut []);
        if let (Some(saved), Some(top)) = (saved, self.frames.last_mut()) {
            top.pc = saved;
        }
        r
    }

    /// The hook a property access on `target` runs: Some(frame) for its
    /// `get` (bit HOOK_GET) or `set` hook; an Error for a virtual
    /// property without that hook; None for a plain access (no hook, the
    /// stored value of a backed one, or the hook's own `$this->name`).
    pub(crate) fn prop_hook(&mut self, target: &Value, name: &Value, bit: u8) -> Option<Result<MethodFrame, Throw>> {
        self.prop_hook_seen(target, name, bit, true)
    }

    pub(crate) fn prop_hook_seen(&mut self, target: &Value, name: &Value, bit: u8, check_visibility: bool) -> Option<Result<MethodFrame, Throw>> {
        let Value::Object(o) = target else { return None };
        if o.class.flags & p8_runtime::object::CLASS_HOOKS == 0 {
            return None;
        }
        let name = name.to_php_string()?;
        let p = self.hooked_property_seen(o, name.as_bytes(), check_visibility)?;
        let kind: &[u8] = if bit == p8_runtime::object::HOOK_GET { b"get" } else { b"set" };
        if p.hooks & bit == 0 {
            if p.hooks & p8_runtime::object::HOOK_VIRTUAL == 0 {
                return None;
            }
            let what = if bit == p8_runtime::object::HOOK_GET { "write-only" } else { "read-only" };
            return Some(Err(self.throw("Error", format!("Property {}::${} is {what}", text(&p.declaring_name), text(&name)))));
        }
        if bit == p8_runtime::object::HOOK_SET && check_visibility && !self.may_set(o, p) {
            return Some(Err(self.set_visibility_error(p, "modify")));
        }
        let key = [b"$", name.as_bytes(), b"::", kind].concat().to_ascii_lowercase();
        let m = o.class.method(&key)?;
        let declaring = self.declaring(&o.class, &m.declaring);
        Some(Ok(MethodFrame::method(self.code_of(m), Some(Rc::clone(o)), declaring, Rc::clone(&o.class))))
    }

    /// The magic method (`__get`, ...) a property access on `target` runs
    /// instead, as zend_std_read_property and friends decide: the
    /// property is missing (not even dynamic), not visible here, or an
    /// untyped one that was unset; and the method is not already running
    /// for this object and property. With the guard key to hold while it runs.
    pub(crate) fn prop_magic(&mut self, target: &Value, name: &Value, kind: Magic) -> Option<(MethodFrame, (usize, PhpStr, Magic))> {
        let Value::Object(o) = target else { return None };
        if o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0 {
            return None;
        }
        // A native class's handler (`$*::get`: hidden, as PHP's internal
        // read_property is no method), else the class's __get.
        let m = o.class.method(kind.native()).or_else(|| o.class.method(kind.method()))?;
        let name = name.to_php_string()?;
        let key = (Rc::as_ptr(o) as usize, name.clone(), kind);
        if self.magic_guard.contains(&key) {
            return None;
        }
        let scope = self.scope().0;
        match o.class.slot(name.as_bytes(), scope.as_ref().map(|s| s.lower.as_slice())) {
            Some(slot) => {
                // A lazy property is uninitialized, not unset: initialization
                // comes before any magic method.
                if o.lazy_slot(slot) {
                    return None;
                }
                let prop = &o.class.properties[slot];
                let declaring = self.declaring(&o.class, &prop.declaring);
                if visible(prop.visibility, &declaring, scope.as_deref()) && !(prop.ty.is_none() && matches!(o.slots.borrow()[slot], Value::Undef)) {
                    return None;
                }
            }
            None => {
                if o.dynamic.borrow().contains_key(&Key::from_string(name.clone())) {
                    return None;
                }
            }
        }
        let declaring = self.declaring(&o.class, &m.declaring);
        Some((MethodFrame::method(self.code_of(m), Some(Rc::clone(o)), declaring, Rc::clone(&o.class)), key))
    }

    /// The hooked property `name` of `o` as the running code sees it (None
    /// when not hooked, not visible here, or inside its own hook on `o`).
    pub(crate) fn hooked_property<'c>(&self, o: &'c Rc<Object>, name: &[u8]) -> Option<&'c Property> {
        self.hooked_property_seen(o, name, true)
    }

    /// As hooked_property; `check_visibility` false: as the dump
    /// functions see it (any visibility).
    pub(crate) fn hooked_property_seen<'c>(&self, o: &'c Rc<Object>, name: &[u8], check_visibility: bool) -> Option<&'c Property> {
        // A class with no hooked property: no lookup by name.
        if o.class.flags & p8_runtime::object::CLASS_HOOKS == 0 {
            return None;
        }
        let slots = o.class.slots.get(name)?;
        let p = slots.iter().map(|&i| &o.class.properties[i]).find(|p| p.hooks != 0)?;
        let declaring = self.class_named(&p.declaring).cloned().unwrap_or_else(|| Rc::clone(&o.class));
        if check_visibility && !visible(p.visibility, &declaring, self.scope().0.as_deref()) {
            return None;
        }
        // In a hook of this property on this object: the stored value.
        if let Some(top) = self.frames.last()
            && top.this.as_ref().is_some_and(|t| Rc::ptr_eq(t, o))
        {
            let fname = top.func.code.name.as_bytes();
            if fname.len() > name.len() + 1 && fname[0] == b'$' && &fname[1..=name.len()] == name && fname[name.len() + 1..].starts_with(b"::") {
                return None;
            }
        }
        Some(p)
    }

    /// Whether a class has default code still to run.
    pub(crate) fn unresolved(&self, class: &Rc<Class>) -> bool {
        class.flags & CLASS_LAZY != 0 && !self.resolved.contains(&(Rc::as_ptr(class) as usize))
    }

    /// The class at `class`, if its defaults' code has not run yet.
    pub(crate) fn lazy_class(&mut self, func: &Function, class: u16, w: &[Value]) -> Result<Option<Rc<Class>>, Throw> {
        let c = self.class_of(func, class, w)?;
        Ok(self.unresolved(&c).then_some(c))
    }

    /// Run a lazy class's default code (its parent's first): static
    /// properties get their values, and its instance defaults are kept
    /// for each new object.
    pub(crate) fn resolve_class(&mut self, regs: &mut Vec<Value>, class: &Rc<Class>) -> Result<(), Throw> {
        let key = Rc::as_ptr(class) as usize;
        if !self.unresolved(class) {
            return Ok(());
        }
        self.resolved.insert(key);
        let r = self.resolve_now(regs, class);
        if r.is_err() {
            self.resolved.remove(&key);
        }
        r
    }

    fn resolve_now(&mut self, regs: &mut Vec<Value>, class: &Rc<Class>) -> Result<(), Throw> {
        if let Some(parent) = &class.parent {
            self.resolve_class(regs, parent)?;
        }
        // Its constants (PHP computes them all before the first use).
        let mut pending: Vec<(&Vec<u8>, &Rc<p8_runtime::object::Constant>)> = class.constants.iter().filter(|(_, c)| c.init.is_some()).collect();
        pending.sort_by(|a, b| a.0.cmp(b.0));
        for (name, c) in pending {
            let key = (c.declaring_name.as_bytes().to_ascii_lowercase(), name.clone());
            if !self.const_cache.contains_key(&key) {
                let scope = self.class_named(&key.0).cloned().unwrap_or_else(|| Rc::clone(class));
                let thunk = c.init.clone().expect("filtered");
                self.eval_constant(regs, &thunk, scope, key)?;
            }
        }
        for (name, thunk) in &class.static_inits {
            let v = self.run_thunk(regs, thunk, Rc::clone(class))?;
            if let Some((cell, _, _)) = class.statics.get(name) {
                *cell.borrow_mut() = v;
            }
        }
        let mut defaults = Vec::new();
        for (i, p) in class.properties.iter().enumerate() {
            if let Some(thunk) = &p.init {
                let scope = self.class_named(&p.declaring).cloned().unwrap_or_else(|| Rc::clone(class));
                defaults.push((i, self.run_thunk(regs, thunk, scope)?));
            }
        }
        self.class_defaults.insert(Rc::as_ptr(class) as usize, Rc::new(defaults));
        Ok(())
    }

    /// Check `v` against a declared type in the running code: Ok(Some)
    /// the converted value, Ok(None) as it is, Err a TypeError's case.
    /// (A deprecation it brings is reported here.)
    /// (Outer Err: what looking up a class a callable names threw.)
    pub(crate) fn check_type(&mut self, ty: &p8_runtime::types::TypeDecl, v: &Value, strict: bool) -> Result<Result<Option<Value>, ()>, Throw> {
        if ty.quick(v) {
            return Ok(Ok(None));
        }
        // `callable`: whether the code checking can call it (zend_is_callable).
        let callable = ty.mask & p8_runtime::types::T_CALLABLE != 0
            && matches!(v.deref(), Value::Str(_) | Value::Array(_) | Value::Object(_))
            && self.why_not_callable(v, true)?.is_none();
        let (scope, called) = self.frames.last().map_or((None, None), |f| (f.scope.as_deref(), f.called.as_deref()));
        let is_a = |o: &Object, name: &[u8]| match name {
            b"self" => scope.is_some_and(|s| o.class.is_a(&s.lower)),
            b"parent" => scope.and_then(|s| s.parent.as_ref()).is_some_and(|p| o.class.is_a(&p.lower)),
            n => o.class.is_a(n),
        };
        let static_ok = |o: &Object| called.is_some_and(|c| o.class.is_a(&c.lower));
        let verdict = ty.verify(v, strict, &is_a, &static_ok, callable);
        Ok(match verdict {
            p8_runtime::types::Verdict::Accepted(c) => Ok(c),
            p8_runtime::types::Verdict::Deprecated(m, c) => {
                self.diags.push(p8_runtime::ops::Diag::Deprecated(m));
                self.report_diags();
                Ok(c)
            }
            p8_runtime::types::Verdict::Refused => Err(()),
        })
    }

    /// How messages name the running function: `f`, `A::m`, `{closure}`.
    pub(crate) fn function_display(&self) -> String {
        let Some(frame) = self.frames.last() else { return String::new() };
        let code = &frame.func.code;
        match (&code.class, &frame.scope) {
            (Some(_), Some(scope)) if !code.name.as_bytes().starts_with(b"{closure") => format!("{}::{}", text(&scope.name), text_of(&code.name)),
            _ => text_of(&code.name),
        }
    }

    /// Function entry: the passed arguments against their declared types
    /// (the caller's strict_types decides the conversions).
    pub(crate) fn verify_args(&mut self, regs: &mut Vec<Value>, base: usize, argc: usize) -> Result<(), Throw> {
        let n = self.frames.len();
        let Some(frame) = self.frames.last() else { return Ok(()) };
        let func = Rc::clone(&frame.func);
        let caller = (n >= 2 && n - 1 > self.frames.floor.saturating_sub(1)).then(|| &self.frames.slots[n - 2]);
        let from_native = self.natives.iter().any(|x| x.depth == n - 1);
        let strict = !from_native && caller.is_some_and(|c| c.func.code.strict);
        // Where the call was made, for a TypeError's message: only once a
        // check fails (the frames are as they were here by then).
        let called_at = |cx: &Self| match (n >= 2 && n - 1 > cx.frames.floor.saturating_sub(1)).then(|| &cx.frames.slots[n - 2]) {
            Some(c) if !from_native => {
                let line = c.func.code.lines.get((c.pc as usize).saturating_sub(1)).copied().unwrap_or(0);
                Some((Rc::<str>::from(&*c.func.code.file), line))
            }
            _ => None,
        };
        for (i, p) in func.code.params.iter().enumerate() {
            let Some(ty) = &p.ty else { continue };
            if p.variadic {
                // Each extra argument, by the same type.
                let Value::Array(list) = regs[base + i].clone() else { continue };
                let mut checked = (*list).clone();
                for (k, v) in list.iter() {
                    let coerced = match self.check_type(ty, &v.deref(), strict)? {
                        Err(()) => self.stringable(regs, ty, &v.deref(), strict)?.map(Some).ok_or(()),
                        other => other,
                    };
                    match coerced {
                        Ok(Some(converted)) => checked.insert(k, converted),
                        Ok(None) => {}
                        Err(()) => {
                            let pos = match k {
                                Key::Int(j) => i as i64 + j + 1,
                                Key::Str(_) => i as i64 + 1,
                            };
                            return Err(self.arg_type_error(pos as usize, None, ty, &v.deref(), called_at(self)));
                        }
                    }
                }
                regs[base + i] = Value::Array(Rc::new(checked));
                continue;
            }
            if i >= argc || matches!(regs[base + i], Value::Undef) || ty.quick(&regs[base + i]) {
                continue;
            }
            let v = regs[base + i].deref();
            let converted = match self.check_type(ty, &v, strict)? {
                Ok(converted) => converted,
                Err(()) => match self.stringable(regs, ty, &v, strict)? {
                    Some(s) => Some(s),
                    None => return Err(self.arg_type_error(i + 1, Some(&p.name), ty, &v, called_at(self))),
                },
            };
            if let Some(converted) = converted {
                match &regs[base + i] {
                    Value::Ref(cell) => *cell.borrow_mut() = converted,
                    _ => regs[base + i] = converted,
                }
            }
        }
        Ok(())
    }

    /// Coercive typing's last resort: an object with __toString() for a
    /// type that takes a string, as the string (None: not so).
    pub(crate) fn stringable(&mut self, regs: &mut Vec<Value>, ty: &p8_runtime::types::TypeDecl, v: &Value, strict: bool) -> Result<Option<Value>, Throw> {
        let Value::Object(o) = v else { return Ok(None) };
        if strict || ty.mask & p8_runtime::types::T_STRING == 0 || o.class.method(b"__tostring").is_none() {
            return Ok(None);
        }
        Ok(match self.string_of(regs, v.clone())? {
            s @ Value::Str(_) => Some(s),
            _ => None,
        })
    }

    fn arg_type_error(&self, pos: usize, name: Option<&p8_runtime::frozen::Text>, ty: &p8_runtime::types::TypeDecl, v: &Value, called_at: Option<(Rc<str>, u32)>) -> Throw {
        let label = name.map_or(String::new(), |n| format!(" (${})", text_of(n)));
        let at = called_at.map_or(String::new(), |(file, line)| format!(", called in {file} on line {line}"));
        self.throw(
            "TypeError",
            format!("{}(): Argument #{pos}{label} must be of type {}, {} given{at}", self.function_display(), self.type_shown(ty), p8_runtime::args::value_name(v)),
        )
    }

    /// A type as a message shows it where the running code is
    /// (zend_type_to_string_resolved): `self` and `parent` as the scope's
    /// classes, `static` as the called class.
    pub(crate) fn type_shown(&self, ty: &p8_runtime::types::TypeDecl) -> String {
        let frame = self.frames.last();
        let scope = frame.and_then(|f| f.scope.as_ref());
        let relative = ty.classes.iter().any(|c| c.as_bytes().eq_ignore_ascii_case(b"self") || c.as_bytes().eq_ignore_ascii_case(b"parent"));
        let called = frame.and_then(|f| f.called.as_ref().or(f.this.as_ref().map(|t| &t.class))).or(scope);
        let is_static = ty.mask & p8_runtime::types::T_STATIC != 0 && called.is_some();
        if scope.is_none() || !(relative || is_static) {
            return ty.shown_resolved();
        }
        let mut classes: Vec<p8_runtime::frozen::Text> = ty
            .classes
            .iter()
            .map(|c| match c.as_bytes().to_ascii_lowercase().as_slice() {
                b"self" => scope.map_or_else(|| c.clone(), |s| p8_runtime::frozen::Text::from(s.name.as_bytes())),
                b"parent" => scope.and_then(|s| s.parent.as_ref()).map_or_else(|| c.clone(), |p| p8_runtime::frozen::Text::from(p.name.as_bytes())),
                _ => c.clone(),
            })
            .collect();
        let mut mask = ty.mask;
        if is_static && let Some(called) = called {
            mask &= !p8_runtime::types::T_STATIC;
            classes.insert(0, p8_runtime::frozen::Text::from(called.name.as_bytes()));
        }
        p8_runtime::types::TypeDecl::new(mask, classes.clone(), classes, ty.intersection).shown_resolved()
    }

    /// `(object) $v`: an object as is; an array's entries as a stdClass's
    /// properties; null an empty stdClass; another value its `scalar`.
    pub(crate) fn object_cast(&mut self, v: Value) -> Value {
        if let Value::Object(_) = v {
            return v;
        }
        let class = Rc::clone(self.class_named(b"stdclass").expect("stdClass"));
        let o = Object::new(class, &self.ids);
        {
            let mut props = o.dynamic.borrow_mut();
            match v {
                Value::Null | Value::Undef => {}
                Value::Array(a) => {
                    for (k, v) in a.iter() {
                        props.insert(k, v.clone());
                    }
                }
                v => {
                    props.insert(Key::from_string(PhpStr::from("scalar")), v);
                }
            }
        }
        Value::Object(o)
    }

    /// `isset(C::$name)`: declared, visible here, and not null (an
    /// unknown class is an error, as in PHP).
    pub(crate) fn isset_static(&mut self, func: &Function, class: u16, name: &Value, w: &[Value]) -> Result<bool, Throw> {
        let class = self.class_of(func, class, w)?;
        let Some(name) = name.to_php_string() else { return Ok(false) };
        let Some((cell, visibility, declaring)) = class.statics.get(name.as_bytes()) else { return Ok(false) };
        let declaring = self.class_named(declaring).cloned().unwrap_or_else(|| Rc::clone(&class));
        Ok(visible(*visibility, &declaring, self.scope().0.as_deref()) && !matches!(cell.borrow().deref(), Value::Null | Value::Undef))
    }

    /// The cell of static property `C::$name`.
    pub(crate) fn static_cell(&mut self, func: &Function, class: u16, name: &Value, w: &[Value]) -> Result<Rc<RefCell<Value>>, Throw> {
        let class = self.class_of(func, class, w)?;
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
    pub(crate) fn instance_of(&self, func: &Function, v: &Value, class: u16, w: &[Value]) -> bool {
        let Value::Object(o) = v.deref() else { return false };
        match &func.classes[class as usize] {
            ClassRef::Dynamic(r) => match w[*r as usize].deref() {
                Value::Object(c) => o.class.is_a(&c.class.lower),
                Value::Str(name) => self.is_a_named(&o.class, &name.as_bytes().strip_prefix(b"\\").unwrap_or(name.as_bytes()).to_ascii_lowercase()),
                _ => false,
            },
            ClassRef::Named(name) | ClassRef::Forward(name) => o.class.is_a(name.lower.as_bytes()) || self.classes.get(&name.sym).is_some_and(|c| c.lower != name.lower.as_bytes() && o.class.is_a(&c.lower)),
            ClassRef::Static => self.scope().1.is_some_and(|c| o.class.is_a(&c.lower)),
            ClassRef::Scope => self.scope().0.is_some_and(|c| o.class.is_a(&c.lower)),
            ClassRef::ParentScope => self.scope().0.and_then(|c| c.parent.clone()).is_some_and(|c| o.class.is_a(&c.lower)),
        }
    }

    /// Whether `class` is the class `lower` names, or one of its subclasses
    /// or implementers: by name, or through an alias (class_alias) of a
    /// declared class; never autoloaded (instanceof and catch don't).
    fn is_a_named(&self, class: &Class, lower: &[u8]) -> bool {
        class.is_a(lower) || self.class_named(lower).is_some_and(|c| c.lower != lower && class.is_a(&c.lower))
    }

    /// `clone $x`: the copy, and its `__clone`'s frame (if any).
    pub(crate) fn clone_object(&mut self, v: &Value) -> Result<(Rc<Object>, Option<MethodFrame>), Throw> {
        let Value::Object(o) = v.deref() else {
            let given = p8_runtime::args::value_name(&v.deref());
            return Err(self.throw("TypeError", format!("clone(): Argument #1 ($object) must be of type object, {given} given")));
        };
        if o.class.flags & p8_runtime::object::CLASS_UNCLONEABLE != 0 {
            return Err(self.throw("Error", format!("Trying to clone an uncloneable object of class {}", text(&o.class.name))));
        }
        // An initialized lazy proxy: a new proxy for a clone of the instance;
        // the instance's __clone runs, the proxy's doesn't.
        if let Some(inst) = o.proxy_instance() {
            let (inst_copy, hook) = self.clone_object(&Value::Object(inst))?;
            let copy = o.copy();
            copy.lazy.set(p8_runtime::object::LAZY_PROXY);
            *copy.lazy_info.borrow_mut() = Some(Box::new(p8_runtime::object::Lazy {
                initializer: Value::Null,
                options: o.lazy_info.borrow().as_ref().map_or(0, |l| l.options),
                lazy_slots: vec![true; o.class.properties.len()],
                instance: Some(inst_copy),
            }));
            return Ok((copy, hook));
        }
        let copy = o.copy();
        let Some(method) = o.class.methods.get(b"__clone".as_slice()).cloned() else { return Ok((copy, None)) };
        let declaring = self.declaring(&o.class, &method.declaring);
        if !visible(method.visibility, &declaring, self.scope().0.as_deref()) {
            return Err(self.throw(
                "Error",
                format!("Call to {} method {}::__clone() from {}", method.visibility.word(), text(&o.class.name), self.scope_name()),
            ));
        }
        let func = self.code_of(&method);
        Ok((Rc::clone(&copy), Some(MethodFrame::method(func, Some(copy), declaring, Rc::clone(&o.class)))))
    }

    /// Run `copy`'s __clone (`hook`), its readonly properties writable
    /// once more while it runs (zend_objects_clone_members).
    pub(crate) fn run_clone_hook(&mut self, regs: &mut Vec<Value>, copy: &Rc<Object>, hook: MethodFrame) -> Result<(), Throw> {
        let reinit = self.reinit_begin(copy);
        let r = self.run_nested(regs, hook, &mut []);
        if reinit {
            self.reinit.pop();
        }
        r.map(drop)
    }

    /// clone($object, $withProperties) (ZEND_FUNCTION(clone) and
    /// zend_objects_clone_obj_with): the clone, its __clone run, then
    /// each of `with`'s properties assigned to it in the calling scope,
    /// its readonly ones writable once more.
    pub(crate) fn clone_with(&mut self, regs: &mut Vec<Value>, object: &Rc<Object>, with: &PhpArray) -> Result<Value, Throw> {
        // A lazy object is initialized before it is cloned.
        if object.is_uninit_lazy() {
            self.lazy_init(regs, object)?;
        }
        let (copy, hook) = self.clone_object(&Value::Object(Rc::clone(object)))?;
        if let Some(hook) = hook {
            self.run_clone_hook(regs, &copy, hook)?;
        }
        if !with.is_empty() {
            let reinit = self.reinit_begin(&copy);
            let r = self.clone_updates(regs, &copy, with);
            if reinit {
                self.reinit.pop();
            }
            r?;
        }
        Ok(Value::Object(copy))
    }

    fn clone_updates(&mut self, regs: &mut Vec<Value>, copy: &Rc<Object>, with: &PhpArray) -> Result<(), Throw> {
        let target = Value::Object(Rc::clone(copy));
        for (key, value) in with.iter() {
            let value = match value {
                // A reference only the array holds is its value.
                Value::Ref(cell) if Rc::strong_count(cell) == 1 => cell.borrow().clone(),
                Value::Ref(_) => return Err(self.throw("Error", "Cannot assign by reference when cloning with updated properties")),
                v => v.clone(),
            };
            let name = match key.to_value() {
                Value::Int(n) => Value::Str(PhpStr::new(n.to_string().into_bytes())),
                name => name,
            };
            self.assign_prop_value(regs, &target, &name, value)?;
        }
        Ok(())
    }

    /// `$target->name = value` as the running code does it: its set hook,
    /// else __set, else the property itself (zend_std_write_property).
    pub(crate) fn assign_prop_value(&mut self, regs: &mut Vec<Value>, target: &Value, name: &Value, value: Value) -> Result<(), Throw> {
        if let Some(hook) = self.prop_hook(target, name, p8_runtime::object::HOOK_SET) {
            self.run_nested(regs, hook?, &mut [value])?;
            return Ok(());
        }
        if let Some(found) = self.prop_magic(target, name, Magic::Set) {
            let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
            self.run_magic(regs, found, &mut [pname, value])?;
            return Ok(());
        }
        let target = self.lazy_forward(target, name).unwrap_or_else(|| target.clone());
        self.write_prop(&target, name, value)
    }

    /// `o`'s readonly properties made writable once more (pushed on
    /// `reinit`), when it has any.
    fn reinit_begin(&mut self, o: &Rc<Object>) -> bool {
        if !o.class.properties.iter().any(|p| p.readonly) {
            return false;
        }
        let slots = o.class.properties.iter().map(|p| p.readonly).collect();
        self.reinit.push((Rc::clone(o), slots));
        true
    }

    /// Whether readonly `slot` of `object` may be written once more.
    fn reinitable(&self, object: &Object, slot: usize) -> bool {
        self.reinit.iter().rev().any(|(o, slots)| std::ptr::eq(Rc::as_ptr(o), object) && slots.get(slot).copied().unwrap_or(false))
    }

    /// Readonly `slot` of `object` written: not again.
    fn reinit_done(&mut self, object: &Object, slot: usize) {
        for (o, slots) in self.reinit.iter_mut() {
            if std::ptr::eq(Rc::as_ptr(o), object)
                && let Some(s) = slots.get_mut(slot)
            {
                *s = false;
            }
        }
    }
}

/// `(array) $object`: initialized properties under PHP's mangled names
/// ("\0A\0p" private to A, "\0*\0p" protected), then dynamic ones.
/// The order PHP keeps a class's properties in (its properties_info, as
/// zend_do_inheritance and trait binding build it): its `own` as declared,
/// then its parent's that it does not redeclare (not the parent's private
/// ones), then what its traits add.
pub(crate) fn property_order(mut own: Vec<PhpStr>, parent: Option<&Class>, traits: Vec<PhpStr>) -> Vec<PhpStr> {
    if let Some(p) = parent {
        for n in &p.property_order {
            if !own.contains(n) && !declared_private(p, n.as_bytes()) {
                own.push(n.clone());
            }
        }
    }
    for n in traits {
        if !own.contains(&n) {
            own.push(n);
        }
    }
    own
}

/// Whether `class` itself declares property `name` private.
fn declared_private(class: &Class, name: &[u8]) -> bool {
    match class.statics.get(name) {
        Some((_, visibility, declaring)) => *visibility == Visibility::Private && *declaring == class.lower,
        None => class.slots.get(name).is_some_and(|slots| {
            slots.iter().all(|&i| class.properties[i].visibility == Visibility::Private && class.properties[i].declaring == class.lower)
        }),
    }
}

pub(crate) fn object_to_array(o: &Object) -> p8_runtime::array::PhpArray {
    if let Some(props) = o.class.get_properties {
        return props(o);
    }
    let slots = o.slots.borrow();
    let mut out = p8_runtime::array::PhpArray::hashed(slots.len() + o.dynamic.borrow().len());
    for (p, v) in o.class.properties.iter().zip(slots.iter()) {
        if matches!(v, Value::Undef) || p.is_internal() {
            continue;
        }
        // A public property's key is its name itself; the others' are
        // mangled ("\0*\0name", "\0Class\0name"), made in one piece.
        let name = match p.visibility {
            Visibility::Public => p.name.clone(),
            Visibility::Protected => PhpStr::concat_all(&[b"\0*\0", p.name.as_bytes()]),
            Visibility::Private => PhpStr::concat_all(&[b"\0", p.declaring_name.as_bytes(), b"\0", p.name.as_bytes()]),
        };
        out.insert(Key::from_string(name), v.clone());
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
/// A magic property method.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Magic {
    Get,
    Set,
    Isset,
    Unset,
}

impl Magic {
    fn method(self) -> &'static [u8] {
        match self {
            Magic::Get => b"__get",
            Magic::Set => b"__set",
            Magic::Isset => b"__isset",
            Magic::Unset => b"__unset",
        }
    }

    /// A native class's handler method for it.
    pub(crate) fn native(self) -> &'static [u8] {
        match self {
            Magic::Get => b"$*::get",
            Magic::Set => b"$*::set",
            Magic::Isset => b"$*::isset",
            Magic::Unset => b"$*::unset",
        }
    }
}

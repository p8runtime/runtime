//! Call-site and property caches: what each site of a function resolved to
//! in this isolate (a feedback vector, in V8's terms), and the closures that
//! hold them.

use super::*;

/// A function as one context runs it: the shared compiled code, plus what
/// belongs to this context alone: its constants as values, and what each
/// call site and constant fetch resolved to (a feedback vector, in V8's
/// terms). Compiled code is shared by every context; this is not.
pub struct Closure {
    pub(crate) code: Arc<Function>,
    pub(crate) consts: Box<[Value]>,
    pub(crate) sites: Box<[OnceCell<Callee>]>,
    /// Where `$o->name` found its property last, one entry per constant
    /// (the name is one): PHP's runtime cache slot of FETCH_OBJ_R.
    pub(crate) props: Box<[Cell<PropSite>]>,
    /// What `$o->m()` called last, one entry per call site: PHP's cache
    /// slot of INIT_METHOD_CALL.
    pub(crate) methods: Box<[RefCell<Option<MethodSite>>]>,
    /// What `new` of each class the code names made last: the class and
    /// how to make its objects (instances.rs).
    pub(crate) news: Box<[RefCell<crate::instances::NewSite>]>,
    /// The class each named class reference (`C`, and `self`/`parent`
    /// in a class's code) resolved to: a name, once declared, is always
    /// the same class. Held weakly.
    pub(crate) class_refs: Box<[RefCell<std::rc::Weak<Class>>]>,
    /// Each parameter's default when it is a constant value, as a value
    /// (empty when none has one): put in place for an argument not passed
    /// as the frame is entered (`fill_defaults`), as PHP's RECV_INIT does
    /// for a constant default. The compiler leaves no code for a literal.
    pub(crate) defaults: Box<[Option<Value>]>,
}

impl Closure {
    /// A closure of `code` for this request, its constants the isolate's
    /// (made once: `Isolate::thaw`).
    pub(crate) fn new(code: Arc<Function>, isolate: &crate::Isolate) -> Rc<Closure> {
        Closure::with_consts(code, |c| isolate.thaw(c))
    }

    fn with_consts(code: Arc<Function>, thaw: impl Fn(&p8_runtime::frozen::Frozen) -> Value) -> Rc<Closure> {
        let consts = code.consts.iter().map(&thaw).collect();
        let sites = code.sites.iter().map(|_| OnceCell::new()).collect();
        let props = code.consts.iter().map(|_| Cell::new(PropSite::default())).collect();
        let methods = code.sites.iter().map(|_| RefCell::new(None)).collect();
        let news = code.classes.iter().map(|_| RefCell::new(std::rc::Weak::new())).collect();
        let class_refs = code.classes.iter().map(|_| RefCell::new(std::rc::Weak::new())).collect();
        let defaults = if code.params.iter().any(|p| p.default.is_some()) {
            code.params
                .iter()
                .map(|p| {
                    let v = p.default.as_ref().map(&thaw)?;
                    // An int default of a `float` parameter is a float (the
                    // one conversion PHP makes of a default).
                    let float = p.ty.as_ref().is_some_and(|t| t.mask & p8_runtime::types::T_FLOAT != 0 && t.mask & p8_runtime::types::T_INT == 0);
                    Some(match v {
                        Value::Int(i) if float => Value::Float(i as f64),
                        v => v,
                    })
                })
                .collect()
        } else {
            Box::default()
        };
        Rc::new(Closure { code, consts, sites, props, methods, news, class_refs, defaults })
    }

    /// The stand-in for a declared function or method whose closure has not
    /// been made yet (one per isolate, shared by every such slot): empty
    /// code, never run. A slot holds it until the function is first
    /// resolved (`Context::closure_at`).
    pub(crate) fn unbuilt(registry: &Registry) -> Rc<Closure> {
        let code = Function {
            name: p8_runtime::frozen::Text::from(""),
            sym: registry.symbols().intern(b""),
            ret_type: None,
            strict: false,
            classes: Vec::new(),
            class: None,
            is_static: false,
            params: Vec::new(),
            returns_ref: false,
            code: Vec::new(),
            lines: Vec::new(),
            consts: Vec::new(),
            consts_at: 0,
            value_consts: 0,
            nregs: 0,
            cv_names: Vec::new(),
            sites: Vec::new(),
            file: Arc::from(""),
            required: 0,
            line: 0,
            handlers: Vec::new(),
            decls: Vec::new(),
            is_generator: false,
            lambdas: Vec::new(),
            bound: Vec::new(),
            silences: Vec::new(),
            meta: Default::default(),
            partials: Vec::new(),
        };
        Closure::with_consts(Arc::new(code), |c| c.to_value())
    }
}

impl Context<'_> {
    /// Register a declared function's or method's code: its closure is
    /// made when it is first resolved, not now (most of a large
    /// application's functions are never called in a request). Returns
    /// its index in `closures` (and `codes`).
    pub(crate) fn add_code(&mut self, func: Arc<Function>) -> u32 {
        self.codes.push(func);
        self.closures.push(Rc::clone(&self.isolate.unbuilt));
        (self.closures.len() - 1) as u32
    }

    /// The closure at `i`, made now if this is its first use. Every way
    /// code is reached for running (a call site's resolution, a method's
    /// frame, a callable) goes through here, so `fast` only ever finds
    /// made ones at the indices its call sites cached.
    pub(crate) fn closure_at(&mut self, i: u32) -> Rc<Closure> {
        let slot = &mut self.closures[i as usize];
        if Rc::ptr_eq(slot, &self.isolate.unbuilt) {
            *slot = Closure::new(Arc::clone(&self.codes[i as usize]), self.isolate);
        }
        Rc::clone(slot)
    }
}

/// A property read's cache entry: objects of `class` (its address), read
/// from code in `scope` (0: any, for a public property), keep it at `slot`.
/// What it skips on a hit is the name lookup and the visibility check, as
/// PHP's does (zend_fetch_property_address / FETCH_OBJ_R's CACHED_PTR).
#[derive(Clone, Copy, Default)]
pub(crate) struct PropSite {
    pub(crate) class: usize,
    pub(crate) scope: usize,
    pub(crate) slot: u32,
}

/// A method call's cache entry: on an object of `class` (its address),
/// from code in `scope`, the call runs this code with this declaring class.
/// Held weakly, so the cache keeps no function or class alive (a method
/// calling itself would otherwise hold its own code).
pub(crate) struct MethodSite {
    pub(crate) class: usize,
    pub(crate) scope: usize,
    pub(crate) code: MethodCode,
    pub(crate) declaring: std::rc::Weak<Class>,
    pub(crate) is_static: bool,
}

pub(crate) enum MethodCode {
    User(std::rc::Weak<Closure>),
    /// A builtin class's method, its name, and which of its first 64
    /// parameters its stub signature takes by reference (bit n: parameter
    /// n; looked up once, when the site remembers it).
    Native(p8_runtime::object::Native, PhpStr, u64),
}

/// The parameters (bit n: parameter n, the first 64) that builtin method
/// `name` declared by `declaring` takes by reference, from its stub
/// signature (as `method_takes_ref` finds it).
fn native_refs(declaring: &Class, name: &PhpStr) -> u64 {
    let stub = format!("{}::{}", String::from_utf8_lossy(declaring.name.as_bytes()), String::from_utf8_lossy(name.as_bytes()));
    p8_runtime::stubs::lookup(stub.as_bytes()).map_or(0, |s| s.params.iter().take(64).enumerate().filter(|(_, p)| p.by_ref).fold(0, |m, (i, _)| m | 1 << i))
}

/// A property read the cache answers: the same class, a scope that may
/// see it, an object not lazy, and a value there (an unset one takes the
/// full path: its warning, __get).
#[inline]
pub(crate) fn cached_prop(site: PropSite, o: &Rc<p8_runtime::object::Object>, scope: usize) -> Option<Value> {
    if site.class == 0 || site.class != Rc::as_ptr(&o.class) as usize || (site.scope != 0 && site.scope != scope) || o.lazy.get() != 0 {
        return None;
    }
    match o.slots.borrow().get(site.slot as usize)? {
        Value::Undef => None,
        Value::Ref(cell) => Some(cell.borrow().clone()),
        v => Some(v.clone()),
    }
}

/// The declared slot a property write the cache answers goes to: as for
/// a read; an unset slot only in a class without __set and its kin (writing
/// one there may be theirs).
#[inline]
pub(crate) fn cached_slot(site: PropSite, o: &Rc<p8_runtime::object::Object>, scope: usize) -> Option<usize> {
    if site.class == 0 || site.class != Rc::as_ptr(&o.class) as usize || (site.scope != 0 && site.scope != scope) || o.lazy.get() != 0 {
        return None;
    }
    let slot = site.slot as usize;
    let unset = matches!(o.slots.borrow().get(slot)?, Value::Undef);
    (!unset || o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0).then_some(slot)
}

impl Context<'_> {
    /// `class_of` through the closure's cache: a named class reference
    /// is looked up by name once.
    #[inline]
    pub(crate) fn class_at(&mut self, clo: &Closure, i: u16, w: &[Value]) -> Result<Rc<Class>, Throw> {
        let named = matches!(clo.code.classes[i as usize], p8_bytecode::ClassRef::Named(..) | p8_bytecode::ClassRef::Forward(..));
        if named && let Some(c) = clo.class_refs[i as usize].borrow().upgrade() {
            return Ok(c);
        }
        let c = self.class_of(&clo.code, i, w)?;
        if named {
            *clo.class_refs[i as usize].borrow_mut() = Rc::downgrade(&c);
        }
        Ok(c)
    }

    /// Whether argument `n` of the call at `site` is passed by reference,
    /// when the site's cache knows the method it calls on `class` from
    /// this scope (the call made last time): no lookup by name.
    #[inline]
    pub(crate) fn cached_takes_ref(&self, site: &RefCell<Option<MethodSite>>, class: &Rc<Class>, n: usize) -> Option<bool> {
        let site = site.borrow();
        let s = site.as_ref()?;
        if s.class != Rc::as_ptr(class) as usize || s.scope != self.scope_addr() {
            return None;
        }
        match &s.code {
            MethodCode::User(c) => Some(c.upgrade()?.code.param_by_ref(n)),
            MethodCode::Native(_, _, refs) if n < 64 => Some(refs & 1 << n != 0),
            MethodCode::Native(..) => None,
        }
    }

    /// The running code's class (its address; 0 outside one): what a
    /// private or protected property's cache entry is valid for.
    #[inline]
    pub(crate) fn scope_addr(&self) -> usize {
        self.frames.last().and_then(|f| f.scope.as_ref()).map_or(0, |c| Rc::as_ptr(c) as usize)
    }

    /// The frame a cached method call makes: the same class from the same
    /// scope, and its code and declaring class still there.
    #[inline]
    pub(crate) fn cached_method(&self, site: &RefCell<Option<MethodSite>>, o: &Rc<p8_runtime::object::Object>) -> Option<crate::classes::MethodFrame> {
        let site = site.borrow();
        let s = site.as_ref()?;
        if s.class != Rc::as_ptr(&o.class) as usize || s.scope != self.scope_addr() {
            return None;
        }
        let declaring = s.declaring.upgrade()?;
        let func = match &s.code {
            MethodCode::User(c) => crate::classes::Code::User(c.upgrade()?),
            MethodCode::Native(n, name, _) => crate::classes::Code::Native(*n, name.clone()),
        };
        let this = (!s.is_static).then(|| Rc::clone(o));
        Some(crate::classes::MethodFrame::method(func, this, declaring, Rc::clone(&o.class)))
    }

    /// After a full method lookup: remember an ordinary call (the object's
    /// own class's method, not __call or a forwarded inner iterator's).
    pub(crate) fn remember_method(&self, site: &RefCell<Option<MethodSite>>, o: &Rc<p8_runtime::object::Object>, m: &crate::classes::MethodFrame) {
        let own_class = m.called.as_ref().is_some_and(|c| Rc::ptr_eq(c, &o.class));
        let on_this = m.this.as_ref().is_none_or(|t| Rc::ptr_eq(t, o));
        if m.trampoline.is_some() || !own_class || !on_this || !m.captured.is_empty() {
            return;
        }
        let Some(declaring) = &m.scope else { return };
        let code = match &m.func {
            crate::classes::Code::User(c) => MethodCode::User(Rc::downgrade(c)),
            crate::classes::Code::Native(n, name) => MethodCode::Native(*n, name.clone(), native_refs(declaring, name)),
        };
        *site.borrow_mut() = Some(MethodSite {
            class: Rc::as_ptr(&o.class) as usize,
            scope: self.scope_addr(),
            code,
            declaring: Rc::downgrade(declaring),
            is_static: m.this.is_none(),
        });
    }

    /// The frame of a static-syntax call (`C::m()`, `self::m()`,
    /// `parent::m()`, `static::m()`) the site's cache answers: the method
    /// `static_frame` found for this target class from this scope, with
    /// `$this` and the called class worked out as it does.
    pub(crate) fn cached_static(&self, site: &RefCell<Option<MethodSite>>, target: &Rc<p8_runtime::object::Class>, func: &Function, class: u16) -> Option<crate::classes::MethodFrame> {
        let site = site.borrow();
        let s = site.as_ref()?;
        if s.class != Rc::as_ptr(target) as usize || s.scope != self.scope_addr() {
            return None;
        }
        let declaring = s.declaring.upgrade()?;
        let code = match &s.code {
            MethodCode::User(c) => crate::classes::Code::User(c.upgrade()?),
            MethodCode::Native(n, name, _) => crate::classes::Code::Native(*n, name.clone()),
        };
        let frame = self.frames.last();
        if s.is_static {
            let forward = matches!(func.classes[class as usize], p8_bytecode::ClassRef::Forward(..) | p8_bytecode::ClassRef::Static | p8_bytecode::ClassRef::Scope | p8_bytecode::ClassRef::ParentScope);
            let called = match frame.and_then(|f| f.called.as_ref()) {
                Some(c) if forward && (extends(c, target) || c.is_a(&target.lower)) => Rc::clone(c),
                _ => Rc::clone(target),
            };
            return Some(crate::classes::MethodFrame::method(code, None, declaring, called));
        }
        // An instance method: `$this` of the target's kind, or `static_frame`
        // raises the error.
        let this = frame.and_then(|f| f.this.clone()).filter(|t| extends(&t.class, target) || t.class.is_a(&target.lower))?;
        let called = Rc::clone(&this.class);
        Some(crate::classes::MethodFrame::method(code, Some(this), declaring, called))
    }

    /// After `static_frame` resolved a static-syntax call: remember the
    /// method for this target class and scope (not a __call/__callStatic
    /// trampoline).
    pub(crate) fn remember_static(&self, site: &RefCell<Option<MethodSite>>, target: &Rc<p8_runtime::object::Class>, m: &crate::classes::MethodFrame) {
        if m.trampoline.is_some() || !m.captured.is_empty() {
            return;
        }
        let Some(declaring) = &m.scope else { return };
        let code = match &m.func {
            crate::classes::Code::User(c) => MethodCode::User(Rc::downgrade(c)),
            crate::classes::Code::Native(n, name) => MethodCode::Native(*n, name.clone(), native_refs(declaring, name)),
        };
        *site.borrow_mut() = Some(MethodSite {
            class: Rc::as_ptr(target) as usize,
            scope: self.scope_addr(),
            code,
            declaring: Rc::downgrade(declaring),
            is_static: m.this.is_none(),
        });
    }

    /// After a full property read: remember where a declared, visible,
    /// hookless property of this class is, for the next read here.
    pub(crate) fn remember_prop(&self, site: &Cell<PropSite>, o: &Rc<p8_runtime::object::Object>, name: &Value) {
        let Value::Str(name) = name else { return };
        let Ok(Some((slot, prop))) = self.prop_slot(o, name.as_bytes()) else { return };
        if prop.hooks != 0 {
            return;
        }
        // Any scope may use a public property's entry, unless a parent's
        // private one of the same name is there too (which one a scope
        // sees then depends on it).
        let shared = o.class.slots.get(name.as_bytes()).is_some_and(|slots| slots.len() == 1);
        let scope = if prop.visibility == p8_runtime::object::Visibility::Public && shared { 0 } else { self.scope_addr() };
        site.set(PropSite { class: Rc::as_ptr(&o.class) as usize, scope, slot: slot as u32 });
    }
}

/// Whether `class` is `target` or a subclass of it, by address (the
/// common case of `is_a`, with no names compared).
#[inline]
fn extends(class: &Rc<Class>, target: &Rc<Class>) -> bool {
    let mut c = Some(class);
    while let Some(k) = c {
        if Rc::ptr_eq(k, target) {
            return true;
        }
        c = k.parent.as_ref();
    }
    false
}

/// What a call site or constant fetch resolved to, in one context.
#[derive(Clone)]
pub(crate) enum Callee {
    /// A user function: its closure's index in the context's table (an
    /// index, not a reference: a recursive function's call site names the
    /// function itself, and a reference would be a cycle that is never
    /// freed).
    User(u32),
    Builtin(usize),
    Constant(Value),
}

#[cfg(test)]
mod tests {
    use crate::tests::{Sink, compile};
    use crate::{Isolate, Registry};
    use std::rc::Rc;
    use std::sync::Arc;

    /// A declared function or method gets its closure when something first
    /// runs it (a call site, a method call, a callable, recursion), never
    /// when it is declared: what is only declared keeps the shared
    /// stand-in. (function_exists, method_exists and reflection read the
    /// code, not the closure: the kit's lazy-calls covers them.)
    #[test]
    fn closures_are_made_on_first_use() {
        let registry = Arc::new(Registry::default());
        let source = "<?php
            function called($n) { return $n < 1 ? 0 : called($n - 1) + 1; }
            function never_called() { return [1, 2, 3]; }
            function by_name() { return 'by name'; }
            class A {
                public function used() { return 'used'; }
                public function unused() { return 'unused'; }
                public static function as_callable() { return 'callable'; }
            }
            $r = called(3);
            $o = new A;
            $u = $o->used();
            $f = 'by_name';
            $b = $f();
            $c = ['A', 'as_callable'];
            $s = $c();
            if ($r !== 3 || $u !== 'used' || $b !== 'by name' || $s !== 'callable') { exit(3); }";
        let program = compile(source, &registry);
        let mut isolate = Isolate::new(Arc::clone(&registry));
        let mut host = Sink;
        let mut cx = isolate.new_context(&mut host);
        for f in &program.functions {
            cx.declare(Arc::clone(f)).expect("declares");
        }
        cx.declare_classes(&program.classes);
        assert_eq!(cx.run_main(Arc::clone(&program.main)), 0);
        let made = |name: &str| {
            let i = cx.codes.iter().position(|c| c.name.as_bytes().eq_ignore_ascii_case(name.as_bytes())).unwrap_or_else(|| panic!("{name} declared"));
            !Rc::ptr_eq(&cx.closures[i], &cx.isolate.unbuilt)
        };
        for name in ["called", "by_name", "used", "as_callable"] {
            assert!(made(name), "{name} ran, so it has its closure");
        }
        for name in ["never_called", "unused"] {
            assert!(!made(name), "{name} never ran: no closure made");
        }
    }
}

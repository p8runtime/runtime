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
}

impl Closure {
    pub(crate) fn new(code: Arc<Function>) -> Rc<Closure> {
        let consts = code.consts.iter().map(|c| c.to_value()).collect();
        let sites = code.sites.iter().map(|_| OnceCell::new()).collect();
        let props = code.consts.iter().map(|_| Cell::new(PropSite::default())).collect();
        let methods = code.sites.iter().map(|_| RefCell::new(None)).collect();
        Rc::new(Closure { code, consts, sites, props, methods })
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
    Native(p8_runtime::object::Native, PhpStr),
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
            MethodCode::Native(n, name) => crate::classes::Code::Native(*n, name.clone()),
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
            crate::classes::Code::Native(n, name) => MethodCode::Native(*n, name.clone()),
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
            MethodCode::Native(n, name) => crate::classes::Code::Native(*n, name.clone()),
        };
        let frame = self.frames.last();
        if s.is_static {
            let forward = matches!(func.classes[class as usize], p8_bytecode::ClassRef::Forward(..) | p8_bytecode::ClassRef::Static | p8_bytecode::ClassRef::Scope | p8_bytecode::ClassRef::ParentScope);
            let called = match frame.and_then(|f| f.called.as_ref()) {
                Some(c) if forward && c.is_a(&target.lower) => Rc::clone(c),
                _ => Rc::clone(target),
            };
            return Some(crate::classes::MethodFrame::method(code, None, declaring, called));
        }
        // An instance method: `$this` of the target's kind, or `static_frame`
        // raises the error.
        let this = frame.and_then(|f| f.this.clone()).filter(|t| t.class.is_a(&target.lower))?;
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
            crate::classes::Code::Native(n, name) => MethodCode::Native(*n, name.clone()),
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

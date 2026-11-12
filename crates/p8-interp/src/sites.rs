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
    pub(crate) methods: Box<[MethodSlot]>,
    /// Where this closure is in the context's `closures`, when it is a
    /// declared function's or method's (`closure_at` made it); u32::MAX
    /// for any other (a lambda, the main code).
    pub(crate) index: Cell<u32>,
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
    /// The class each declared type last accepted an object of (`quick`'s
    /// class test, an `instanceof` walk by name, made once per class):
    /// one per parameter, then the return type's; empty when no type
    /// names a class.
    pub(crate) type_memo: Box<[ClassMemo]>,
    /// `C::NAME`, a public constant that is not an enum case (or
    /// `C::class`): its value, as last read, by the name's constant (one
    /// entry per constant, as `props`), with the class it was read for
    /// (`ConstSite::key`; another class reads past it, and its read
    /// replaces the entry). A class's constants never change, and a name
    /// is always the same class (`class_refs`): PHP's runtime cache slot of
    /// FETCH_CLASS_CONSTANT, polymorphic as PHP's is for `static::`.
    pub(crate) class_consts: Box<[RefCell<ConstSite>]>,
    /// `C::$name`: the property's cell, as last read, by the name's
    /// constant (as `class_consts`), with the class and the scope it was
    /// read for (its visibility was checked from that scope): PHP's
    /// runtime cache slot of FETCH_STATIC_PROP. A class's static
    /// properties are its cells for good (a reference bound into one is a
    /// value in its cell).
    pub(crate) statics: Box<[RefCell<StaticSite>]>,
    /// `global $name` in a function: where the global's cell was found
    /// last, by the name's constant (as `props`); made at the first
    /// `global` the closure runs (none for the code that has none). PHP's
    /// runtime cache slot of ZEND_BIND_GLOBAL.
    pub(crate) globals: OnceCell<Box<[Cell<GlobalSite>]>>,
}

/// A `global $name` cache entry (`Closure::globals`): the global's index
/// in `Context::globals` when the main script has no variable of that
/// name. Valid only while the main frame runs the code it was found for
/// (`main`, its address: which names are the main script's variables
/// depends on it) and while the entry at `index` has that name (a removal
/// moves the entries after it: the name is checked on use, as PHP checks
/// the bucket's key). `main` 0: empty.
#[derive(Clone, Copy, Default)]
pub(crate) struct GlobalSite {
    pub(crate) main: usize,
    pub(crate) index: usize,
}

/// A static property's cache entry (`Closure::statics`).
#[derive(Default)]
pub(crate) struct StaticSite {
    /// The class, as `ConstSite::key` (0: empty), and the scope's address.
    pub(crate) key: usize,
    pub(crate) scope: usize,
    /// (Held, never read: see `ConstSite::_keep`.)
    pub(crate) _keep: std::rc::Weak<Class>,
    pub(crate) cell: Option<Rc<RefCell<Value>>>,
}

/// A class constant's cache entry (`Closure::class_consts`).
#[derive(Default)]
pub(crate) struct ConstSite {
    /// The class it was read for: a named class reference, its index + 1;
    /// the called class (`static::`) or a trait's class, its address,
    /// which `_keep` keeps from being reused by another class. 0: empty.
    pub(crate) key: usize,
    /// (Held, never read: a weak handle keeps the class's allocation, so
    /// its address, from being reused while the entry names it.)
    pub(crate) _keep: std::rc::Weak<Class>,
    /// The scope it was read from (its class's address) for a private or
    /// protected constant, whose access depends on it; 0 for a public one
    /// (any scope).
    pub(crate) scope: usize,
    pub(crate) value: Value,
}

/// A class as a cache entry keys it (`ConstSite::key`), with the handle
/// that keeps the key's address its own.
pub(crate) type ClassKey = (usize, std::rc::Weak<Class>);

/// A class a declared type accepted (`Closure::type_memo`): its address,
/// compared; and a weak handle, which keeps the address from being reused
/// by another class while the memo holds it.
#[derive(Default)]
pub(crate) struct ClassMemo {
    addr: Cell<usize>,
    keep: Cell<std::rc::Weak<Class>>,
}

impl ClassMemo {
    /// `ty.quick(v)`, an object's class test answered by the memo when it
    /// is the class accepted last (an object's verdict depends on its
    /// class alone).
    #[inline(always)]
    pub(crate) fn quick(memo: Option<&ClassMemo>, ty: &p8_runtime::types::TypeDecl, v: &Value) -> bool {
        match (v.as_object(), memo) {
            (Some(o), Some(m)) => m.addr.get() == Rc::as_ptr(&o.class) as usize || m.learn(ty, o),
            _ => ty.quick(v),
        }
    }

    #[inline(never)]
    fn learn(&self, ty: &p8_runtime::types::TypeDecl, o: &Rc<Object>) -> bool {
        if !ty.quick_object(o) {
            return false;
        }
        self.addr.set(Rc::as_ptr(&o.class) as usize);
        self.keep.set(Rc::downgrade(&o.class));
        true
    }
}

impl Closure {
    /// A closure of `code` for this request, its constants the isolate's
    /// (made once: `Isolate::thaw`).
    pub(crate) fn new(code: Arc<Function>, isolate: &crate::Isolate) -> Rc<Closure> {
        let program = !code.eval;
        Closure::with_consts(code, |c| isolate.thaw(c, program))
    }

    fn with_consts(code: Arc<Function>, thaw: impl Fn(&p8_runtime::frozen::Frozen) -> Value) -> Rc<Closure> {
        let consts = code.consts.iter().map(&thaw).collect();
        let sites = code.sites.iter().map(|_| OnceCell::new()).collect();
        let props = code.consts.iter().map(|_| Cell::new(PropSite::default())).collect();
        let methods = code.sites.iter().map(|_| MethodSlot::default()).collect();
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
                    Some(match v.as_int() {
                        Some(i) if float => Value::float(i as f64),
                        _ => v,
                    })
                })
                .collect()
        } else {
            Box::default()
        };
        let names_class = |t: &Option<p8_runtime::types::TypeDecl>| t.as_ref().is_some_and(|t| !t.lower.is_empty());
        let type_memo = if code.params.iter().any(|p| names_class(&p.ty)) || names_class(&code.ret_type) {
            (0..=code.params.len()).map(|_| ClassMemo::default()).collect()
        } else {
            Box::default()
        };
        let class_consts = code.consts.iter().map(|_| RefCell::default()).collect();
        let statics = code.consts.iter().map(|_| RefCell::default()).collect();
        Rc::new(Closure { code, consts, sites, props, methods, index: Cell::new(u32::MAX), news, class_refs, defaults, type_memo, class_consts, statics, globals: OnceCell::new() })
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
            insns: Vec::new(),
            lines: Vec::new(),
            consts: Vec::new(),
            ncv: 0,
            nregs: 0,
            frees_temps: true,
            cv_names: Vec::new(),
            sites: Vec::new(),
            file: Arc::from(""),
            required: 0,
            line: 0,
            handlers: Vec::new(),
            decls: Vec::new(),
            is_generator: false,
            eval: false,
            own_location: false,
            lambdas: Vec::new(),
            bound: Vec::new(),
            silences: Vec::new(),
            arg_vars: Vec::new(),
            meta: Default::default(),
            partials: Vec::new(),
            compile_diags: Vec::new(),
            native: None,
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
            slot.index.set(i);
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

/// What `$o->m()` at one site ran last when that was a PHP method called
/// on an object: plain numbers, read by the loop's arms (`MethodCall`,
/// `SendVarMethod`) with no borrow and nothing to upgrade, as a function
/// call site keeps its callee's index (`Callee::User`). An answer for one
/// class and scope never changes (a class's methods are fixed once it is
/// declared, and classes live as long as their context), so an entry is
/// only ever replaced, never cleared.
#[derive(Clone, Copy, Default)]
pub(crate) struct UserMethod {
    /// The receiver's class (its address; 0: nothing remembered) and the
    /// calling code's class (its address; 0 outside one).
    pub(crate) class: usize,
    pub(crate) scope: usize,
    /// The method's code: its index in the context's `closures`.
    pub(crate) code: u32,
    /// The class that declares the method: this many parents up from the
    /// receiver's class (0: that class itself).
    pub(crate) up: u32,
    /// Bit n: parameter n is taken by reference (`by_ref`).
    pub(crate) refs: u64,
}

impl UserMethod {
    /// The most parameters a method may declare to be remembered here:
    /// bit 63 then answers for every argument from the 64th on (they are
    /// all past the parameters: the variadic one's mode).
    const PARAMS: usize = 63;

    /// Whether argument `n` goes by reference.
    #[inline(always)]
    pub(crate) fn by_ref(self, n: usize) -> bool {
        self.refs >> n.min(Self::PARAMS) & 1 != 0
    }

    /// `code`'s parameters as `refs` has them; None for more than `PARAMS`.
    fn refs_of(code: &Function) -> Option<u64> {
        let n = code.params.len();
        if n > Self::PARAMS {
            return None;
        }
        let rest = if code.params.last().is_some_and(|p| p.variadic && p.by_ref) { !0u64 << n } else { 0 };
        Some(code.params.iter().enumerate().fold(rest, |m, (i, p)| if p.by_ref { m | 1 << i } else { m }))
    }
}

/// A method call site's cache: `user` for the loop's arms, and the full
/// entry (any method, native ones and static-syntax calls too) for
/// everything else, which this dereferences to.
#[derive(Default)]
pub(crate) struct MethodSlot {
    pub(crate) user: Cell<UserMethod>,
    site: RefCell<Option<MethodSite>>,
}

impl std::ops::Deref for MethodSlot {
    type Target = RefCell<Option<MethodSite>>;

    fn deref(&self) -> &RefCell<Option<MethodSite>> {
        &self.site
    }
}

pub(crate) enum MethodCode {
    User(std::rc::Weak<Closure>),
    /// A builtin class's method, its name, and which of its first 64
    /// parameters its stub signature takes by reference (bit n: parameter
    /// n; looked up once, the first time a send asks: most calls never
    /// do, and a site that misses remembers its method again each time).
    Native(p8_runtime::object::Native, PhpStr, Cell<Option<u64>>),
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
/// full path: its warning, __get). Inline: the loop's property reads are
/// its most frequent instructions after moves and jumps, and a call
/// returned the value through memory.
#[inline(always)]
pub(crate) fn cached_prop(site: PropSite, o: &Rc<p8_runtime::object::Object>, scope: usize) -> Option<Value> {
    if site.class == 0 || site.class != Rc::as_ptr(&o.class) as usize || (site.scope != 0 && site.scope != scope) || o.lazy.get() != 0 {
        return None;
    }
    p8_runtime::object::Slot::peek(o.slots().get(site.slot as usize)?, |v| {
        if v.is_undef() {
            None
        } else if let Some(cell) = v.as_reference() {
            Some(cell.borrow().clone())
        } else {
            Some(v.clone())
        }
    })
}

/// The cache entry for property `name` of `o`'s class at `slot` (`prop`,
/// found visible from code in `scope`, its address): none for a hooked
/// one. Any scope may use a public property's entry, unless a parent's
/// private one of the same name is there too (which one a scope sees then
/// depends on it). `remember_prop` and the loop's refill share it.
pub(crate) fn prop_entry(o: &p8_runtime::object::Object, name: &[u8], slot: usize, prop: &p8_runtime::object::Property, scope: usize) -> Option<PropSite> {
    if prop.hooks != 0 {
        return None;
    }
    let shared = o.class.slots.get(name).is_some_and(|slots| slots.len() == 1);
    let scope = if prop.visibility == p8_runtime::object::Visibility::Public && shared { 0 } else { scope };
    Some(PropSite { class: Rc::as_ptr(&o.class) as usize, scope, slot: slot as u32 })
}

/// The declared slot of property `name` of `o` that code in `scope` sees
/// (`prop_slot`'s lookup and visibility test, the declaring class found
/// in the object's chain), from the class alone: None when it cannot say
/// (no such declared property, one this scope may not see: the full path,
/// with its error or magic).
pub(crate) fn visible_slot<'o>(o: &'o p8_runtime::object::Object, name: &[u8], scope: Option<&Rc<Class>>) -> Option<(usize, &'o p8_runtime::object::Property)> {
    let slot = o.class.slot(name, scope.map(|s| s.lower.as_slice()))?;
    let prop = &o.class.properties[slot];
    let mut c = Some(&o.class);
    let declaring = loop {
        let k = c?;
        if k.lower == prop.declaring {
            break k;
        }
        c = k.parent.as_ref();
    };
    p8_runtime::object::visible(prop.visibility, declaring, scope.map(|s| &**s)).then_some((slot, prop))
}

/// The entry a property access with the name in a register (`$o->$name`)
/// would have, made for this access alone (`visible_slot`, `prop_entry`):
/// the loop's dynamic-name arms read and write through it as the cached
/// ones do. None when the class alone cannot say.
#[inline(never)]
pub(crate) fn dyn_entry(o: &p8_runtime::object::Object, name: &Value, scope: Option<&Rc<Class>>) -> Option<PropSite> {
    let name = name.as_str()?;
    let (slot, prop) = visible_slot(o, name.as_bytes(), scope)?;
    prop_entry(o, name.as_bytes(), slot, prop, scope.map_or(0, |c| Rc::as_ptr(c) as usize))
}

/// A read of property `name` that is no declared one of `o`'s class: the
/// object's dynamic property of that name (a stdClass's, one added at run
/// time), when the class has no __get and its kin and the object is not
/// lazy. None for the rest (unset: the warning; magic: the full path).
#[cold]
#[inline(never)]
pub(crate) fn dynamic_prop(o: &p8_runtime::object::Object, name: &Value) -> Option<Value> {
    if o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS != 0 || o.lazy.get() != 0 {
        return None;
    }
    let name = name.as_str()?;
    if o.class.properties.iter().any(|p| p.name.as_bytes() == name.as_bytes()) {
        return None;
    }
    let v = o.dynamic().get_str(name)?.deref();
    (!v.is_undef()).then_some(v)
}

/// `refill_prop` when the entry does not answer for `o`'s class and this
/// scope, for the hand arms that go on to their own path test.
#[inline(always)]
pub(crate) fn refill_if_missed(site: &Cell<PropSite>, o: &p8_runtime::object::Object, name: &Value, scope: Option<&Rc<Class>>) {
    let e = site.get();
    if e.class != Rc::as_ptr(&o.class) as usize || (e.scope != 0 && e.scope != scope.map_or(0, |c| Rc::as_ptr(c) as usize)) {
        let _ = refill_prop(site, o, name, scope);
    }
}

/// A property access's cache entry `site` that does not answer for `o`'s
/// class or this scope (a method of a base class running on objects of
/// several subclasses): filled again for this class, from the class
/// alone (`prop_slot`'s lookup and visibility test, the declaring class
/// found in the object's chain), as the full path would fill it. False
/// when the entry answers already (the miss is something else: an unset
/// slot, a lazy object), or when the class alone cannot say (no such
/// declared property, a property this scope may not see: the full path,
/// with its error or magic).
#[cold]
#[inline(never)]
pub(crate) fn refill_prop(site: &Cell<PropSite>, o: &p8_runtime::object::Object, name: &Value, scope: Option<&Rc<Class>>) -> bool {
    let scope_addr = scope.map_or(0, |c| Rc::as_ptr(c) as usize);
    let e = site.get();
    if e.class == Rc::as_ptr(&o.class) as usize && (e.scope == 0 || e.scope == scope_addr) {
        return false;
    }
    let Some(name) = name.as_str() else { return false };
    let name = name.as_bytes();
    let Some((slot, prop)) = visible_slot(o, name, scope) else { return false };
    match prop_entry(o, name, slot, prop, scope_addr) {
        Some(entry) => {
            site.set(entry);
            true
        }
        None => false,
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
    let unset = p8_runtime::object::Slot::peek(o.slots().get(slot)?, Value::is_undef);
    (!unset || o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0).then_some(slot)
}

/// The declared slot a property write the cache answers goes to when the
/// write is a plain store (`write_cached`'s test): `cached_slot`'s
/// conditions but the slot's own state (the caller looks at the slot
/// once, to write it), a defined value that is not a reference, and a
/// property untyped or plainly of its type. A property with a set
/// visibility only from code of its declaring class (`may_set`'s first
/// test, which every set visibility passes); a readonly one the same, and
/// only for its first write (`Plain::Unset`: the caller tests the slot is
/// unset) outside any clone's re-initialization (`reinit` empty):
/// `write_slot`'s rules where they pass with nothing to report. Everything
/// else is the full write's.
#[inline(always)]
pub(crate) fn plain_slot(site: PropSite, o: &p8_runtime::object::Object, scope: (usize, Option<&Class>), v: &Value, reinit: bool) -> Option<(usize, Plain)> {
    if site.class == 0 || site.class != Rc::as_ptr(&o.class) as usize || (site.scope != 0 && site.scope != scope.0) || o.lazy.get() != 0 {
        return None;
    }
    if v.is_undef() || v.is_reference() {
        return None;
    }
    let slot = site.slot as usize;
    let prop = o.class.properties.get(slot)?;
    let mut plain = Plain::Any;
    if prop.readonly || prop.set_visibility.is_some() {
        plain = guarded_write(prop, scope.1, reinit)?;
    }
    if prop.ty_decl.as_ref().is_some_and(|t| !t.quick(v)) {
        return None;
    }
    Some((slot, plain))
}

/// What a plain write to a slot needs of the slot's state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plain {
    Any,
    /// A readonly property's first write: the slot must be unset.
    Unset,
}

/// `plain_slot`'s test of a readonly or set-visibility property: written
/// from its declaring class (and a readonly one with no clone being
/// re-initialized). Out of line: the common property has neither.
#[inline(never)]
fn guarded_write(prop: &p8_runtime::object::Property, scope: Option<&Class>, reinit: bool) -> Option<Plain> {
    let scope = scope?;
    if scope.lower != prop.declaring {
        return None;
    }
    if prop.readonly {
        return (!reinit).then_some(Plain::Unset);
    }
    Some(Plain::Any)
}

impl Context<'_> {
    /// The cell of static property `C::$name` for an instruction whose
    /// name is the constant `name_reg` names: the cell that constant's
    /// cache entry (`Closure::statics`, FETCH_STATIC_PROP's) holds for this
    /// class and scope, as PHP's runtime cache slot of
    /// ASSIGN_STATIC_PROP and FETCH_STATIC_PROP_W is; else `static_cell`'s,
    /// kept there. The class is not resolved for a hit: a named class
    /// reference is its index, `static` and `self` the frame's classes.
    pub(crate) fn static_cell_at(&mut self, clo: &Closure, class: u16, name_reg: Reg, name: &Value, w: &[Value]) -> Result<Rc<RefCell<Value>>, Throw> {
        let site = const_index(name_reg).and_then(|i| clo.statics.get(i));
        if let Some(site) = site
            && let Some(frame) = self.frames.last()
        {
            let key = match clo.code.classes[class as usize] {
                p8_bytecode::ClassRef::Named(..) | p8_bytecode::ClassRef::Forward(..) => Some(class as usize + 1),
                // As `class_key` (through `class_at` and `scope`): the called
                // class, `$this`'s class in a method entered with one.
                p8_bytecode::ClassRef::Static => frame.called_class().map(|c| Rc::as_ptr(c) as usize),
                p8_bytecode::ClassRef::Scope => frame.scope.as_ref().map(|c| Rc::as_ptr(c) as usize),
                p8_bytecode::ClassRef::ParentScope | p8_bytecode::ClassRef::Dynamic(_) => None,
            };
            let s = site.borrow();
            if let Some(cell) = &s.cell
                && key == Some(s.key)
                && s.scope == self.scope_addr()
            {
                return Ok(Rc::clone(cell));
            }
        }
        let cell = self.static_cell(&clo.code, class, name, w)?;
        if let Some(site) = site
            && let Some((_, (key, keep))) = self.class_key(clo, class, w)
        {
            *site.borrow_mut() = StaticSite { key, scope: self.scope_addr(), _keep: keep, cell: Some(Rc::clone(&cell)) };
        }
        Ok(cell)
    }

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
            MethodCode::Native(_, name, refs) if n < 64 => {
                let bits = match refs.get() {
                    Some(bits) => bits,
                    None => {
                        let bits = native_refs(&*s.declaring.upgrade()?, name);
                        refs.set(Some(bits));
                        bits
                    }
                };
                Some(bits & 1 << n != 0)
            }
            MethodCode::Native(..) => None,
        }
    }

    /// `cached_takes_ref` out of line: for the loop's arms (MethodArgByRef,
    /// SendVarMethod past its user-method test), where an inlined copy
    /// would hold registers through the loop's other arms.
    #[inline(never)]
    pub(crate) fn cached_takes_ref_out(&self, site: &RefCell<Option<MethodSite>>, class: &Rc<Class>, n: usize) -> Option<bool> {
        self.cached_takes_ref(site, class, n)
    }

    /// The same for a static-syntax call (`C::m($a['k'])`, StaticArgByRef):
    /// the class known without a lookup (`static_target`), the method the
    /// site's cache has for it. Out of line, as `cached_takes_ref_out`.
    #[inline(never)]
    pub(crate) fn static_takes_ref_out(&self, clo: &Closure, class: u16, method: usize, n: usize) -> Option<bool> {
        let target = self.static_target(clo, class)?;
        self.cached_takes_ref(&clo.methods[method], &target, n)
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
        if m.trampoline.is_some() || !own_class || !on_this || m.closure.is_some() {
            return;
        }
        let Some(declaring) = &m.scope else { return };
        let code = match &m.func {
            crate::classes::Code::User(c) => MethodCode::User(Rc::downgrade(c)),
            crate::classes::Code::Native(n, name) => MethodCode::Native(*n, name.clone(), Cell::new(None)),
        };
        *site.borrow_mut() = Some(MethodSite {
            class: Rc::as_ptr(&o.class) as usize,
            scope: self.scope_addr(),
            code,
            declaring: Rc::downgrade(declaring),
            is_static: m.this.is_none(),
        });
    }

    /// After a full method lookup on object `o`: the same answer as plain
    /// numbers for the loop's arms (`UserMethod`), when it is an ordinary
    /// call of a PHP method with `$this` (what `remember_method` keeps,
    /// less native and static methods, which the arms do not run).
    pub(crate) fn remember_user_method(&self, slot: &MethodSlot, o: &Rc<p8_runtime::object::Object>, m: &crate::classes::MethodFrame) {
        let own_class = m.called.as_ref().is_some_and(|c| Rc::ptr_eq(c, &o.class));
        let on_this = m.this.as_ref().is_some_and(|t| Rc::ptr_eq(t, o));
        if m.trampoline.is_some() || !own_class || !on_this || m.closure.is_some() {
            return;
        }
        let (Some(declaring), crate::classes::Code::User(code)) = (&m.scope, &m.func) else { return };
        // (Its index: only a declared method's closure has one, and the
        // context's closure there is this one.)
        let index = code.index.get();
        if !self.closures.get(index as usize).is_some_and(|c| Rc::ptr_eq(c, code)) {
            return;
        }
        let Some(refs) = UserMethod::refs_of(&code.code) else { return };
        let (mut class, mut up) = (&o.class, 0);
        while !Rc::ptr_eq(class, declaring) {
            let Some(parent) = &class.parent else { return };
            (class, up) = (parent, up + 1);
        }
        slot.user.set(UserMethod { class: Rc::as_ptr(&o.class) as usize, scope: self.scope_addr(), code: index, up, refs });
    }

    /// The frame of a static-syntax call (`C::m()`, `self::m()`,
    /// `parent::m()`, `static::m()`) the site's cache answers: the method
    /// `static_frame` found for this target class from this scope, with
    /// `$this` and the called class worked out as it does.
    /// For the loop's StaticCall: the method the site's cache has for
    /// class reference `class` from this scope, the class known without
    /// a lookup (a named one its cache has, `self`, `parent`, `static`).
    /// None for anything else, and for a hook's `parent::$p::get()` site.
    #[inline]
    pub(crate) fn cached_static_call(&self, clo: &Closure, class: u16, method: usize) -> Option<crate::classes::MethodFrame> {
        if clo.code.sites[method].display.as_bytes().first() == Some(&b'$') {
            return None;
        }
        let target = self.static_target(clo, class)?;
        self.cached_static(&clo.methods[method], &target, &clo.code, class)
    }

    /// The class a static-syntax site's class reference names, when it is
    /// known without a lookup (a named class resolved before, the scope,
    /// its parent, the called class); None for the rest (a class in a
    /// register, a name not resolved yet).
    #[inline]
    pub(crate) fn static_target(&self, clo: &Closure, class: u16) -> Option<Rc<p8_runtime::object::Class>> {
        Some(match &clo.code.classes[class as usize] {
            p8_bytecode::ClassRef::Named(..) | p8_bytecode::ClassRef::Forward(..) => clo.class_refs[class as usize].borrow().upgrade()?,
            p8_bytecode::ClassRef::Scope => self.scope().0?,
            p8_bytecode::ClassRef::ParentScope => self.scope().0?.parent.clone()?,
            p8_bytecode::ClassRef::Static => self.scope().1?,
            p8_bytecode::ClassRef::Dynamic(_) => return None,
        })
    }

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
            let called = match frame.and_then(|f| f.called_class()) {
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
        if m.trampoline.is_some() || m.closure.is_some() {
            return;
        }
        let Some(declaring) = &m.scope else { return };
        let code = match &m.func {
            crate::classes::Code::User(c) => MethodCode::User(Rc::downgrade(c)),
            crate::classes::Code::Native(n, name) => MethodCode::Native(*n, name.clone(), Cell::new(None)),
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
        let Some(name) = name.as_str() else { return };
        let Ok(Some((slot, prop))) = self.prop_slot(o, name.as_bytes()) else { return };
        if let Some(entry) = prop_entry(o, name.as_bytes(), slot, prop, self.scope_addr()) {
            site.set(entry);
        }
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

//! Classes as one request links them, and objects (APS 4).
//!
//! A class is linked once per request from its compiled declaration: its
//! parent, every interface it implements, its method table (inherited
//! methods included), its instance property layout (inherited slots first,
//! so a parent's code finds its properties at the same index in every
//! subclass), its constants and its static properties.
//!
//! An object is a shared, mutable handle: `Rc<Object>`, its declared
//! properties in slots, others in an ordered map. Object ids are reused
//! after an object is freed, most recently freed first, as PHP's handles
//! are.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_object_handlers.c,
//! $PHP_SRC_PATH/Zend/zend_objects_API.c

use crate::array::PhpArray;
use crate::ext::{Ctx, Throw};
use crate::string::PhpStr;
use crate::symbol::{Sym, SymbolCache};
use crate::value::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// A class's members by name ([`Sym`]), in the order they were added:
/// methods (`FOLD`: names in any case), constants and properties (names
/// as written). Compiled code looks members up by symbol; a name given as
/// bytes (a builtin's, a dynamic one) is found through the isolate's
/// symbol cache the table holds: no allocation, and no lock for a name
/// the isolate has seen.
#[derive(Debug, Clone)]
pub struct Names<V, const FOLD: bool> {
    map: crate::fast::IndexMap<Sym, V>,
    symbols: Rc<SymbolCache>,
}

impl<V, const FOLD: bool> Names<V, FOLD> {
    pub fn new(symbols: &Rc<SymbolCache>) -> Self {
        Self::with_capacity(symbols, 0)
    }

    pub fn with_capacity(symbols: &Rc<SymbolCache>, n: usize) -> Self {
        Names { map: crate::fast::IndexMap::with_capacity_and_hasher(n, Default::default()), symbols: Rc::clone(symbols) }
    }

    /// An empty table on the same symbols.
    pub fn empty_like(&self) -> Self {
        Self::new(&self.symbols)
    }

    pub fn symbols(&self) -> &Rc<SymbolCache> {
        &self.symbols
    }

    /// The symbol a member named `name` would have, if any code named it.
    pub fn find(&self, name: &[u8]) -> Option<Sym> {
        if FOLD { self.symbols.find_lower(name) } else { self.symbols.find(name) }
    }

    /// The symbol of `name`, interned.
    pub fn intern(&self, name: &[u8]) -> Sym {
        if FOLD { self.symbols.intern_lower(name) } else { self.symbols.intern(name) }
    }

    /// The name a symbol stands for (lowercase for methods).
    pub fn name(&self, s: Sym) -> Arc<[u8]> {
        self.symbols.name(s)
    }

    pub fn get(&self, name: &[u8]) -> Option<&V> {
        self.map.get(&self.find(name)?)
    }

    pub fn get_mut(&mut self, name: &[u8]) -> Option<&mut V> {
        let s = self.find(name)?;
        self.map.get_mut(&s)
    }

    pub fn contains_key(&self, name: &[u8]) -> bool {
        self.get(name).is_some()
    }

    #[inline]
    pub fn get_sym(&self, s: Sym) -> Option<&V> {
        self.map.get(&s)
    }

    pub fn get_sym_mut(&mut self, s: Sym) -> Option<&mut V> {
        self.map.get_mut(&s)
    }

    pub fn insert(&mut self, name: impl AsRef<[u8]>, v: V) -> Option<V> {
        let s = self.intern(name.as_ref());
        self.map.insert(s, v)
    }

    pub fn insert_sym(&mut self, s: Sym, v: V) -> Option<V> {
        self.map.insert(s, v)
    }

    pub fn entry(&mut self, s: Sym) -> indexmap::map::Entry<'_, Sym, V> {
        self.map.entry(s)
    }

    pub fn shift_remove(&mut self, name: &[u8]) -> Option<V> {
        let s = self.find(name)?;
        self.map.shift_remove(&s)
    }

    pub fn iter(&self) -> indexmap::map::Iter<'_, Sym, V> {
        self.map.iter()
    }

    pub fn iter_mut(&mut self) -> indexmap::map::IterMut<'_, Sym, V> {
        self.map.iter_mut()
    }

    pub fn keys(&self) -> indexmap::map::Keys<'_, Sym, V> {
        self.map.keys()
    }

    pub fn values(&self) -> indexmap::map::Values<'_, Sym, V> {
        self.map.values()
    }

    pub fn values_mut(&mut self) -> indexmap::map::ValuesMut<'_, Sym, V> {
        self.map.values_mut()
    }

    pub fn get_index(&self, i: usize) -> Option<(&Sym, &V)> {
        self.map.get_index(i)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl<V, const FOLD: bool> Extend<(Sym, V)> for Names<V, FOLD> {
    fn extend<I: IntoIterator<Item = (Sym, V)>>(&mut self, entries: I) {
        self.map.extend(entries);
    }
}

impl<V, const FOLD: bool> IntoIterator for Names<V, FOLD> {
    type Item = (Sym, V);
    type IntoIter = indexmap::map::IntoIter<Sym, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.into_iter()
    }
}

impl<V, const FOLD: bool> std::ops::Index<&[u8]> for Names<V, FOLD> {
    type Output = V;

    fn index(&self, name: &[u8]) -> &V {
        self.get(name).expect("a member of that name")
    }
}

impl<'a, V, const FOLD: bool> IntoIterator for &'a Names<V, FOLD> {
    type Item = (&'a Sym, &'a V);
    type IntoIter = indexmap::map::Iter<'a, Sym, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

/// Who may see a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

impl Visibility {
    pub fn word(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Protected => "protected",
            Visibility::Private => "private",
        }
    }

    /// How narrow it is: public 0, protected 1, private 2.
    pub fn rank(self) -> u8 {
        match self {
            Visibility::Public => 0,
            Visibility::Protected => 1,
            Visibility::Private => 2,
        }
    }
}

pub const CLASS_ABSTRACT: u32 = 1;
pub const CLASS_FINAL: u32 = 2;
pub const CLASS_INTERFACE: u32 = 4;
pub const CLASS_TRAIT: u32 = 8;
pub const CLASS_ENUM: u32 = 16;
pub const CLASS_READONLY: u32 = 32;
/// A resource's class (see `stream::Resource`): not a PHP class.
pub const CLASS_RESOURCE: u32 = 64;
/// The class has a `__destruct` (its own or inherited).
pub const CLASS_DESTRUCTOR: u32 = 128;
/// Some property default is code to run before the class is first used
/// (a constant expression naming other constants).
pub const CLASS_LAZY: u32 = 256;
/// Some property has hooks (get/set run as methods `$name::get`, `$name::set`).
pub const CLASS_HOOKS: u32 = 512;
/// A builtin class (not written in PHP).
pub const CLASS_INTERNAL: u32 = 1024;
/// `clone` throws (a native class with no clone handler; its subclasses too).
pub const CLASS_UNCLONEABLE: u32 = 2048;
/// serialize() and unserialize() refuse it (a native class's state).
pub const CLASS_NOT_SERIALIZABLE: u32 = 4096;
/// Has __get, __set, __isset or __unset (its own or inherited).
pub const CLASS_MAGIC_PROPS: u32 = 8192;
/// #[AllowDynamicProperties] (its own or a parent's): no deprecation for
/// creating a dynamic property.
pub const CLASS_ALLOW_DYNAMIC: u32 = 16384;

/// A property's hooks (Property::hooks): a get hook, a set hook, and
/// whether it is virtual (no stored value: no hook uses `$this->name`).
pub const HOOK_GET: u8 = 1;
pub const HOOK_SET: u8 = 2;
pub const HOOK_VIRTUAL: u8 = 4;

/// The code a declaration's value comes from, run when PHP evaluates it
/// (the interpreter's compiled function; opaque here).
#[derive(Clone)]
pub struct Thunk(pub Rc<dyn std::any::Any>);

impl std::fmt::Debug for Thunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Thunk")
    }
}

/// An instance property's declaration, at its slot.
#[derive(Debug, Clone)]
pub struct Property {
    pub name: PhpStr,
    /// The value a new object starts with; Undef for a typed property
    /// with no default (uninitialized until assigned).
    pub default: Value,
    pub visibility: Visibility,
    /// Who may write it (`private(set)`, `protected(set)`), when narrower
    /// than `visibility`; None: whoever may read it.
    pub set_visibility: Option<Visibility>,
    /// The class that declares it (lowercase), for private access.
    pub declaring: Vec<u8>,
    /// As declared, for messages and var_dump.
    pub declaring_name: PhpStr,
    pub readonly: bool,
    pub is_final: bool,
    /// The declared type as written (`int`, `?Foo`), if typed.
    pub ty: Option<PhpStr>,
    /// The declared type, checked when a value is stored.
    pub ty_decl: Option<Rc<crate::types::TypeDecl>>,
    /// The default is this code's value (computed once per class).
    pub init: Option<Thunk>,
    /// HOOK_GET | HOOK_SET | HOOK_VIRTUAL (0: a plain property).
    pub hooks: u8,
}

impl Property {
    /// A builtin class's hidden state (a name starting with NUL): never
    /// listed (var_dump, get_object_vars, foreach, serialize, (array), json).
    pub fn is_internal(&self) -> bool {
        self.name.as_bytes().first() == Some(&0)
    }
}

/// A method written in Rust (a builtin class's): `$this`, the arguments.
pub type NativeMethod = fn(&mut dyn Ctx, &Rc<Object>, &mut [Value]) -> Result<Value, Throw>;
/// A static method written in Rust: the arguments.
pub type NativeStatic = fn(&mut dyn Ctx, &mut [Value]) -> Result<Value, Throw>;

/// A builtin class's method in Rust.
#[derive(Debug, Clone, Copy)]
pub enum Native {
    Method(NativeMethod),
    Static(NativeStatic),
}

/// A class's methods by lowercase name, in declaration order (inherited
/// ones first): PHP reports and lists them in that order. A subclass's
/// table shares its parent's entries (zend_duplicate_function: the same
/// op_array, its refcount bumped); only a method the subclass declares,
/// overrides or takes from a trait is a new one.
pub type Methods = Names<Rc<Method>, true>;

/// A method in a class's table.
#[derive(Debug, Clone)]
pub struct Method {
    /// As declared.
    pub name: PhpStr,
    /// What runs: the context's closure of the method's code (unused for
    /// a native or abstract builtin method).
    pub func: u32,
    pub native: Option<Native>,
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    /// The class that declares it (lowercase / as declared), shared by
    /// all the methods it declares.
    pub declaring: Rc<[u8]>,
    pub declaring_name: PhpStr,
}

/// A class's constants by name, in the order they were declared; shared
/// with the parent and interfaces as methods are.
/// (By name as bytes: a constant fetch's name is an operand, not a
/// symbol, so a symbol key would cost a symbol lookup per fetch.)
pub type Constants = crate::fast::IndexMap<Vec<u8>, Rc<Constant>>;

/// Each property name's slots (see [`Class::slots`]); by name as bytes,
/// as constants are.
pub type Slots = crate::fast::HashMap<Vec<u8>, Vec<usize>>;

/// A class constant.
#[derive(Debug, Clone)]
pub struct Constant {
    /// For an enum case: its backing value (null for a pure enum's).
    pub value: Value,
    pub visibility: Visibility,
    pub declaring_name: PhpStr,
    /// An enum case: the constant is the case's object (one per request).
    pub is_case: bool,
    /// The value is this code's (computed when first read).
    pub init: Option<Thunk>,
}

/// A static property's declaration. Its value is request state, not the
/// class's: each request keeps its own cell for it, made from `default`
/// when the request first uses it (`Ctx::static_cell`), so a class can be
/// shared by requests that never see each other's values.
#[derive(Debug, Clone)]
pub struct Static {
    /// The cell its value is kept under, in each request: shared with the
    /// subclasses that don't redeclare it (one value for them all).
    pub id: StaticId,
    pub visibility: Visibility,
    /// The class that declares it (lowercase).
    pub declaring: Vec<u8>,
    /// Its value at the start of each request (one computed by code is
    /// set when the class's defaults first run in the request).
    pub default: Value,
}

/// Which static property's cell (see [`Static::id`]): unique in its
/// isolate, never reused.
pub type StaticId = u64;

/// A class as linked for one request.
#[derive(Debug)]
pub struct Class {
    /// As declared.
    pub name: PhpStr,
    pub lower: Vec<u8>,
    pub flags: u32,
    pub parent: Option<Rc<Class>>,
    /// Every interface this class implements, its parent's included
    /// (lowercase), for instanceof; the names are shared with the parent's.
    pub interfaces: Vec<Rc<[u8]>>,
    /// Instance property slots, the parent's first.
    pub properties: Vec<Property>,
    /// Each property name's slots; more than one when a parent's private
    /// property shares a name with a subclass's.
    pub slots: Slots,
    /// Lowercase name → method, inherited ones included.
    pub methods: Methods,
    /// In declaration order (reflection lists them so).
    pub constants: Constants,
    /// Static properties (declarations; their values are each request's):
    /// a subclass shares its parent's cell unless it redeclares the
    /// property. In PHP's order (as `property_order`): the
    /// class's own as declared, then its parent's, then its traits'.
    pub statics: crate::fast::IndexMap<Vec<u8>, Static>,
    /// Every property name, static or not, in the order PHP keeps them
    /// (zend_do_inheritance's properties_info): the class's own as
    /// declared, then its parent's that it does not redeclare (not the
    /// parent's private ones), then those its traits add. Reflection and
    /// get_class_vars() list properties in this order.
    pub property_order: Vec<PhpStr>,
    /// What var_dump and print_r show of an object instead of its
    /// properties (a Closure's name, file, bindings).
    pub debug_info: Option<fn(&Object) -> PhpArray>,
    /// A native class's properties as PHP sees them ((array), var_dump,
    /// get_object_vars, json_encode): zend's get_properties handler.
    pub get_properties: Option<fn(&Object) -> PhpArray>,
    /// `(bool)` of its objects: zend's cast_object to bool (an empty
    /// SimpleXML element is false). None: always true.
    pub cast_bool: Option<fn(&Object) -> bool>,
    /// Its objects as a scalar (zend's cast_object to string): what
    /// `(int)`, `(float)`, arithmetic and comparisons with scalars read. None:
    /// no such cast.
    pub cast_string: Option<fn(&Object) -> PhpStr>,
    /// `(int)` of its objects when the class casts to int itself (zend's
    /// cast_object to IS_LONG: a CurlHandle is its object id). None: as
    /// cast_string reads, else 1.
    pub cast_int: Option<fn(&Object) -> i64>,
    /// The native state a clone gets (zend's clone_obj for internal
    /// classes): None shares the original's.
    pub clone_data: Option<CloneData>,
    /// Where a call to a method the class lacks goes (zend's get_method
    /// handler of the SPL iterators: their inner iterator).
    pub forward_calls: Option<ForwardCalls>,
    /// A native class's per-object Rust state, made for each new object
    /// (of it or a subclass): see [`Data`].
    pub make_data: Option<MakeData>,
    /// A native class's cleanup when an object is destroyed (before any
    /// __destruct, as PHP's internal dtor_obj): what libxml flushes on
    /// free, say.
    pub native_dtor: Option<NativeMethod>,
    /// An enum: its cases in order, and its backing type ("int", "string").
    pub enum_cases: Vec<PhpStr>,
    pub enum_backing: Option<PhpStr>,
    /// The class's own static properties whose default is code (run
    /// before the class is first used).
    pub static_inits: Vec<(Vec<u8>, Thunk)>,
}

impl Class {
    /// A plain class with nothing declared (for tests).
    #[cfg(test)]
    pub(crate) fn stub(name: &str) -> Class {
        let symbols = Rc::new(SymbolCache::new(&Arc::new(crate::symbol::Symbols::default())));
        Class {
            name: PhpStr::from(name),
            lower: name.to_ascii_lowercase().into_bytes(),
            flags: 0,
            parent: None,
            interfaces: Vec::new(),
            properties: Vec::new(),
            slots: Slots::default(),
            methods: Methods::new(&symbols),
            constants: Constants::default(),
            statics: crate::fast::IndexMap::default(),
            property_order: Vec::new(),
            debug_info: None,
            get_properties: None,
            cast_bool: None,
            cast_string: None,
            cast_int: None,
            clone_data: None,
            forward_calls: None,
            make_data: None,
            native_dtor: None,
            enum_cases: Vec::new(),
            enum_backing: None,
            static_inits: Vec::new(),
        }
    }

    /// The name as PHP prints it with "%s": up to a NUL (an anonymous
    /// class's name goes on with where it was declared).
    pub fn shown(&self) -> &[u8] {
        let n = self.name.as_bytes();
        n.iter().position(|&b| b == 0).map_or(n, |i| &n[..i])
    }

    /// Whether this class is `other` or extends or implements it
    /// (`other` lowercase).
    pub fn is_a(&self, other: &[u8]) -> bool {
        let mut class = Some(self);
        while let Some(c) = class {
            if c.lower == other {
                return true;
            }
            class = c.parent.as_deref();
        }
        self.interfaces.iter().any(|i| **i == *other)
    }

    /// The slot of property `name` as code in class `scope` (lowercase,
    /// None outside any class) sees it: the scope's own private
    /// property first, then the visible declaration.
    pub fn slot(&self, name: &[u8], scope: Option<&[u8]>) -> Option<usize> {
        let slots = self.slots.get(name)?;
        if let Some(scope) = scope
            && let Some(&i) = slots.iter().find(|&&i| {
                let p = &self.properties[i];
                p.visibility == Visibility::Private && p.declaring == scope
            })
        {
            return Some(i);
        }
        slots.iter().rev().copied().find(|&i| self.properties[i].visibility != Visibility::Private).or_else(|| slots.last().copied())
    }

    pub fn method(&self, lower: &[u8]) -> Option<&Method> {
        self.methods.get(lower).map(|m| &**m)
    }

    /// The method whose (lowercase) name is `sym`.
    #[inline]
    pub fn method_sym(&self, sym: Sym) -> Option<&Method> {
        self.methods.get_sym(sym).map(|m| &**m)
    }

    /// The value of the property `name` as code in class `scope` sees it
    /// (Undef when there is none).
    pub fn read(&self, object: &Object, name: &[u8], scope: &[u8]) -> Value {
        match self.slot(name, Some(scope)) {
            Some(i) => object.slots.borrow()[i].deref(),
            None => Value::Undef,
        }
    }

    /// Set property `name` (as code in class `scope` sees it), if declared.
    pub fn write(&self, object: &Object, name: &[u8], scope: &[u8], value: Value) {
        if let Some(i) = self.slot(name, Some(scope)) {
            object.slots.borrow_mut()[i] = value;
        }
    }
}

/// Whether code in class `scope` (None outside any class) may use a
/// member of `visibility` declared by `declaring` (zend_check_protected:
/// protected members are visible when either class extends the other).
pub fn visible(visibility: Visibility, declaring: &Class, scope: Option<&Class>) -> bool {
    match (visibility, scope) {
        (Visibility::Public, _) => true,
        (_, None) => false,
        (Visibility::Private, Some(scope)) => scope.lower == declaring.lower,
        (Visibility::Protected, Some(scope)) => scope.is_a(&declaring.lower) || declaring.is_a(&scope.lower),
    }
}

/// The class in `class`'s chain named `lower` (the declaring class of an
/// inherited member).
pub fn ancestor<'c>(class: &'c Class, lower: &[u8]) -> Option<&'c Class> {
    let mut c = Some(class);
    while let Some(k) = c {
        if k.lower == lower {
            return Some(k);
        }
        c = k.parent.as_deref();
    }
    None
}

/// Ids of live objects. A freed object's id goes back here and the next
/// object takes the most recently freed one, as PHP's object store does.
#[derive(Debug, Default)]
pub struct Ids {
    next: Cell<u32>,
    free: RefCell<Vec<u32>>,
    /// Never reuse a number (resources: PHP numbers them up only).
    monotonic: bool,
    /// Objects whose last reference went and whose destructor is due, in
    /// the order they went (each one revived to run it).
    pending: RefCell<std::collections::VecDeque<Rc<Object>>>,
    /// How many are pending (read on hot paths: one load).
    due: Cell<u32>,
    /// Every object with a destructor, for the ones the script leaves
    /// alive (PHP destructs them at its end, oldest first).
    live: RefCell<Vec<std::rc::Weak<Object>>>,
    /// Every object, by id (an id is one live object's at a time):
    /// gc_collect_cycles walks them.
    all: RefCell<Vec<std::rc::Weak<Object>>>,
    /// Where these objects' destructors are queued, when not here (a
    /// resource's: with the objects', which the interpreter runs).
    queue: Option<Rc<Ids>>,
    /// Destructors queued and destructors run, ever: positions in the
    /// queue's history.
    queued: Cell<u64>,
    ran: Cell<u64>,
    /// Ids of objects freed while destructors were due: each goes back
    /// once the destructors queued up to its position have run, as PHP
    /// frees an object's handle after those (which run right where their
    /// objects go, its properties' among them) freed theirs.
    parked: RefCell<std::collections::VecDeque<(u64, u32)>>,
}

impl Ids {
    /// Numbers that only go up.
    pub fn monotonic() -> Ids {
        Ids { monotonic: true, ..Ids::default() }
    }

    /// Numbers that only go up, for objects whose destructors queue with
    /// `objects`' (resources: a user-space stream is closed when it goes).
    pub fn monotonic_beside(objects: &Rc<Ids>) -> Ids {
        Ids { monotonic: true, queue: Some(Rc::clone(objects)), ..Ids::default() }
    }

    pub fn take(&self) -> u32 {
        if let Some(id) = self.free.borrow_mut().pop() {
            return id;
        }
        let id = self.next.get() + 1;
        self.next.set(id);
        id
    }

    fn give_back(&self, id: u32) {
        if !self.monotonic {
            self.free.borrow_mut().push(id);
        }
    }

    /// An object went while destructors are due: its id waits for them.
    #[cold]
    #[inline(never)]
    fn park(&self, id: u32) {
        self.parked.borrow_mut().push_back((self.queued.get(), id));
    }

    /// One queued destructor has run (the object it ran on let go): the
    /// ids parked behind it go back.
    pub fn destructor_ran(&self) {
        self.ran.set(self.ran.get() + 1);
        loop {
            let next = self.parked.borrow().front().copied();
            match next {
                Some((after, id)) if after <= self.ran.get() => {
                    self.parked.borrow_mut().pop_front();
                    self.give_back(id);
                }
                _ => break,
            }
        }
    }

    /// Whether a destructor is due.
    #[inline(always)]
    pub fn has_pending(&self) -> bool {
        self.due.get() != 0
    }

    /// Every object whose destructor is due now, in the order they went,
    /// the queue left empty. The interpreter runs them as one batch, each
    /// destructor to its end before the next: a destructor runs PHP code
    /// (a nested run that also runs what is due), and so sees only what it
    /// frees itself, which it destroys within, as PHP does.
    pub fn take_pending(&self) -> std::collections::VecDeque<Rc<Object>> {
        let batch = std::mem::take(&mut *self.pending.borrow_mut());
        self.due.set(self.due.get() - batch.len() as u32);
        batch
    }

    /// Objects a batch did not get to (a destructor threw): due again,
    /// ahead of any queued since.
    pub fn requeue(&self, mut rest: std::collections::VecDeque<Rc<Object>>) {
        let mut pending = self.pending.borrow_mut();
        rest.append(&mut pending);
        *pending = rest;
        self.due.set(pending.len() as u32);
    }

    /// The objects with a destructor still alive and not yet destructed,
    /// oldest (lowest id) first.
    pub fn undestructed(&self) -> Vec<Rc<Object>> {
        let mut live = self.live.borrow_mut();
        live.retain(|w| w.strong_count() > 0);
        let mut out: Vec<Rc<Object>> = live.iter().filter_map(std::rc::Weak::upgrade).filter(|o| !o.destructed.get()).collect();
        out.sort_by_key(|o| o.id);
        out
    }

    /// Every object still alive (for the cycle collector).
    pub fn all_live(&self) -> Vec<Rc<Object>> {
        self.all.borrow().iter().filter_map(std::rc::Weak::upgrade).collect()
    }

    fn track(&self, o: &Rc<Object>) {
        {
            let mut all = self.all.borrow_mut();
            let i = o.id as usize;
            if all.len() <= i {
                all.resize(i + 1, std::rc::Weak::new());
            }
            all[i] = Rc::downgrade(o);
        }
        if o.class.flags & CLASS_DESTRUCTOR != 0 {
            let mut live = self.live.borrow_mut();
            // Drop the dead now and then, so the list stays the size of the living.
            if live.len() >= 64 && live.len().is_power_of_two() {
                live.retain(|w| w.strong_count() > 0);
            }
            live.push(Rc::downgrade(o));
        }
    }
}

/// What an object of a builtin class holds beyond its properties (a
/// Closure's function and bindings), for the code that knows its type.
pub struct Data(pub Rc<dyn std::any::Any>);

/// Makes a new object's [`Data`] (a native class's state).
pub type MakeData = fn() -> Rc<dyn std::any::Any>;

/// The object's initialization is deferred until a lazy property is used.
pub const LAZY_UNINIT: u8 = 1;
/// A lazy proxy: once initialized, its properties are its instance's.
pub const LAZY_PROXY: u8 = 2;

/// A lazy object's state (zend_lazy_object_info).
#[derive(Debug)]
pub struct Lazy {
    /// The initializer (ghost) or factory (proxy) callable.
    pub initializer: Value,
    /// ReflectionClass::SKIP_* options given.
    pub options: i64,
    /// Which declared property slots are still lazy.
    pub lazy_slots: Vec<bool>,
    /// A proxy's real instance, once its factory ran.
    pub instance: Option<Rc<Object>>,
}

impl Object {
    /// Uninitialized (ghost or proxy).
    pub fn is_uninit_lazy(&self) -> bool {
        self.lazy.get() & LAZY_UNINIT != 0
    }

    /// Slot `i` is lazy (not yet initialized, initialization pending).
    pub fn lazy_slot(&self, i: usize) -> bool {
        self.lazy.get() != 0 && self.lazy_info.borrow().as_ref().is_some_and(|l| l.lazy_slots.get(i).copied().unwrap_or(false))
    }

    /// An initialized proxy's real instance.
    pub fn proxy_instance(&self) -> Option<Rc<Object>> {
        if self.lazy.get() & (LAZY_PROXY | LAZY_UNINIT) != LAZY_PROXY {
            return None;
        }
        self.lazy_info.borrow().as_ref().and_then(|l| l.instance.clone())
    }
}

/// The object a call to a missing method goes to (see [`Class::forward_calls`]).
pub type ForwardCalls = fn(&Object) -> Option<Rc<Object>>;

/// A native class's state for a clone (see [`Class::clone_data`]).
pub type CloneData = fn(&Object) -> Rc<dyn std::any::Any>;

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Data")
    }
}

/// A PHP object.
#[derive(Debug)]
pub struct Object {
    pub class: Rc<Class>,
    pub data: Option<Data>,
    /// Declared properties by slot; Undef where unset or uninitialized.
    pub slots: RefCell<Vec<Value>>,
    /// Properties added at run time (`$o->new = 1`), in order.
    pub dynamic: RefCell<PhpArray>,
    pub id: u32,
    ids: Rc<Ids>,
    /// Its destructor ran, or must not (its constructor threw).
    pub destructed: Cell<bool>,
    /// LAZY_UNINIT / LAZY_PROXY (PHP 8.4 lazy objects): 0 for a plain object.
    pub lazy: Cell<u8>,
    /// A lazy object's initializer and state (see [`Lazy`]).
    pub lazy_info: RefCell<Option<Box<Lazy>>>,
}

impl Object {
    /// A new object of `class` with its properties' defaults.
    pub fn new(class: Rc<Class>, ids: &Rc<Ids>) -> Rc<Object> {
        let data = class.make_data.map(|make| make());
        Object::with_data(class, ids, data)
    }

    /// A new object of `class` beside this one (same id space), holding
    /// `data`: for native code that has an object but no context.
    pub fn sibling(&self, class: Rc<Class>, data: Option<Rc<dyn std::any::Any>>) -> Rc<Object> {
        Object::with_data(class, &self.ids, data)
    }

    /// This object's native state, if it is a `T`.
    pub fn native<T: 'static>(&self) -> Option<&T> {
        self.data.as_ref().and_then(|d| d.0.downcast_ref::<T>())
    }

    /// A new object holding `data` (see [`Data`]).
    pub fn with_data(class: Rc<Class>, ids: &Rc<Ids>, data: Option<Rc<dyn std::any::Any>>) -> Rc<Object> {
        let slots = class.properties.iter().map(|p| p.default.clone()).collect();
        Object::with_slots(class, ids, data, slots)
    }

    /// A new object whose slots start as `slots` (one value per declared
    /// property: the class's defaults, prepared once by whoever makes many).
    pub fn with_slots(class: Rc<Class>, ids: &Rc<Ids>, data: Option<Rc<dyn std::any::Any>>, slots: Vec<Value>) -> Rc<Object> {
        debug_assert_eq!(slots.len(), class.properties.len());
        let o = Rc::new(Object {
            class,
            data: data.map(Data),
            slots: RefCell::new(slots),
            dynamic: RefCell::new(PhpArray::new()),
            id: ids.take(),
            ids: Rc::clone(ids),
            destructed: Cell::new(false),
            lazy: Cell::new(0),
            lazy_info: RefCell::new(None),
        });
        ids.track(&o);
        o
    }

    /// A shallow copy (clone): same class, the properties copied, a new id.
    pub fn copy(&self) -> Rc<Object> {
        let o = Rc::new(Object {
            class: Rc::clone(&self.class),
            data: match self.class.clone_data {
                Some(clone) => Some(Data(clone(self))),
                None => self.data.as_ref().map(|d| Data(Rc::clone(&d.0))),
            },
            // A property once passed by reference (ksort($this->a)) is a
            // plain value again in the clone, not shared with it.
            slots: RefCell::new(self.slots.borrow().iter().map(crate::array::dup).collect()),
            dynamic: RefCell::new(self.dynamic.borrow().clone()),
            id: self.ids.take(),
            ids: Rc::clone(&self.ids),
            destructed: Cell::new(false),
            lazy: Cell::new(0),
            lazy_info: RefCell::new(None),
        });
        self.ids.track(&o);
        o
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        // A lazy object's destructor never runs (an initialized proxy's real
        // instance has its own), as zend_objects_destroy_object skips them.
        if self.class.flags & CLASS_DESTRUCTOR != 0 && !self.destructed.get() && self.lazy.get() == 0 {
            // Its destructor is due: the object lives on (same id, same
            // properties) until the interpreter has run it.
            let revived = Rc::new(Object {
                class: Rc::clone(&self.class),
                data: self.data.take(),
                slots: RefCell::new(std::mem::take(self.slots.get_mut())),
                dynamic: RefCell::new(std::mem::take(self.dynamic.get_mut())),
                id: self.id,
                ids: Rc::clone(&self.ids),
                destructed: Cell::new(true),
                lazy: Cell::new(0),
                lazy_info: RefCell::new(None),
            });
            let queue = self.ids.queue.as_deref().unwrap_or(&self.ids);
            queue.pending.borrow_mut().push_back(revived);
            queue.due.set(queue.due.get() + 1);
            queue.queued.set(queue.queued.get() + 1);
            return;
        }
        if self.ids.monotonic {
            return;
        }
        // Its properties go before its id does (PHP frees an object, and
        // runs the destructors that brings, before its handle is free):
        // emptied in place, their storage freed with the object after.
        self.slots.get_mut().clear();
        let dynamic = self.dynamic.get_mut();
        if !dynamic.is_empty() {
            drop(std::mem::take(dynamic));
        }
        if self.data.is_some() || self.lazy_info.get_mut().is_some() {
            drop((self.data.take(), self.lazy_info.get_mut().take()));
        }
        // A destructor not yet run (one these properties queued, or one
        // queued before) would in PHP have run, and freed its object's
        // handle, before this one goes: the id waits for those.
        if self.ids.due.get() == 0 {
            self.ids.give_back(self.id);
        } else {
            self.ids.park(self.id);
        }
    }
}

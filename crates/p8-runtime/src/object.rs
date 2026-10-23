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
use crate::value::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

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
    /// The class that declares it (lowercase), for private access.
    pub declaring: Vec<u8>,
    /// As declared, for messages and var_dump.
    pub declaring_name: PhpStr,
    pub readonly: bool,
    /// The declared type as written (`int`, `?Foo`), if typed.
    pub ty: Option<PhpStr>,
    /// The declared type, checked when a value is stored.
    pub ty_decl: Option<Rc<crate::types::TypeDecl>>,
    /// The default is this code's value (computed once per class).
    pub init: Option<Thunk>,
    /// HOOK_GET | HOOK_SET | HOOK_VIRTUAL (0: a plain property).
    pub hooks: u8,
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
/// ones first): PHP reports and lists them in that order.
pub type Methods = indexmap::IndexMap<Vec<u8>, Method>;

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
    /// The class that declares it (lowercase / as declared).
    pub declaring: Vec<u8>,
    pub declaring_name: PhpStr,
}

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

/// A static property: its cell (shared with subclasses that don't
/// redeclare it), its visibility, and the declaring class (lowercase).
pub type Static = (Rc<RefCell<Value>>, Visibility, Vec<u8>);

/// A class as linked for one request.
#[derive(Debug)]
pub struct Class {
    /// As declared.
    pub name: PhpStr,
    pub lower: Vec<u8>,
    pub flags: u32,
    pub parent: Option<Rc<Class>>,
    /// Every interface this class implements, its parent's included
    /// (lowercase), for instanceof.
    pub interfaces: Vec<Vec<u8>>,
    /// Instance property slots, the parent's first.
    pub properties: Vec<Property>,
    /// Each property name's slots; more than one when a parent's private
    /// property shares a name with a subclass's.
    pub slots: HashMap<Vec<u8>, Vec<usize>>,
    /// Lowercase name → method, inherited ones included.
    pub methods: Methods,
    pub constants: HashMap<Vec<u8>, Constant>,
    /// Static properties: a subclass shares its parent's cell unless it
    /// redeclares the property.
    pub statics: HashMap<Vec<u8>, Static>,
    /// What var_dump and print_r show of an object instead of its
    /// properties (a Closure's name, file, bindings).
    pub debug_info: Option<fn(&Object) -> PhpArray>,
    /// An enum: its cases in order, and its backing type ("int", "string").
    pub enum_cases: Vec<PhpStr>,
    pub enum_backing: Option<PhpStr>,
    /// The class's own static properties whose default is code (run
    /// before the class is first used).
    pub static_inits: Vec<(Vec<u8>, Thunk)>,
}

impl Class {
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
        self.interfaces.iter().any(|i| i == other)
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
        self.methods.get(lower)
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
}

impl Ids {
    /// Numbers that only go up.
    pub fn monotonic() -> Ids {
        Ids { monotonic: true, ..Ids::default() }
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

    /// Whether a destructor is due.
    #[inline(always)]
    pub fn has_pending(&self) -> bool {
        self.due.get() != 0
    }

    /// The next object whose destructor is due.
    pub fn next_pending(&self) -> Option<Rc<Object>> {
        let o = self.pending.borrow_mut().pop_front()?;
        self.due.set(self.due.get() - 1);
        Some(o)
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

    fn track(&self, o: &Rc<Object>) {
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
}

impl Object {
    /// A new object of `class` with its properties' defaults.
    pub fn new(class: Rc<Class>, ids: &Rc<Ids>) -> Rc<Object> {
        Object::with_data(class, ids, None)
    }

    /// A new object holding `data` (see [`Data`]).
    pub fn with_data(class: Rc<Class>, ids: &Rc<Ids>, data: Option<Rc<dyn std::any::Any>>) -> Rc<Object> {
        let slots = class.properties.iter().map(|p| p.default.clone()).collect();
        let o = Rc::new(Object {
            class,
            data: data.map(Data),
            slots: RefCell::new(slots),
            dynamic: RefCell::new(PhpArray::new()),
            id: ids.take(),
            ids: Rc::clone(ids),
            destructed: Cell::new(false),
        });
        ids.track(&o);
        o
    }

    /// A shallow copy (clone): same class, the properties copied, a new id.
    pub fn copy(&self) -> Rc<Object> {
        let o = Rc::new(Object {
            class: Rc::clone(&self.class),
            data: self.data.as_ref().map(|d| Data(Rc::clone(&d.0))),
            slots: RefCell::new(self.slots.borrow().clone()),
            dynamic: RefCell::new(self.dynamic.borrow().clone()),
            id: self.ids.take(),
            ids: Rc::clone(&self.ids),
            destructed: Cell::new(false),
        });
        self.ids.track(&o);
        o
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        if self.class.flags & CLASS_DESTRUCTOR != 0 && !self.destructed.get() {
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
            });
            self.ids.pending.borrow_mut().push_back(revived);
            self.ids.due.set(self.ids.due.get() + 1);
            return;
        }
        self.ids.give_back(self.id);
    }
}

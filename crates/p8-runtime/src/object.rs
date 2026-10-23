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
}

impl Class {
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
}

impl Object {
    /// A new object of `class` with its properties' defaults.
    pub fn new(class: Rc<Class>, ids: &Rc<Ids>) -> Rc<Object> {
        Object::with_data(class, ids, None)
    }

    /// A new object holding `data` (see [`Data`]).
    pub fn with_data(class: Rc<Class>, ids: &Rc<Ids>, data: Option<Rc<dyn std::any::Any>>) -> Rc<Object> {
        let slots = class.properties.iter().map(|p| p.default.clone()).collect();
        Rc::new(Object {
            class,
            data: data.map(Data),
            slots: RefCell::new(slots),
            dynamic: RefCell::new(PhpArray::new()),
            id: ids.take(),
            ids: Rc::clone(ids),
        })
    }

    /// A shallow copy (clone): same class, the properties copied, a new id.
    pub fn copy(&self) -> Rc<Object> {
        Rc::new(Object {
            class: Rc::clone(&self.class),
            data: self.data.as_ref().map(|d| Data(Rc::clone(&d.0))),
            slots: RefCell::new(self.slots.borrow().clone()),
            dynamic: RefCell::new(self.dynamic.borrow().clone()),
            id: self.ids.take(),
            ids: Rc::clone(&self.ids),
        })
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        self.ids.give_back(self.id);
    }
}

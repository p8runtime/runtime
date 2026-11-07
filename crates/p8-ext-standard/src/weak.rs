//! WeakReference and WeakMap: references that don't keep an object alive.
//! An entry whose object has been destroyed is gone (Rc's weak pointers
//! notice it at the next look).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_weakrefs.c

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{ClassHooks, ClassSpec, Ctx, Throw};
use p8_runtime::object::{CLASS_FINAL, CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

type R = Result<Value, Throw>;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

// ---- WeakReference ----

#[derive(Default)]
pub struct WeakRef {
    target: RefCell<Weak<Object>>,
}

pub fn make_weakref() -> Rc<dyn std::any::Any> {
    Rc::new(WeakRef::default())
}

thread_local! {
    /// The WeakReference made for each object (by its address), so
    /// create() gives the same one while it lives.
    static REFS: RefCell<HashMap<usize, Weak<Object>>> = RefCell::new(HashMap::new());
}

fn weakref_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Direct instantiation of WeakReference is not allowed, use WeakReference::create instead".into()))
}

fn weakref_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(target) = arg(args, 0).into_object() else {
        return Err(ctx.throw("TypeError", "WeakReference::create(): Argument #1 ($object) must be of type object".into()));
    };
    let key = Rc::as_ptr(&target) as usize;
    let existing = REFS.with(|m| m.borrow().get(&key).and_then(Weak::upgrade));
    if let Some(wr) = existing
        && wr.native::<WeakRef>().and_then(|w| w.target.borrow().upgrade()).is_some_and(|t| Rc::ptr_eq(&t, &target))
    {
        return Ok(Value::object(wr));
    }
    let class = ctx.class(b"WeakReference").expect("declared");
    let wr = ctx.new_object(&class);
    *wr.native::<WeakRef>().expect("WeakReference state").target.borrow_mut() = Rc::downgrade(&target);
    REFS.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(key, Rc::downgrade(&wr));
    });
    Ok(Value::object(wr))
}

fn weakref_get(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(this.native::<WeakRef>().and_then(|w| w.target.borrow().upgrade()).map_or(Value::NULL, Value::object))
}

// ---- WeakMap ----

#[derive(Default)]
pub struct WeakMapData {
    entries: RefCell<Vec<(Weak<Object>, Value)>>,
}

pub fn make_weakmap() -> Rc<dyn std::any::Any> {
    Rc::new(WeakMapData::default())
}

fn map(o: &Object) -> &WeakMapData {
    o.native::<WeakMapData>().expect("WeakMap state")
}

/// The live entries (destroyed keys dropped).
fn live(o: &Object) -> Vec<(Rc<Object>, Value)> {
    let mut e = map(o).entries.borrow_mut();
    e.retain(|(k, _)| k.strong_count() > 0);
    e.iter().filter_map(|(k, v)| k.upgrade().map(|k| (k, v.clone()))).collect()
}

fn key_object(ctx: &mut dyn Ctx, k: &Value) -> Result<Rc<Object>, Throw> {
    if let Some(o) = k.as_object() {
        Ok(Rc::clone(o))
    } else if k.is_null_or_undef() {
        Err(ctx.throw("Error", "Cannot append to WeakMap".into()))
    } else {
        Err(ctx.throw("TypeError", "WeakMap key must be an object".into()))
    }
}

fn position(o: &Object, key: &Rc<Object>) -> Option<usize> {
    map(o).entries.borrow().iter().position(|(k, _)| k.upgrade().is_some_and(|k| Rc::ptr_eq(&k, key)))
}

fn offset_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let key = key_object(ctx, &arg(args, 0))?;
    match position(this, &key) {
        Some(i) => Ok(map(this).entries.borrow()[i].1.clone()),
        None => Err(ctx.throw("Error", format!("Object {}#{} not contained in WeakMap", String::from_utf8_lossy(key.class.name.as_bytes()), key.id))),
    }
}

fn offset_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let key = key_object(ctx, &arg(args, 0))?;
    let v = arg(args, 1);
    match position(this, &key) {
        Some(i) => map(this).entries.borrow_mut()[i].1 = v,
        None => map(this).entries.borrow_mut().push((Rc::downgrade(&key), v)),
    }
    Ok(Value::NULL)
}

fn offset_exists(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let key = key_object(ctx, &arg(args, 0))?;
    // isset(): set and not null.
    Ok(Value::bool(position(this, &key).is_some_and(|i| !map(this).entries.borrow()[i].1.deref().is_null())))
}

fn offset_unset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let key = key_object(ctx, &arg(args, 0))?;
    if let Some(i) = position(this, &key) {
        let removed = map(this).entries.borrow_mut().remove(i);
        drop(removed);
    }
    Ok(Value::NULL)
}

fn count(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::int(live(this).len() as i64))
}

fn get_iterator(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let entries = live(this).into_iter().map(|(k, v)| (Value::object(k), v)).collect();
    internal_iterator(ctx, entries)
}

/// var_dump: a list of ["key" => object, "value" => value].
fn weakmap_info(o: &Object) -> PhpArray {
    let mut out = PhpArray::new();
    for (k, v) in live(o) {
        let mut pair = PhpArray::new();
        pair.insert(Key::from_string(PhpStr::from("key")), Value::object(k));
        pair.insert(Key::from_string(PhpStr::from("value")), v);
        let _ = out.push(Value::array(pair));
    }
    out
}

fn weakmap_clone(o: &Object) -> Rc<dyn std::any::Any> {
    Rc::new(WeakMapData { entries: RefCell::new(map(o).entries.borrow().clone()) })
}

pub const WEAKMAP_HOOKS: ClassHooks = ClassHooks { get_properties: None, cast_bool: None, cast_string: None, cast_int: None, clone_data: Some(weakmap_clone), forward_calls: None };

// ---- InternalIterator: an internal class's iteration as an Iterator ----

#[derive(Default)]
pub struct Internal {
    items: RefCell<Vec<(Value, Value)>>,
    at: Cell<usize>,
}

pub fn make_internal() -> Rc<dyn std::any::Any> {
    Rc::new(Internal::default())
}

pub(crate) fn internal_iterator(ctx: &mut dyn Ctx, items: Vec<(Value, Value)>) -> R {
    let class = ctx.class(b"InternalIterator").expect("declared");
    let it = ctx.new_object(&class);
    *it.native::<Internal>().expect("InternalIterator state").items.borrow_mut() = items;
    Ok(Value::object(it))
}

fn internal(o: &Object) -> &Internal {
    o.native::<Internal>().expect("InternalIterator state")
}

fn it_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Cannot directly construct InternalIterator, use getIterator() instead".into()))
}

fn it_current(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let s = internal(this);
    Ok(s.items.borrow().get(s.at.get()).map_or(Value::NULL, |(_, v)| v.clone()))
}

fn it_key(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let s = internal(this);
    Ok(s.items.borrow().get(s.at.get()).map_or(Value::NULL, |(k, _)| k.clone()))
}

fn it_next(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let s = internal(this);
    s.at.set(s.at.get() + 1);
    Ok(Value::NULL)
}

fn it_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let s = internal(this);
    Ok(Value::bool(s.at.get() < s.items.borrow().len()))
}

fn it_rewind(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    internal(this).at.set(0);
    Ok(Value::NULL)
}

macro_rules! m {
    ($name:literal, $f:path) => {
        ($name, Some(Native::Method($f)))
    };
}

pub const CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "WeakReference",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL | CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE,
        properties: &[],
        methods: &[m!("__construct", weakref_construct), ("create", Some(Native::Static(weakref_create))), m!("get", weakref_get)],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "WeakMap",
        parent: None,
        interfaces: &["ArrayAccess", "Countable", "IteratorAggregate"],
        flags: CLASS_FINAL | CLASS_NOT_SERIALIZABLE,
        properties: &[],
        methods: &[
            m!("offsetGet", offset_get),
            m!("offsetSet", offset_set),
            m!("offsetExists", offset_exists),
            m!("offsetUnset", offset_unset),
            m!("count", count),
            m!("getIterator", get_iterator),
        ],
        constants: &[],
        debug_info: Some(weakmap_info),
    },
    ClassSpec {
        name: "InternalIterator",
        parent: None,
        interfaces: &["Iterator"],
        flags: CLASS_FINAL,
        properties: &[],
        methods: &[
            m!("__construct", it_construct),
            m!("current", it_current),
            m!("key", it_key),
            m!("next", it_next),
            m!("valid", it_valid),
            m!("rewind", it_rewind),
        ],
        constants: &[],
        debug_info: None,
    },
];

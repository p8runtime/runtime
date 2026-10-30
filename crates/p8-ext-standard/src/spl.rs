//! SPL's data structures: ArrayObject, ArrayIterator,
//! RecursiveArrayIterator, SplFixedArray, SplObjectStorage,
//! SplDoublyLinkedList (SplStack, SplQueue), SplHeap (SplMinHeap,
//! SplMaxHeap), SplPriorityQueue; the SPL interfaces; class_implements,
//! class_parents, class_uses.
//!
//! What PHP shows of them (var_dump) is a private property each keeps
//! (storage, dllist, heap...); other state is hidden ("\0..." properties).
//!
//! Reference: $PHP_SRC_PATH/ext/spl/spl_array.c, spl_fixedarray.c,
//! spl_observer.c, spl_dllist.c, spl_heap.c

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::{CLASS_ABSTRACT, CLASS_INTERFACE, Native, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

const PRIVATE: Visibility = Visibility::Private;
const PUBLIC: Visibility = Visibility::Public;

// ---------------------------------------------------------------------------
// Helpers

/// The slot of the property `name` (the class's own, or inherited).
pub(crate) fn slot(o: &Object, name: &str) -> Option<usize> {
    o.class.properties.iter().position(|p| p.name.as_bytes() == name.as_bytes())
}

pub(crate) fn get(o: &Object, name: &str) -> Value {
    slot(o, name).map_or(Value::Null, |i| o.slots.borrow()[i].deref())
}

pub(crate) fn set(o: &Object, name: &str, v: Value) {
    if let Some(i) = slot(o, name) {
        o.slots.borrow_mut()[i] = v;
    }
}

pub(crate) fn int(o: &Object, name: &str) -> i64 {
    get(o, name).to_int()
}

pub(crate) fn array_of(v: Value) -> PhpArray {
    match v {
        Value::Array(a) => Rc::unwrap_or_clone(a),
        _ => PhpArray::new(),
    }
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn class_name(o: &Object) -> String {
    String::from_utf8_lossy(o.class.name.as_bytes()).into_owned()
}

fn shown_key(k: &Key) -> String {
    match k {
        Key::Int(i) => i.to_string(),
        Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
    }
}

/// A method of `this` by name, through the engine and whatever its
/// visibility (a subclass may override it: SplHeap::compare is protected).
pub(crate) fn call_this(ctx: &mut dyn Ctx, this: &Rc<Object>, name: &str, args: &mut [Value]) -> Result<Value, Throw> {
    let callee = p8_runtime::reflect::Callee::Method(Rc::clone(&this.class), PhpStr::from(name));
    ctx.invoke(&callee, Some(Rc::clone(this)), args)
}

/// A key as an array key (PHP's conversions; a bad one is a TypeError).
fn array_key(ctx: &mut dyn Ctx, k: &Value) -> Result<Key, Throw> {
    match Key::from_value(k) {
        Ok((key, _)) => Ok(key),
        Err(_) => Err(ctx.throw("TypeError", format!("Cannot access offset of type {} on array", k.type_name()))),
    }
}

// ---------------------------------------------------------------------------
// ArrayObject / ArrayIterator: the private "storage" (an array, or an
// object whose properties are the elements)

const STORAGE: (&str, Constant, Visibility) = ("storage", Constant::Null, PRIVATE);
const H_FLAGS: (&str, Constant, Visibility) = ("\0flags", Constant::Int(0), PUBLIC);
const H_ITCLASS: (&str, Constant, Visibility) = ("\0itclass", Constant::Str(b"ArrayIterator"), PUBLIC);
const H_POS: (&str, Constant, Visibility) = ("\0pos", Constant::Int(0), PUBLIC);

/// The elements: the array, or the object's properties.
fn elements(o: &Object) -> PhpArray {
    match get(o, "storage") {
        Value::Array(a) => Rc::unwrap_or_clone(a),
        Value::Object(inner) => {
            let mut out = PhpArray::new();
            let slots = inner.slots.borrow();
            for (p, v) in inner.class.properties.iter().zip(slots.iter()) {
                if p.visibility == Visibility::Public && !p.is_internal() && !matches!(v, Value::Undef) {
                    out.insert(Key::from_string(p.name.clone()), v.deref());
                }
            }
            for (k, v) in inner.dynamic.borrow().iter() {
                out.insert(k, v.deref());
            }
            out
        }
        _ => PhpArray::new(),
    }
}

/// Change the elements (an object's properties are written back to it).
fn change<R>(o: &Object, f: impl FnOnce(&mut PhpArray) -> R) -> R {
    match get(o, "storage") {
        Value::Object(inner) => {
            let mut els = elements(o);
            let r = f(&mut els);
            let mut dynamic = PhpArray::new();
            for (k, v) in els.iter() {
                let declared = match &k {
                    Key::Str(n) => inner.class.slot(n.as_bytes(), None),
                    Key::Int(_) => None,
                };
                match declared {
                    Some(i) => inner.slots.borrow_mut()[i] = v.clone(),
                    None => dynamic.insert(k, v.clone()),
                }
            }
            *inner.dynamic.borrow_mut() = dynamic;
            r
        }
        other => {
            let mut a = array_of(other);
            let r = f(&mut a);
            set(o, "storage", Value::Array(Rc::new(a)));
            r
        }
    }
}

fn ao_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let storage = match arg(args, 0) {
        Value::Null | Value::Undef => Value::Array(Rc::new(PhpArray::new())),
        v @ (Value::Array(_) | Value::Object(_)) => v,
        other => {
            let class = class_name(this);
            return Err(ctx.throw("TypeError", format!("{class}::__construct(): Argument #1 ($array) must be of type array, {} given", other.type_name())));
        }
    };
    // An ArrayObject/ArrayIterator given: its elements.
    let storage = match &storage {
        Value::Object(o) if slot(o, "storage").is_some() && (o.class.is_a(b"arrayobject") || o.class.is_a(b"arrayiterator")) => Value::Array(Rc::new(elements(o))),
        _ => storage,
    };
    set(this, "storage", storage);
    if args.len() > 1 {
        set(this, "\0flags", Value::Int(arg(args, 1).to_int()));
    }
    if args.len() > 2 {
        set(this, "\0itclass", arg(args, 2));
    }
    set(this, "\0pos", Value::Int(0));
    Ok(Value::Null)
}

fn ao_offset_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let key = array_key(ctx, &arg(args, 0))?;
    match elements(this).get(&key) {
        Some(v) => Ok(v.deref()),
        None => {
            ctx.warn(&format!("Undefined array key {}", shown_key(&key)));
            Ok(Value::Null)
        }
    }
}

fn ao_offset_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 1);
    match arg(args, 0) {
        Value::Null => {
            change(this, |a| {
                let _ = a.push(v);
            });
        }
        k => {
            let key = array_key(ctx, &k)?;
            change(this, |a| a.insert(key, v));
        }
    }
    Ok(Value::Null)
}

fn ao_offset_exists(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let key = array_key(ctx, &arg(args, 0))?;
    Ok(Value::Bool(elements(this).get(&key).is_some_and(|v| !matches!(v.deref(), Value::Null))))
}

fn ao_offset_unset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let key = array_key(ctx, &arg(args, 0))?;
    change(this, |a| {
        a.remove(&key);
    });
    Ok(Value::Null)
}

fn ao_append(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    change(this, |a| {
        let _ = a.push(v);
    });
    Ok(Value::Null)
}

fn ao_count(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(elements(this).len() as i64))
}

fn ao_get_array_copy(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Array(Rc::new(elements(this))))
}

fn ao_exchange_array(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let old = elements(this);
    let new = match arg(args, 0) {
        v @ (Value::Array(_) | Value::Object(_)) => v,
        other => {
            return Err(ctx.throw("TypeError", format!("ArrayObject::exchangeArray(): Argument #1 ($array) must be of type array, {} given", other.type_name())));
        }
    };
    set(this, "storage", new);
    Ok(Value::Array(Rc::new(old)))
}

/// The object's own properties for Serializable's form: those a subclass
/// declares and the dynamic ones, by their mangled names.
fn members(o: &Object) -> PhpArray {
    let mut out = PhpArray::new();
    let slots = o.slots.borrow();
    for (p, v) in o.class.properties.iter().zip(slots.iter()) {
        let name = p.name.as_bytes();
        if name.starts_with(b"\0") || name == b"storage" || matches!(v, Value::Undef) {
            continue;
        }
        let key = match p.visibility {
            Visibility::Public => name.to_vec(),
            Visibility::Protected => [b"\0*\0".as_slice(), name].concat(),
            Visibility::Private => [b"\0".as_slice(), p.declaring_name.as_bytes(), b"\0", name].concat(),
        };
        out.insert(Key::from_string(PhpStr::new(key)), v.clone());
    }
    for (k, v) in o.dynamic.borrow().iter() {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// Serializable::serialize(): `x:i:FLAGS;STORAGE;m:MEMBERS` (spl_array_serialize).
fn ao_serialize(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let mut out = format!("x:i:{};", int(this, "\0flags")).into_bytes();
    out.extend(crate::serial::serialized(ctx, &get(this, "storage"))?);
    out.extend_from_slice(b";m:");
    out.extend(crate::serial::serialized(ctx, &Value::Array(Rc::new(members(this))))?);
    Ok(Value::Str(PhpStr::new(out)))
}

/// Serializable::unserialize(): what ao_serialize wrote, back.
fn ao_unserialize(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let data = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let bytes = data.as_bytes();
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let bad = |ctx: &mut dyn Ctx, at: usize| {
        Err(ctx.throw("UnexpectedValueException", format!("Error at offset {at} of {} bytes", bytes.len())))
    };
    let Some(rest) = bytes.strip_prefix(b"x:") else { return bad(ctx, 0) };
    let Some((Value::Int(flags), n)) = crate::serial::read_value(ctx, rest) else { return bad(ctx, 2) };
    let at = 2 + n;
    let Some((storage, n)) = crate::serial::read_value(ctx, &bytes[at..]) else { return bad(ctx, at) };
    let mut at = at + n;
    if bytes.get(at) == Some(&b';') {
        at += 1;
    }
    let Some(rest) = bytes[at..].strip_prefix(b"m:") else { return bad(ctx, at) };
    let Some((Value::Array(m), _)) = crate::serial::read_value(ctx, rest) else { return bad(ctx, at + 2) };
    set(this, "\0flags", Value::Int(flags));
    set(this, "storage", storage);
    for (k, v) in m.iter() {
        let key = match k {
            Key::Int(i) => i.to_string().into_bytes(),
            Key::Str(s) => s.as_bytes().to_vec(),
        };
        // A mangled name ("\0Class\0p", "\0*\0p") is a declared property's.
        let name = key.iter().rposition(|&c| c == 0).map_or(key.as_slice(), |i| &key[i + 1..]);
        if this.class.slots.contains_key(name) {
            this.class.write(this, name, this.class.name.as_bytes(), v.clone());
        } else {
            let k = Key::from_string(PhpStr::from(name));
            if !this.dynamic.borrow().contains_key(&k) {
                ctx.deprecated(&format!("Creation of dynamic property {}::${} is deprecated", String::from_utf8_lossy(this.class.name.as_bytes()), String::from_utf8_lossy(name)));
            }
            this.dynamic.borrow_mut().insert(k, v.clone());
        }
    }
    Ok(Value::Null)
}

fn ao_get_flags(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(int(this, "\0flags")))
}

fn ao_set_flags(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    set(this, "\0flags", Value::Int(arg(args, 0).to_int()));
    Ok(Value::Null)
}

fn ao_get_iterator_class(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0itclass"))
}

fn ao_set_iterator_class(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    set(this, "\0itclass", arg(args, 0));
    Ok(Value::Null)
}

fn ao_get_iterator(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let name = get(this, "\0itclass").to_php_string().unwrap_or_else(|| PhpStr::from("ArrayIterator"));
    let Some(class) = ctx.class_load(name.as_bytes())? else {
        return Err(ctx.throw("Error", format!("Class \"{}\" not found", String::from_utf8_lossy(name.as_bytes()))));
    };
    let it = ctx.new_instance(&class, &mut [Value::Array(Rc::new(elements(this)))], true)?;
    Ok(Value::Object(it))
}

/// The array sorts: the builtin of that name on the elements.
fn sorted(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], builtin: &str) -> Result<Value, Throw> {
    let cell = Rc::new(RefCell::new(Value::Array(Rc::new(elements(this)))));
    let mut call_args = vec![Value::Ref(Rc::clone(&cell))];
    call_args.extend(args.iter().map(Value::deref));
    let r = ctx.call(&s(builtin), &mut call_args)?;
    let sorted = array_of(cell.borrow().clone());
    change(this, |a| *a = sorted);
    Ok(r)
}

fn ao_asort(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    sorted(ctx, this, args, "asort")
}

fn ao_ksort(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    sorted(ctx, this, args, "ksort")
}

fn ao_uasort(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    sorted(ctx, this, args, "uasort")
}

fn ao_uksort(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    sorted(ctx, this, args, "uksort")
}

fn ao_natsort(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    sorted(ctx, this, args, "natsort")
}

fn ao_natcasesort(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    sorted(ctx, this, args, "natcasesort")
}

// ArrayIterator: a position (a slot of the elements' hash) in "\0pos".

fn ai_current(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let els = elements(this);
    Ok(els.entry_at(int(this, "\0pos") as usize).map_or(Value::Null, |(_, _, v)| v.deref()))
}

fn ai_key(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let els = elements(this);
    Ok(els.entry_at(int(this, "\0pos") as usize).map_or(Value::Null, |(_, k, _)| k.into_value()))
}

fn ai_next(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let els = elements(this);
    let pos = els.entry_at(int(this, "\0pos") as usize).map_or(els.end_pos(), |(p, _, _)| p + 1);
    set(this, "\0pos", Value::Int(pos as i64));
    Ok(Value::Null)
}

fn ai_rewind(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    set(this, "\0pos", Value::Int(0));
    Ok(Value::Null)
}

fn ai_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(elements(this).entry_at(int(this, "\0pos") as usize).is_some()))
}

fn ai_seek(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let n = arg(args, 0).to_int();
    let els = elements(this);
    let mut pos = 0;
    for _ in 0..n.max(0) {
        match els.entry_at(pos) {
            Some((p, _, _)) => pos = p + 1,
            None => break,
        }
    }
    match els.entry_at(pos) {
        Some((p, _, _)) if n >= 0 => {
            set(this, "\0pos", Value::Int(p as i64));
            Ok(Value::Null)
        }
        _ => Err(ctx.throw("OutOfBoundsException", format!("Seek position {n} is out of range"))),
    }
}

// RecursiveArrayIterator

fn rai_has_children(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let els = elements(this);
    let child_arrays_only = int(this, "\0flags") & 4 != 0;
    Ok(Value::Bool(match els.entry_at(int(this, "\0pos") as usize).map(|(_, _, v)| v.deref()) {
        Some(Value::Array(_)) => true,
        Some(Value::Object(_)) => !child_arrays_only,
        _ => false,
    }))
}

fn rai_get_children(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let els = elements(this);
    let child = els.entry_at(int(this, "\0pos") as usize).map_or(Value::Null, |(_, _, v)| v.deref());
    let child = match child {
        Value::Object(o) if o.class.is_a(b"recursivearrayiterator") => return Ok(Value::Object(o)),
        v => v,
    };
    // A new iterator of this one's class, with its flags.
    let class = Rc::clone(&this.class);
    let it = ctx.new_instance(&class, &mut [child, Value::Int(int(this, "\0flags"))], true)?;
    Ok(Value::Object(it))
}

// ---------------------------------------------------------------------------
// SplFixedArray: "\0data" (a list of its size); var_dump shows the elements.

const H_DATA: (&str, Constant, Visibility) = ("\0data", Constant::Null, PUBLIC);

fn fixed(o: &Object) -> PhpArray {
    array_of(get(o, "\0data"))
}

fn fixed_info(o: &Object) -> PhpArray {
    fixed(o)
}

/// An SplFixedArray index (spl_offset_convert_to_long), checked.
fn fixed_index(ctx: &mut dyn Ctx, o: &Object, k: &Value) -> Result<usize, Throw> {
    let i = match k.deref() {
        Value::Int(i) => i,
        Value::Bool(b) => i64::from(b),
        Value::Float(f) => f as i64,
        Value::Str(s) => match p8_runtime::numeric::numeric_string(s.as_bytes()) {
            Some(p8_runtime::numeric::Number::Int(i)) => i,
            _ => return Err(ctx.throw("TypeError", "Cannot access offset of type string on SplFixedArray".into())),
        },
        other => return Err(ctx.throw("TypeError", format!("Cannot access offset of type {} on SplFixedArray", other.type_name()))),
    };
    let len = fixed(o).len() as i64;
    if i < 0 || i >= len {
        return Err(ctx.throw("OutOfBoundsException", "Index invalid or out of range".into()));
    }
    Ok(i as usize)
}

fn fa_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let n = if args.is_empty() { 0 } else { arg(args, 0).to_int() };
    if n < 0 {
        return Err(ctx.throw("ValueError", "SplFixedArray::__construct(): Argument #1 ($size) must be greater than or equal to 0".into()));
    }
    set(this, "\0data", Value::Array(Rc::new(PhpArray::from_list(vec![Value::Null; n as usize]))));
    Ok(Value::Null)
}

fn fa_offset_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let i = fixed_index(ctx, this, &arg(args, 0))?;
    Ok(fixed(this).get_int(i as i64).map_or(Value::Null, Value::deref))
}

fn fa_offset_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    if matches!(arg(args, 0), Value::Null) {
        return Err(ctx.throw("Error", "[] operator not supported for SplFixedArray".into()));
    }
    let i = fixed_index(ctx, this, &arg(args, 0))?;
    let mut data = fixed(this);
    data.insert(Key::Int(i as i64), arg(args, 1));
    set(this, "\0data", Value::Array(Rc::new(data)));
    Ok(Value::Null)
}

fn fa_offset_exists(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let i = arg(args, 0).to_int();
    Ok(Value::Bool(fixed(this).get_int(i).is_some_and(|v| !matches!(v.deref(), Value::Null))))
}

fn fa_offset_unset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let i = fixed_index(ctx, this, &arg(args, 0))?;
    let mut data = fixed(this);
    data.insert(Key::Int(i as i64), Value::Null);
    set(this, "\0data", Value::Array(Rc::new(data)));
    Ok(Value::Null)
}

fn fa_count(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(fixed(this).len() as i64))
}

fn fa_set_size(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let n = arg(args, 0).to_int();
    if n < 0 {
        return Err(ctx.throw("ValueError", "SplFixedArray::setSize(): Argument #1 ($size) must be greater than or equal to 0".into()));
    }
    let mut list: Vec<Value> = fixed(this).values().cloned().collect();
    list.resize(n as usize, Value::Null);
    set(this, "\0data", Value::Array(Rc::new(PhpArray::from_list(list))));
    Ok(Value::Bool(true))
}

fn fa_to_array(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Array(Rc::new(fixed(this))))
}

fn fa_from_array(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Array(a) = arg(args, 0) else {
        return Err(ctx.throw("TypeError", "SplFixedArray::fromArray(): Argument #1 ($array) must be of type array".into()));
    };
    let keep = args.len() < 2 || arg(args, 1).to_bool();
    let list = if keep {
        let mut max = -1i64;
        for (k, _) in a.iter() {
            match k {
                Key::Int(i) if i >= 0 => max = max.max(i),
                _ => return Err(ctx.throw("ValueError", "array must contain only positive integer keys".into())),
            }
        }
        let mut list = vec![Value::Null; (max + 1) as usize];
        for (k, v) in a.iter() {
            if let Key::Int(i) = k {
                list[i as usize] = v.deref();
            }
        }
        list
    } else {
        a.values().map(Value::deref).collect()
    };
    let class = ctx.class(b"SplFixedArray").expect("SplFixedArray");
    let o = ctx.new_object(&class);
    set(&o, "\0data", Value::Array(Rc::new(PhpArray::from_list(list))));
    Ok(Value::Object(o))
}

fn fa_get_iterator(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let class = ctx.class(b"ArrayIterator").expect("ArrayIterator");
    let it = ctx.new_instance(&class, &mut [Value::Array(Rc::new(fixed(this)))], true)?;
    Ok(Value::Object(it))
}

fn no_op(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// SplObjectStorage: "\0map" (object id → [object, info]), "\0pos" (a
// slot of the map) and "\0index" (the iterator's key).

const H_MAP: (&str, Constant, Visibility) = ("\0map", Constant::Null, PUBLIC);
const H_INDEX: (&str, Constant, Visibility) = ("\0index", Constant::Int(0), PUBLIC);

fn store(o: &Object) -> PhpArray {
    array_of(get(o, "\0map"))
}

fn put_store(o: &Object, map: PhpArray) {
    set(o, "\0map", Value::Array(Rc::new(map)));
}

fn store_info(o: &Object) -> PhpArray {
    let mut list = PhpArray::new();
    for v in store(o).values() {
        let pair = array_of(v.clone());
        let mut e = PhpArray::new();
        e.insert(Key::from_string(PhpStr::from("obj")), pair.get_int(0).cloned().unwrap_or(Value::Null));
        e.insert(Key::from_string(PhpStr::from("inf")), pair.get_int(1).cloned().unwrap_or(Value::Null));
        let _ = list.push(Value::Array(Rc::new(e)));
    }
    let mut out = PhpArray::new();
    out.insert(Key::from_string(PhpStr::from("\0SplObjectStorage\0storage")), Value::Array(Rc::new(list)));
    out
}

fn object_arg(ctx: &mut dyn Ctx, args: &[Value], method: &str) -> Result<Rc<Object>, Throw> {
    match arg(args, 0) {
        Value::Object(o) => Ok(o),
        other => Err(ctx.throw("TypeError", format!("SplObjectStorage::{method}(): Argument #1 ($object) must be of type object, {} given", other.type_name()))),
    }
}

fn os_attach_to(this: &Object, o: Rc<Object>, inf: Value) {
    let mut map = store(this);
    let pair = PhpArray::from_list(vec![Value::Object(Rc::clone(&o)), inf]);
    map.insert(Key::Int(i64::from(o.id)), Value::Array(Rc::new(pair)));
    put_store(this, map);
}

fn os_attach(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = object_arg(ctx, args, "attach")?;
    os_attach_to(this, o, arg(args, 1));
    Ok(Value::Null)
}

fn os_detach(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = object_arg(ctx, args, "detach")?;
    let mut map = store(this);
    map.remove(&Key::Int(i64::from(o.id)));
    put_store(this, map);
    Ok(Value::Null)
}

fn os_contains(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = object_arg(ctx, args, "contains")?;
    Ok(Value::Bool(store(this).contains_key(&Key::Int(i64::from(o.id)))))
}

fn os_offset_exists(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = object_arg(ctx, args, "offsetExists")?;
    Ok(Value::Bool(store(this).contains_key(&Key::Int(i64::from(o.id)))))
}

fn os_offset_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = object_arg(ctx, args, "offsetGet")?;
    match store(this).get(&Key::Int(i64::from(o.id))) {
        Some(pair) => Ok(array_of(pair.clone()).get_int(1).cloned().unwrap_or(Value::Null)),
        None => Err(ctx.throw("UnexpectedValueException", "Object not found".into())),
    }
}

fn os_offset_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = object_arg(ctx, args, "offsetSet")?;
    os_attach_to(this, o, arg(args, 1));
    Ok(Value::Null)
}

fn os_offset_unset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    os_detach(ctx, this, args)
}

fn os_count(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let map = store(this);
    if arg(args, 0).to_int() == 1 {
        // COUNT_RECURSIVE: the stored objects' own counts too.
        let mut n = map.len() as i64;
        for v in map.values() {
            if let Some(Value::Object(inner)) = array_of(v.clone()).get_int(0).cloned()
                && inner.class.is_a(b"countable")
            {
                n += call_this(ctx, &inner, "count", &mut [])?.to_int();
            }
        }
        return Ok(Value::Int(n));
    }
    Ok(Value::Int(map.len() as i64))
}

fn storage_arg(ctx: &mut dyn Ctx, args: &[Value], method: &str) -> Result<Rc<Object>, Throw> {
    match arg(args, 0) {
        Value::Object(o) if o.class.is_a(b"splobjectstorage") => Ok(o),
        other => Err(ctx.throw("TypeError", format!("SplObjectStorage::{method}(): Argument #1 ($storage) must be of type SplObjectStorage, {} given", other.type_name()))),
    }
}

fn os_add_all(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let other = storage_arg(ctx, args, "addAll")?;
    let mut map = store(this);
    for (k, v) in store(&other).iter() {
        map.insert(k, v.clone());
    }
    let n = map.len();
    put_store(this, map);
    Ok(Value::Int(n as i64))
}

fn os_remove_all(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let other = storage_arg(ctx, args, "removeAll")?;
    let mut map = store(this);
    for (k, _) in store(&other).iter() {
        map.remove(&k);
    }
    let n = map.len();
    put_store(this, map);
    Ok(Value::Int(n as i64))
}

fn os_remove_all_except(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let other = storage_arg(ctx, args, "removeAllExcept")?;
    let keep = store(&other);
    let mut map = PhpArray::new();
    for (k, v) in store(this).iter() {
        if keep.contains_key(&k) {
            map.insert(k, v.clone());
        }
    }
    let n = map.len();
    put_store(this, map);
    Ok(Value::Int(n as i64))
}

/// The entry at the position, if valid.
fn os_entry(o: &Object) -> Option<PhpArray> {
    store(o).entry_at(int(o, "\0pos") as usize).map(|(_, _, v)| array_of(v.clone()))
}

fn os_rewind(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    set(this, "\0pos", Value::Int(0));
    set(this, "\0index", Value::Int(0));
    Ok(Value::Null)
}

fn os_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(os_entry(this).is_some()))
}

fn os_key(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(int(this, "\0index")))
}

fn os_current(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    match os_entry(this) {
        Some(pair) => Ok(pair.get_int(0).cloned().unwrap_or(Value::Null)),
        None => Err(ctx.throw("RuntimeException", "Called current() on invalid iterator".into())),
    }
}

fn os_next(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let map = store(this);
    if let Some((p, _, _)) = map.entry_at(int(this, "\0pos") as usize) {
        set(this, "\0pos", Value::Int(p as i64 + 1));
        set(this, "\0index", Value::Int(int(this, "\0index") + 1));
    }
    Ok(Value::Null)
}

fn os_get_info(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(os_entry(this).and_then(|p| p.get_int(1).cloned()).unwrap_or(Value::Null))
}

fn os_set_info(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut map = store(this);
    if let Some((_, k, v)) = map.entry_at(int(this, "\0pos") as usize) {
        let mut pair = array_of(v.clone());
        pair.insert(Key::Int(1), arg(args, 0));
        map.insert(k, Value::Array(Rc::new(pair)));
        put_store(this, map);
    }
    Ok(Value::Null)
}

fn os_get_hash(ctx: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = object_arg(ctx, args, "getHash")?;
    Ok(Value::Str(PhpStr::from(format!("{:032x}", o.id))))
}

// ---------------------------------------------------------------------------
// SplDoublyLinkedList (SplStack, SplQueue): private "flags" and "dllist";
// "\0pos" the iterator's index.

const IT_MODE_LIFO: i64 = 2;
const IT_MODE_DELETE: i64 = 1;
/// SplStack/SplQueue: the direction is fixed.
const IT_FIX: i64 = 4;

const DLL_FLAGS: (&str, Constant, Visibility) = ("flags", Constant::Int(0), PRIVATE);
const DLLIST: (&str, Constant, Visibility) = ("dllist", Constant::Null, PRIVATE);

/// Its flags (an SplStack's or SplQueue's fixed direction set the first time).
fn dll_flags(o: &Object) -> i64 {
    let f = int(o, "flags");
    if f & IT_FIX == 0 && (o.class.is_a(b"splstack") || o.class.is_a(b"splqueue")) {
        let f = if o.class.is_a(b"splstack") { IT_MODE_LIFO | IT_FIX } else { IT_FIX };
        set(o, "flags", Value::Int(f));
        return f;
    }
    f
}

fn dll(o: &Object) -> Vec<Value> {
    dll_flags(o);
    array_of(get(o, "dllist")).values().cloned().collect()
}

fn put_dll(o: &Object, list: Vec<Value>) {
    set(o, "dllist", Value::Array(Rc::new(PhpArray::from_list(list))));
}

/// The list's properties besides its own state, as __serialize() gives them.
fn dll_members(o: &Object) -> PhpArray {
    let mut out = members(o);
    for internal in ["\0SplDoublyLinkedList\0flags", "\0SplDoublyLinkedList\0dllist"] {
        out.remove(&Key::from_string(PhpStr::from(internal)));
    }
    out
}

/// Serializable::serialize(): `i:FLAGS;` then `:VALUE` per element (spl_dllist.c).
fn dll_serialize(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let mut out = format!("i:{};", dll_flags(this)).into_bytes();
    for v in dll(this) {
        out.push(b':');
        out.extend(crate::serial::serialized(ctx, &v)?);
    }
    Ok(Value::Str(PhpStr::new(out)))
}

/// Serializable::unserialize(): what dll_serialize wrote, back.
fn dll_unserialize(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let data = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let bytes = data.as_bytes();
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let bad = |ctx: &mut dyn Ctx, at: usize| Err(ctx.throw("UnexpectedValueException", format!("Error at offset {at} of {} bytes", bytes.len())));
    let Some((Value::Int(flags), mut at)) = crate::serial::read_value(ctx, bytes) else { return bad(ctx, 0) };
    let mut list = dll(this);
    while at < bytes.len() {
        if bytes[at] != b':' {
            return bad(ctx, at);
        }
        at += 1;
        let Some((v, n)) = crate::serial::read_value(ctx, &bytes[at..]) else { return bad(ctx, at) };
        list.push(v);
        at += n;
    }
    set(this, "flags", Value::Int(flags));
    put_dll(this, list);
    Ok(Value::Null)
}

/// __serialize(): [flags, elements, other properties].
fn dll_magic_serialize(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let list = Value::Array(Rc::new(PhpArray::from_list(dll(this))));
    let parts = vec![Value::Int(dll_flags(this)), list, Value::Array(Rc::new(dll_members(this)))];
    Ok(Value::Array(Rc::new(PhpArray::from_list(parts))))
}

/// __unserialize(): what __serialize gave, back.
fn dll_magic_unserialize(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let data = array_of(arg(args, 0));
    let (Some(Value::Int(flags)), Some(Value::Array(list)), Some(Value::Array(members))) =
        (data.get_int(0).map(Value::deref), data.get_int(1).map(Value::deref), data.get_int(2).map(Value::deref))
    else {
        return Err(ctx.throw("UnexpectedValueException", "Incomplete or ill-typed serialization data".into()));
    };
    set(this, "flags", Value::Int(flags));
    put_dll(this, list.values().map(Value::deref).collect());
    for (k, v) in members.iter() {
        let key = match &k {
            Key::Int(i) => i.to_string().into_bytes(),
            Key::Str(s) => s.as_bytes().to_vec(),
        };
        // A mangled name ("\0Class\0p", "\0*\0p") is a declared property's.
        let name = key.iter().rposition(|&c| c == 0).map_or(key.as_slice(), |i| &key[i + 1..]);
        if this.class.slots.contains_key(name) {
            this.class.write(this, name, this.class.name.as_bytes(), v.clone());
        } else {
            this.dynamic.borrow_mut().insert(Key::from_string(PhpStr::from(name)), v.clone());
        }
    }
    Ok(Value::Null)
}

fn dll_push(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut l = dll(this);
    l.push(arg(args, 0));
    put_dll(this, l);
    Ok(Value::Null)
}

fn dll_unshift(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut l = dll(this);
    l.insert(0, arg(args, 0));
    put_dll(this, l);
    Ok(Value::Null)
}

fn dll_pop(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let mut l = dll(this);
    let Some(v) = l.pop() else { return Err(ctx.throw("RuntimeException", "Can't pop from an empty datastructure".into())) };
    put_dll(this, l);
    Ok(v)
}

fn dll_shift(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let mut l = dll(this);
    if l.is_empty() {
        return Err(ctx.throw("RuntimeException", "Can't shift from an empty datastructure".into()));
    }
    let v = l.remove(0);
    put_dll(this, l);
    Ok(v)
}

fn dll_top(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    match dll(this).last() {
        Some(v) => Ok(v.clone()),
        None => Err(ctx.throw("RuntimeException", "Can't peek at an empty datastructure".into())),
    }
}

fn dll_bottom(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    match dll(this).first() {
        Some(v) => Ok(v.clone()),
        None => Err(ctx.throw("RuntimeException", "Can't peek at an empty datastructure".into())),
    }
}

fn dll_is_empty(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(dll(this).is_empty()))
}

fn dll_count(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(dll(this).len() as i64))
}

/// An index into the list (`what` names the method for the error).
fn dll_index(ctx: &mut dyn Ctx, len: usize, k: &Value, what: &str, inclusive: bool) -> Result<usize, Throw> {
    let i = match k.deref() {
        Value::Int(i) => i,
        Value::Str(s) => match p8_runtime::numeric::numeric_string(s.as_bytes()) {
            Some(p8_runtime::numeric::Number::Int(i)) => i,
            _ => -1,
        },
        Value::Float(f) => f as i64,
        Value::Bool(b) => i64::from(b),
        _ => -1,
    };
    let limit = if inclusive { len as i64 } else { len as i64 - 1 };
    if i < 0 || i > limit {
        return Err(ctx.throw("OutOfRangeException", format!("SplDoublyLinkedList::{what}(): Argument #1 ($index) is out of range")));
    }
    Ok(i as usize)
}

fn dll_offset_exists(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let i = arg(args, 0).to_int();
    Ok(Value::Bool(i >= 0 && (i as usize) < dll(this).len()))
}

fn dll_offset_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let l = dll(this);
    let i = dll_index(ctx, l.len(), &arg(args, 0), "offsetGet", false)?;
    Ok(l[i].clone())
}

fn dll_offset_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut l = dll(this);
    match arg(args, 0) {
        Value::Null => l.push(arg(args, 1)),
        k => {
            let i = dll_index(ctx, l.len(), &k, "offsetSet", false)?;
            l[i] = arg(args, 1);
        }
    }
    put_dll(this, l);
    Ok(Value::Null)
}

fn dll_offset_unset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut l = dll(this);
    let i = dll_index(ctx, l.len(), &arg(args, 0), "offsetUnset", false)?;
    l.remove(i);
    put_dll(this, l);
    Ok(Value::Null)
}

fn dll_add(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut l = dll(this);
    let i = dll_index(ctx, l.len(), &arg(args, 0), "add", true)?;
    l.insert(i, arg(args, 1));
    put_dll(this, l);
    Ok(Value::Null)
}

fn dll_set_iterator_mode(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mode = arg(args, 0).to_int();
    let f = dll_flags(this);
    if f & IT_FIX != 0 && (f & IT_MODE_LIFO) != (mode & IT_MODE_LIFO) {
        return Err(ctx.throw("RuntimeException", "Iterators' LIFO/FIFO modes for SplStack/SplQueue objects are frozen".into()));
    }
    let new = (mode & (IT_MODE_LIFO | IT_MODE_DELETE)) | (f & IT_FIX);
    set(this, "flags", Value::Int(new));
    Ok(Value::Int(new))
}

fn dll_get_iterator_mode(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(dll_flags(this) & !IT_FIX))
}

fn dll_rewind(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let len = dll(this).len() as i64;
    let lifo = dll_flags(this) & IT_MODE_LIFO != 0;
    set(this, "\0pos", Value::Int(if lifo { len - 1 } else { 0 }));
    Ok(Value::Null)
}

fn dll_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let pos = int(this, "\0pos");
    Ok(Value::Bool(pos >= 0 && (pos as usize) < dll(this).len()))
}

fn dll_current(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let pos = int(this, "\0pos");
    Ok(if pos < 0 { Value::Null } else { dll(this).get(pos as usize).cloned().unwrap_or(Value::Null) })
}

fn dll_key(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(int(this, "\0pos")))
}

fn dll_step(this: &Object, forward: bool) {
    let f = dll_flags(this);
    let lifo = f & IT_MODE_LIFO != 0;
    let pos = int(this, "\0pos");
    if f & IT_MODE_DELETE != 0 {
        // Delete mode: the current element goes; the next is at the same
        // end.
        let mut l = dll(this);
        if pos >= 0 && (pos as usize) < l.len() {
            l.remove(pos as usize);
        }
        let len = l.len() as i64;
        put_dll(this, l);
        set(this, "\0pos", Value::Int(if lifo { len - 1 } else { 0 }));
        return;
    }
    let step = if lifo == forward { -1 } else { 1 };
    set(this, "\0pos", Value::Int(pos + step));
}

fn dll_next(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    dll_step(this, true);
    Ok(Value::Null)
}

fn dll_prev(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    dll_step(this, false);
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// SplHeap (SplMinHeap, SplMaxHeap) and SplPriorityQueue: private "flags",
// "isCorrupted" and "heap" (a binary heap, in heap order). compare() is a
// method a subclass may override, so it is called as PHP code.

const HEAP_FLAGS: (&str, Constant, Visibility) = ("flags", Constant::Int(0), PRIVATE);
const PQ_FLAGS: (&str, Constant, Visibility) = ("flags", Constant::Int(1), PRIVATE);
const CORRUPTED: (&str, Constant, Visibility) = ("isCorrupted", Constant::Bool(false), PRIVATE);
const HEAP: (&str, Constant, Visibility) = ("heap", Constant::Null, PRIVATE);

fn heap(o: &Object) -> Vec<Value> {
    array_of(get(o, "heap")).values().cloned().collect()
}

fn put_heap(o: &Object, h: Vec<Value>) {
    set(o, "heap", Value::Array(Rc::new(PhpArray::from_list(h))));
}

fn is_pq(o: &Object) -> bool {
    o.class.is_a(b"splpriorityqueue")
}

/// What compare() looks at: the element, or a queue entry's priority.
fn compared(o: &Object, v: &Value) -> Value {
    if is_pq(o) {
        array_of(v.clone()).get(&Key::from_string(PhpStr::from("priority"))).cloned().unwrap_or(Value::Null)
    } else {
        v.clone()
    }
}

fn cmp(ctx: &mut dyn Ctx, this: &Rc<Object>, a: &Value, b: &Value) -> Result<i64, Throw> {
    let (x, y) = (compared(this, a), compared(this, b));
    match call_this(ctx, this, "compare", &mut [x, y]) {
        Ok(v) => Ok(v.to_int()),
        Err(e) => {
            set(this, "isCorrupted", Value::Bool(true));
            Err(e)
        }
    }
}

fn check_heap(ctx: &mut dyn Ctx, this: &Object) -> Result<(), Throw> {
    if get(this, "isCorrupted").to_bool() {
        return Err(ctx.throw("RuntimeException", "Heap is corrupted, heap properties are no longer ensured.".into()));
    }
    Ok(())
}

fn heap_insert_value(ctx: &mut dyn Ctx, this: &Rc<Object>, v: Value) -> Result<(), Throw> {
    check_heap(ctx, this)?;
    let mut h = heap(this);
    h.push(v);
    let mut i = h.len() - 1;
    while i > 0 {
        let p = (i - 1) / 2;
        if cmp(ctx, this, &h[i], &h[p])? > 0 {
            h.swap(i, p);
            i = p;
        } else {
            break;
        }
    }
    put_heap(this, h);
    Ok(())
}

fn heap_extract_value(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<Value, Throw> {
    check_heap(ctx, this)?;
    let mut h = heap(this);
    if h.is_empty() {
        return Err(ctx.throw("RuntimeException", "Can't extract from an empty heap".into()));
    }
    let top = h.swap_remove(0);
    let n = h.len();
    let mut i = 0;
    loop {
        let (l, r) = (2 * i + 1, 2 * i + 2);
        let mut best = i;
        if l < n && cmp(ctx, this, &h[l], &h[best])? > 0 {
            best = l;
        }
        if r < n && cmp(ctx, this, &h[r], &h[best])? > 0 {
            best = r;
        }
        if best == i {
            break;
        }
        h.swap(i, best);
        i = best;
    }
    put_heap(this, h);
    Ok(top)
}

/// A queue entry as its extract flags give it.
fn pq_out(this: &Object, entry: &Value) -> Value {
    let e = array_of(entry.clone());
    let data = e.get(&Key::from_string(PhpStr::from("data"))).cloned().unwrap_or(Value::Null);
    let priority = e.get(&Key::from_string(PhpStr::from("priority"))).cloned().unwrap_or(Value::Null);
    match int(this, "flags") & 3 {
        2 => priority,
        3 => entry.clone(),
        _ => data,
    }
}

fn heap_insert(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    heap_insert_value(ctx, this, arg(args, 0))?;
    Ok(Value::Bool(true))
}

fn heap_extract(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let v = heap_extract_value(ctx, this)?;
    Ok(if is_pq(this) { pq_out(this, &v) } else { v })
}

fn heap_top(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    check_heap(ctx, this)?;
    match heap(this).first() {
        Some(v) => Ok(if is_pq(this) { pq_out(this, v) } else { v.clone() }),
        None => Err(ctx.throw("RuntimeException", "Can't peek at an empty heap".into())),
    }
}

fn heap_count(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(heap(this).len() as i64))
}

fn heap_is_empty(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(heap(this).is_empty()))
}

fn heap_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(!heap(this).is_empty()))
}

fn heap_current(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(heap(this).first().map_or(Value::Null, |v| if is_pq(this) { pq_out(this, v) } else { v.clone() }))
}

fn heap_key(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(heap(this).len() as i64 - 1))
}

fn heap_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    if !heap(this).is_empty() {
        heap_extract_value(ctx, this)?;
    }
    Ok(Value::Null)
}

fn heap_recover(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    set(this, "isCorrupted", Value::Bool(false));
    Ok(Value::Bool(true))
}

fn heap_is_corrupted(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "isCorrupted"))
}

/// $a <=> $b (PHP's comparison).
fn spaceship(a: &Value, b: &Value) -> i64 {
    match a.compare(b) {
        Some(std::cmp::Ordering::Less) => -1,
        Some(std::cmp::Ordering::Greater) => 1,
        _ => 0,
    }
}

fn min_compare(_: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(spaceship(&arg(args, 1), &arg(args, 0))))
}

fn max_compare(_: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(spaceship(&arg(args, 0), &arg(args, 1))))
}

fn pq_insert(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut e = PhpArray::new();
    e.insert(Key::from_string(PhpStr::from("data")), arg(args, 0));
    e.insert(Key::from_string(PhpStr::from("priority")), arg(args, 1));
    heap_insert_value(ctx, this, Value::Array(Rc::new(e)))?;
    Ok(Value::Bool(true))
}

fn pq_set_extract_flags(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let f = arg(args, 0).to_int() & 3;
    if f == 0 {
        return Err(ctx.throw("RuntimeException", "Must specify at least one extract flag".into()));
    }
    set(this, "flags", Value::Int(f));
    Ok(Value::Int(f))
}

fn pq_get_extract_flags(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(int(this, "flags")))
}

// ---------------------------------------------------------------------------
// Functions

/// The class an object or name gives (after autoloading when asked).
fn class_from(ctx: &mut dyn Ctx, v: &Value, autoload: bool, func: &str) -> Result<Option<Rc<p8_runtime::object::Class>>, Throw> {
    match v {
        Value::Object(o) => Ok(Some(Rc::clone(&o.class))),
        other => {
            let name = other.to_php_string().unwrap_or_else(PhpStr::empty);
            let found = if autoload { ctx.class_load(name.as_bytes())? } else { ctx.class(name.as_bytes()) };
            if found.is_none() {
                let what = if autoload { " and could not be loaded" } else { "" };
                ctx.warn(&format!("{func}(): Class {} does not exist{what}", String::from_utf8_lossy(name.as_bytes())));
            }
            Ok(found)
        }
    }
}

/// class_implements(object|string $object_or_class, bool $autoload = true): name => name
fn class_implements(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let autoload = args.len() < 2 || arg(args, 1).to_bool();
    let Some(class) = class_from(ctx, &arg(args, 0), autoload, "class_implements")? else { return Ok(Value::Bool(false)) };
    let mut out = PhpArray::new();
    for i in &class.interfaces {
        let name = ctx.class(i).map_or_else(|| PhpStr::from(i.as_slice()), |c| c.name.clone());
        out.insert(Key::from_string(name.clone()), Value::Str(name));
    }
    Ok(Value::Array(Rc::new(out)))
}

/// class_parents(object|string $object_or_class, bool $autoload = true): name => name
fn class_parents(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let autoload = args.len() < 2 || arg(args, 1).to_bool();
    let Some(class) = class_from(ctx, &arg(args, 0), autoload, "class_parents")? else { return Ok(Value::Bool(false)) };
    let mut out = PhpArray::new();
    let mut c = class.parent.clone();
    while let Some(p) = c {
        out.insert(Key::from_string(p.name.clone()), Value::Str(p.name.clone()));
        c = p.parent.clone();
    }
    Ok(Value::Array(Rc::new(out)))
}

/// iterator_apply(Traversable $iterator, callable $callback, ?array $args = null): the count
fn iterator_apply(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(it) = arg(args, 0) else {
        return Err(ctx.throw("TypeError", "iterator_apply(): Argument #1 ($iterator) must be of type Traversable".into()));
    };
    let callback = arg(args, 1);
    let call_args: Vec<Value> = match arg(args, 2) {
        Value::Array(a) => a.values().cloned().collect(),
        _ => Vec::new(),
    };
    call_this(ctx, &it, "rewind", &mut [])?;
    let mut n = 0;
    while call_this(ctx, &it, "valid", &mut [])?.to_bool() {
        n += 1;
        let mut a = call_args.clone();
        if !ctx.call(&callback, &mut a)?.to_bool() {
            break;
        }
        call_this(ctx, &it, "next", &mut [])?;
    }
    Ok(Value::Int(n))
}

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "class_implements", run: class_implements, by_ref: &[] },
    Builtin { name: "class_parents", run: class_parents, by_ref: &[] },
    Builtin { name: "iterator_apply", run: iterator_apply, by_ref: &[] },
];

/// They call PHP code (autoloaders, the callback).
pub const REENTER: &[&str] = &["class_implements", "class_parents", "iterator_apply"];

// ---------------------------------------------------------------------------
// The classes


const ARRAY_FLAGS: &[(&str, Constant)] = &[("STD_PROP_LIST", Constant::Int(1)), ("ARRAY_AS_PROPS", Constant::Int(2))];

const DLL_METHODS: &[(&str, Option<Native>)] = &[
    ("push", Some(Native::Method(dll_push))),
    ("pop", Some(Native::Method(dll_pop))),
    ("shift", Some(Native::Method(dll_shift))),
    ("unshift", Some(Native::Method(dll_unshift))),
    ("top", Some(Native::Method(dll_top))),
    ("bottom", Some(Native::Method(dll_bottom))),
    ("isEmpty", Some(Native::Method(dll_is_empty))),
    ("count", Some(Native::Method(dll_count))),
    ("offsetExists", Some(Native::Method(dll_offset_exists))),
    ("offsetGet", Some(Native::Method(dll_offset_get))),
    ("offsetSet", Some(Native::Method(dll_offset_set))),
    ("offsetUnset", Some(Native::Method(dll_offset_unset))),
    ("add", Some(Native::Method(dll_add))),
    ("setIteratorMode", Some(Native::Method(dll_set_iterator_mode))),
    ("getIteratorMode", Some(Native::Method(dll_get_iterator_mode))),
    ("rewind", Some(Native::Method(dll_rewind))),
    ("valid", Some(Native::Method(dll_valid))),
    ("current", Some(Native::Method(dll_current))),
    ("key", Some(Native::Method(dll_key))),
    ("next", Some(Native::Method(dll_next))),
    ("prev", Some(Native::Method(dll_prev))),
    ("serialize", Some(Native::Method(dll_serialize))),
    ("unserialize", Some(Native::Method(dll_unserialize))),
    ("__serialize", Some(Native::Method(dll_magic_serialize))),
    ("__unserialize", Some(Native::Method(dll_magic_unserialize))),
];

const HEAP_METHODS: &[(&str, Option<Native>)] = &[
    ("insert", Some(Native::Method(heap_insert))),
    ("extract", Some(Native::Method(heap_extract))),
    ("top", Some(Native::Method(heap_top))),
    ("count", Some(Native::Method(heap_count))),
    ("isEmpty", Some(Native::Method(heap_is_empty))),
    ("rewind", Some(Native::Method(no_op))),
    ("valid", Some(Native::Method(heap_valid))),
    ("current", Some(Native::Method(heap_current))),
    ("key", Some(Native::Method(heap_key))),
    ("next", Some(Native::Method(heap_next))),
    ("recoverFromCorruption", Some(Native::Method(heap_recover))),
    ("isCorrupted", Some(Native::Method(heap_is_corrupted))),
    ("compare", None),
];

const fn spec(name: &'static str, parent: Option<&'static str>, interfaces: &'static [&'static str], flags: u32) -> ClassSpec {
    ClassSpec { name, parent, interfaces, flags, properties: &[], methods: &[], constants: &[], debug_info: None }
}

pub static CLASSES: &[ClassSpec] = &[
    ClassSpec { methods: &[("serialize", None), ("unserialize", None)], ..spec("Serializable", None, &[], CLASS_INTERFACE) },
    ClassSpec { methods: &[("seek", None)], ..spec("SeekableIterator", None, &["Iterator"], CLASS_INTERFACE) },
    ClassSpec { methods: &[("hasChildren", None), ("getChildren", None)], ..spec("RecursiveIterator", None, &["Iterator"], CLASS_INTERFACE) },
    ClassSpec { methods: &[("getInnerIterator", None)], ..spec("OuterIterator", None, &["Iterator"], CLASS_INTERFACE) },
    ClassSpec { methods: &[("update", None)], ..spec("SplObserver", None, &[], CLASS_INTERFACE) },
    ClassSpec { methods: &[("attach", None), ("detach", None), ("notify", None)], ..spec("SplSubject", None, &[], CLASS_INTERFACE) },
    ClassSpec {
        properties: &[STORAGE, H_FLAGS, H_ITCLASS],
        methods: &[
            ("__construct", Some(Native::Method(ao_construct))),
            ("offsetExists", Some(Native::Method(ao_offset_exists))),
            ("offsetGet", Some(Native::Method(ao_offset_get))),
            ("offsetSet", Some(Native::Method(ao_offset_set))),
            ("offsetUnset", Some(Native::Method(ao_offset_unset))),
            ("append", Some(Native::Method(ao_append))),
            ("count", Some(Native::Method(ao_count))),
            ("getArrayCopy", Some(Native::Method(ao_get_array_copy))),
            ("exchangeArray", Some(Native::Method(ao_exchange_array))),
            ("getFlags", Some(Native::Method(ao_get_flags))),
            ("setFlags", Some(Native::Method(ao_set_flags))),
            ("getIterator", Some(Native::Method(ao_get_iterator))),
            ("getIteratorClass", Some(Native::Method(ao_get_iterator_class))),
            ("setIteratorClass", Some(Native::Method(ao_set_iterator_class))),
            ("asort", Some(Native::Method(ao_asort))),
            ("ksort", Some(Native::Method(ao_ksort))),
            ("uasort", Some(Native::Method(ao_uasort))),
            ("uksort", Some(Native::Method(ao_uksort))),
            ("natsort", Some(Native::Method(ao_natsort))),
            ("natcasesort", Some(Native::Method(ao_natcasesort))),
            ("serialize", Some(Native::Method(ao_serialize))),
            ("unserialize", Some(Native::Method(ao_unserialize))),
        ],
        constants: ARRAY_FLAGS,
        ..spec("ArrayObject", None, &["IteratorAggregate", "ArrayAccess", "Serializable", "Countable"], 0)
    },
    ClassSpec {
        properties: &[STORAGE, H_FLAGS, H_POS],
        methods: &[
            ("__construct", Some(Native::Method(ao_construct))),
            ("offsetExists", Some(Native::Method(ao_offset_exists))),
            ("offsetGet", Some(Native::Method(ao_offset_get))),
            ("offsetSet", Some(Native::Method(ao_offset_set))),
            ("offsetUnset", Some(Native::Method(ao_offset_unset))),
            ("append", Some(Native::Method(ao_append))),
            ("count", Some(Native::Method(ao_count))),
            ("getArrayCopy", Some(Native::Method(ao_get_array_copy))),
            ("getFlags", Some(Native::Method(ao_get_flags))),
            ("setFlags", Some(Native::Method(ao_set_flags))),
            ("asort", Some(Native::Method(ao_asort))),
            ("ksort", Some(Native::Method(ao_ksort))),
            ("uasort", Some(Native::Method(ao_uasort))),
            ("uksort", Some(Native::Method(ao_uksort))),
            ("natsort", Some(Native::Method(ao_natsort))),
            ("natcasesort", Some(Native::Method(ao_natcasesort))),
            ("serialize", Some(Native::Method(ao_serialize))),
            ("unserialize", Some(Native::Method(ao_unserialize))),
            ("current", Some(Native::Method(ai_current))),
            ("key", Some(Native::Method(ai_key))),
            ("next", Some(Native::Method(ai_next))),
            ("rewind", Some(Native::Method(ai_rewind))),
            ("valid", Some(Native::Method(ai_valid))),
            ("seek", Some(Native::Method(ai_seek))),
        ],
        constants: ARRAY_FLAGS,
        ..spec("ArrayIterator", None, &["SeekableIterator", "ArrayAccess", "Serializable", "Countable"], 0)
    },
    ClassSpec {
        methods: &[("hasChildren", Some(Native::Method(rai_has_children))), ("getChildren", Some(Native::Method(rai_get_children)))],
        constants: &[("CHILD_ARRAYS_ONLY", Constant::Int(4))],
        ..spec("RecursiveArrayIterator", Some("ArrayIterator"), &["RecursiveIterator"], 0)
    },
    ClassSpec {
        properties: &[H_DATA],
        methods: &[
            ("__construct", Some(Native::Method(fa_construct))),
            ("offsetExists", Some(Native::Method(fa_offset_exists))),
            ("offsetGet", Some(Native::Method(fa_offset_get))),
            ("offsetSet", Some(Native::Method(fa_offset_set))),
            ("offsetUnset", Some(Native::Method(fa_offset_unset))),
            ("count", Some(Native::Method(fa_count))),
            ("getSize", Some(Native::Method(fa_count))),
            ("setSize", Some(Native::Method(fa_set_size))),
            ("toArray", Some(Native::Method(fa_to_array))),
            ("jsonSerialize", Some(Native::Method(fa_to_array))),
            ("fromArray", Some(Native::Static(fa_from_array))),
            ("getIterator", Some(Native::Method(fa_get_iterator))),
            ("__wakeup", Some(Native::Method(no_op))),
        ],
        debug_info: Some(fixed_info),
        ..spec("SplFixedArray", None, &["IteratorAggregate", "ArrayAccess", "Countable", "JsonSerializable"], 0)
    },
    ClassSpec {
        properties: &[H_MAP, H_POS, H_INDEX],
        methods: &[
            ("attach", Some(Native::Method(os_attach))),
            ("detach", Some(Native::Method(os_detach))),
            ("contains", Some(Native::Method(os_contains))),
            ("addAll", Some(Native::Method(os_add_all))),
            ("removeAll", Some(Native::Method(os_remove_all))),
            ("removeAllExcept", Some(Native::Method(os_remove_all_except))),
            ("getInfo", Some(Native::Method(os_get_info))),
            ("setInfo", Some(Native::Method(os_set_info))),
            ("count", Some(Native::Method(os_count))),
            ("rewind", Some(Native::Method(os_rewind))),
            ("valid", Some(Native::Method(os_valid))),
            ("key", Some(Native::Method(os_key))),
            ("current", Some(Native::Method(os_current))),
            ("next", Some(Native::Method(os_next))),
            ("offsetExists", Some(Native::Method(os_offset_exists))),
            ("offsetGet", Some(Native::Method(os_offset_get))),
            ("offsetSet", Some(Native::Method(os_offset_set))),
            ("offsetUnset", Some(Native::Method(os_offset_unset))),
            ("getHash", Some(Native::Method(os_get_hash))),
        ],
        debug_info: Some(store_info),
        ..spec("SplObjectStorage", None, &["Countable", "Iterator", "Serializable", "ArrayAccess"], 0)
    },
    ClassSpec {
        properties: &[DLL_FLAGS, DLLIST, H_POS],
        methods: DLL_METHODS,
        constants: &[("IT_MODE_LIFO", Constant::Int(2)), ("IT_MODE_FIFO", Constant::Int(0)), ("IT_MODE_DELETE", Constant::Int(1)), ("IT_MODE_KEEP", Constant::Int(0))],
        ..spec("SplDoublyLinkedList", None, &["Iterator", "Countable", "ArrayAccess", "Serializable"], 0)
    },
    ClassSpec { methods: &[("enqueue", Some(Native::Method(dll_push))), ("dequeue", Some(Native::Method(dll_shift)))], ..spec("SplQueue", Some("SplDoublyLinkedList"), &[], 0) },
    spec("SplStack", Some("SplDoublyLinkedList"), &[], 0),
    ClassSpec { properties: &[HEAP_FLAGS, CORRUPTED, HEAP], methods: HEAP_METHODS, ..spec("SplHeap", None, &["Iterator", "Countable"], CLASS_ABSTRACT) },
    ClassSpec { methods: &[("compare", Some(Native::Method(min_compare)))], ..spec("SplMinHeap", Some("SplHeap"), &[], 0) },
    ClassSpec { methods: &[("compare", Some(Native::Method(max_compare)))], ..spec("SplMaxHeap", Some("SplHeap"), &[], 0) },
    ClassSpec {
        properties: &[PQ_FLAGS, CORRUPTED, HEAP],
        methods: &[
            ("insert", Some(Native::Method(pq_insert))),
            ("extract", Some(Native::Method(heap_extract))),
            ("top", Some(Native::Method(heap_top))),
            ("count", Some(Native::Method(heap_count))),
            ("isEmpty", Some(Native::Method(heap_is_empty))),
            ("rewind", Some(Native::Method(no_op))),
            ("valid", Some(Native::Method(heap_valid))),
            ("current", Some(Native::Method(heap_current))),
            ("key", Some(Native::Method(heap_key))),
            ("next", Some(Native::Method(heap_next))),
            ("recoverFromCorruption", Some(Native::Method(heap_recover))),
            ("isCorrupted", Some(Native::Method(heap_is_corrupted))),
            ("setExtractFlags", Some(Native::Method(pq_set_extract_flags))),
            ("getExtractFlags", Some(Native::Method(pq_get_extract_flags))),
            ("compare", Some(Native::Method(max_compare))),
        ],
        constants: &[("EXTR_BOTH", Constant::Int(3)), ("EXTR_PRIORITY", Constant::Int(2)), ("EXTR_DATA", Constant::Int(1))],
        ..spec("SplPriorityQueue", None, &["Iterator", "Countable"], 0)
    },
];

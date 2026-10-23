//! SPL's iterators: IteratorIterator, FilterIterator,
//! CallbackFilterIterator, RecursiveFilterIterator,
//! RecursiveCallbackFilterIterator, ParentIterator, LimitIterator,
//! CachingIterator, RecursiveCachingIterator, NoRewindIterator,
//! InfiniteIterator, AppendIterator, EmptyIterator, RecursiveIteratorIterator.
//!
//! They wrap an inner iterator (called as PHP code: a user class, a
//! builtin, a Generator) and, as PHP does, fetch its valid/current/key
//! once after each move and answer from that ("\0valid", "\0cur", "\0key").
//!
//! Reference: $PHP_SRC_PATH/ext/spl/spl_iterators.c (spl_dual_it_*,
//! spl_recursive_it_move_forward_ex)

use crate::arg;
use crate::spl::{array_of, call_this, get, int, set};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::{CLASS_ABSTRACT, Native, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

const PUBLIC: Visibility = Visibility::Public;

fn inner(o: &Object) -> Option<Rc<Object>> {
    match get(o, "\0inner") {
        Value::Object(i) => Some(i),
        _ => None,
    }
}

/// A method of the inner iterator.
fn on_inner(ctx: &mut dyn Ctx, o: &Object, name: &str, args: &mut [Value]) -> Result<Value, Throw> {
    match inner(o) {
        Some(i) => call_this(ctx, &i, name, args),
        None => Err(ctx.throw("LogicException", "The object is in an invalid state as the parent constructor was not called".into())),
    }
}

/// An Iterator from a Traversable (an IteratorAggregate's getIterator(), in turn).
fn iterator_of(ctx: &mut dyn Ctx, v: Value, class: &str) -> Result<Rc<Object>, Throw> {
    let mut v = v;
    loop {
        match v {
            Value::Object(o) if o.class.is_a(b"iterator") => return Ok(o),
            Value::Object(o) if o.class.is_a(b"iteratoraggregate") => {
                v = call_this(ctx, &o, "getIterator", &mut [])?;
                if !matches!(&v, Value::Object(i) if i.class.is_a(b"traversable")) {
                    return Err(ctx.throw(
                        "LogicException",
                        format!("{}::getIterator() must return an object that implements Traversable", String::from_utf8_lossy(o.class.name.as_bytes())),
                    ));
                }
            }
            other => {
                return Err(ctx.throw("TypeError", format!("{class}::__construct(): Argument #1 ($iterator) must be of type Traversable, {} given", other.type_name())));
            }
        }
    }
}

/// Fetch the inner's valid/current/key (spl_dual_it_fetch).
fn fetch(ctx: &mut dyn Ctx, o: &Object) -> Result<bool, Throw> {
    let valid = on_inner(ctx, o, "valid", &mut [])?.to_bool();
    set(o, "\0valid", Value::Bool(valid));
    if valid {
        let cur = on_inner(ctx, o, "current", &mut [])?;
        let key = on_inner(ctx, o, "key", &mut [])?;
        set(o, "\0cur", cur.deref());
        set(o, "\0key", key.deref());
    } else {
        set(o, "\0cur", Value::Null);
        set(o, "\0key", Value::Null);
    }
    Ok(valid)
}

fn class_of(o: &Object) -> String {
    String::from_utf8_lossy(o.class.name.as_bytes()).into_owned()
}

// ---------------------------------------------------------------------------
// IteratorIterator (and its subclasses' shared moves)

fn ii_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = class_of(this);
    let it = iterator_of(ctx, arg(args, 0), &class)?;
    set(this, "\0inner", Value::Object(it));
    Ok(Value::Null)
}

fn ii_get_inner(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0inner"))
}

fn ii_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "rewind", &mut [])?;
    set(this, "\0pos", Value::Int(0));
    fetch(ctx, this)?;
    Ok(Value::Null)
}

fn ii_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(get(this, "\0valid").to_bool()))
}

fn ii_current(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0cur"))
}

fn ii_key(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0key"))
}

fn ii_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "next", &mut [])?;
    set(this, "\0pos", Value::Int(int(this, "\0pos") + 1));
    fetch(ctx, this)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// FilterIterator: moves on until accept() (PHP code) says yes.

fn fetch_accepted(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<(), Throw> {
    while fetch(ctx, this)? {
        if call_this(ctx, this, "accept", &mut [])?.to_bool() {
            return Ok(());
        }
        on_inner(ctx, this, "next", &mut [])?;
    }
    Ok(())
}

fn fi_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "rewind", &mut [])?;
    fetch_accepted(ctx, this)?;
    Ok(Value::Null)
}

fn fi_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "next", &mut [])?;
    fetch_accepted(ctx, this)?;
    Ok(Value::Null)
}

fn cfi_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    ii_construct(ctx, this, args)?;
    set(this, "\0cb", arg(args, 1));
    Ok(Value::Null)
}

/// CallbackFilterIterator::accept(): callback($current, $key, $iterator).
fn cfi_accept(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let cb = get(this, "\0cb");
    let mut a = [get(this, "\0cur"), get(this, "\0key"), get(this, "\0inner")];
    Ok(Value::Bool(ctx.call(&cb, &mut a)?.to_bool()))
}

fn rfi_has_children(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "hasChildren", &mut [])
}

/// RecursiveFilterIterator::getChildren(): a new one of this class, on
/// the inner's children (with the callback, for the callback kind).
fn rfi_get_children(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let children = on_inner(ctx, this, "getChildren", &mut [])?;
    let class = Rc::clone(&this.class);
    let mut a = vec![children];
    if this.class.is_a(b"recursivecallbackfilteriterator") {
        a.push(get(this, "\0cb"));
    }
    Ok(Value::Object(ctx.new_instance(&class, &mut a, true)?))
}

/// ParentIterator::accept(): only elements with children.
fn pi_accept(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "hasChildren", &mut [])
}

// ---------------------------------------------------------------------------
// LimitIterator: "\0offset", "\0count" (-1: all), "\0pos".

fn li_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    ii_construct(ctx, this, args)?;
    let offset = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    let count = if args.len() > 2 { arg(args, 2).to_int() } else { -1 };
    if offset < 0 {
        return Err(ctx.throw("ValueError", "LimitIterator::__construct(): Argument #2 ($offset) must be greater than or equal to 0".into()));
    }
    if count < -1 {
        return Err(ctx.throw("ValueError", "LimitIterator::__construct(): Argument #3 ($limit) must be greater than or equal to -1".into()));
    }
    set(this, "\0offset", Value::Int(offset));
    set(this, "\0count", Value::Int(count));
    Ok(Value::Null)
}

fn li_seek_to(ctx: &mut dyn Ctx, this: &Rc<Object>, pos: i64) -> Result<(), Throw> {
    let (offset, count) = (int(this, "\0offset"), int(this, "\0count"));
    if pos < offset {
        return Err(ctx.throw("OutOfBoundsException", format!("Cannot seek to {pos} which is below the offset {offset}")));
    }
    if count != -1 && pos >= offset + count {
        return Err(ctx.throw("OutOfBoundsException", format!("Cannot seek to {pos} which is behind offset {offset} plus count {count}")));
    }
    let seekable = inner(this).is_some_and(|i| i.class.is_a(b"seekableiterator"));
    if pos != int(this, "\0pos") && seekable {
        on_inner(ctx, this, "seek", &mut [Value::Int(pos)])?;
        set(this, "\0pos", Value::Int(pos));
        fetch(ctx, this)?;
        return Ok(());
    }
    if pos < int(this, "\0pos") {
        on_inner(ctx, this, "rewind", &mut [])?;
        set(this, "\0pos", Value::Int(0));
        fetch(ctx, this)?;
    }
    while int(this, "\0pos") < pos && get(this, "\0valid").to_bool() {
        on_inner(ctx, this, "next", &mut [])?;
        set(this, "\0pos", Value::Int(int(this, "\0pos") + 1));
        fetch(ctx, this)?;
    }
    Ok(())
}

fn li_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "rewind", &mut [])?;
    set(this, "\0pos", Value::Int(0));
    fetch(ctx, this)?;
    let offset = int(this, "\0offset");
    li_seek_to(ctx, this, offset)?;
    Ok(Value::Null)
}

fn li_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (offset, count, pos) = (int(this, "\0offset"), int(this, "\0count"), int(this, "\0pos"));
    Ok(Value::Bool((count == -1 || pos < offset + count) && get(this, "\0valid").to_bool()))
}

fn li_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (offset, count) = (int(this, "\0offset"), int(this, "\0count"));
    on_inner(ctx, this, "next", &mut [])?;
    let pos = int(this, "\0pos") + 1;
    set(this, "\0pos", Value::Int(pos));
    if count == -1 || pos < offset + count {
        fetch(ctx, this)?;
    }
    Ok(Value::Null)
}

fn li_seek(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    li_seek_to(ctx, this, arg(args, 0).to_int())?;
    Ok(Value::Int(int(this, "\0pos")))
}

fn li_get_position(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(int(this, "\0pos")))
}

// ---------------------------------------------------------------------------
// CachingIterator: one element ahead; "\0flags", "\0cache", "\0str".

const CALL_TOSTRING: i64 = 1;
const TOSTRING_USE_KEY: i64 = 2;
const TOSTRING_USE_CURRENT: i64 = 4;
const TOSTRING_USE_INNER: i64 = 8;
const FULL_CACHE: i64 = 256;
/// Whether the element taken is valid (internal).
const CIT_VALID: i64 = 0x10000;

fn ci_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    ii_construct(ctx, this, args)?;
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { CALL_TOSTRING };
    set(this, "\0flags", Value::Int(flags));
    set(this, "\0cache", Value::Array(Rc::new(PhpArray::new())));
    Ok(Value::Null)
}

/// spl_caching_it_next: take the inner's element, then move the inner on.
fn ci_take(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<(), Throw> {
    let flags = int(this, "\0flags");
    if fetch(ctx, this)? {
        set(this, "\0flags", Value::Int(flags | CIT_VALID));
        if flags & FULL_CACHE != 0 {
            let mut cache = array_of(get(this, "\0cache"));
            if let Ok((k, _)) = Key::from_value(&get(this, "\0key")) {
                cache.insert(k, get(this, "\0cur"));
            }
            set(this, "\0cache", Value::Array(Rc::new(cache)));
        }
        if flags & CALL_TOSTRING != 0 {
            let cur = get(this, "\0cur");
            let text = match &cur {
                Value::Object(o) => call_this(ctx, o, "__toString", &mut [])?,
                v => Value::Str(v.to_php_string().unwrap_or_else(PhpStr::empty)),
            };
            set(this, "\0str", text);
        }
        on_inner(ctx, this, "next", &mut [])?;
    } else {
        set(this, "\0flags", Value::Int(flags & !CIT_VALID));
    }
    Ok(())
}

fn ci_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "rewind", &mut [])?;
    set(this, "\0cache", Value::Array(Rc::new(PhpArray::new())));
    ci_take(ctx, this)?;
    Ok(Value::Null)
}

fn ci_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(int(this, "\0flags") & CIT_VALID != 0))
}

fn ci_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    ci_take(ctx, this)?;
    Ok(Value::Null)
}

fn ci_has_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "valid", &mut [])
}

fn ci_to_string(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let flags = int(this, "\0flags");
    if flags & (CALL_TOSTRING | TOSTRING_USE_KEY | TOSTRING_USE_CURRENT | TOSTRING_USE_INNER) == 0 {
        return Err(ctx.throw("BadMethodCallException", format!("{} does not fetch string value (see CachingIterator::__construct)", class_of(this))));
    }
    let v = if flags & TOSTRING_USE_KEY != 0 {
        get(this, "\0key")
    } else if flags & TOSTRING_USE_CURRENT != 0 {
        get(this, "\0cur")
    } else if flags & TOSTRING_USE_INNER != 0 {
        on_inner(ctx, this, "__toString", &mut [])?
    } else {
        get(this, "\0str")
    };
    Ok(Value::Str(v.to_php_string().unwrap_or_else(PhpStr::empty)))
}

fn ci_full_cache(ctx: &mut dyn Ctx, this: &Object) -> Result<PhpArray, Throw> {
    if int(this, "\0flags") & FULL_CACHE == 0 {
        return Err(ctx.throw("BadMethodCallException", format!("{} does not use a full cache (see CachingIterator::__construct)", class_of(this))));
    }
    Ok(array_of(get(this, "\0cache")))
}

fn ci_get_cache(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Array(Rc::new(ci_full_cache(ctx, this)?)))
}

fn ci_count(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(ci_full_cache(ctx, this)?.len() as i64))
}

fn ci_offset_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let cache = ci_full_cache(ctx, this)?;
    let Ok((k, _)) = Key::from_value(&arg(args, 0)) else { return Ok(Value::Null) };
    match cache.get(&k) {
        Some(v) => Ok(v.deref()),
        None => {
            let shown = match &k {
                Key::Int(i) => i.to_string(),
                Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
            };
            ctx.warn(&format!("Undefined array key {shown}"));
            Ok(Value::Null)
        }
    }
}

fn ci_offset_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut cache = ci_full_cache(ctx, this)?;
    if let Ok((k, _)) = Key::from_value(&arg(args, 0)) {
        cache.insert(k, arg(args, 1));
    }
    set(this, "\0cache", Value::Array(Rc::new(cache)));
    Ok(Value::Null)
}

fn ci_offset_exists(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let cache = ci_full_cache(ctx, this)?;
    Ok(Value::Bool(Key::from_value(&arg(args, 0)).is_ok_and(|(k, _)| cache.contains_key(&k))))
}

fn ci_offset_unset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut cache = ci_full_cache(ctx, this)?;
    if let Ok((k, _)) = Key::from_value(&arg(args, 0)) {
        cache.remove(&k);
    }
    set(this, "\0cache", Value::Array(Rc::new(cache)));
    Ok(Value::Null)
}

fn ci_get_flags(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(int(this, "\0flags") & !CIT_VALID))
}

fn ci_set_flags(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let new = arg(args, 0).to_int();
    let old = int(this, "\0flags");
    if old & CALL_TOSTRING != 0 && new & CALL_TOSTRING == 0 {
        return Err(ctx.throw("InvalidArgumentException", "Unsetting flag CALL_TO_STRING is not possible".into()));
    }
    if old & TOSTRING_USE_INNER != 0 && new & TOSTRING_USE_INNER == 0 {
        return Err(ctx.throw("InvalidArgumentException", "Unsetting flag TOSTRING_USE_INNER is not possible".into()));
    }
    if new & FULL_CACHE != 0 && old & FULL_CACHE == 0 {
        set(this, "\0cache", Value::Array(Rc::new(PhpArray::new())));
    }
    set(this, "\0flags", Value::Int(new | (old & CIT_VALID)));
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// NoRewindIterator, InfiniteIterator, EmptyIterator

fn nri_rewind(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

fn nri_valid(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "valid", &mut [])
}

fn nri_current(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "current", &mut [])
}

fn nri_key(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "key", &mut [])
}

fn nri_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "next", &mut [])?;
    Ok(Value::Null)
}

fn inf_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    on_inner(ctx, this, "next", &mut [])?;
    if !on_inner(ctx, this, "valid", &mut [])?.to_bool() {
        on_inner(ctx, this, "rewind", &mut [])?;
    }
    fetch(ctx, this)?;
    Ok(Value::Null)
}

fn ei_false(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(false))
}

fn ei_current(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(ctx.throw("BadMethodCallException", "Accessing the value of an EmptyIterator".into()))
}

fn ei_key(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(ctx.throw("BadMethodCallException", "Accessing the key of an EmptyIterator".into()))
}

fn ei_none(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// AppendIterator: "\0list" (the iterators), "\0idx" (the one in use).

fn ai_construct(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    set(this, "\0list", Value::Array(Rc::new(PhpArray::new())));
    set(this, "\0idx", Value::Int(0));
    Ok(Value::Null)
}

fn ai_list(o: &Object) -> Vec<Value> {
    array_of(get(o, "\0list")).values().cloned().collect()
}

/// Use iterator `idx` (from its start when `rewind`), or the next ones
/// that have elements.
fn ai_settle(ctx: &mut dyn Ctx, this: &Rc<Object>, mut idx: usize, rewind: bool) -> Result<(), Throw> {
    let list = ai_list(this);
    let mut first = rewind;
    while idx < list.len() {
        set(this, "\0inner", list[idx].clone());
        set(this, "\0idx", Value::Int(idx as i64));
        if first {
            on_inner(ctx, this, "rewind", &mut [])?;
        }
        if fetch(ctx, this)? {
            return Ok(());
        }
        idx += 1;
        first = true;
    }
    set(this, "\0valid", Value::Bool(false));
    Ok(())
}

fn ai_append(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(it) = arg(args, 0) else {
        return Err(ctx.throw("TypeError", "AppendIterator::append(): Argument #1 ($iterator) must be of type Iterator".into()));
    };
    let mut list = array_of(get(this, "\0list"));
    let _ = list.push(Value::Object(Rc::clone(&it)));
    let n = list.len();
    set(this, "\0list", Value::Array(Rc::new(list)));
    // The first one: rewound and used; a later one, used once the current runs out.
    if n == 1 || !get(this, "\0valid").to_bool() {
        let idx = if n == 1 { 0 } else { int(this, "\0idx") as usize };
        if n == 1 {
            call_this(ctx, &it, "rewind", &mut [])?;
        }
        ai_settle(ctx, this, idx, false)?;
    }
    Ok(Value::Null)
}

fn ai_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    ai_settle(ctx, this, 0, true)?;
    Ok(Value::Null)
}

fn ai_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    if inner(this).is_none() {
        return Ok(Value::Null);
    }
    on_inner(ctx, this, "next", &mut [])?;
    if !fetch(ctx, this)? {
        let idx = int(this, "\0idx") as usize + 1;
        ai_settle(ctx, this, idx, true)?;
    }
    Ok(Value::Null)
}

fn ai_get_iterator_index(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(if get(this, "\0valid").to_bool() { Value::Int(int(this, "\0idx")) } else { Value::Null })
}

fn ai_get_array_iterator(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let class = ctx.class(b"ArrayIterator").expect("ArrayIterator");
    Ok(Value::Object(ctx.new_instance(&class, &mut [get(this, "\0list")], true)?))
}

// ---------------------------------------------------------------------------
// RecursiveIteratorIterator: a stack of iterators ("\0stack") and each
// one's state ("\0states"); PHP's spl_recursive_it_move_forward_ex.

const LEAVES_ONLY: i64 = 0;
const SELF_FIRST: i64 = 1;
const CHILD_FIRST: i64 = 2;
const CATCH_GET_CHILD: i64 = 16;

const RS_NEXT: i64 = 0;
const RS_TEST: i64 = 1;
const RS_SELF: i64 = 2;
const RS_CHILD: i64 = 3;
const RS_START: i64 = 4;

fn rii_stack(o: &Object) -> Vec<Rc<Object>> {
    array_of(get(o, "\0stack"))
        .values()
        .filter_map(|v| match v {
            Value::Object(i) => Some(Rc::clone(i)),
            _ => None,
        })
        .collect()
}

fn rii_states(o: &Object) -> Vec<i64> {
    array_of(get(o, "\0states")).values().map(Value::to_int).collect()
}

fn rii_put(o: &Object, stack: &[Rc<Object>], states: &[i64]) {
    set(o, "\0stack", Value::Array(Rc::new(PhpArray::from_list(stack.iter().map(|i| Value::Object(Rc::clone(i))).collect()))));
    set(o, "\0states", Value::Array(Rc::new(PhpArray::from_list(states.iter().map(|s| Value::Int(*s)).collect()))));
}

fn rii_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = class_of(this);
    let it = match arg(args, 0) {
        Value::Object(o) if o.class.is_a(b"iteratoraggregate") => iterator_of(ctx, Value::Object(o), &class)?,
        Value::Object(o) => o,
        other => {
            return Err(ctx.throw("TypeError", format!("{class}::__construct(): Argument #1 ($iterator) must be of type Traversable, {} given", other.type_name())));
        }
    };
    if !it.class.is_a(b"recursiveiterator") {
        return Err(ctx.throw("InvalidArgumentException", "An instance of RecursiveIterator or IteratorAggregate creating it is required".into()));
    }
    let mode = if args.len() > 1 { arg(args, 1).to_int() } else { LEAVES_ONLY };
    let flags = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    set(this, "\0mode", Value::Int(mode));
    set(this, "\0flags", Value::Int(flags));
    set(this, "\0maxdepth", Value::Int(-1));
    set(this, "\0inner", Value::Object(Rc::clone(&it)));
    rii_put(this, &[it], &[RS_START]);
    Ok(Value::Null)
}

/// Whether the class (a subclass) has its own version of a hook.
fn overrides(o: &Object, name: &[u8]) -> bool {
    o.class.method(name).is_some_and(|m| m.native.is_none())
}

fn rii_move(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<(), Throw> {
    let mode = int(this, "\0mode");
    let flags = int(this, "\0flags");
    let max_depth = int(this, "\0maxdepth");
    loop {
        let mut stack = rii_stack(this);
        let mut states = rii_states(this);
        let level = stack.len() - 1;
        let it = Rc::clone(&stack[level]);
        let mut state = states[level];
        if state == RS_NEXT {
            call_this(ctx, &it, "next", &mut [])?;
            state = RS_START;
        }
        if state == RS_START {
            if !call_this(ctx, &it, "valid", &mut [])?.to_bool() {
                // This level is done: up one (or the end).
                if level == 0 {
                    states[level] = RS_START;
                    rii_put(this, &stack, &states);
                    return Ok(());
                }
                if overrides(this, b"endchildren") {
                    call_this(ctx, this, "endChildren", &mut [])?;
                }
                stack.pop();
                states.pop();
                rii_put(this, &stack, &states);
                continue;
            }
            state = RS_TEST;
        }
        if state == RS_TEST {
            let has_children = call_this(ctx, this, "callHasChildren", &mut [])?.to_bool();
            if has_children {
                if max_depth == -1 || max_depth > level as i64 {
                    states[level] = if mode == SELF_FIRST { RS_SELF } else { RS_CHILD };
                    rii_put(this, &stack, &states);
                    continue;
                }
                if mode == LEAVES_ONLY {
                    states[level] = RS_NEXT;
                    rii_put(this, &stack, &states);
                    continue;
                }
            }
            if overrides(this, b"nextelement") {
                call_this(ctx, this, "nextElement", &mut [])?;
            }
            states[level] = RS_NEXT;
            rii_put(this, &stack, &states);
            return Ok(());
        }
        if state == RS_SELF {
            if overrides(this, b"nextelement") && (mode == SELF_FIRST || mode == CHILD_FIRST) {
                call_this(ctx, this, "nextElement", &mut [])?;
            }
            states[level] = if mode == SELF_FIRST { RS_CHILD } else { RS_NEXT };
            rii_put(this, &stack, &states);
            return Ok(());
        }
        if state == RS_CHILD {
            let child = match call_this(ctx, this, "callGetChildren", &mut []) {
                Ok(c) => c,
                Err(e) if flags & CATCH_GET_CHILD == 0 => return Err(e),
                Err(_) => {
                    states[level] = RS_NEXT;
                    rii_put(this, &stack, &states);
                    continue;
                }
            };
            let Value::Object(child) = child else {
                return Err(ctx.throw("UnexpectedValueException", "Objects returned by RecursiveIterator::getChildren() must implement RecursiveIterator".into()));
            };
            if !child.class.is_a(b"recursiveiterator") {
                return Err(ctx.throw("UnexpectedValueException", "Objects returned by RecursiveIterator::getChildren() must implement RecursiveIterator".into()));
            }
            states[level] = if mode == CHILD_FIRST { RS_SELF } else { RS_NEXT };
            stack.push(Rc::clone(&child));
            states.push(RS_START);
            rii_put(this, &stack, &states);
            call_this(ctx, &child, "rewind", &mut [])?;
            if overrides(this, b"beginchildren") {
                call_this(ctx, this, "beginChildren", &mut [])?;
            }
            continue;
        }
        return Ok(());
    }
}

fn rii_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    // Back to the outermost level (its children end).
    let mut stack = rii_stack(this);
    let mut states = rii_states(this);
    while stack.len() > 1 {
        stack.pop();
        states.pop();
        rii_put(this, &stack, &states);
        if overrides(this, b"endchildren") {
            call_this(ctx, this, "endChildren", &mut [])?;
        }
    }
    states[0] = RS_START;
    rii_put(this, &stack, &states);
    call_this(ctx, &stack[0], "rewind", &mut [])?;
    if !get(this, "\0started").to_bool() && overrides(this, b"beginiteration") {
        call_this(ctx, this, "beginIteration", &mut [])?;
    }
    set(this, "\0started", Value::Bool(true));
    rii_move(ctx, this)?;
    Ok(Value::Null)
}

fn rii_valid(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    for it in rii_stack(this).iter().rev() {
        if call_this(ctx, it, "valid", &mut [])?.to_bool() {
            return Ok(Value::Bool(true));
        }
    }
    if get(this, "\0started").to_bool() && overrides(this, b"enditeration") {
        set(this, "\0started", Value::Bool(false));
        call_this(ctx, this, "endIteration", &mut [])?;
    }
    set(this, "\0started", Value::Bool(false));
    Ok(Value::Bool(false))
}

fn rii_top(this: &Object) -> Rc<Object> {
    rii_stack(this).pop().expect("a level")
}

fn rii_current(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let it = rii_top(this);
    call_this(ctx, &it, "current", &mut [])
}

fn rii_key(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let it = rii_top(this);
    call_this(ctx, &it, "key", &mut [])
}

fn rii_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    rii_move(ctx, this)?;
    Ok(Value::Null)
}

fn rii_get_depth(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(rii_stack(this).len() as i64 - 1))
}

fn rii_get_sub_iterator(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let stack = rii_stack(this);
    let level = match arg(args, 0) {
        Value::Null => stack.len() as i64 - 1,
        v => v.to_int(),
    };
    Ok(if level < 0 || level as usize >= stack.len() { Value::Null } else { Value::Object(Rc::clone(&stack[level as usize])) })
}

fn rii_get_inner(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Object(rii_top(this)))
}

fn rii_call_has_children(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let it = rii_top(this);
    call_this(ctx, &it, "hasChildren", &mut [])
}

fn rii_call_get_children(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let it = rii_top(this);
    call_this(ctx, &it, "getChildren", &mut [])
}

fn rii_set_max_depth(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let d = if args.is_empty() { -1 } else { arg(args, 0).to_int() };
    if d < -1 {
        return Err(ctx.throw("ValueError", "RecursiveIteratorIterator::setMaxDepth(): Argument #1 ($maxDepth) must be greater than or equal to -1".into()));
    }
    set(this, "\0maxdepth", Value::Int(d));
    Ok(Value::Null)
}

fn rii_get_max_depth(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let d = int(this, "\0maxdepth");
    Ok(if d == -1 { Value::Bool(false) } else { Value::Int(d) })
}

fn no_op(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// The classes

const H: Visibility = PUBLIC;
const DUAL: &[(&str, Constant, Visibility)] =
    &[("\0inner", Constant::Null, H), ("\0valid", Constant::Bool(false), H), ("\0cur", Constant::Null, H), ("\0key", Constant::Null, H), ("\0pos", Constant::Int(0), H)];

const DUAL_METHODS: &[(&str, Option<Native>)] = &[
    ("__construct", Some(Native::Method(ii_construct))),
    ("getInnerIterator", Some(Native::Method(ii_get_inner))),
    ("rewind", Some(Native::Method(ii_rewind))),
    ("valid", Some(Native::Method(ii_valid))),
    ("current", Some(Native::Method(ii_current))),
    ("key", Some(Native::Method(ii_key))),
    ("next", Some(Native::Method(ii_next))),
];

const fn spec(name: &'static str, parent: Option<&'static str>, interfaces: &'static [&'static str], flags: u32) -> ClassSpec {
    ClassSpec { name, parent, interfaces, flags, properties: &[], methods: &[], constants: &[], debug_info: None }
}

fn no_info(_: &Object) -> PhpArray {
    PhpArray::new()
}

pub static CLASSES: &[ClassSpec] = &[
    ClassSpec { properties: DUAL, methods: DUAL_METHODS, debug_info: Some(no_info), ..spec("IteratorIterator", None, &["OuterIterator"], 0) },
    ClassSpec {
        methods: &[("rewind", Some(Native::Method(fi_rewind))), ("next", Some(Native::Method(fi_next))), ("accept", None)],
        ..spec("FilterIterator", Some("IteratorIterator"), &[], CLASS_ABSTRACT)
    },
    ClassSpec {
        properties: &[("\0cb", Constant::Null, H)],
        methods: &[("__construct", Some(Native::Method(cfi_construct))), ("accept", Some(Native::Method(cfi_accept)))],
        ..spec("CallbackFilterIterator", Some("FilterIterator"), &[], 0)
    },
    ClassSpec {
        methods: &[("hasChildren", Some(Native::Method(rfi_has_children))), ("getChildren", Some(Native::Method(rfi_get_children)))],
        ..spec("RecursiveFilterIterator", Some("FilterIterator"), &["RecursiveIterator"], CLASS_ABSTRACT)
    },
    ClassSpec {
        properties: &[("\0cb", Constant::Null, H)],
        methods: &[
            ("__construct", Some(Native::Method(cfi_construct))),
            ("accept", Some(Native::Method(cfi_accept))),
            ("hasChildren", Some(Native::Method(rfi_has_children))),
            ("getChildren", Some(Native::Method(rfi_get_children))),
        ],
        ..spec("RecursiveCallbackFilterIterator", Some("CallbackFilterIterator"), &["RecursiveIterator"], 0)
    },
    ClassSpec { methods: &[("accept", Some(Native::Method(pi_accept)))], ..spec("ParentIterator", Some("RecursiveFilterIterator"), &[], 0) },
    ClassSpec {
        properties: &[("\0offset", Constant::Int(0), H), ("\0count", Constant::Int(-1), H)],
        methods: &[
            ("__construct", Some(Native::Method(li_construct))),
            ("rewind", Some(Native::Method(li_rewind))),
            ("valid", Some(Native::Method(li_valid))),
            ("next", Some(Native::Method(li_next))),
            ("seek", Some(Native::Method(li_seek))),
            ("getPosition", Some(Native::Method(li_get_position))),
        ],
        ..spec("LimitIterator", Some("IteratorIterator"), &[], 0)
    },
    ClassSpec {
        properties: &[("\0flags", Constant::Int(1), H), ("\0cache", Constant::Null, H), ("\0str", Constant::Null, H)],
        methods: &[
            ("__construct", Some(Native::Method(ci_construct))),
            ("rewind", Some(Native::Method(ci_rewind))),
            ("valid", Some(Native::Method(ci_valid))),
            ("next", Some(Native::Method(ci_next))),
            ("hasNext", Some(Native::Method(ci_has_next))),
            ("__toString", Some(Native::Method(ci_to_string))),
            ("getCache", Some(Native::Method(ci_get_cache))),
            ("count", Some(Native::Method(ci_count))),
            ("offsetGet", Some(Native::Method(ci_offset_get))),
            ("offsetSet", Some(Native::Method(ci_offset_set))),
            ("offsetExists", Some(Native::Method(ci_offset_exists))),
            ("offsetUnset", Some(Native::Method(ci_offset_unset))),
            ("getFlags", Some(Native::Method(ci_get_flags))),
            ("setFlags", Some(Native::Method(ci_set_flags))),
        ],
        constants: &[
            ("CALL_TOSTRING", Constant::Int(1)),
            ("CATCH_GET_CHILD", Constant::Int(16)),
            ("TOSTRING_USE_KEY", Constant::Int(2)),
            ("TOSTRING_USE_CURRENT", Constant::Int(4)),
            ("TOSTRING_USE_INNER", Constant::Int(8)),
            ("FULL_CACHE", Constant::Int(256)),
        ],
        ..spec("CachingIterator", Some("IteratorIterator"), &["ArrayAccess", "Countable", "Stringable"], 0)
    },
    ClassSpec {
        methods: &[("hasChildren", Some(Native::Method(rfi_has_children))), ("getChildren", Some(Native::Method(rfi_get_children)))],
        ..spec("RecursiveCachingIterator", Some("CachingIterator"), &["RecursiveIterator"], 0)
    },
    ClassSpec {
        methods: &[
            ("rewind", Some(Native::Method(nri_rewind))),
            ("valid", Some(Native::Method(nri_valid))),
            ("current", Some(Native::Method(nri_current))),
            ("key", Some(Native::Method(nri_key))),
            ("next", Some(Native::Method(nri_next))),
        ],
        ..spec("NoRewindIterator", Some("IteratorIterator"), &[], 0)
    },
    ClassSpec { methods: &[("next", Some(Native::Method(inf_next)))], ..spec("InfiniteIterator", Some("IteratorIterator"), &[], 0) },
    ClassSpec {
        methods: &[
            ("current", Some(Native::Method(ei_current))),
            ("key", Some(Native::Method(ei_key))),
            ("next", Some(Native::Method(ei_none))),
            ("rewind", Some(Native::Method(ei_none))),
            ("valid", Some(Native::Method(ei_false))),
        ],
        ..spec("EmptyIterator", None, &["Iterator"], 0)
    },
    ClassSpec {
        properties: &[("\0list", Constant::Null, H), ("\0idx", Constant::Int(0), H)],
        methods: &[
            ("__construct", Some(Native::Method(ai_construct))),
            ("append", Some(Native::Method(ai_append))),
            ("rewind", Some(Native::Method(ai_rewind))),
            ("next", Some(Native::Method(ai_next))),
            ("getIteratorIndex", Some(Native::Method(ai_get_iterator_index))),
            ("getArrayIterator", Some(Native::Method(ai_get_array_iterator))),
        ],
        ..spec("AppendIterator", Some("IteratorIterator"), &[], 0)
    },
    ClassSpec {
        properties: &[
            ("\0inner", Constant::Null, H),
            ("\0stack", Constant::Null, H),
            ("\0states", Constant::Null, H),
            ("\0mode", Constant::Int(0), H),
            ("\0flags", Constant::Int(0), H),
            ("\0maxdepth", Constant::Int(-1), H),
            ("\0started", Constant::Bool(false), H),
        ],
        methods: &[
            ("__construct", Some(Native::Method(rii_construct))),
            ("rewind", Some(Native::Method(rii_rewind))),
            ("valid", Some(Native::Method(rii_valid))),
            ("current", Some(Native::Method(rii_current))),
            ("key", Some(Native::Method(rii_key))),
            ("next", Some(Native::Method(rii_next))),
            ("getDepth", Some(Native::Method(rii_get_depth))),
            ("getSubIterator", Some(Native::Method(rii_get_sub_iterator))),
            ("getInnerIterator", Some(Native::Method(rii_get_inner))),
            ("callHasChildren", Some(Native::Method(rii_call_has_children))),
            ("callGetChildren", Some(Native::Method(rii_call_get_children))),
            ("beginIteration", Some(Native::Method(no_op))),
            ("endIteration", Some(Native::Method(no_op))),
            ("beginChildren", Some(Native::Method(no_op))),
            ("endChildren", Some(Native::Method(no_op))),
            ("nextElement", Some(Native::Method(no_op))),
            ("setMaxDepth", Some(Native::Method(rii_set_max_depth))),
            ("getMaxDepth", Some(Native::Method(rii_get_max_depth))),
        ],
        constants: &[("LEAVES_ONLY", Constant::Int(0)), ("SELF_FIRST", Constant::Int(1)), ("CHILD_FIRST", Constant::Int(2)), ("CATCH_GET_CHILD", Constant::Int(16))],
        debug_info: Some(no_info),
        ..spec("RecursiveIteratorIterator", None, &["OuterIterator"], 0)
    },
];

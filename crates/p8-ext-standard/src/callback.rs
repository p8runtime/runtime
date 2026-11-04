//! The functions that call PHP code: call_user_func, array_map,
//! array_filter, array_reduce, array_walk, usort and friends. Callbacks
//! run through `Ctx::call`; an exception one throws ends the builtin.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/array.c,
//! $PHP_SRC_PATH/ext/standard/basic_functions.c (call_user_func)

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "call_user_func", run: call_user_func, by_ref: &[] },
    Builtin { name: "call_user_func_array", run: call_user_func_array, by_ref: &[] },
    Builtin { name: "is_callable", run: is_callable, by_ref: &[] },
    Builtin { name: "array_map", run: array_map, by_ref: &[] },
    Builtin { name: "array_filter", run: array_filter, by_ref: &[] },
    Builtin { name: "array_reduce", run: array_reduce, by_ref: &[] },
    Builtin { name: "array_find", run: array_find, by_ref: &[] },
    Builtin { name: "array_find_key", run: array_find_key, by_ref: &[] },
    Builtin { name: "array_any", run: array_any, by_ref: &[] },
    Builtin { name: "array_all", run: array_all, by_ref: &[] },
    Builtin { name: "array_walk", run: array_walk, by_ref: &[0] },
    Builtin { name: "usort", run: usort, by_ref: &[0] },
    Builtin { name: "array_udiff", run: array_udiff, by_ref: &[] },
    Builtin { name: "array_udiff_assoc", run: array_udiff_assoc, by_ref: &[] },
    Builtin { name: "array_udiff_uassoc", run: array_udiff_uassoc, by_ref: &[] },
    Builtin { name: "array_diff_ukey", run: array_diff_ukey, by_ref: &[] },
    Builtin { name: "array_diff_uassoc", run: array_diff_uassoc, by_ref: &[] },
    Builtin { name: "array_uintersect", run: array_uintersect, by_ref: &[] },
    Builtin { name: "array_uintersect_assoc", run: array_uintersect_assoc, by_ref: &[] },
    Builtin { name: "array_uintersect_uassoc", run: array_uintersect_uassoc, by_ref: &[] },
    Builtin { name: "array_intersect_ukey", run: array_intersect_ukey, by_ref: &[] },
    Builtin { name: "array_intersect_uassoc", run: array_intersect_uassoc, by_ref: &[] },
    Builtin { name: "uasort", run: uasort, by_ref: &[0] },
    Builtin { name: "uksort", run: uksort, by_ref: &[0] },
    Builtin { name: "iterator_to_array", run: iterator_to_array, by_ref: &[] },
    Builtin { name: "iterator_count", run: iterator_count, by_ref: &[] },
];

/// iterator_to_array(Traversable|array $iterator, bool $preserve_keys = true)
fn iterator_to_array(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let preserve = args.len() < 2 || arg(args, 1).to_bool();
    let mut out = PhpArray::new();
    for (k, v) in ctx.iterate(&arg(args, 0))? {
        if preserve {
            match Key::from_value(&k) {
                Ok((key, _)) => out.insert(key, v),
                Err(_) => return Err(ctx.throw("TypeError", "Cannot access offset of type array on array".into())),
            }
        } else {
            let _ = out.push(v);
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

fn iterator_count(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(ctx.iterate(&arg(args, 0))?.len() as i64))
}

fn array(args: &[Value], i: usize) -> Rc<PhpArray> {
    match arg(args, i) {
        Value::Array(a) => a,
        _ => Rc::new(PhpArray::new()),
    }
}

fn call_user_func(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let callable = arg(args, 0);
    let mut rest: Vec<Value> = args[1..].iter().map(Value::deref).collect();
    ctx.call(&callable, &mut rest)
}

fn call_user_func_array(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let callable = arg(args, 0);
    // References in the list stay ones: the callee takes them where its
    // parameters do.
    let mut rest: Vec<Value> = array(args, 1).values().cloned().collect();
    ctx.call(&callable, &mut rest)
}

fn is_callable(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    // A syntax-only check passes any string or two-element array.
    if args.len() > 1 && arg(args, 1).to_bool() {
        return Ok(Value::Bool(match &v {
            Value::Str(_) => true,
            Value::Array(a) => a.len() == 2,
            Value::Object(_) => ctx.is_callable(&v),
            _ => false,
        }));
    }
    // The class a callable names is looked up (autoloaded) first, as
    // zend_is_callable_check_class does ([Class, 'm'], 'Class::m'): an
    // exception an autoloader throws goes on, and a class still missing
    // makes it not callable. self, parent and static name no class.
    let class = match &v {
        Value::Array(a) if a.len() == 2 => match a.get_int(0).map(Value::deref) {
            Some(Value::Str(c)) => Some(c),
            _ => None,
        },
        Value::Str(s) => s.as_bytes().windows(2).position(|w| w == b"::").map(|at| PhpStr::from(&s.as_bytes()[..at])),
        _ => None,
    };
    if let Some(c) = class
        && !matches!(c.as_bytes().to_ascii_lowercase().as_slice(), b"self" | b"parent" | b"static")
        && ctx.class_load(c.as_bytes())?.is_none()
    {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(ctx.is_callable(&v)))
}

/// array_map(?callable $callback, array $array, array ...$arrays)
fn array_map(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let callback = arg(args, 0);
    let arrays: Vec<Rc<PhpArray>> = (1..args.len()).map(|i| array(args, i)).collect();
    if arrays.len() == 1 {
        let a = &arrays[0];
        if matches!(callback, Value::Null) {
            return Ok(Value::Array(Rc::clone(a)));
        }
        let mut out = PhpArray::new();
        for (k, v) in a.iter() {
            let r = ctx.call(&callback, &mut [v.deref()])?;
            out.insert(k, r);
        }
        return Ok(Value::Array(Rc::new(out)));
    }
    // Several arrays: by position, the shorter ones padded with null.
    let longest = arrays.iter().map(|a| a.len()).max().unwrap_or(0);
    let columns: Vec<Vec<Value>> = arrays.iter().map(|a| a.values().map(Value::deref).collect()).collect();
    let mut out = Vec::with_capacity(longest);
    for i in 0..longest {
        let mut row: Vec<Value> = columns.iter().map(|c| c.get(i).cloned().unwrap_or(Value::Null)).collect();
        out.push(if matches!(callback, Value::Null) {
            Value::Array(Rc::new(PhpArray::from_list(row)))
        } else {
            ctx.call(&callback, &mut row)?
        });
    }
    Ok(Value::Array(Rc::new(PhpArray::from_list(out))))
}

const ARRAY_FILTER_USE_BOTH: i64 = 1;
const ARRAY_FILTER_USE_KEY: i64 = 2;

/// array_filter(array $array, ?callable $callback = null, int $mode = 0)
fn array_filter(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = array(args, 0);
    let callback = arg(args, 1);
    let mode = arg(args, 2).to_int();
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let keep = match (&callback, mode) {
            (Value::Null, _) => v.to_bool(),
            (_, ARRAY_FILTER_USE_KEY) => ctx.call(&callback, &mut [k.to_value()])?.to_bool(),
            (_, ARRAY_FILTER_USE_BOTH) => ctx.call(&callback, &mut [v.deref(), k.to_value()])?.to_bool(),
            _ => ctx.call(&callback, &mut [v.deref()])?.to_bool(),
        };
        if keep {
            out.insert(k, v.clone());
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

/// The first entry for which `callback($value, $key)` is true (PHP 8.4's
/// array_find family; ext/standard/array.c php_array_find).
fn find_entry(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Option<(Value, Value)>, Throw> {
    let a = array(args, 0);
    let callback = arg(args, 1);
    for (k, v) in a.iter() {
        let (key, value) = (k.to_value(), v.deref());
        if ctx.call(&callback, &mut [value.clone(), key.clone()])?.to_bool() {
            return Ok(Some((key, value)));
        }
    }
    Ok(None)
}

/// array_find(array $array, callable $callback): the first value the callback accepts, or null.
fn array_find(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(find_entry(ctx, args)?.map_or(Value::Null, |(_, v)| v))
}

/// array_find_key(array $array, callable $callback): its key, or null.
fn array_find_key(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(find_entry(ctx, args)?.map_or(Value::Null, |(k, _)| k))
}

/// array_any(array $array, callable $callback): whether the callback accepts any entry.
fn array_any(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(find_entry(ctx, args)?.is_some()))
}

/// array_all(array $array, callable $callback): whether it accepts every entry (true for none).
fn array_all(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = array(args, 0);
    let callback = arg(args, 1);
    for (k, v) in a.iter() {
        if !ctx.call(&callback, &mut [v.deref(), k.to_value()])?.to_bool() {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

/// array_reduce(array $array, callable $callback, mixed $initial = null)
fn array_reduce(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = array(args, 0);
    let callback = arg(args, 1);
    let mut carry = arg(args, 2);
    for v in a.values() {
        carry = ctx.call(&callback, &mut [carry, v.deref()])?;
    }
    Ok(carry)
}

/// The by-reference array argument's current value (not held while
/// callbacks run: they may use the same variable).
fn take_array(args: &[Value]) -> Option<(Rc<RefCell<Value>>, Rc<PhpArray>)> {
    let Some(Value::Ref(cell)) = args.first() else { return None };
    let a = match &*cell.borrow() {
        Value::Array(a) => Rc::clone(a),
        _ => return None,
    };
    Some((Rc::clone(cell), a))
}

/// array_walk(array|object &$array, callable $callback, mixed $arg = UNKNOWN):
/// the callback gets each value by reference, its key, and `$arg`.
fn array_walk(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some((cell, _)) = take_array(args) else { return Ok(Value::Bool(true)) };
    let callback = arg(args, 1);
    let extra = (args.len() > 2).then(|| arg(args, 2));
    // The live array, walked by position as foreach walks it: an element a
    // callback removed is not visited, one it changed is passed as it is
    // now, one it added is visited too (PHP's array_walk iterates the
    // array itself).
    let mut pos = 0;
    loop {
        let next = match &*cell.borrow() {
            Value::Array(current) => current.entry_at(pos).map(|(p, k, v)| (p, k, v.deref())),
            _ => None,
        };
        let Some((at, k, live)) = next else { break };
        pos = at + 1;
        let element = Rc::new(RefCell::new(live));
        let mut call_args = vec![Value::Ref(Rc::clone(&element)), k.to_value()];
        call_args.extend(extra.clone());
        ctx.call(&callback, &mut call_args)?;
        let new = element.borrow().clone();
        if let Value::Array(current) = &mut *cell.borrow_mut()
            && current.contains_key(&k)
        {
            Rc::make_mut(current).insert(k, new);
        }
    }
    Ok(Value::Bool(true))
}

/// Sort the by-reference array with a user comparison: of values or of
/// keys, keeping keys or renumbering. The first exception a comparison
/// throws ends the sort and leaves the array as it was.
fn user_sort(ctx: &mut dyn Ctx, args: &[Value], by_key: bool, keep_keys: bool) -> Result<Value, Throw> {
    let Some((cell, a)) = take_array(args) else { return Ok(Value::Bool(true)) };
    let callback = arg(args, 1);
    // (position, key, value): zend_sort calls the callback with PHP's
    // pairs; ties are broken on the position.
    let mut entries: Vec<(usize, Key, Value)> = a.iter().enumerate().map(|(i, (k, v))| (i, k, v.clone())).collect();
    let mut error = None;
    p8_runtime::sort::zend_sort(&mut entries, &mut |x, y| {
        if error.is_some() {
            return Ordering::Equal;
        }
        let mut pair = if by_key { [x.1.to_value(), y.1.to_value()] } else { [x.2.deref(), y.2.deref()] };
        match ctx.call(&callback, &mut pair) {
            Ok(r) => r.to_int().cmp(&0).then(x.0.cmp(&y.0)),
            Err(e) => {
                error = Some(e);
                Ordering::Equal
            }
        }
    });
    let entries: Vec<(Key, Value)> = entries.into_iter().map(|(_, k, v)| (k, v)).collect();
    if let Some(e) = error {
        return Err(e);
    }
    let sorted = if keep_keys {
        let mut out = PhpArray::new();
        for (k, v) in entries {
            out.insert(k, v);
        }
        out
    } else {
        PhpArray::from_list(entries.into_iter().map(|(_, v)| v).collect())
    };
    *cell.borrow_mut() = Value::Array(Rc::new(sorted));
    Ok(Value::Bool(true))
}

fn usort(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    user_sort(ctx, args, false, false)
}

fn uasort(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    user_sort(ctx, args, false, true)
}

fn uksort(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    user_sort(ctx, args, true, true)
}

/// The callback variants take their callbacks in `...$rest`, which their
/// stubs do not type as callable: they are marked as calling back.
pub(crate) const REENTER: &[&str] = &[
    "array_udiff", "array_udiff_assoc", "array_udiff_uassoc", "array_diff_ukey", "array_diff_uassoc",
    "array_uintersect", "array_uintersect_assoc", "array_uintersect_uassoc", "array_intersect_ukey", "array_intersect_uassoc",
];

/// How an array_*diff / array_*intersect variant compares one part of an
/// entry (its value or its key).
enum Compare {
    /// Not compared.
    Ignored,
    /// As strings (the plain variants).
    Strings,
    /// With the callback at this argument (0 means equal).
    User(usize),
}

/// The callback variants of array_diff/array_intersect: each entry of the
/// first array is kept when (diff) no other array, or (intersect) every
/// other array, has an entry equal to it in its value and key as
/// `values` and `keys` compare them.
fn user_set(ctx: &mut dyn Ctx, args: &[Value], callbacks: usize, values: Compare, keys: Compare, intersect: bool) -> Result<Value, Throw> {
    let arrays = args.len().saturating_sub(callbacks);
    let list: Vec<Rc<PhpArray>> = (0..arrays)
        .map(|i| match args[i].deref() {
            Value::Array(a) => a,
            _ => Rc::new(PhpArray::new()),
        })
        .collect();
    let text = |v: &Value| v.to_php_string().unwrap_or_else(p8_runtime::string::PhpStr::empty);
    let same = |ctx: &mut dyn Ctx, how: &Compare, a: Value, b: Value| -> Result<bool, Throw> {
        match how {
            Compare::Ignored => Ok(true),
            Compare::Strings => Ok(text(&a) == text(&b)),
            Compare::User(i) => {
                let mut pair = [a, b];
                Ok(ctx.call(&args[*i].deref(), &mut pair)?.to_int() == 0)
            }
        }
    };
    let mut out = PhpArray::new();
    let Some(first) = list.first() else { return Ok(Value::Array(Rc::new(out))) };
    for (k, v) in first.iter() {
        let mut found_in_all = true;
        let mut found_in_any = false;
        for other in &list[1..] {
            let mut found = false;
            for (ok, ov) in other.iter() {
                if same(ctx, &keys, k.to_value(), ok.to_value())? && same(ctx, &values, v.deref(), ov.deref())? {
                    found = true;
                    break;
                }
            }
            found_in_all &= found;
            found_in_any |= found;
        }
        if if intersect { found_in_all } else { !found_in_any } {
            out.insert(k, v.clone());
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

fn array_udiff(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 1, Compare::User(n - 1), Compare::Ignored, false)
}

fn array_udiff_assoc(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 1, Compare::User(n - 1), Compare::Strings, false)
}

fn array_udiff_uassoc(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 2, Compare::User(n - 2), Compare::User(n - 1), false)
}

fn array_diff_ukey(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 1, Compare::Ignored, Compare::User(n - 1), false)
}

fn array_diff_uassoc(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 1, Compare::Strings, Compare::User(n - 1), false)
}

fn array_uintersect(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 1, Compare::User(n - 1), Compare::Ignored, true)
}

fn array_uintersect_assoc(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 1, Compare::User(n - 1), Compare::Strings, true)
}

fn array_uintersect_uassoc(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 2, Compare::User(n - 2), Compare::User(n - 1), true)
}

fn array_intersect_ukey(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 1, Compare::Ignored, Compare::User(n - 1), true)
}

fn array_intersect_uassoc(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = args.len();
    user_set(ctx, args, 1, Compare::Strings, Compare::User(n - 1), true)
}

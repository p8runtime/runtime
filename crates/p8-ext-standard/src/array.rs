//! ext/standard's array functions (those that take no callback). Arguments
//! arrive already checked and converted to their declared types
//! (p8_runtime::args); a by-reference parameter receives the variable's
//! reference cell.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/array.c

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Fault, FastBuiltin, Throw};
use p8_runtime::lent;
use p8_runtime::object::CLASS_ENUM;
use p8_runtime::string::{PhpStr, StrView};
use p8_runtime::value::{Kind, Value};
use std::cmp::Ordering;
use std::rc::Rc;

/// enum SortDirection (PHP 8.6, basic_functions.stub.php), a pure enum no
/// builtin takes: `case Ascending; case Descending;`.
pub const CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "SortDirection",
    parent: None,
    interfaces: &[],
    // Not final: zend_register_internal_enum sets only ZEND_ACC_ENUM.
    flags: CLASS_ENUM,
    properties: &[],
    methods: &[],
    constants: &[("Ascending", Constant::Null), ("Descending", Constant::Null)],
    debug_info: None,
}];

/// The array functions on the borrowed ABI.
pub static FAST: &[FastBuiltin] = &[
    FastBuiltin { name: "sizeof", run: crate::count },
    FastBuiltin { name: "array_key_exists", run: array_key_exists },
    FastBuiltin { name: "key_exists", run: array_key_exists },
    FastBuiltin { name: "in_array", run: in_array },
    FastBuiltin { name: "array_search", run: array_search },
    FastBuiltin { name: "array_key_first", run: array_key_first },
    FastBuiltin { name: "array_key_last", run: array_key_last },
    FastBuiltin { name: "array_first", run: array_first },
    FastBuiltin { name: "array_last", run: array_last },
    FastBuiltin { name: "array_is_list", run: array_is_list },
];

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "array_multisort", run: array_multisort, by_ref: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] },
    Builtin { name: "array_values", run: array_values, by_ref: &[] },
    Builtin { name: "array_merge", run: array_merge, by_ref: &[] },
    Builtin { name: "array_slice", run: array_slice, by_ref: &[] },
    Builtin { name: "array_reverse", run: array_reverse, by_ref: &[] },
    Builtin { name: "array_flip", run: array_flip, by_ref: &[] },
    Builtin { name: "array_unique", run: array_unique, by_ref: &[] },
    Builtin { name: "array_fill", run: array_fill, by_ref: &[] },
    Builtin { name: "array_fill_keys", run: array_fill_keys, by_ref: &[] },
    Builtin { name: "array_combine", run: array_combine, by_ref: &[] },
    Builtin { name: "array_sum", run: array_sum, by_ref: &[] },
    Builtin { name: "array_product", run: array_product, by_ref: &[] },
    Builtin { name: "array_pad", run: array_pad, by_ref: &[] },
    Builtin { name: "array_count_values", run: array_count_values, by_ref: &[] },
    Builtin { name: "array_chunk", run: array_chunk, by_ref: &[] },
    Builtin { name: "array_diff", run: array_diff, by_ref: &[] },
    Builtin { name: "array_intersect", run: array_intersect, by_ref: &[] },
    Builtin { name: "array_diff_key", run: array_diff_key, by_ref: &[] },
    Builtin { name: "array_intersect_key", run: array_intersect_key, by_ref: &[] },
    Builtin { name: "array_replace", run: array_replace, by_ref: &[] },
    Builtin { name: "array_push", run: array_push, by_ref: &[0] },
    Builtin { name: "array_pop", run: array_pop, by_ref: &[0] },
    Builtin { name: "array_shift", run: array_shift, by_ref: &[0] },
    Builtin { name: "array_unshift", run: array_unshift, by_ref: &[0] },
    Builtin { name: "sort", run: sort, by_ref: &[0] },
    Builtin { name: "rsort", run: rsort, by_ref: &[0] },
    Builtin { name: "asort", run: asort, by_ref: &[0] },
    Builtin { name: "arsort", run: arsort, by_ref: &[0] },
    Builtin { name: "ksort", run: ksort, by_ref: &[0] },
    Builtin { name: "krsort", run: krsort, by_ref: &[0] },
];

fn array(values: PhpArray) -> Result<Value, Throw> {
    Ok(Value::array(values))
}

fn list(values: Vec<Value>) -> Result<Value, Throw> {
    array(PhpArray::from_list(values))
}

/// Argument `i` as an array (the signature check has made sure it is one).
fn arr(args: &[Value], i: usize) -> Rc<PhpArray> {
    arg(args, i).into_array().unwrap_or_else(|_| Rc::new(PhpArray::new()))
}

/// The array a by-reference parameter holds, to change in place.
fn with_array<R>(args: &[Value], change: impl FnOnce(&mut PhpArray) -> R) -> Option<R> {
    let cell = args.first()?.as_reference()?;
    let mut value = cell.borrow_mut();
    let a = value.array_mut()?;
    Some(change(a))
}

fn array_values(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = arr(args, 0);
    if a.is_list() {
        return Ok(Value::array(a));
    }
    list(a.values().cloned().collect())
}

/// array_merge: int keys are renumbered, string keys overwrite.
fn array_merge(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    // Two arrays, one empty: the other as it is when merging would give the
    // same (a list without holes, or string keys only), as PHP does.
    if args.len() == 2 {
        let (a, b) = (arr(args, 0), arr(args, 1));
        let other = if a.is_empty() { Some(b) } else if b.is_empty() { Some(a) } else { None };
        if let Some(other) = other
            && (other.is_dense_list() || (!other.is_packed() && other.iter().all(|(k, _)| matches!(k, Key::Str(_)))))
        {
            return Ok(Value::array(other));
        }
    }
    let count = (0..args.len()).map(|i| arr(args, i).len()).sum();
    // Shaped as the first array is (zend_hash_real_init_packed or _mixed),
    // sized for everything once.
    let mut out = if args.is_empty() || arr(args, 0).is_packed() { PhpArray::list_with_capacity(count) } else { PhpArray::hashed(count) };
    for i in 0..args.len() {
        for (k, v) in arr(args, i).iter() {
            match k {
                Key::Int(_) => {
                    let _ = out.push(v.clone());
                }
                k => out.insert(k, v.clone()),
            }
        }
    }
    array(out)
}

fn key_of(v: &Value) -> Option<Key> {
    if v.is_null_or_undef() {
        return Some(Key::Str(p8_runtime::string::PhpStr::empty()));
    }
    match v.as_float() {
        Some(f) => Some(Key::Int(f as i64)),
        None => Key::from_value(v).ok().map(|(k, _)| k),
    }
}

fn array_key_exists(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let Some(a) = lent::array(args, 1) else { return Ok(Value::FALSE) };
    let key = lent::at(args, 0);
    // An int or a string key, looked up in place (no Key made).
    if let Some(k) = key.as_int() {
        return Ok(Value::bool(a.contains_key(&Key::Int(k))));
    }
    if let Some(s) = key.as_str() {
        return Ok(Value::bool(a.get_str(s).is_some()));
    }
    if key.is_null() {
        ctx.deprecated("Using null as the key parameter for array_key_exists() is deprecated, use an empty string instead");
    }
    Ok(Value::bool(key_of(&key.deref()).is_some_and(|k| a.contains_key(&k))))
}

pub(crate) fn matches(needle: &Value, v: &Value, strict: bool) -> bool {
    if strict { needle.identical(v) } else { needle.loose_eq(v) }
}

// (`identical` and `loose_eq` look through a reference themselves: the
// needle and the elements are compared in place.)
fn in_array(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let (needle, strict) = (lent::at(args, 0), lent::bool(args, 2));
    let Some(a) = lent::array(args, 1) else { return Ok(Value::FALSE) };
    Ok(Value::bool(a.values().any(|v| matches(needle, v, strict))))
}

fn array_search(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let (needle, strict) = (lent::at(args, 0), lent::bool(args, 2));
    let Some(a) = lent::array(args, 1) else { return Ok(Value::FALSE) };
    Ok(a.iter().find(|(_, v)| matches(needle, v, strict)).map_or(Value::FALSE, |(k, _)| k.to_value()))
}

/// array_slice($array, $offset, $length = null, $preserve_keys = false):
/// string keys are always kept, int keys renumbered unless preserved.
fn array_slice(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = arr(args, 0);
    let len = a.len() as i64;
    let offset = arg(args, 1).to_int();
    let start = if offset < 0 { (len + offset).max(0) } else { offset.min(len) };
    let end = match args.get(2).map(Value::deref) {
        Some(l) if !l.is_null() => {
            let l = l.to_int();
            if l < 0 { (len + l).max(start) } else { (start + l).min(len) }
        }
        _ => len,
    };
    let preserve = args.len() > 3 && arg(args, 3).to_bool();
    let mut out = PhpArray::new();
    for (k, v) in a.iter().skip(start as usize).take((end - start).max(0) as usize) {
        match k {
            Key::Int(_) if !preserve => {
                let _ = out.push(v.clone());
            }
            k => out.insert(k, v.clone()),
        }
    }
    array(out)
}

fn array_reverse(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = arr(args, 0);
    let preserve = args.len() > 1 && arg(args, 1).to_bool();
    let entries: Vec<(Key, Value)> = a.iter().map(|(k, v)| (k, v.clone())).collect();
    let mut out = PhpArray::new();
    for (k, v) in entries.into_iter().rev() {
        match k {
            Key::Int(_) if !preserve => {
                let _ = out.push(v);
            }
            k => out.insert(k, v),
        }
    }
    array(out)
}

fn array_flip(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = PhpArray::new();
    for (k, v) in arr(args, 0).iter() {
        let v = v.deref();
        if v.is_int() || v.is_str() {
            if let Ok((key, _)) = Key::from_value(&v) {
                out.insert(key, k.to_value());
            }
        } else {
            ctx.warn("array_flip(): Can only flip string and integer values, entry skipped");
        }
    }
    array(out)
}

/// array_unique($array, $flags = SORT_STRING): the first of each group of
/// equal values (compared as strings by default) keeps its key.
/// array_unique($array, $flags = SORT_STRING), as php_array.c does it:
/// for SORT_STRING one pass with a set of the values' strings; for the
/// other flags the entries sorted (ties by position), each run of equal
/// ones kept as its first. The first of equal entries stays, keys kept.
fn array_unique(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 2 };
    let input = arr(args, 0);
    let mut out = PhpArray::new();
    if flags == 2 {
        // The strings seen, as a set of their bytes borrowed from the
        // array (see `by_values`); values with no string (arrays) are one
        // kind, kept once.
        let texts: Vec<Option<StrView<'_>>> = input
            .iter()
            .map(|(_, v)| match v.as_str() {
                Some(s) => Some(StrView::Borrowed(s)),
                None => v.deref().to_php_string().map(StrView::Owned),
            })
            .collect();
        let mut seen: p8_runtime::fast::HashSet<&[u8]> = p8_runtime::fast::HashSet::with_capacity_and_hasher(texts.len(), Default::default());
        let mut seen_none = false;
        for ((k, v), t) in input.iter().zip(&texts) {
            let new = match t {
                Some(t) => seen.insert(t.as_bytes()),
                None => !std::mem::replace(&mut seen_none, true),
            };
            if new {
                out.insert(k, v.deref());
            }
        }
        return array(out);
    }
    let cmp = order(flags);
    let mut entries: Vec<(usize, Value)> = input.iter().enumerate().map(|(i, (_, v))| (i, v.deref())).collect();
    p8_runtime::sort::zend_sort(&mut entries, &mut |x, y| cmp(&x.1, &y.1).then(x.0.cmp(&y.0)));
    let mut keep = vec![false; entries.len()];
    let mut last: Option<&(usize, Value)> = None;
    for e in &entries {
        match last {
            Some(l) if cmp(&l.1, &e.1) == Ordering::Equal => {
                // A duplicate: the earlier of the two stays.
                if l.0 > e.0 {
                    keep[l.0] = false;
                    keep[e.0] = true;
                    last = Some(e);
                }
            }
            _ => {
                keep[e.0] = true;
                last = Some(e);
            }
        }
    }
    for (i, (k, v)) in input.iter().enumerate() {
        if keep[i] {
            out.insert(k, v.deref());
        }
    }
    array(out)
}

fn array_fill(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (start, count, value) = (arg(args, 0).to_int(), arg(args, 1).to_int(), arg(args, 2));
    if count < 0 {
        return Err(ctx.throw("ValueError", "array_fill(): Argument #2 ($count) must be greater than or equal to 0".into()));
    }
    let mut out = PhpArray::new();
    for i in 0..count {
        out.insert(Key::Int(start.wrapping_add(i)), value.clone());
    }
    array(out)
}

fn array_fill_keys(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let value = arg(args, 1);
    let mut out = PhpArray::new();
    for k in arr(args, 0).values() {
        if let Some(key) = key_of(&k.deref()).or_else(|| k.to_php_string().map(Key::from_string)) {
            out.insert(key, value.clone());
        }
    }
    array(out)
}

fn array_combine(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (keys, values) = (arr(args, 0), arr(args, 1));
    if keys.len() != values.len() {
        return Err(ctx.throw("ValueError", "array_combine(): Argument #1 ($keys) and argument #2 ($values) must have the same number of elements".into()));
    }
    let mut out = PhpArray::new();
    for (k, v) in keys.values().zip(values.values()) {
        if let Some(key) = key_of(&k.deref()).or_else(|| k.to_php_string().map(Key::from_string)) {
            out.insert(key, v.clone());
        }
    }
    array(out)
}

/// array_sum / array_product: + and * over the values, as PHP 8.3 does
/// (an array or other unsupported value warns and is skipped).
fn fold(ctx: &mut dyn Ctx, args: &[Value], op: p8_runtime::ops::BinOp, start: Value, verb: &str) -> Result<Value, Throw> {
    let mut acc = start;
    for v in arr(args, 0).values() {
        let v = v.deref();
        if v.is_array() {
            ctx.warn(&format!("{verb} is not supported on type array"));
            continue;
        }
        let mut diags = Vec::new();
        if let Ok(r) = p8_runtime::ops::binary(op, &acc, &v, &mut diags) {
            acc = r;
        }
        for d in diags {
            match d {
                p8_runtime::ops::Diag::Warning(m) => ctx.warn(&m),
                p8_runtime::ops::Diag::Deprecated(m) => ctx.deprecated(&m),
            }
        }
    }
    Ok(acc)
}

fn array_sum(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    fold(ctx, args, p8_runtime::ops::BinOp::Add, Value::int(0), "Addition")
}

fn array_product(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    fold(ctx, args, p8_runtime::ops::BinOp::Mul, Value::int(1), "Multiplication")
}

fn array_pad(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (a, size, value) = (arr(args, 0), arg(args, 1).to_int(), arg(args, 2));
    let target = size.unsigned_abs() as usize;
    if target > 1 << 26 {
        return Err(ctx.throw("ValueError", "array_pad(): Argument #2 ($length) must not exceed the maximum allowed array size".into()));
    }
    if target <= a.len() {
        return Ok(Value::array(a));
    }
    let fill = vec![value; target - a.len()];
    let mut out = PhpArray::new();
    let append = |out: &mut PhpArray, k: Key, v: Value| match k {
        Key::Int(_) => {
            let _ = out.push(v);
        }
        k => out.insert(k, v),
    };
    if size < 0 {
        for v in fill.iter() {
            let _ = out.push(v.clone());
        }
    }
    for (k, v) in a.iter() {
        append(&mut out, k, v.clone());
    }
    if size > 0 {
        for v in fill {
            let _ = out.push(v);
        }
    }
    array(out)
}

fn array_key_first(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(lent::array(args, 0).and_then(|a| a.iter().next()).map_or(Value::NULL, |(k, _)| k.to_value()))
}

fn array_key_last(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(lent::array(args, 0).and_then(|a| a.iter().last()).map_or(Value::NULL, |(k, _)| k.to_value()))
}

/// array_first(array $array): mixed (PHP 8.5): the first value, dereferenced
/// (RETURN_COPY_DEREF), or null for an empty array.
fn array_first(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(lent::array(args, 0).and_then(|a| a.values().next()).map_or(Value::NULL, Value::deref))
}

/// array_last(array $array): mixed (PHP 8.5): the last value, dereferenced,
/// or null for an empty array.
fn array_last(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(lent::array(args, 0).and_then(|a| a.values().last()).map_or(Value::NULL, Value::deref))
}

fn array_count_values(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = PhpArray::new();
    for v in arr(args, 0).values() {
        let v = v.deref();
        if v.is_int() || v.is_str() {
            if let Ok((key, _)) = Key::from_value(&v) {
                let n = out.get(&key).map_or(0, Value::to_int);
                out.insert(key, Value::int(n + 1));
            }
        } else {
            ctx.warn("array_count_values(): Can only count string and integer values, entry skipped");
        }
    }
    array(out)
}

fn array_chunk(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (a, size) = (arr(args, 0), arg(args, 1).to_int());
    if size < 1 {
        return Err(ctx.throw("ValueError", "array_chunk(): Argument #2 ($length) must be greater than 0".into()));
    }
    let preserve = args.len() > 2 && arg(args, 2).to_bool();
    let mut chunks = Vec::new();
    let mut chunk = PhpArray::new();
    for (k, v) in a.iter() {
        if preserve {
            chunk.insert(k, v.clone());
        } else {
            let _ = chunk.push(v.clone());
        }
        if chunk.len() as i64 == size {
            chunks.push(Value::array(std::mem::take(&mut chunk)));
        }
    }
    if !chunk.is_empty() {
        chunks.push(Value::array(chunk));
    }
    list(chunks)
}

/// The string forms of a value, for array_diff / array_intersect, which
/// compare `(string) $a === (string) $b`.
fn text(v: &Value) -> Vec<u8> {
    v.deref().to_str_view().map(|s| s.as_bytes().to_vec()).unwrap_or_else(|| b"Array".to_vec())
}

/// `(string) $v` to read: a string as it is (no copy, no handle made),
/// anything else converted; an array is "Array".
fn text_view(v: &Value) -> StrView<'_> {
    match v.as_str() {
        Some(s) => StrView::Borrowed(s),
        None => StrView::Owned(v.deref().to_php_string().unwrap_or_else(|| ARRAY_TEXT.with(PhpStr::clone))),
    }
}

thread_local! {
    /// "Array", an array's text here (PHP's ZSTR_KNOWN(ZEND_STR_ARRAY_CAPITALIZED)):
    /// one string per thread, shared, not one made per value compared (#412:
    /// array_diff and array_intersect made 83k of them on Laravel's
    /// Collection tests).
    static ARRAY_TEXT: PhpStr = PhpStr::from("Array");
}

fn by_values(args: &[Value], keep_if_found: bool) -> Result<Value, Throw> {
    // The other arrays' values as sets of their bytes, borrowed from the
    // arrays (a string is not copied), hashed by the name tables' hasher:
    // not `std` sets of `Vec<u8>`, which copied every value and hashed it
    // with SipHash (a third of a Laravel collection test run's hashing).
    let arrays: Vec<Rc<PhpArray>> = (1..args.len()).map(|i| arr(args, i)).collect();
    let views: Vec<Vec<StrView<'_>>> = arrays.iter().map(|a| a.values().map(text_view).collect()).collect();
    let others: Vec<p8_runtime::fast::HashSet<&[u8]>> = views.iter().map(|vs| vs.iter().map(|s| s.as_bytes()).collect()).collect();
    let mut out = PhpArray::new();
    for (k, v) in arr(args, 0).iter() {
        let t = text_view(v);
        let t = t.as_bytes();
        let keep = if keep_if_found { others.iter().all(|o| o.contains(t)) } else { !others.iter().any(|o| o.contains(t)) };
        if keep {
            out.insert(k, v.clone());
        }
    }
    array(out)
}

fn array_diff(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    by_values(args, false)
}

fn array_intersect(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    by_values(args, true)
}

fn by_keys(args: &[Value], keep_if_found: bool) -> Result<Value, Throw> {
    let others: Vec<Rc<PhpArray>> = (1..args.len()).map(|i| arr(args, i)).collect();
    let mut out = PhpArray::new();
    for (k, v) in arr(args, 0).iter() {
        let keep = if keep_if_found { others.iter().all(|o| o.contains_key(&k)) } else { !others.iter().any(|o| o.contains_key(&k)) };
        if keep {
            out.insert(k, v.clone());
        }
    }
    array(out)
}

fn array_diff_key(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    by_keys(args, false)
}

fn array_intersect_key(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    by_keys(args, true)
}

fn array_replace(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = (*arr(args, 0)).clone();
    for i in 1..args.len() {
        for (k, v) in arr(args, i).iter() {
            out.insert(k, v.clone());
        }
    }
    array(out)
}

fn array_is_list(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::array(args, 0).is_none_or(PhpArray::is_list)))
}

fn array_push(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let values: Vec<Value> = args[1..].iter().map(Value::deref).collect();
    let pushed = with_array(args, |a| {
        for v in values {
            if a.push(v).is_err() {
                return Err(());
            }
        }
        Ok(a.len() as i64)
    });
    match pushed {
        Some(Ok(n)) => Ok(Value::int(n)),
        Some(Err(())) => Err(ctx.throw("Error", "Cannot add element to the array as the next element is already occupied".into())),
        None => Ok(Value::NULL),
    }
}

fn array_pop(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(with_array(args, PhpArray::pop).flatten().unwrap_or(Value::NULL))
}

/// The entries again, int keys renumbered from 0 (string keys kept).
fn renumbered(entries: impl Iterator<Item = (Key, Value)>) -> PhpArray {
    let mut out = PhpArray::new();
    for (k, v) in entries {
        match k {
            Key::Int(_) => {
                let _ = out.push(v);
            }
            k => out.insert(k, v),
        }
    }
    out
}

fn array_shift(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let shifted = with_array(args, |a| {
        let mut entries = a.iter().map(|(k, v)| (k, v.clone())).collect::<Vec<_>>().into_iter();
        let first = entries.next().map(|(_, v)| v);
        *a = renumbered(entries);
        first
    });
    Ok(shifted.flatten().unwrap_or(Value::NULL))
}

fn array_unshift(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let values: Vec<Value> = args[1..].iter().map(Value::deref).collect();
    let n = with_array(args, |a| {
        let front = values.into_iter().map(|v| (Key::Int(0), v));
        let rest: Vec<(Key, Value)> = a.iter().map(|(k, v)| (k, v.clone())).collect();
        *a = renumbered(front.chain(rest));
        a.len() as i64
    });
    Ok(n.map_or(Value::NULL, Value::int))
}

/// How sort() and friends compare two values for `$flags`: SORT_REGULAR
/// (PHP 8's comparison), SORT_NUMERIC, SORT_STRING, with SORT_FLAG_CASE.
fn order(flags: i64) -> impl Fn(&Value, &Value) -> Ordering {
    move |a, b| match flags & !8 {
        1 => a.to_float().partial_cmp(&b.to_float()).unwrap_or(Ordering::Equal),
        2 | 5 => {
            let (x, y) = (text(a), text(b));
            if flags & 8 != 0 { x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()) } else { x.cmp(&y) }
        }
        6 => crate::string2::natural(&text(a), &text(b), flags & 8 != 0),
        _ => regular(&a.deref(), &b.deref()),
    }
}

/// SORT_REGULAR's order (php_array_data_compare_unstable_i): zend_compare,
/// where values that do not compare are 1 (ZEND_UNCOMPARABLE), except
/// that enum cases group by identity and go after other values, so that
/// array_unique finds equal cases next to each other (GH-9775).
fn regular(a: &Value, b: &Value) -> Ordering {
    use p8_runtime::object::CLASS_ENUM;
    match a.compare(b) {
        Some(o) => o,
        None => match (a.as_object(), b.as_object()) {
            (Some(x), Some(y)) if y.class.flags & CLASS_ENUM != 0 && x.class.flags & CLASS_ENUM != 0 => {
                (Rc::as_ptr(x) as usize).cmp(&(Rc::as_ptr(y) as usize))
            }
            (_, Some(y)) if y.class.flags & CLASS_ENUM != 0 => Ordering::Less,
            _ => Ordering::Greater,
        },
    }
}

/// array_multisort(array &$array, mixed $array_sort_order = SORT_ASC,
/// mixed $array_sort_flags = SORT_REGULAR, mixed &...$rest): sorts the
/// arrays together, by the first, ties by the next, as ORDER BY does.
fn array_multisort(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    // Each array, with its order (descending?) and flags.
    let mut arrays: Vec<(usize, bool, i64)> = Vec::new();
    let (mut order_free, mut type_free) = (false, false);
    for (i, a) in args.iter().enumerate() {
        let a = a.deref();
        match a.kind() {
            Kind::Array => {
                arrays.push((i, false, 0));
                (order_free, type_free) = (true, true);
            }
            Kind::Int => {
                let n = a.as_int().expect("an int");
                match n & !8 {
                    3 | 4 if order_free => {
                        arrays.last_mut().expect("an array before").1 = n == 3;
                        order_free = false;
                    }
                    0 | 1 | 2 | 5 | 6 if type_free => {
                        arrays.last_mut().expect("an array before").2 = n;
                        type_free = false;
                    }
                    0..=6 => {
                        return Err(ctx.throw("TypeError", format!("array_multisort(): Argument #{} must be an array or a sort flag that has not already been specified", i + 1)));
                    }
                    _ => return Err(ctx.throw("ValueError", format!("array_multisort(): Argument #{} must be a valid sort flag", i + 1))),
                }
            }
            _ => return Err(ctx.throw("TypeError", format!("array_multisort(): Argument #{} must be an array or a sort flag", i + 1))),
        }
    }
    let columns: Vec<Vec<(Key, Value)>> = arrays
        .iter()
        .map(|(i, _, _)| match args[*i].deref().as_array() {
            Some(a) => a.iter().map(|(k, v)| (k, v.clone())).collect(),
            None => Vec::new(),
        })
        .collect();
    let size = columns[0].len();
    if columns.iter().any(|c| c.len() != size) {
        return Err(ctx.throw("ValueError", "Array sizes are inconsistent".into()));
    }
    if size == 0 {
        return Ok(Value::TRUE);
    }
    let orders: Vec<_> = arrays.iter().map(|(_, desc, flags)| (*desc, order(*flags))).collect();
    let mut rows: Vec<usize> = (0..size).collect();
    // Stable: equal rows keep their order (PHP's fallback on position).
    p8_runtime::sort::zend_sort(&mut rows, &mut |&x, &y| {
        for (c, (desc, cmp)) in orders.iter().enumerate() {
            let o = cmp(&columns[c][x].1, &columns[c][y].1);
            if o != Ordering::Equal {
                return if *desc { o.reverse() } else { o };
            }
        }
        x.cmp(&y)
    });
    for (c, (i, _, _)) in arrays.iter().enumerate() {
        // String keys stay; integer keys are renumbered.
        let mut out = PhpArray::new();
        for &r in &rows {
            let (k, v) = &columns[c][r];
            match k {
                Key::Str(_) => out.insert(k.clone(), v.clone()),
                Key::Int(_) => {
                    let _ = out.push(v.clone());
                }
            }
        }
        if let Some(cell) = args[*i].as_reference() {
            *cell.borrow_mut() = Value::array(out);
        }
    }
    Ok(Value::TRUE)
}

fn key_value(k: &Key) -> Value {
    k.to_value()
}

/// Sort the by-reference array: by value or by key, ascending or not,
/// keeping keys or renumbering. PHP's sort is stable.
fn sort_with(args: &[Value], by_key: bool, reverse: bool, keep_keys: bool) -> Result<Value, Throw> {
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    let cmp = order(flags);
    let done = with_array(args, |a| {
        // (position, key, value): zend_sort, ties broken on the position.
        let mut entries: Vec<(usize, Key, Value)> = a.iter().enumerate().map(|(i, (k, v))| (i, k, v.clone())).collect();
        p8_runtime::sort::zend_sort(&mut entries, &mut |x, y| {
            let o = match (&x.1, &y.1) {
                // Two integer keys compare as integers under every flag
                // (php_array_key_compare_*: no float, so PHP_INT_MAX - 1
                // stays below PHP_INT_MAX).
                (Key::Int(p), Key::Int(q)) if by_key && flags & !8 == 1 => p.cmp(q),
                _ if by_key => cmp(&key_value(&x.1), &key_value(&y.1)),
                _ => cmp(&x.2, &y.2),
            };
            (if reverse { o.reverse() } else { o }).then(x.0.cmp(&y.0))
        });
        let entries: Vec<(Key, Value)> = entries.into_iter().map(|(_, k, v)| (k, v)).collect();
        *a = if keep_keys {
            let mut out = PhpArray::new();
            for (k, v) in entries {
                out.insert(k, v);
            }
            out
        } else {
            PhpArray::from_list(entries.into_iter().map(|(_, v)| v).collect())
        };
    });
    Ok(Value::bool(done.is_some()))
}

fn sort(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    sort_with(args, false, false, false)
}

fn rsort(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    sort_with(args, false, true, false)
}

fn asort(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    sort_with(args, false, false, true)
}

fn arsort(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    sort_with(args, false, true, true)
}

fn ksort(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    sort_with(args, true, false, true)
}

fn krsort(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    sort_with(args, true, true, true)
}

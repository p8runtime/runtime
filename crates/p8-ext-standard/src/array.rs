//! ext/standard's array functions (those that take no callback). Arguments
//! arrive already checked and converted to their declared types
//! (p8_runtime::args); a by-reference parameter receives the variable's
//! reference cell.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/array.c

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::CLASS_ENUM;
use p8_runtime::value::Value;
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

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "array_multisort", run: array_multisort, by_ref: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] },
    Builtin { name: "sizeof", run: crate::count, by_ref: &[] },
    Builtin { name: "array_values", run: array_values, by_ref: &[] },
    Builtin { name: "array_merge", run: array_merge, by_ref: &[] },
    Builtin { name: "array_key_exists", run: array_key_exists, by_ref: &[] },
    Builtin { name: "key_exists", run: array_key_exists, by_ref: &[] },
    Builtin { name: "in_array", run: in_array, by_ref: &[] },
    Builtin { name: "array_search", run: array_search, by_ref: &[] },
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
    Builtin { name: "array_key_first", run: array_key_first, by_ref: &[] },
    Builtin { name: "array_key_last", run: array_key_last, by_ref: &[] },
    Builtin { name: "array_first", run: array_first, by_ref: &[] },
    Builtin { name: "array_last", run: array_last, by_ref: &[] },
    Builtin { name: "array_count_values", run: array_count_values, by_ref: &[] },
    Builtin { name: "array_chunk", run: array_chunk, by_ref: &[] },
    Builtin { name: "array_diff", run: array_diff, by_ref: &[] },
    Builtin { name: "array_intersect", run: array_intersect, by_ref: &[] },
    Builtin { name: "array_diff_key", run: array_diff_key, by_ref: &[] },
    Builtin { name: "array_intersect_key", run: array_intersect_key, by_ref: &[] },
    Builtin { name: "array_replace", run: array_replace, by_ref: &[] },
    Builtin { name: "array_is_list", run: array_is_list, by_ref: &[] },
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
    Ok(Value::Array(Rc::new(values)))
}

fn list(values: Vec<Value>) -> Result<Value, Throw> {
    array(PhpArray::from_list(values))
}

/// Argument `i` as an array (the signature check has made sure it is one).
fn arr(args: &[Value], i: usize) -> Rc<PhpArray> {
    match arg(args, i) {
        Value::Array(a) => a,
        _ => Rc::new(PhpArray::new()),
    }
}

/// The array a by-reference parameter holds, to change in place.
fn with_array<R>(args: &[Value], change: impl FnOnce(&mut PhpArray) -> R) -> Option<R> {
    let Some(Value::Ref(cell)) = args.first() else { return None };
    let mut value = cell.borrow_mut();
    let Value::Array(a) = &mut *value else { return None };
    Some(change(Rc::make_mut(a)))
}

fn array_values(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = arr(args, 0);
    if a.is_list() {
        return Ok(Value::Array(a));
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
            return Ok(Value::Array(other));
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
    match v {
        Value::Null | Value::Undef => Some(Key::Str(p8_runtime::string::PhpStr::empty())),
        Value::Float(f) => Some(Key::Int(*f as i64)),
        v => Key::from_value(v).ok().map(|(k, _)| k),
    }
}

fn array_key_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    if matches!(arg(args, 0), Value::Null) {
        ctx.deprecated("Using null as the key parameter for array_key_exists() is deprecated, use an empty string instead");
    }
    let found = key_of(&arg(args, 0)).is_some_and(|k| arr(args, 1).contains_key(&k));
    Ok(Value::Bool(found))
}

pub(crate) fn matches(needle: &Value, v: &Value, strict: bool) -> bool {
    if strict { needle.identical(v) } else { needle.loose_eq(v) }
}

fn in_array(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (needle, strict) = (arg(args, 0), args.len() > 2 && arg(args, 2).to_bool());
    Ok(Value::Bool(arr(args, 1).values().any(|v| matches(&needle, &v.deref(), strict))))
}

fn array_search(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (needle, strict) = (arg(args, 0), args.len() > 2 && arg(args, 2).to_bool());
    Ok(arr(args, 1)
        .iter()
        .find(|(_, v)| matches(&needle, &v.deref(), strict))
        .map_or(Value::Bool(false), |(k, _)| k.to_value()))
}

/// array_slice($array, $offset, $length = null, $preserve_keys = false):
/// string keys are always kept, int keys renumbered unless preserved.
fn array_slice(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = arr(args, 0);
    let len = a.len() as i64;
    let offset = arg(args, 1).to_int();
    let start = if offset < 0 { (len + offset).max(0) } else { offset.min(len) };
    let end = match args.get(2).map(Value::deref) {
        None | Some(Value::Null) => len,
        Some(l) => {
            let l = l.to_int();
            if l < 0 { (len + l).max(start) } else { (start + l).min(len) }
        }
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
        match v.deref() {
            v @ (Value::Int(_) | Value::Str(_)) => {
                if let Ok((key, _)) = Key::from_value(&v) {
                    out.insert(key, k.to_value());
                }
            }
            _ => ctx.warn("array_flip(): Can only flip string and integer values, entry skipped"),
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
        let mut seen: std::collections::HashSet<Option<Vec<u8>>> = std::collections::HashSet::with_capacity(input.len());
        for (k, v) in input.iter() {
            let v = v.deref();
            if seen.insert(v.to_php_string().map(|x| x.as_bytes().to_vec())) {
                out.insert(k, v);
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
        if matches!(v, Value::Array(_)) {
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
    fold(ctx, args, p8_runtime::ops::BinOp::Add, Value::Int(0), "Addition")
}

fn array_product(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    fold(ctx, args, p8_runtime::ops::BinOp::Mul, Value::Int(1), "Multiplication")
}

fn array_pad(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (a, size, value) = (arr(args, 0), arg(args, 1).to_int(), arg(args, 2));
    let target = size.unsigned_abs() as usize;
    if target > 1 << 26 {
        return Err(ctx.throw("ValueError", "array_pad(): Argument #2 ($length) must not exceed the maximum allowed array size".into()));
    }
    if target <= a.len() {
        return Ok(Value::Array(a));
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

fn array_key_first(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(arr(args, 0).iter().next().map_or(Value::Null, |(k, _)| k.to_value()))
}

fn array_key_last(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(arr(args, 0).iter().last().map_or(Value::Null, |(k, _)| k.to_value()))
}

/// array_first(array $array): mixed (PHP 8.5): the first value, dereferenced
/// (RETURN_COPY_DEREF), or null for an empty array.
fn array_first(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(arr(args, 0).values().next().map_or(Value::Null, Value::deref))
}

/// array_last(array $array): mixed (PHP 8.5): the last value, dereferenced,
/// or null for an empty array.
fn array_last(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(arr(args, 0).values().last().map_or(Value::Null, Value::deref))
}

fn array_count_values(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = PhpArray::new();
    for v in arr(args, 0).values() {
        match v.deref() {
            v @ (Value::Int(_) | Value::Str(_)) => {
                if let Ok((key, _)) = Key::from_value(&v) {
                    let n = out.get(&key).map_or(0, Value::to_int);
                    out.insert(key, Value::Int(n + 1));
                }
            }
            _ => ctx.warn("array_count_values(): Can only count string and integer values, entry skipped"),
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
            chunks.push(Value::Array(Rc::new(std::mem::take(&mut chunk))));
        }
    }
    if !chunk.is_empty() {
        chunks.push(Value::Array(Rc::new(chunk)));
    }
    list(chunks)
}

/// The string forms of a value, for array_diff / array_intersect, which
/// compare `(string) $a === (string) $b`.
fn text(v: &Value) -> Vec<u8> {
    v.deref().to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_else(|| b"Array".to_vec())
}

fn by_values(args: &[Value], keep_if_found: bool) -> Result<Value, Throw> {
    let others: Vec<std::collections::HashSet<Vec<u8>>> =
        (1..args.len()).map(|i| arr(args, i).values().map(text).collect()).collect();
    let mut out = PhpArray::new();
    for (k, v) in arr(args, 0).iter() {
        let t = text(v);
        let keep = if keep_if_found { others.iter().all(|o| o.contains(&t)) } else { !others.iter().any(|o| o.contains(&t)) };
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

fn array_is_list(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(arr(args, 0).is_list()))
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
        Some(Ok(n)) => Ok(Value::Int(n)),
        Some(Err(())) => Err(ctx.throw("Error", "Cannot add element to the array as the next element is already occupied".into())),
        None => Ok(Value::Null),
    }
}

fn array_pop(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(with_array(args, PhpArray::pop).flatten().unwrap_or(Value::Null))
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
    Ok(shifted.flatten().unwrap_or(Value::Null))
}

fn array_unshift(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let values: Vec<Value> = args[1..].iter().map(Value::deref).collect();
    let n = with_array(args, |a| {
        let front = values.into_iter().map(|v| (Key::Int(0), v));
        let rest: Vec<(Key, Value)> = a.iter().map(|(k, v)| (k, v.clone())).collect();
        *a = renumbered(front.chain(rest));
        a.len() as i64
    });
    Ok(n.map_or(Value::Null, Value::Int))
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
        None => match (a, b) {
            (Value::Object(x), Value::Object(y)) if y.class.flags & CLASS_ENUM != 0 && x.class.flags & CLASS_ENUM != 0 => {
                (Rc::as_ptr(x) as usize).cmp(&(Rc::as_ptr(y) as usize))
            }
            (_, Value::Object(y)) if y.class.flags & CLASS_ENUM != 0 => Ordering::Less,
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
        match a.deref() {
            Value::Array(_) => {
                arrays.push((i, false, 0));
                (order_free, type_free) = (true, true);
            }
            Value::Int(n) => match n & !8 {
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
            },
            _ => return Err(ctx.throw("TypeError", format!("array_multisort(): Argument #{} must be an array or a sort flag", i + 1))),
        }
    }
    let columns: Vec<Vec<(Key, Value)>> = arrays
        .iter()
        .map(|(i, _, _)| match args[*i].deref() {
            Value::Array(a) => a.iter().map(|(k, v)| (k, v.clone())).collect(),
            _ => Vec::new(),
        })
        .collect();
    let size = columns[0].len();
    if columns.iter().any(|c| c.len() != size) {
        return Err(ctx.throw("ValueError", "Array sizes are inconsistent".into()));
    }
    if size == 0 {
        return Ok(Value::Bool(true));
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
        if let Value::Ref(cell) = &args[*i] {
            *cell.borrow_mut() = Value::Array(Rc::new(out));
        }
    }
    Ok(Value::Bool(true))
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
    Ok(Value::Bool(done.is_some()))
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

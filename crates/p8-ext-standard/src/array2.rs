//! More of ext/standard's array functions: the internal pointer
//! (current, next, reset, ...), array_splice, the recursive merges,
//! array_walk_recursive, array_column, the _assoc set operations; and
//! the functions that reach the caller's variables (compact, extract,
//! get_defined_vars, func_get_arg, func_num_args) and settype.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/array.c, basic_functions.c,
//! $PHP_SRC_PATH/Zend/zend_builtin_functions.c

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "current", run: current, by_ref: &[] },
    Builtin { name: "pos", run: current, by_ref: &[] },
    Builtin { name: "key", run: key, by_ref: &[] },
    Builtin { name: "next", run: next, by_ref: &[0] },
    Builtin { name: "prev", run: prev, by_ref: &[0] },
    Builtin { name: "reset", run: reset, by_ref: &[0] },
    Builtin { name: "end", run: end, by_ref: &[0] },
    Builtin { name: "array_splice", run: array_splice, by_ref: &[0] },
    Builtin { name: "array_merge_recursive", run: array_merge_recursive, by_ref: &[] },
    Builtin { name: "array_replace_recursive", run: array_replace_recursive, by_ref: &[] },
    Builtin { name: "array_walk_recursive", run: array_walk_recursive, by_ref: &[0] },
    Builtin { name: "array_column", run: array_column, by_ref: &[] },
    Builtin { name: "array_diff_assoc", run: array_diff_assoc, by_ref: &[] },
    Builtin { name: "array_intersect_assoc", run: array_intersect_assoc, by_ref: &[] },
    Builtin { name: "compact", run: compact, by_ref: &[] },
    Builtin { name: "extract", run: extract, by_ref: &[0] },
    Builtin { name: "get_defined_vars", run: get_defined_vars, by_ref: &[] },
    Builtin { name: "func_get_arg", run: func_get_arg, by_ref: &[] },
    Builtin { name: "func_num_args", run: func_num_args, by_ref: &[] },
    Builtin { name: "settype", run: settype, by_ref: &[0] },
    Builtin { name: "array_change_key_case", run: array_change_key_case, by_ref: &[] },
];

/// array_change_key_case($array, $case = CASE_LOWER): string keys in ASCII
/// lower (or upper, CASE_UPPER = 1) case; a later key that comes out the
/// same overwrites the value, in the first one's place.
fn array_change_key_case(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let upper = args.len() > 1 && arg(args, 1).to_int() != 0;
    let Value::Array(a) = arg(args, 0) else { return Ok(Value::Null) };
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let k = match k {
            Key::Str(s) => {
                let b = s.as_bytes();
                Key::Str(PhpStr::from(&*if upper { b.to_ascii_uppercase() } else { b.to_ascii_lowercase() }))
            }
            k => k.clone(),
        };
        out.insert(k, v.clone());
    }
    Ok(Value::Array(Rc::new(out)))
}

/// The builtins here that reach the caller's variables or run PHP code.
pub const REENTER: &[&str] = &["compact", "extract", "get_defined_vars", "func_get_arg", "func_num_args"];

fn array_of(v: &Value) -> Option<Rc<PhpArray>> {
    match v.deref() {
        Value::Array(a) => Some(a),
        _ => None,
    }
}

/// current($array): the element at the internal pointer, or false.
fn current(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(a) = array_of(&arg(args, 0)) else { return Ok(Value::Bool(false)) };
    Ok(a.entry_at(a.cursor()).map_or(Value::Bool(false), |(_, _, v)| v.deref()))
}

fn key(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(a) = array_of(&arg(args, 0)) else { return Ok(Value::Null) };
    Ok(a.entry_at(a.cursor()).map_or(Value::Null, |(_, k, _)| k.to_value()))
}

/// Move the by-reference array's pointer (`step` gives the new position)
/// and return the element there, or false.
fn move_cursor(args: &[Value], step: impl FnOnce(&PhpArray) -> Option<usize>) -> Value {
    let Some(Value::Ref(cell)) = args.first() else { return Value::Bool(false) };
    let mut value = cell.borrow_mut();
    let Value::Array(a) = &mut *value else { return Value::Bool(false) };
    let to = step(a).unwrap_or(a.end_pos());
    let a = Rc::make_mut(a);
    a.set_cursor(to);
    a.entry_at(to).map_or(Value::Bool(false), |(_, _, v)| v.deref())
}

fn next(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(move_cursor(args, |a| a.entry_at(a.cursor()).and_then(|(p, _, _)| a.entry_at(p + 1)).map(|(p, _, _)| p)))
}

fn prev(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(move_cursor(args, |a| a.entry_before(a.cursor().min(a.end_pos())).map(|(p, _, _)| p)))
}

fn reset(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(move_cursor(args, |a| a.entry_at(0).map(|(p, _, _)| p)))
}

fn end(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(move_cursor(args, |a| a.entry_before(a.end_pos()).map(|(p, _, _)| p)))
}

/// array_splice(array &$array, int $offset, ?int $length = null, mixed $replacement = []):
/// the removed elements; the rest renumbered (string keys kept).
fn array_splice(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(Value::Ref(cell)) = args.first().cloned() else { return Ok(Value::Array(Rc::new(PhpArray::new()))) };
    let a = match &*cell.borrow() {
        Value::Array(a) => Rc::clone(a),
        _ => return Ok(Value::Array(Rc::new(PhpArray::new()))),
    };
    let entries: Vec<(Key, Value)> = a.iter().map(|(k, v)| (k, v.clone())).collect();
    let n = entries.len() as i64;
    let offset = arg(args, 1).to_int();
    let start = if offset < 0 { (n + offset).max(0) } else { offset.min(n) } as usize;
    let length = match args.get(2).map(Value::deref) {
        None | Some(Value::Null) => n - start as i64,
        Some(v) => v.to_int(),
    };
    let stop = if length < 0 { (n + length).max(start as i64) } else { (start as i64 + length).min(n) } as usize;
    let replacement: Vec<Value> = match args.get(3).map(Value::deref) {
        None => Vec::new(),
        Some(Value::Array(r)) => r.values().cloned().collect(),
        Some(v) => vec![v],
    };
    let mut kept = PhpArray::new();
    let mut removed = PhpArray::new();
    let put = |out: &mut PhpArray, k: Key, v: Value| match k {
        Key::Str(_) => out.insert(k, v),
        Key::Int(_) => {
            let _ = out.push(v);
        }
    };
    for (i, (k, v)) in entries.into_iter().enumerate() {
        if i == start {
            for r in &replacement {
                let _ = kept.push(r.clone());
            }
        }
        if i >= start && i < stop {
            put(&mut removed, k, v);
        } else {
            put(&mut kept, k, v);
        }
    }
    if start >= n as usize {
        for r in replacement {
            let _ = kept.push(r);
        }
    }
    *cell.borrow_mut() = Value::Array(Rc::new(kept));
    Ok(Value::Array(Rc::new(removed)))
}

/// Merge `from` into `into` recursively (array_merge_recursive): int keys
/// appended, string keys colliding merged into arrays.
fn merge_into(into: &mut PhpArray, from: &PhpArray) {
    for (k, v) in from.iter() {
        let v = v.deref();
        match k {
            Key::Int(_) => {
                let _ = into.push(v);
            }
            Key::Str(_) => match into.get(&k).map(Value::deref) {
                Some(existing) => {
                    let mut merged = match existing {
                        Value::Array(e) => (*e).clone(),
                        other => PhpArray::from_list(vec![other]),
                    };
                    match &v {
                        Value::Array(va) => merge_into(&mut merged, va),
                        other => {
                            let _ = merged.push(other.clone());
                        }
                    }
                    into.insert(k, Value::Array(Rc::new(merged)));
                }
                None => into.insert(k, v),
            },
        }
    }
}

fn array_merge_recursive(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = PhpArray::new();
    for v in args.iter() {
        if let Some(a) = array_of(v) {
            merge_into(&mut out, &a);
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

fn replace_into(into: &mut PhpArray, from: &PhpArray) {
    for (k, v) in from.iter() {
        let v = v.deref();
        match (into.get(&k).map(Value::deref), &v) {
            (Some(Value::Array(e)), Value::Array(va)) => {
                let mut merged = (*e).clone();
                replace_into(&mut merged, va);
                into.insert(k, Value::Array(Rc::new(merged)));
            }
            _ => into.insert(k, v),
        }
    }
}

fn array_replace_recursive(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = array_of(&arg(args, 0)).map(|a| (*a).clone()).unwrap_or_default();
    for v in &args[1..] {
        if let Some(a) = array_of(v) {
            replace_into(&mut out, &a);
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

/// array_walk_recursive(array &$array, callable $callback, mixed $arg)
fn array_walk_recursive(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(Value::Ref(cell)) = args.first().cloned() else { return Ok(Value::Bool(true)) };
    let callback = arg(args, 1);
    let extra = (args.len() > 2).then(|| arg(args, 2));
    let walked = walk(ctx, cell.borrow().clone(), &callback, &extra)?;
    *cell.borrow_mut() = walked;
    Ok(Value::Bool(true))
}

fn walk(ctx: &mut dyn Ctx, v: Value, callback: &Value, extra: &Option<Value>) -> Result<Value, Throw> {
    let Value::Array(a) = v else { return Ok(v) };
    let mut out = (*a).clone();
    for (k, v) in a.iter() {
        let v = v.deref();
        let new = if let Value::Array(_) = v {
            walk(ctx, v, callback, extra)?
        } else {
            let element = Rc::new(RefCell::new(v));
            let mut call = vec![Value::Ref(Rc::clone(&element)), k.to_value()];
            call.extend(extra.clone());
            ctx.call(callback, &mut call)?;
            element.replace(Value::Null)
        };
        out.insert(k, new);
    }
    Ok(Value::Array(Rc::new(out)))
}

/// array_column(array $array, int|string|null $column_key, int|string|null $index_key = null)
fn array_column(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let rows = array_of(&arg(args, 0)).unwrap_or_default();
    let column = arg(args, 1);
    let index = if args.len() > 2 { arg(args, 2) } else { Value::Null };
    let pick = |row: &Value, k: &Value| -> Option<Value> {
        let key = Key::from_value(k).ok()?.0;
        match row.deref() {
            Value::Array(r) => r.get(&key).map(Value::deref),
            Value::Object(o) => {
                let name = k.to_php_string()?;
                let slot = o.class.slot(name.as_bytes(), None)?;
                let p = &o.class.properties[slot];
                (p.visibility == p8_runtime::object::Visibility::Public)
                    .then(|| o.slots.borrow()[slot].deref())
                    .or_else(|| o.dynamic.borrow().get(&key).map(Value::deref))
            }
            _ => None,
        }
    };
    let mut out = PhpArray::new();
    for row in rows.values() {
        let value = match &column {
            Value::Null => row.deref(),
            k => match pick(row, k) {
                Some(v) => v,
                None => continue,
            },
        };
        match (!matches!(index, Value::Null)).then(|| pick(row, &index)).flatten() {
            Some(i) => match Key::from_value(&i) {
                Ok((k, _)) => out.insert(k, value),
                Err(_) => {
                    let _ = out.push(value);
                }
            },
            None => {
                let _ = out.push(value);
            }
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

fn assoc_set(args: &[Value], keep_when_found: bool) -> Value {
    let first = array_of(&arg(args, 0)).unwrap_or_default();
    let others: Vec<Rc<PhpArray>> = args[1..].iter().filter_map(array_of).collect();
    let text = |v: &Value| v.to_php_string().unwrap_or_else(PhpStr::empty);
    let mut out = PhpArray::new();
    for (k, v) in first.iter() {
        let found_in = |o: &Rc<PhpArray>| o.get(&k).is_some_and(|x| text(&x.deref()) == text(&v.deref()));
        let keep = if keep_when_found { others.iter().all(found_in) } else { !others.iter().any(found_in) };
        if keep {
            out.insert(k, v.clone());
        }
    }
    Value::Array(Rc::new(out))
}

fn array_diff_assoc(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(assoc_set(args, false))
}

fn array_intersect_assoc(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(assoc_set(args, true))
}

/// compact(array|string $var_name, array|string ...$var_names)
fn compact(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut names = Vec::new();
    fn collect(v: &Value, names: &mut Vec<PhpStr>) {
        match v.deref() {
            Value::Array(a) => {
                for x in a.values() {
                    collect(x, names);
                }
            }
            other => names.push(other.to_php_string().unwrap_or_else(PhpStr::empty)),
        }
    }
    for v in args.iter() {
        collect(v, &mut names);
    }
    let mut out = PhpArray::new();
    for n in names {
        match ctx.local(n.as_bytes()) {
            Some(v) => out.insert(Key::from_string(n), v),
            None => ctx.warn(&format!("compact(): Undefined variable ${}", String::from_utf8_lossy(n.as_bytes()))),
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

const EXTR_OVERWRITE: i64 = 0;
const EXTR_SKIP: i64 = 1;
const EXTR_PREFIX_SAME: i64 = 2;
const EXTR_PREFIX_ALL: i64 = 3;
const EXTR_IF_EXISTS: i64 = 6;

fn valid_name(n: &[u8]) -> bool {
    matches!(n.first(), Some(c) if c.is_ascii_alphabetic() || *c == b'_' || *c >= 0x80)
        && n.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c >= 0x80)
}

/// extract(array &$array, int $flags = EXTR_OVERWRITE, string $prefix = ""): how many it set.
fn extract(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let a = array_of(&arg(args, 0)).unwrap_or_default();
    let flags = if args.len() > 1 { arg(args, 1).to_int() & 0xff } else { EXTR_OVERWRITE };
    let prefix = str_arg(args, 2);
    let mut n = 0;
    for (k, v) in a.iter() {
        let Key::Str(name) = k else { continue };
        let exists = ctx.local(name.as_bytes()).is_some();
        let target: Option<Vec<u8>> = match flags {
            EXTR_SKIP if exists => None,
            EXTR_IF_EXISTS if !exists => None,
            EXTR_PREFIX_ALL => Some([prefix.as_bytes(), b"_", name.as_bytes()].concat()),
            EXTR_PREFIX_SAME if exists => Some([prefix.as_bytes(), b"_", name.as_bytes()].concat()),
            _ => Some(name.as_bytes().to_vec()),
        };
        let Some(target) = target else { continue };
        if !valid_name(&target) || target == b"this" {
            continue;
        }
        if ctx.set_local(&target, v.deref()) {
            n += 1;
        }
    }
    Ok(Value::Int(n))
}

fn get_defined_vars(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let mut out = PhpArray::new();
    for (n, v) in ctx.locals() {
        out.insert(Key::from_string(n), v);
    }
    Ok(Value::Array(Rc::new(out)))
}

fn func_get_arg(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = arg(args, 0).to_int();
    if n < 0 {
        return Err(ctx.throw("ValueError", "func_get_arg(): Argument #1 ($position) must be greater than or equal to 0".into()));
    }
    let all = ctx.call_args();
    match all.get(n as usize) {
        Some(v) => Ok(v.clone()),
        None => Err(ctx.throw(
            "ValueError",
            "func_get_arg(): Argument #1 ($position) must be less than the number of the arguments passed to the currently executed function".into(),
        )),
    }
}

fn func_num_args(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(ctx.call_args().len() as i64))
}

/// settype(mixed &$var, string $type)
fn settype(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(Value::Ref(cell)) = args.first().cloned() else { return Ok(Value::Bool(false)) };
    let ty = str_arg(args, 1).as_bytes().to_ascii_lowercase();
    let v = cell.borrow().clone();
    let new = match ty.as_slice() {
        b"bool" | b"boolean" => Value::Bool(v.to_bool()),
        b"int" | b"integer" => Value::Int(v.to_int()),
        b"float" | b"double" => Value::Float(v.to_float()),
        b"string" => Value::Str(v.to_php_string().unwrap_or_else(|| PhpStr::from("Array"))),
        b"array" => match v {
            Value::Array(_) => v,
            Value::Null => Value::Array(Rc::new(PhpArray::new())),
            other => Value::Array(Rc::new(PhpArray::from_list(vec![other]))),
        },
        b"null" => Value::Null,
        b"resource" => return Err(ctx.throw("ValueError", "Cannot convert to resource type".into())),
        _ => return Err(ctx.throw("ValueError", "settype(): Argument #2 ($type) must be a valid type".into())),
    };
    *cell.borrow_mut() = new;
    Ok(Value::Bool(true))
}

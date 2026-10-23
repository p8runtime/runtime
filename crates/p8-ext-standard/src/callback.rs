//! The functions that call PHP code: call_user_func, array_map,
//! array_filter, array_reduce, array_walk, usort and friends. Callbacks
//! run through `Ctx::call`; an exception one throws ends the builtin.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/array.c,
//! $PHP_SRC_PATH/ext/standard/basic_functions.c (call_user_func)

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
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
    Builtin { name: "array_walk", run: array_walk, by_ref: &[0] },
    Builtin { name: "usort", run: usort, by_ref: &[0] },
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
    let mut rest: Vec<Value> = array(args, 1).values().map(Value::deref).collect();
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
    let Some((cell, a)) = take_array(args) else { return Ok(Value::Bool(true)) };
    let callback = arg(args, 1);
    let extra = (args.len() > 2).then(|| arg(args, 2));
    for (k, v) in a.iter() {
        let element = Rc::new(RefCell::new(v.deref()));
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
    let mut entries: Vec<(Key, Value)> = a.iter().map(|(k, v)| (k, v.clone())).collect();
    let mut error = None;
    entries.sort_by(|x, y| {
        if error.is_some() {
            return Ordering::Equal;
        }
        let mut pair = if by_key { [x.0.to_value(), y.0.to_value()] } else { [x.1.deref(), y.1.deref()] };
        match ctx.call(&callback, &mut pair) {
            Ok(r) => r.to_int().cmp(&0),
            Err(e) => {
                error = Some(e);
                Ordering::Equal
            }
        }
    });
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

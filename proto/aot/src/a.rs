//! V4a, "dumb AOT": what a compiler that knows nothing about the operands
//! emits. The frame's variables are one array on the native stack (index
//! = the CV number in p8's bytecode); a literal is read from the constant
//! table; every operation is a call of its generic helper (`rt::h_*`),
//! which resolves undefined variables and references itself. No dispatch:
//! the control flow is the function's own (`loop`, `if`), straight from
//! the bytecode's jumps.
//!
//! The code below follows each program's bytecode (V1's `--dump` of the
//! same file) instruction by instruction:
//!
//! ```text
//!   $i = 0                 ASSIGN_CONST_COPY      h_assign(h_copy(k))
//!   jmp test               JMP                    loop {
//!   test: $i < $n          JCMP_LT_T_CV_CV          if !h_less { break }
//!   body                   ...                      ...
//!   ++$i                   STEP_JMP_INC_LT_CV       h_inc }
//! ```

use crate::k;
use crate::rt::*;
use p8_runtime::object::Object;
use p8_runtime::ops::BinOp;
use p8_runtime::value::Value;
use std::mem;
use std::rc::Rc;

// The variables at the reads that may warn (name, line in the source).
static N7: Var = Var { name: "n", line: 7 };
static I7: Var = Var { name: "i", line: 7 };
static I8: Var = Var { name: "i", line: 8 };
static Y8: Var = Var { name: "y", line: 8 };
static O8: Var = Var { name: "o", line: 8 };
static S8: Var = Var { name: "s", line: 8 };
static A8: Var = Var { name: "a", line: 8 };
static I10: Var = Var { name: "i", line: 10 };
static X10: Var = Var { name: "x", line: 10 };
static O10: Var = Var { name: "o", line: 10 };
static A10: Var = Var { name: "a", line: 10 };
static FA: Var = Var { name: "a", line: 2 };
static MA: Var = Var { name: "a", line: 4 };

/// A frame of `N` variables, the arguments moved into the first ones.
#[inline(always)]
fn frame<const N: usize>(args: &mut [Value]) -> [Value; N] {
    let mut v = [const { Value::UNDEF }; N];
    for (d, a) in v.iter_mut().zip(args.iter_mut()) {
        *d = mem::replace(a, Value::UNDEF);
    }
    v
}

/// `echo $i, " ", <x>, "\n";` (the check variants' last line).
fn tail(cx: &Cx, st: &mut St, i: &Value, x: &Value, vx: &Var) -> R<()> {
    h_echo(cx, st, i, &I10)?;
    h_echo(cx, st, &cx.k[k::SPACE], &NOVAR)?;
    h_echo(cx, st, x, vx)?;
    h_echo(cx, st, &cx.k[k::NL], &NOVAR)
}

// ---- the other functions of the file ----------------------------------------

/// `function f($a) { return $a; }`
#[inline(never)]
fn f(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<1>(args);
    Ok(h_return(cx, st, &mut v[0], &FA))
}

/// `P::m($a) { return $a; }`
#[inline(never)]
fn p_m(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<1>(args);
    Ok(h_return(cx, st, &mut v[0], &MA))
}

pub fn methods() -> Vec<MethodEntry> {
    vec![MethodEntry { class: b"p", name: b"m", nparams: 1, required: 1, generic: p_m, one: None }]
}

// ---- t, per program -----------------------------------------------------------
//
// Variables: 0 = $n, 1 = $i (or the variable set before the loop, $i
// then 2), the rest in the bytecode's order.

#[inline(never)]
fn t_empty(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<2>(args); // $n $i
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[1], &I7, &v[0], &N7) {
            break;
        }
        h_inc(cx, st, &mut v[1], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[1], &cx.k[k::DASH], &NOVAR)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_add(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<3>(args); // $n $i $x
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[1], &I7, &v[0], &N7) {
            break;
        }
        let r = h_binary(cx, st, BinOp::Add, &v[1], &I8, &cx.k[k::ONE], &NOVAR)?;
        h_assign(&mut v[2], r);
        h_inc(cx, st, &mut v[1], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[1], &v[2], &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_assign(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<4>(args); // $n $y $i $x
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::FIVE], &NOVAR));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let r = h_copy(cx, st, &v[1], &Y8);
        h_assign(&mut v[3], r);
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[2], &v[3], &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_call(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<3>(args); // $n $i $x
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[1], &I7, &v[0], &N7) {
            break;
        }
        let mut a = [h_copy(cx, st, &v[1], &I8)];
        let r = h_call(cx, st, f, 1, 1, &mut a)?;
        h_assign(&mut v[2], r);
        h_inc(cx, st, &mut v[1], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[1], &v[2], &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_method(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<4>(args); // $n $o $i $x
    h_assign(&mut v[1], h_new(cx, st, 0));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let mut a = [h_copy(cx, st, &v[2], &I8)];
        let r = h_method_call(cx, st, &v[1], &O8, &cx.caches[0], b"m", &mut a)?;
        h_assign(&mut v[3], r);
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[2], &v[3], &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_prop_read(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<4>(args); // $n $o $i $x
    h_assign(&mut v[1], h_new(cx, st, 0));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let r = h_fetch_prop(cx, st, &v[1], &O8, &cx.caches[0], b"p")?;
        h_assign(&mut v[3], r);
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[2], &v[3], &X10)?;
    }
    Ok(Value::NULL)
}

/// `prop_write` and `prop_typed`: `$o->{name} = $i`.
#[inline(always)]
fn prop_store(cx: &Cx, st: &mut St, args: &mut [Value], name: &'static [u8]) -> R<Value> {
    let mut v = frame::<3>(args); // $n $o $i
    h_assign(&mut v[1], h_new(cx, st, 0));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let x = h_copy(cx, st, &v[2], &I8);
        h_assign_prop(cx, st, &v[1], &O8, &cx.caches[0], name, x)?;
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        let mut p = h_fetch_prop(cx, st, &v[1], &O10, &cx.caches[1], name)?;
        tail(cx, st, &v[2], &p, &NOVAR)?;
        h_free(&mut p);
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_prop_write(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    prop_store(cx, st, args, b"p")
}

#[inline(never)]
fn t_prop_typed(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    prop_store(cx, st, args, b"t")
}

#[inline(never)]
fn t_arr_read(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<4>(args); // $n $a $i $x
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::ARR], &NOVAR));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let r = h_fetch_dim(cx, st, &v[1], &A8, &cx.k[k::TWO], 8)?;
        h_assign(&mut v[3], r);
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[2], &v[3], &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_arr_write(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<3>(args); // $n $a $i
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::ARR], &NOVAR));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let x = h_copy(cx, st, &v[2], &I8);
        h_assign_dim(&mut v[1], &cx.k[k::TWO], x)?;
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        let mut e = h_fetch_dim(cx, st, &v[1], &A10, &cx.k[k::TWO], 10)?;
        tail(cx, st, &v[2], &e, &NOVAR)?;
        h_free(&mut e);
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_isset_dim(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<4>(args); // $n $a $i $x
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::ARR], &NOVAR));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let r = h_isset_dim(&v[1], &cx.k[k::KEY_K])?;
        h_assign(&mut v[3], Value::bool(r));
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[2], &v[3], &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_concat(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<4>(args); // $n $s $i $x
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::AB], &NOVAR));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let r = h_binary(cx, st, BinOp::Concat, &v[1], &S8, &cx.k[k::C], &NOVAR)?;
        h_assign(&mut v[3], r);
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[2], &v[3], &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_new(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<3>(args); // $n $i $x
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[1], &I7, &v[0], &N7) {
            break;
        }
        let o = h_new(cx, st, 0);
        h_assign(&mut v[2], o);
        h_inc(cx, st, &mut v[1], &I7)?;
    }
    if cx.check {
        let mut p = h_fetch_prop(cx, st, &v[2], &X10, &cx.caches[1], b"p")?;
        tail(cx, st, &v[1], &p, &NOVAR)?;
        h_free(&mut p);
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_identical(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let mut v = frame::<4>(args); // $n $s $i $x
    h_assign(&mut v[1], h_copy(cx, st, &cx.k[k::AB], &NOVAR));
    h_assign(&mut v[2], h_copy(cx, st, &cx.k[k::ZERO], &NOVAR));
    loop {
        if !h_less(cx, st, &v[2], &I7, &v[0], &N7) {
            break;
        }
        let r = h_identical(cx, st, &v[1], &S8, &cx.k[k::AB], &NOVAR);
        h_assign(&mut v[3], Value::bool(r));
        h_inc(cx, st, &mut v[2], &I7)?;
    }
    if cx.check {
        tail(cx, st, &v[2], &v[3], &X10)?;
    }
    Ok(Value::NULL)
}

/// Run `t(n)` of the program `name`; None if it is not compiled here.
pub fn run(name: &str, cx: &Cx, st: &mut St, n: i64) -> Option<R<Value>> {
    let t: GenericFn = match name {
        "empty" => t_empty,
        "add" => t_add,
        "assign" => t_assign,
        "call" => t_call,
        "method" => t_method,
        "prop_read" => t_prop_read,
        "prop_write" => t_prop_write,
        "prop_typed" => t_prop_typed,
        "arr_read" => t_arr_read,
        "arr_write" => t_arr_write,
        "isset_dim" => t_isset_dim,
        "concat" => t_concat,
        "new" => t_new,
        "identical" => t_identical,
        _ => return None,
    };
    let mut args = [Value::int(n)];
    Some(h_call(cx, st, t, 1, 1, &mut args))
}

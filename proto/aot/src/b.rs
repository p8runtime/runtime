//! V4b, "kind-aware AOT": what a compiler emits when it uses what the
//! bytecode tells about each operand, the knowledge V1's translator
//! specialises on, and nothing more:
//!
//! - a literal's value and type (an int literal is an immediate, a string
//!   literal its bytes or its program-memory string);
//! - a temporary (a call's result, `new`'s object, an isset's bool) is
//!   never undefined and never a reference, so it is assigned as it is;
//! - a CV may be undefined or a reference: every read keeps both tests,
//!   every write the reference test and the release, as V1's do;
//! - a value's type is not known (no type inference): an int operation is
//!   an inline test of both tags with the overflow check, the generic
//!   helper (`rt::h_*`) the cold fallback, as V1's handlers have it.
//!
//! The variables are Rust locals. Calls of user functions are direct
//! (static binding, as V1 has it); a method call goes through the site's
//! inline cache to the cached entry point (an indirect call).

use crate::rt::*;
use crate::{k, s};
use p8_runtime::array::Key;
use p8_runtime::object::{Object, Slot};
use p8_runtime::ops::BinOp;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::mem;
use std::rc::Rc;

static N7: Var = Var { name: "n", line: 7 };
static I7: Var = Var { name: "i", line: 7 };
static I8: Var = Var { name: "i", line: 8 };
static Y8: Var = Var { name: "y", line: 8 };
static S8: Var = Var { name: "s", line: 8 };
static I10: Var = Var { name: "i", line: 10 };
static X10: Var = Var { name: "x", line: 10 };
static FA: Var = Var { name: "a", line: 2 };
static MA: Var = Var { name: "a", line: 4 };

// ---- the inline pieces ---------------------------------------------------------

/// A copy of a CV: undefined (warning, null) and reference tested inline,
/// both cold.
#[inline(always)]
fn rd_cv(cx: &Cx, st: &mut St, v: &Value, var: &Var) -> Value {
    if v.is_undef() {
        return undef_var(cx, st, var);
    }
    if v.is_reference() {
        return deref_ref(v);
    }
    v.clone()
}

/// `$a = v`: a plain old value overwritten (one tag test), a reference
/// written through, one holding memory released.
#[inline(always)]
fn wr_cv(dst: &mut Value, v: Value) {
    if dst.is_plain() {
        mem::forget(mem::replace(dst, v));
        return;
    }
    if dst.is_reference() {
        write_ref(dst, v);
        return;
    }
    let _ = mem::replace(dst, v).release();
}

/// `$a < $b` on two CVs: two ints inline, the rest the generic helper.
#[inline(always)]
fn lt_cv(cx: &Cx, st: &mut St, a: &Value, va: &Var, b: &Value, vb: &Var) -> bool {
    if let (Some(x), Some(y)) = (a.as_int(), b.as_int()) { x < y } else { h_less(cx, st, a, va, b, vb) }
}

/// `++$a` on a CV: an int that does not overflow in place, the rest
/// (overflow to float, undefined, a reference, other types) the helper.
#[inline(always)]
fn inc_cv(cx: &Cx, st: &mut St, a: &mut Value, var: &Var) -> R<()> {
    if let Some(p) = a.as_int_mut()
        && let Some(n) = p.checked_add(1)
    {
        *p = n;
        return Ok(());
    }
    h_inc(cx, st, a, var)
}

/// `for ($i = 0; $i < $n; $i++) body`, as the bytecode has it: `$i`
/// assigned, a jump to the test, the body, the step, the test.
macro_rules! for_loop {
    ($cx:ident, $st:ident, $i:ident, $n:ident, $body:block) => {
        wr_cv(&mut $i, Value::int(0));
        while lt_cv($cx, $st, &$i, &I7, &$n, &N7) {
            $body
            inc_cv($cx, $st, &mut $i, &I7)?;
        }
    };
}

/// `echo $i, " ", <x>, "\n";` (the check variants' last line).
fn tail(cx: &Cx, st: &mut St, i: &Value, x: &Value, vx: &Var) -> R<()> {
    h_echo(cx, st, i, &I10)?;
    h_echo(cx, st, &cx.k[k::SPACE], &NOVAR)?;
    h_echo(cx, st, x, vx)?;
    h_echo(cx, st, &cx.k[k::NL], &NOVAR)
}

/// A call's depth guard around `f`.
macro_rules! guarded {
    ($st:ident, $call:expr) => {{
        if $st.depth >= MAX_DEPTH {
            return too_deep();
        }
        $st.depth += 1;
        let r = $call;
        $st.depth -= 1;
        r?
    }};
}

// ---- the other functions of the file ---------------------------------------------

/// `return $a` of a CV: moved out, undefined and reference tested.
#[inline(always)]
fn ret_cv(cx: &Cx, st: &mut St, a: Value, var: &Var) -> Value {
    if a.is_undef() {
        undef_var(cx, st, var)
    } else if a.is_reference() {
        deref_ref(&a)
    } else {
        a
    }
}

/// `function f($a) { return $a; }`
#[inline(never)]
fn f(cx: &Cx, st: &mut St, a: Value) -> R<Value> {
    Ok(ret_cv(cx, st, a, &FA))
}

/// `P::m($a) { return $a; }`
#[inline(never)]
fn p_m(cx: &Cx, st: &mut St, _this: &Rc<Object>, a: Value) -> R<Value> {
    Ok(ret_cv(cx, st, a, &MA))
}

/// The same method by the generic convention (a site with another
/// argument count would call this).
fn p_m_generic(cx: &Cx, st: &mut St, _this: Option<Rc<Object>>, args: &mut [Value]) -> R<Value> {
    let a = args.first_mut().map_or(Value::UNDEF, |a| mem::replace(a, Value::UNDEF));
    Ok(ret_cv(cx, st, a, &MA))
}

pub fn methods() -> Vec<MethodEntry> {
    vec![MethodEntry { class: b"p", name: b"m", nparams: 1, required: 1, generic: p_m_generic, one: Some(p_m) }]
}

/// The object a CV holds, for `->`: not undefined, not a reference
/// (as V1: anything else faults).
#[inline(always)]
fn obj_cv(v: &Value) -> R<&Rc<Object>> {
    match v.as_object() {
        Some(o) => Ok(o),
        None => fault_kind("->", v),
    }
}

/// The slot a property site answers for `o`: the cache's class test.
#[inline(always)]
fn prop_slot(o: &Object, cache: &Cache, name: &[u8]) -> R<usize> {
    if cache.class.get() == Rc::as_ptr(&o.class) as usize { Ok(cache.slot.get() as usize) } else { fill_prop(o, cache, name) }
}

/// A property's value: copied, read through a reference; unset faults.
#[inline(always)]
fn prop_read(o: &Object, slot: usize) -> R<Value> {
    let v = o.slots()[slot].peek(|v| {
        if v.is_undef() {
            None
        } else if v.is_reference() {
            Some(deref_ref(v))
        } else {
            Some(v.clone())
        }
    });
    match v {
        Some(v) => Ok(v),
        None => fault("read of an unset or uninitialized property not in the prototype"),
    }
}

// ---- t, per program ------------------------------------------------------------

#[inline(never)]
fn t_empty(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let mut i = Value::UNDEF;
    for_loop!(cx, st, i, n, {});
    if cx.check {
        tail(cx, st, &i, &cx.k[k::DASH], &NOVAR)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_add(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut i, mut x) = (Value::UNDEF, Value::UNDEF);
    for_loop!(cx, st, i, n, {
        // $x = $i + 1: CV + int literal.
        let r = match i.as_int().and_then(|a| a.checked_add(1)) {
            Some(r) => Value::int(r),
            None => h_binary(cx, st, BinOp::Add, &i, &I8, &cx.k[k::ONE], &NOVAR)?,
        };
        wr_cv(&mut x, r);
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_assign(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut y, mut i, mut x) = (Value::UNDEF, Value::UNDEF, Value::UNDEF);
    wr_cv(&mut y, Value::int(5));
    for_loop!(cx, st, i, n, {
        let r = rd_cv(cx, st, &y, &Y8);
        wr_cv(&mut x, r);
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_call(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut i, mut x) = (Value::UNDEF, Value::UNDEF);
    for_loop!(cx, st, i, n, {
        let a = rd_cv(cx, st, &i, &I8);
        let r = guarded!(st, f(cx, st, a));
        wr_cv(&mut x, r);
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_method(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut o, mut i, mut x) = (Value::UNDEF, Value::UNDEF, Value::UNDEF);
    wr_cv(&mut o, Value::object(Object::new(Rc::clone(&cx.classes[0]), &st.ids)));
    for_loop!(cx, st, i, n, {
        let obj = obj_cv(&o)?;
        let cache = &cx.caches[0];
        if cache.class.get() != Rc::as_ptr(&obj.class) as usize {
            fill_method(cx, obj, cache, b"m", 1)?;
        }
        let a = rd_cv(cx, st, &i, &I8);
        let r = match cache.one.get() {
            Some(m) => guarded!(st, m(cx, st, obj, a)),
            None => {
                let g = cache.generic.get().expect("a filled cache");
                let mut args = [a];
                guarded!(st, g(cx, st, Some(Rc::clone(obj)), &mut args))
            }
        };
        wr_cv(&mut x, r);
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_prop_read(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut o, mut i, mut x) = (Value::UNDEF, Value::UNDEF, Value::UNDEF);
    wr_cv(&mut o, Value::object(Object::new(Rc::clone(&cx.classes[0]), &st.ids)));
    for_loop!(cx, st, i, n, {
        let obj = obj_cv(&o)?;
        let slot = prop_slot(obj, &cx.caches[0], b"p")?;
        let r = prop_read(obj, slot)?;
        wr_cv(&mut x, r);
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

/// `prop_write` and `prop_typed`: `$o->{name} = $i`, the typed check when
/// the cache says the property is typed.
#[inline(always)]
fn prop_store(cx: &Cx, st: &mut St, n: Value, name: &'static [u8]) -> R<Value> {
    let (mut o, mut i) = (Value::UNDEF, Value::UNDEF);
    wr_cv(&mut o, Value::object(Object::new(Rc::clone(&cx.classes[0]), &st.ids)));
    for_loop!(cx, st, i, n, {
        let v = rd_cv(cx, st, &i, &I8);
        let obj = obj_cv(&o)?;
        let cache = &cx.caches[0];
        let slot = prop_slot(obj, cache, name)?;
        if cache.typed.get() && !typed_ok(obj, slot, &v) {
            return fault("typed property coercion or TypeError not in the prototype");
        }
        obj.slots()[slot].assign(v);
    });
    if cx.check {
        let obj = obj_cv(&o)?;
        let slot = prop_slot(obj, &cx.caches[1], name)?;
        let p = prop_read(obj, slot)?;
        tail(cx, st, &i, &p, &NOVAR)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_prop_write(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    prop_store(cx, st, n, b"p")
}

#[inline(never)]
fn t_prop_typed(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    prop_store(cx, st, n, b"t")
}

#[inline(never)]
fn t_arr_read(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut a, mut i, mut x) = (Value::UNDEF, Value::UNDEF, Value::UNDEF);
    wr_cv(&mut a, cx.k[k::ARR].clone());
    for_loop!(cx, st, i, n, {
        // $x = $a[2]: an int literal key.
        let r = match a.as_array() {
            Some(arr) => match arr.get_int(2) {
                Some(e) if e.is_reference() => deref_ref(e),
                Some(e) => e.clone(),
                None => undefined_key(cx, st, &cx.k[k::TWO], 8),
            },
            None => h_fetch_dim(cx, st, &a, &I8, &cx.k[k::TWO], 8)?,
        };
        wr_cv(&mut x, r);
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_arr_write(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut a, mut i) = (Value::UNDEF, Value::UNDEF);
    wr_cv(&mut a, cx.k[k::ARR].clone());
    for_loop!(cx, st, i, n, {
        // $a[2] = $i: an int literal key; the element there written in
        // place (a new one through the generic slot).
        let v = rd_cv(cx, st, &i, &I8);
        if let Some(arr) = a.array_mut() {
            let e = match arr.get_int_mut(2) {
                Some(e) => e,
                None => arr.slot_mut(&Key::Int(2)),
            };
            if e.is_reference() {
                write_ref(e, v);
            } else {
                *e = v;
            }
        } else {
            h_assign_dim(&mut a, &cx.k[k::TWO], v)?;
        }
    });
    if cx.check {
        let e = h_fetch_dim(cx, st, &a, &NOVAR, &cx.k[k::TWO], 10)?;
        tail(cx, st, &i, &e, &NOVAR)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_isset_dim(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut a, mut i, mut x) = (Value::UNDEF, Value::UNDEF, Value::UNDEF);
    wr_cv(&mut a, cx.k[k::ARR].clone());
    for_loop!(cx, st, i, n, {
        // $x = isset($a['k']): a string literal key.
        let set = match a.as_array() {
            Some(arr) => arr.get_str(cx.strs[s::K]).is_some_and(is_set),
            None => h_isset_dim(&a, &cx.k[k::KEY_K])?,
        };
        wr_cv(&mut x, Value::bool(set));
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_concat(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut sv, mut i, mut x) = (Value::UNDEF, Value::UNDEF, Value::UNDEF);
    wr_cv(&mut sv, Value::istr(cx.strs[s::AB]));
    for_loop!(cx, st, i, n, {
        // $x = $s . 'c': a string literal on the right.
        let r = match sv.as_str() {
            Some(p) => Value::str(PhpStr::concat(p.as_bytes(), b"c")),
            None => h_binary(cx, st, BinOp::Concat, &sv, &S8, &cx.k[k::C], &NOVAR)?,
        };
        wr_cv(&mut x, r);
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_new(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut i, mut x) = (Value::UNDEF, Value::UNDEF);
    for_loop!(cx, st, i, n, {
        // $x = new P: P has no constructor (known when t is compiled).
        let o = Value::object(Object::new(Rc::clone(&cx.classes[0]), &st.ids));
        wr_cv(&mut x, o);
    });
    if cx.check {
        let p = h_fetch_prop(cx, st, &x, &X10, &cx.caches[1], b"p")?;
        tail(cx, st, &i, &p, &NOVAR)?;
    }
    Ok(Value::NULL)
}

#[inline(never)]
fn t_identical(cx: &Cx, st: &mut St, n: Value) -> R<Value> {
    let (mut sv, mut i, mut x) = (Value::UNDEF, Value::UNDEF, Value::UNDEF);
    wr_cv(&mut sv, Value::istr(cx.strs[s::AB]));
    for_loop!(cx, st, i, n, {
        // $x = $s === 'ab': a string literal on the right, so only a
        // string can be identical to it.
        let r = match sv.as_str() {
            Some(p) => p.as_bytes() == b"ab",
            None => h_identical(cx, st, &sv, &S8, &cx.k[k::AB], &NOVAR),
        };
        wr_cv(&mut x, Value::bool(r));
    });
    if cx.check {
        tail(cx, st, &i, &x, &X10)?;
    }
    Ok(Value::NULL)
}

/// Run `t(n)` of the program `name`; None if it is not compiled here.
pub fn run(name: &str, cx: &Cx, st: &mut St, n: i64) -> Option<R<Value>> {
    let t: fn(&Cx, &mut St, Value) -> R<Value> = match name {
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
    st.depth += 1;
    let r = t(cx, st, Value::int(n));
    st.depth -= 1;
    Some(r)
}

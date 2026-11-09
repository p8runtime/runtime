//! MEASUREMENT ONLY (#397). The C-ABI boundary between the harness and the
//! object file Cranelift wrote: the only `unsafe` in this prototype, and it
//! is here because a call across a foreign boundary cannot be anything
//! else in Rust (an `extern` block's functions are `unsafe` to call, and
//! exporting a symbol by name is `#[unsafe(no_mangle)]`). It never goes
//! near the engine's crates, which stay `#![forbid(unsafe_code)]`; this
//! crate is outside the workspace, and is never merged.
//!
//! What is unsafe, all of it:
//! - the calls of the generated `clif_t_*` (`run`);
//! - the helpers' `#[unsafe(no_mangle)]` and their raw pointers turned back
//!   into `&Cx`, `&mut St`, `&Value`, `&mut Value` (the generated code
//!   passes what the harness gave it, or 16-byte stack slots it owns);
//! - a value moved to and from its two words (`to_ret`, `from_raw`), with
//!   the tag read first so no padding byte is ever read as a payload;
//! - the layout checks `check_layout` runs before anything else.
//!
//! Every helper is V4b's cold path (`aot::rt`), called as V4b calls it.

#![allow(unsafe_code)]

use crate::abi::{self, T_ERR};
use aot::rt::{self, Cache, Cx, Fault, NOVAR, R, St, Var};
use p8_runtime::object::Object;
use p8_runtime::ops::BinOp;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::mem::{self, offset_of};
use std::rc::Rc;

/// A value or a fault, in rax:rdx.
#[repr(C)]
pub struct Ret {
    tag: u64,
    pay: u64,
}

#[allow(improper_ctypes, reason = "Cx and St are opaque to the generated code: it passes them on, and reads St::depth at the offset the generator took with offset_of!")]
unsafe extern "C" {
    fn clif_t_add(cx: *const Cx, st: *mut St, t: u64, p: u64) -> Ret;
    fn clif_t_call(cx: *const Cx, st: *mut St, t: u64, p: u64) -> Ret;
    fn clif_t_prop_read(cx: *const Cx, st: *mut St, t: u64, p: u64) -> Ret;
}

static VARS: [Var; 6] = [
    Var { name: "n", line: 7 },
    Var { name: "i", line: 7 },
    Var { name: "i", line: 8 },
    Var { name: "i", line: 10 },
    Var { name: "x", line: 10 },
    Var { name: "a", line: 2 },
];
const _: () = assert!(abi::var::FA as usize == VARS.len() - 1);
static PROPS: [&[u8]; 1] = [b"p"];

thread_local! {
    /// The fault a helper returned T_ERR (or 1) for.
    static ERR: RefCell<Option<Fault>> = const { RefCell::new(None) };
}

fn tag_of(v: &Value) -> u64 {
    // SAFETY: a `Value` is `repr(u64)`: its first word is always the tag.
    unsafe { *(v as *const Value as *const u64) }
}

fn pay_of(v: &Value) -> u64 {
    match tag_of(v) {
        // Null and Undef have no payload (padding), a bool one byte.
        0 | 4 => 0,
        1 => u64::from(v.as_bool().expect("a bool")),
        // SAFETY: every other variant's payload is one initialised word at 8.
        _ => unsafe { *(v as *const Value as *const u64).add(1) },
    }
}

fn to_ret(v: Value) -> Ret {
    let r = Ret { tag: tag_of(&v), pay: pay_of(&v) };
    mem::forget(v);
    r
}

fn from_raw(tag: u64, pay: u64) -> Value {
    assert!(tag < T_ERR as u64, "a value's tag");
    // SAFETY: a valid tag with its payload word: what `to_ret` took apart.
    unsafe { mem::transmute::<[u64; 2], Value>([tag, pay]) }
}

fn fail(e: Fault) -> Ret {
    ERR.with(|s| *s.borrow_mut() = Some(e));
    Ret { tag: T_ERR as u64, pay: 0 }
}

fn fail_b(e: Fault) -> u8 {
    fail(e);
    1
}

fn ret(r: R<Value>) -> Ret {
    match r {
        Ok(v) => to_ret(v),
        Err(e) => fail(e),
    }
}

/// The layout the generator baked in, against the live one.
pub fn check_layout(o: &Value) {
    let obj = o.as_object().expect("an object");
    let base = pay_of(o) as usize;
    assert_eq!(base + 16, Rc::as_ptr(obj) as usize, "Rc<Object>: the object 16 bytes into its allocation");
    // SAFETY: reads the word `Object::class` is (an `Rc`, one pointer).
    let w = unsafe { *((Rc::as_ptr(obj) as *const u8).add(offset_of!(Object, class)) as *const usize) };
    assert_eq!(w + 16, Rc::as_ptr(&obj.class) as usize, "Rc<Class>: one word, the allocation's address");
    assert_eq!(mem::size_of::<Value>(), 16);
}

/// Run the generated `t(n)` of `name`.
pub fn run(name: &str, cx: &Cx, st: &mut St, n: i64) -> Option<R<Value>> {
    let f = match name {
        "add" => clif_t_add,
        "call" => clif_t_call,
        "prop_read" => clif_t_prop_read,
        _ => return None,
    };
    let probe = Value::object(Object::new(Rc::clone(&cx.classes[0]), &st.ids));
    check_layout(&probe);
    drop(probe);
    st.depth += 1;
    // SAFETY: the generated function's signature is the one declared
    // (`abi.rs`); it reads and writes only through `cx`, `st` and its own
    // stack, and the helpers below.
    let r = unsafe { f(cx, st, abi::T_INT as u64, n as u64) };
    st.depth -= 1;
    Some(if r.tag == T_ERR as u64 { Err(ERR.with(|s| s.borrow_mut().take()).expect("a fault")) } else { Ok(from_raw(r.tag, r.pay)) })
}

// ---- the helpers (each V4b's cold path) --------------------------------------------

// SAFETY (all helpers): the generated code passes `cx`/`st` as `run`
// received them, and value pointers to 16-byte slots it owns or to an
// object's slot, valid for the call.

#[unsafe(no_mangle)]
extern "C" fn p8c_undef(cx: *const Cx, st: *mut St, var: u32) -> Ret {
    let (cx, st) = unsafe { (&*cx, &mut *st) };
    to_ret(rt::undef_var(cx, st, &VARS[var as usize]))
}

#[unsafe(no_mangle)]
extern "C" fn p8c_deref(v: *const Value) -> Ret {
    to_ret(rt::deref_ref(unsafe { &*v }))
}

#[unsafe(no_mangle)]
extern "C" fn p8c_clone_heap(v: *const Value) -> Ret {
    to_ret(unsafe { &*v }.clone())
}

#[unsafe(no_mangle)]
extern "C" fn p8c_assign_slow(dst: *mut Value, tag: u64, pay: u64) {
    rt::h_assign(unsafe { &mut *dst }, from_raw(tag, pay));
}

#[unsafe(no_mangle)]
extern "C" fn p8c_drop(tag: u64, pay: u64) {
    drop(from_raw(tag, pay));
}

#[unsafe(no_mangle)]
extern "C" fn p8c_less(cx: *const Cx, st: *mut St, a: *const Value, va: u32, b: *const Value, vb: u32) -> u8 {
    let (cx, st) = unsafe { (&*cx, &mut *st) };
    u8::from(rt::h_less(cx, st, unsafe { &*a }, &VARS[va as usize], unsafe { &*b }, &VARS[vb as usize]))
}

#[unsafe(no_mangle)]
extern "C" fn p8c_inc(cx: *const Cx, st: *mut St, a: *mut Value, var: u32) -> u8 {
    let (cx, st) = unsafe { (&*cx, &mut *st) };
    match rt::h_inc(cx, st, unsafe { &mut *a }, &VARS[var as usize]) {
        Ok(()) => 0,
        Err(e) => fail_b(e),
    }
}

#[unsafe(no_mangle)]
extern "C" fn p8c_add1(cx: *const Cx, st: *mut St, a: *const Value, var: u32) -> Ret {
    let (cx, st) = unsafe { (&*cx, &mut *st) };
    ret(rt::h_binary(cx, st, BinOp::Add, unsafe { &*a }, &VARS[var as usize], &cx.k[aot::k::ONE], &NOVAR))
}

#[unsafe(no_mangle)]
extern "C" fn p8c_too_deep() -> Ret {
    ret(rt::too_deep())
}

#[unsafe(no_mangle)]
extern "C" fn p8c_tail(cx: *const Cx, st: *mut St, i: *const Value, x: *const Value, vx: u32) -> u8 {
    let (cx, st) = unsafe { (&*cx, &mut *st) };
    if !cx.check {
        return 0;
    }
    let (i, x) = unsafe { (&*i, &*x) };
    let r = (|| {
        rt::h_echo(cx, st, i, &VARS[abi::var::I10 as usize])?;
        rt::h_echo(cx, st, &cx.k[aot::k::SPACE], &NOVAR)?;
        rt::h_echo(cx, st, x, &VARS[vx as usize])?;
        rt::h_echo(cx, st, &cx.k[aot::k::NL], &NOVAR)
    })();
    match r {
        Ok(()) => 0,
        Err(e) => fail_b(e),
    }
}

#[unsafe(no_mangle)]
extern "C" fn p8c_new(cx: *const Cx, st: *mut St, class: u32) -> Ret {
    let (cx, st) = unsafe { (&*cx, &mut *st) };
    to_ret(Value::object(Object::new(Rc::clone(&cx.classes[class as usize]), &st.ids)))
}

#[unsafe(no_mangle)]
extern "C" fn p8c_not_object(v: *const Value) -> Ret {
    ret(rt::fault_kind("->", unsafe { &*v }))
}

/// The inline cache's fill (V4b's `fill_prop`), keeping what the hit
/// needs: the word `Object::class` holds, and the slot's byte offset from
/// the object value's payload. A slot outside the object's allocation (a
/// class with more properties than fit inline) is refused.
#[unsafe(no_mangle)]
extern "C" fn p8c_fill_prop(o: *const Value, ic: *mut [u64; 2], name: u32) -> u8 {
    let v = unsafe { &*o };
    let Some(obj) = v.as_object() else { return fail_b(Box::new("fill of a non-object".to_owned())) };
    let cache = Cache::new();
    let slot = match rt::fill_prop(obj, &cache, PROPS[name as usize]) {
        Ok(s) => s,
        Err(e) => return fail_b(e),
    };
    let base = pay_of(v) as usize;
    let off = (&obj.slots()[slot] as *const _ as usize).wrapping_sub(base);
    if !(16..16 + mem::size_of::<Object>()).contains(&off) {
        return fail_b(Box::new("a property slot outside the object not in the prototype".to_owned()));
    }
    let class_word = Rc::as_ptr(&obj.class) as usize - 16;
    unsafe { *ic = [class_word as u64, off as u64] };
    0
}

#[unsafe(no_mangle)]
extern "C" fn p8c_unset_prop() -> Ret {
    ret(rt::fault("read of an unset or uninitialized property not in the prototype"))
}

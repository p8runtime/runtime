//! What the compiled code runs against: the program's data (`Cx`, fixed
//! once linked: classes, literals, the per-site inline caches, the method
//! entry points), the request's state (`St`: output, object ids, call
//! depth), faults, the cold paths both flavours share, and the generic
//! helpers that V4a calls for every operation.
//!
//! Conventions, the same as V1's (`ops.rs`): a CV may be undefined (a
//! warning, then null) or a reference (read and written through it); a
//! temporary or a literal is neither. What the prototype does not
//! implement faults with a message, so a wrong result is impossible and a
//! missing case is loud.

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::object::{Class, Ids, Object, Slot, Visibility};
use p8_runtime::ops::{self as rops, BinOp};
use p8_runtime::string::IStr;
use p8_runtime::value::Value;
use std::cell::Cell;
use std::cmp::Ordering;
use std::mem;
use std::rc::Rc;

/// A fault: the run stops with the message (exceptions are not in the
/// prototype). Boxed so a `Result<Value, Fault>` is a value's 16 bytes.
pub type Fault = Box<String>;
pub type R<T> = Result<T, Fault>;

/// A compiled variable as the compiler knows it at one read: its name and
/// the line of the read, for the warning (PHP's opline carries the line).
pub struct Var {
    pub name: &'static str,
    pub line: u32,
}

/// For an operand that is not a variable (a literal or a temporary).
pub static NOVAR: Var = Var { name: "", line: 0 };

/// A method entry point as V4a calls it: the generic convention, every
/// argument in a slice, `$this` an owned handle.
pub type GenericFn = fn(&Cx, &mut St, Option<Rc<Object>>, &mut [Value]) -> R<Value>;
/// A method entry point as V4b calls it: one argument by value, `$this`
/// borrowed (the caller's variable keeps the object alive; Rust's borrow
/// proves nothing can reassign it meanwhile).
pub type Fn1 = fn(&Cx, &mut St, &Rc<Object>, Value) -> R<Value>;

/// A compiled method, as the cache fill finds it.
pub struct MethodEntry {
    pub class: &'static [u8],
    pub name: &'static [u8],
    pub nparams: u16,
    pub required: u16,
    pub generic: GenericFn,
    pub one: Option<Fn1>,
}

/// An inline cache (PHP's run-time cache slot) for one property or method
/// site: the class seen, and the slot or the entry point it answered.
pub struct Cache {
    pub class: Cell<usize>,
    pub slot: Cell<u32>,
    pub typed: Cell<bool>,
    pub generic: Cell<Option<GenericFn>>,
    pub one: Cell<Option<Fn1>>,
}

impl Cache {
    pub fn new() -> Cache {
        Cache { class: Cell::new(usize::MAX), slot: Cell::new(0), typed: Cell::new(false), generic: Cell::new(None), one: Cell::new(None) }
    }
}

impl Default for Cache {
    fn default() -> Cache {
        Cache::new()
    }
}

/// The program, linked: fixed while it runs.
pub struct Cx {
    pub classes: Vec<Rc<Class>>,
    /// `t`'s literals (thawed once; read in place).
    pub k: Vec<Value>,
    /// The program-memory string literals V4b reads directly.
    pub strs: [&'static IStr; 3],
    pub caches: Vec<Cache>,
    pub methods: Vec<MethodEntry>,
    pub file: String,
    /// The output-check variant: the echo after the loop runs.
    pub check: bool,
}

/// The request.
pub struct St {
    pub ids: Rc<Ids>,
    pub out: Vec<u8>,
    pub depth: u32,
}

/// The call depth past which a call faults (a real AOT needs this guard:
/// PHP frames live on the native stack here).
pub const MAX_DEPTH: u32 = 10_000;

// ---- cold paths ----------------------------------------------------------

#[cold]
#[inline(never)]
pub fn fault<T>(msg: &str) -> R<T> {
    Err(Box::new(msg.to_owned()))
}

#[cold]
#[inline(never)]
pub fn fault_kind<T>(what: &str, v: &Value) -> R<T> {
    Err(Box::new(format!("{what} on a {:?} not in the prototype", v.kind())))
}

#[cold]
#[inline(never)]
pub fn warning(cx: &Cx, st: &mut St, msg: &str, line: u32) {
    st.out.extend_from_slice(format!("\nWarning: {msg} in {} on line {line}\n", cx.file).as_bytes());
}

/// A read of an undefined variable: the warning, and null.
#[cold]
#[inline(never)]
pub fn undef_var(cx: &Cx, st: &mut St, var: &Var) -> Value {
    warning(cx, st, &format!("Undefined variable ${}", var.name), var.line);
    Value::NULL
}

/// A reference's value.
#[cold]
#[inline(never)]
pub fn deref_ref(v: &Value) -> Value {
    v.deref()
}

/// Write through the reference a variable holds.
#[cold]
#[inline(never)]
pub fn write_ref(slot: &Value, v: Value) {
    let cell = slot.as_reference().expect("a reference");
    let old = mem::replace(&mut *cell.borrow_mut(), v);
    drop(old);
}

/// `x op y` through p8's operators (the operands resolved: defined, not
/// references). A diagnostic or an error faults, as in V1.
#[cold]
#[inline(never)]
pub fn binary_cold(op: BinOp, x: &Value, y: &Value) -> R<Value> {
    let mut diags = Vec::new();
    match rops::binary(op, x, y, &mut diags) {
        Ok(v) if diags.is_empty() => Ok(v),
        Ok(_) => Err(Box::new(format!("arithmetic diagnostic not in the prototype: {diags:?}"))),
        Err(e) => Err(Box::new(format!("arithmetic error not in the prototype: {e:?}"))),
    }
}

/// `++` of anything but an int that does not overflow.
#[cold]
#[inline(never)]
pub fn inc_cold(v: &Value) -> R<Value> {
    let mut diags = Vec::new();
    match rops::increment(v, &mut diags) {
        Ok(n) if diags.is_empty() => Ok(n),
        _ => fault("++ diagnostic or error not in the prototype"),
    }
}

/// `x < y` on anything (both resolved), as `compare` decides.
#[cold]
#[inline(never)]
pub fn less_cold(x: &Value, y: &Value) -> bool {
    x.compare(y) == Some(Ordering::Less)
}

#[cold]
#[inline(never)]
pub fn identical_cold(x: &Value, y: &Value) -> bool {
    x.identical(y)
}

#[cold]
#[inline(never)]
pub fn undefined_key(cx: &Cx, st: &mut St, key: &Value, line: u32) -> Value {
    let shown = match key.as_str() {
        Some(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
        None => key.as_int().map_or_else(String::new, |i| i.to_string()),
    };
    warning(cx, st, &format!("Undefined array key {shown}"), line);
    Value::NULL
}

/// isset's test: there and not null (through a reference).
#[inline(always)]
pub fn is_set(v: &Value) -> bool {
    if v.is_null_or_undef() {
        false
    } else if let Some(cell) = v.as_reference() {
        !cell.borrow().is_null_or_undef()
    } else {
        true
    }
}

/// The slot a property site answers for `o` after a miss: looked up by
/// name, public and plain (V1's `fill_prop`); the cache filled.
#[cold]
#[inline(never)]
pub fn fill_prop(o: &Object, cache: &Cache, name: &[u8]) -> R<usize> {
    let Some(slot) = o.class.slot(name, None) else {
        return fault("dynamic or undeclared property not in the prototype");
    };
    let p = &o.class.properties[slot];
    if p.visibility != Visibility::Public || p.hooks != 0 || p.readonly || p.set_visibility.is_some() {
        return fault("not a plain public property");
    }
    cache.class.set(Rc::as_ptr(&o.class) as usize);
    cache.slot.set(slot as u32);
    cache.typed.set(p.ty_decl.is_some());
    Ok(slot)
}

/// A typed property's check of the value stored. False: a coercion or a
/// TypeError, which the prototype does not do.
#[cold]
#[inline(never)]
pub fn typed_ok(o: &Object, slot: usize, v: &Value) -> bool {
    o.class.properties[slot].ty_decl.as_ref().is_none_or(|t| t.quick(v))
}

/// The method a site answers for `o` after a miss, for `argc` arguments:
/// looked up by name, public and plain (V1's `fill_method`), the
/// argument count checked once per class; the cache filled.
#[cold]
#[inline(never)]
pub fn fill_method(cx: &Cx, o: &Object, cache: &Cache, name: &[u8], argc: u16) -> R<()> {
    let Some(m) = o.class.method(name) else {
        return Err(Box::new(format!("Call to undefined method {}::{}()", String::from_utf8_lossy(o.class.name.as_bytes()), String::from_utf8_lossy(name))));
    };
    if m.visibility != Visibility::Public || m.is_static || m.is_abstract || m.native.is_some() {
        return fault("not a plain public instance method");
    }
    let lname = m.name.as_bytes().to_ascii_lowercase();
    let Some(e) = cx.methods.iter().find(|e| e.class == o.class.lower.as_slice() && e.name == lname.as_slice()) else {
        return fault("method not compiled");
    };
    if argc < e.required || argc > e.nparams {
        return fault("argument count not in the prototype");
    }
    cache.class.set(Rc::as_ptr(&o.class) as usize);
    cache.generic.set(Some(e.generic));
    cache.one.set(if argc == 1 { e.one } else { None });
    Ok(())
}

#[cold]
#[inline(never)]
pub fn too_deep<T>() -> R<T> {
    fault("call depth beyond the prototype's limit")
}

#[cold]
#[inline(never)]
pub fn flush(st: &mut St) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(&st.out);
    st.out.clear();
}

/// `echo v` (v resolved).
#[inline(always)]
pub fn echo_value(st: &mut St, v: &Value) -> R<()> {
    match v.to_php_string() {
        Some(s) => st.out.extend_from_slice(s.as_bytes()),
        None => return fault("echo of an array or object not in the prototype"),
    }
    if st.out.len() > 1 << 16 {
        flush(st);
    }
    Ok(())
}

// ---- V4a: the generic helpers ---------------------------------------------
//
// One out-of-line function per operation, taking every operand by
// reference and resolving it itself (undefined: the warning and null; a
// reference: its value), as PHP's JIT calls `zend_jit_*` helpers and its
// VM's generic handlers call `add_function` and friends. The code V4a
// emits is a sequence of calls to these, with the frame's variables in
// one array on the native stack.

/// An operand resolved: the value itself, or (undefined, a reference) a
/// resolved copy in `tmp`.
#[inline(always)]
fn resolve<'v>(cx: &Cx, st: &mut St, v: &'v Value, var: &Var, tmp: &'v mut Value) -> &'v Value {
    if v.is_undef() {
        *tmp = undef_var(cx, st, var);
        tmp
    } else if v.is_reference() {
        *tmp = deref_ref(v);
        tmp
    } else {
        v
    }
}

/// A copy of an operand (`Move`, `SendVar`).
#[inline(never)]
pub fn h_copy(cx: &Cx, st: &mut St, v: &Value, var: &Var) -> Value {
    if v.is_undef() {
        return undef_var(cx, st, var);
    }
    if v.is_reference() {
        return deref_ref(v);
    }
    v.clone()
}

/// `$a = v`: through a reference the variable holds; the old value released.
#[inline(never)]
pub fn h_assign(dst: &mut Value, v: Value) {
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

/// Release a temporary.
#[inline(never)]
pub fn h_free(v: &mut Value) {
    let _ = mem::replace(v, Value::UNDEF).release();
}

/// `a op b`.
#[inline(never)]
pub fn h_binary(cx: &Cx, st: &mut St, op: BinOp, a: &Value, va: &Var, b: &Value, vb: &Var) -> R<Value> {
    let (mut ta, mut tb) = (Value::UNDEF, Value::UNDEF);
    let x = resolve(cx, st, a, va, &mut ta);
    let y = resolve(cx, st, b, vb, &mut tb);
    let mut diags = Vec::new();
    match rops::binary(op, x, y, &mut diags) {
        Ok(v) if diags.is_empty() => Ok(v),
        _ => fault("arithmetic diagnostic or error not in the prototype"),
    }
}

/// `a < b`.
#[inline(never)]
pub fn h_less(cx: &Cx, st: &mut St, a: &Value, va: &Var, b: &Value, vb: &Var) -> bool {
    let (mut ta, mut tb) = (Value::UNDEF, Value::UNDEF);
    let x = resolve(cx, st, a, va, &mut ta);
    let y = resolve(cx, st, b, vb, &mut tb);
    x.compare(y) == Some(Ordering::Less)
}

/// `a === b`.
#[inline(never)]
pub fn h_identical(cx: &Cx, st: &mut St, a: &Value, va: &Var, b: &Value, vb: &Var) -> bool {
    let (mut ta, mut tb) = (Value::UNDEF, Value::UNDEF);
    let x = resolve(cx, st, a, va, &mut ta);
    let y = resolve(cx, st, b, vb, &mut tb);
    x.identical(y)
}

/// `++$a`.
#[inline(never)]
pub fn h_inc(cx: &Cx, st: &mut St, a: &mut Value, var: &Var) -> R<()> {
    if a.is_undef() {
        undef_var(cx, st, var);
    }
    let mut diags = Vec::new();
    let n = match rops::increment(a, &mut diags) {
        Ok(n) if diags.is_empty() => n,
        _ => return fault("++ diagnostic or error not in the prototype"),
    };
    h_assign(a, n);
    Ok(())
}

/// The key an operand names.
fn key_of(k: &Value) -> R<Key> {
    if let Some(n) = k.as_int() {
        Ok(Key::Int(n))
    } else if let Some(s) = k.cloned_str() {
        Ok(Key::from_string(s))
    } else {
        fault_kind("array key", k)
    }
}

/// The element of `a` at `k`.
fn get<'a>(a: &'a PhpArray, k: &Value) -> R<Option<&'a Value>> {
    if let Some(n) = k.as_int() {
        Ok(a.get_int(n))
    } else if let Some(s) = k.as_str() {
        // (`get_str` normalises a canonical-integer string itself.)
        Ok(a.get_str(s))
    } else {
        fault_kind("array key", k)
    }
}

/// `$a[k]` (read).
#[inline(never)]
pub fn h_fetch_dim(cx: &Cx, st: &mut St, arr: &Value, va: &Var, key: &Value, line: u32) -> R<Value> {
    let mut ta = Value::UNDEF;
    let arr = resolve(cx, st, arr, va, &mut ta);
    let Some(a) = arr.as_array() else {
        return fault_kind("$a[k]", arr);
    };
    match get(a, key)? {
        Some(e) if e.is_reference() => Ok(deref_ref(e)),
        Some(e) => Ok(e.clone()),
        None => Ok(undefined_key(cx, st, key, line)),
    }
}

/// `isset($a[k])`.
#[inline(never)]
pub fn h_isset_dim(arr: &Value, key: &Value) -> R<bool> {
    let tmp;
    let arr = if arr.is_reference() {
        tmp = deref_ref(arr);
        &tmp
    } else {
        arr
    };
    if let Some(a) = arr.as_array() {
        Ok(get(a, key)?.is_some_and(is_set))
    } else if arr.is_null_or_undef() {
        Ok(false)
    } else {
        fault_kind("isset($a[k])", arr)
    }
}

/// `$a[k] = v`.
#[inline(never)]
pub fn h_assign_dim(arr: &mut Value, key: &Value, v: Value) -> R<()> {
    if arr.is_reference() {
        return fault("$a[k] = v through a reference not in the prototype");
    }
    let key = key_of(key)?;
    let Some(a) = arr.array_mut() else {
        return fault_kind("$a[k] = v", arr);
    };
    let e = a.slot_mut(&key);
    if e.is_reference() {
        write_ref(e, v);
    } else {
        *e = v;
    }
    Ok(())
}

/// The object an operand holds.
fn object_of<'v>(cx: &Cx, st: &mut St, v: &'v Value, var: &Var, tmp: &'v mut Value) -> R<&'v Rc<Object>> {
    let v = resolve(cx, st, v, var, tmp);
    match v.as_object() {
        Some(o) => Ok(o),
        None => fault_kind("->", v),
    }
}

/// `$o->name` (read), through the site's cache.
#[inline(never)]
pub fn h_fetch_prop(cx: &Cx, st: &mut St, obj: &Value, vo: &Var, cache: &Cache, name: &[u8]) -> R<Value> {
    let mut t = Value::UNDEF;
    let o = object_of(cx, st, obj, vo, &mut t)?;
    let slot = if cache.class.get() == Rc::as_ptr(&o.class) as usize { cache.slot.get() as usize } else { fill_prop(o, cache, name)? };
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

/// `$o->name = v`, through the site's cache.
#[inline(never)]
pub fn h_assign_prop(cx: &Cx, st: &mut St, obj: &Value, vo: &Var, cache: &Cache, name: &[u8], v: Value) -> R<()> {
    let mut t = Value::UNDEF;
    let o = object_of(cx, st, obj, vo, &mut t)?;
    let slot = if cache.class.get() == Rc::as_ptr(&o.class) as usize { cache.slot.get() as usize } else { fill_prop(o, cache, name)? };
    if cache.typed.get() && !typed_ok(o, slot, &v) {
        return fault("typed property coercion or TypeError not in the prototype");
    }
    o.slots()[slot].assign(v);
    Ok(())
}

/// `new C` (a class with no constructor).
#[inline(never)]
pub fn h_new(cx: &Cx, st: &mut St, class: usize) -> Value {
    Value::object(Object::new(Rc::clone(&cx.classes[class]), &st.ids))
}

/// A call of a user function by the generic convention: the arguments in
/// a slice, the count checked, the depth guarded.
#[inline(never)]
pub fn h_call(cx: &Cx, st: &mut St, f: GenericFn, nparams: u16, required: u16, args: &mut [Value]) -> R<Value> {
    let argc = args.len() as u16;
    if argc < required || argc > nparams {
        return fault("argument count not in the prototype");
    }
    if st.depth >= MAX_DEPTH {
        return too_deep();
    }
    st.depth += 1;
    let r = f(cx, st, None, args);
    st.depth -= 1;
    r
}

/// `$o->name(args)`, through the site's cache.
#[inline(never)]
pub fn h_method_call(cx: &Cx, st: &mut St, obj: &Value, vo: &Var, cache: &Cache, name: &[u8], args: &mut [Value]) -> R<Value> {
    let mut t = Value::UNDEF;
    let o = object_of(cx, st, obj, vo, &mut t)?;
    if cache.class.get() != Rc::as_ptr(&o.class) as usize {
        fill_method(cx, o, cache, name, args.len() as u16)?;
    }
    let f = cache.generic.get().expect("a filled cache");
    let this = Rc::clone(o);
    if st.depth >= MAX_DEPTH {
        return too_deep();
    }
    st.depth += 1;
    let r = f(cx, st, Some(this), args);
    st.depth -= 1;
    r
}

/// `return $a`: the variable moved out.
#[inline(never)]
pub fn h_return(cx: &Cx, st: &mut St, v: &mut Value, var: &Var) -> Value {
    let v = mem::replace(v, Value::UNDEF);
    if v.is_undef() {
        undef_var(cx, st, var)
    } else if v.is_reference() {
        deref_ref(&v)
    } else {
        v
    }
}

/// `echo a`.
#[inline(never)]
pub fn h_echo(cx: &Cx, st: &mut St, v: &Value, var: &Var) -> R<()> {
    let mut t = Value::UNDEF;
    let v = resolve(cx, st, v, var, &mut t);
    echo_value(st, v)
}

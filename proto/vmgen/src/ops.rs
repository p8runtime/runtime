//! Every instruction, written once. The operand kinds are `const`
//! parameters; `build.rs` names one handler per live combination. Inside,
//! `if K == CV { … }` is a comparison of constants that rustc folds per
//! instantiation, so a `P8_MOVE_TMP_CONST` has no undefined test and no
//! reference test left in it (the PR shows the disassembly).
//!
//! Each handler's straight path is the common case; what is rare (an
//! undefined variable, a reference, a non-int operand, a cache miss) goes
//! to a `#[cold] #[inline(never)]` function below. Nothing here leaves for
//! a generic slow path: what the prototype does not implement faults with
//! a message, so a wrong result is impossible and a missing case is loud.
//!
//! Operand conventions (`Insn`): `a` is the destination or the variable
//! written, `b` and `c` the sources, `d` a second result or count, `k0`
//! the literal of the first literal-capable source and `k1` of the second.

use crate::kinds::*;
use crate::vm::*;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::object::{Object, Slot, Visibility};
use p8_runtime::ops::{self as rops, BinOp};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cmp::Ordering;
use std::mem;
use std::rc::Rc;

// ---- shared pieces -----------------------------------------------------

/// Raise a fault: the loop stops with the message. (The message is made
/// here, out of line: a `format!` at the call site would put its code, and
/// a stack frame for it, into the handler's straight path.)
#[cold]
#[inline(never)]
fn fault<M: Into<String>>(st: &mut State, msg: M) -> u32 {
    st.event = Event::Fault(msg.into());
    REFRAME
}

/// A fault naming the kind of the value that was not expected there.
#[cold]
#[inline(never)]
fn fault_kind(st: &mut State, what: &str, v: &Value) -> u32 {
    fault(st, format!("{what} on a {:?} not in the prototype", v.kind()))
}

/// A fault for a call with the wrong number of arguments.
#[cold]
#[inline(never)]
fn fault_args(st: &mut State, argc: u8, name: &str) -> u32 {
    fault(st, format!("{argc} arguments to {name}() not in the prototype"))
}

/// A PHP warning, as the CLI prints it (display_errors to stdout).
#[cold]
#[inline(never)]
fn warning(st: &mut State, f: &Func, pc: u32, msg: &str) {
    let line = f.src.lines.get(pc as usize).copied().unwrap_or(0);
    st.out.extend_from_slice(format!("\nWarning: {msg} in {} on line {line}\n", f.src.file).as_bytes());
}

/// A read of an undefined variable: the warning, and null.
#[cold]
#[inline(never)]
fn undef_cv(st: &mut State, f: &Func, pc: u32, r: u8) -> Value {
    let name = f.src.cv_names.get(usize::from(r)).map(|t| String::from_utf8_lossy(t.as_bytes()).into_owned()).unwrap_or_default();
    warning(st, f, pc, &format!("Undefined variable ${name}"));
    Value::NULL
}

/// A reference's value.
#[cold]
#[inline(never)]
fn deref_ref(v: &Value) -> Value {
    v.deref()
}

/// Write through the reference a variable holds.
#[cold]
#[inline(never)]
fn write_ref(slot: &Value, v: Value) {
    let cell = slot.as_reference().expect("a reference");
    let old = mem::replace(&mut *cell.borrow_mut(), v);
    drop(old);
}

/// The operand of kind `K`, borrowed: the literal, or the register.
#[inline(always)]
fn view<'w, const K: u8>(w: &'w Window, k: &'w Value, r: u8) -> &'w Value {
    if K == CONST { k } else { &w[usize::from(r)] }
}

/// A copy of the operand of kind `K`. A variable may be undefined (the
/// warning, null) or a reference (its value); the other kinds cannot.
#[inline(always)]
fn rd<const K: u8>(st: &mut State, f: &Func, w: &Window, k: &Value, r: u8, pc: u32) -> Value {
    if K == CONST {
        return k.clone();
    }
    let v = &w[usize::from(r)];
    if K == CV {
        if v.is_undef() {
            return undef_cv(st, f, pc, r);
        }
        if v.is_reference() {
            return deref_ref(v);
        }
    }
    v.clone()
}

/// A temporary moved out of its register (read for the last time).
#[inline(always)]
fn take(st: &mut State, f: &Func, w: &mut Window, r: u8, pc: u32) -> Value {
    let v = mem::replace(&mut w[usize::from(r)], Value::UNDEF);
    if v.is_undef() {
        return undef_cv(st, f, pc, r);
    }
    if v.is_reference() {
        return deref_ref(&v);
    }
    v
}

/// Write `v` over register `r`: a plain old value is overwritten (one tag
/// test); one holding memory is released; a variable holding a reference
/// is written through it (PHP's ASSIGN on a CV: `ZVAL_DEREF`).
#[inline(always)]
fn wr<const K: u8>(w: &mut Window, r: u8, v: Value) {
    let slot = &mut w[usize::from(r)];
    if slot.is_plain() {
        mem::forget(mem::replace(slot, v));
        return;
    }
    if K == CV && slot.is_reference() {
        write_ref(slot, v);
        return;
    }
    let _ = mem::replace(slot, v).release();
}

#[inline(always)]
fn int_cmp<const CMP: u8>(x: i64, y: i64) -> bool {
    match CMP {
        LT => x < y,
        LE => x <= y,
        GT => x > y,
        GE => x >= y,
        EQ | ID => x == y,
        _ => x != y,
    }
}

/// `x cmp y` on anything (both defined, not references), as `compare` decides.
#[cold]
#[inline(never)]
fn cmp_values(cmp: u8, x: &Value, y: &Value) -> bool {
    match cmp {
        EQ => x.loose_eq(y),
        NE => !x.loose_eq(y),
        ID => x.identical(y),
        NI => !x.identical(y),
        LT => x.compare(y) == Some(Ordering::Less),
        LE => matches!(x.compare(y), Some(Ordering::Less | Ordering::Equal)),
        GT => y.compare(x) == Some(Ordering::Less),
        _ => matches!(y.compare(x), Some(Ordering::Less | Ordering::Equal)),
    }
}

/// A comparison beyond two ints: the operands resolved (an undefined
/// variable warns, a reference is read through), then compared.
#[cold]
#[inline(never)]
fn cmp_cold<const A: u8, const B: u8>(st: &mut State, f: &Func, w: &Window, i: &Insn, pc: u32, cmp: u8, ra: u8, rb: u8) -> bool {
    let x = rd::<A>(st, f, w, i.k0, ra, pc);
    let y = rd::<B>(st, f, w, i.k1, rb, pc);
    cmp_values(cmp, &x, &y)
}

#[inline(always)]
fn binop<const OP: u8>() -> BinOp {
    match OP {
        ADD => BinOp::Add,
        SUB => BinOp::Sub,
        _ => BinOp::Mul,
    }
}

#[inline(always)]
fn int_op<const OP: u8>(x: i64, y: i64) -> Option<i64> {
    match OP {
        ADD => x.checked_add(y),
        SUB => x.checked_sub(y),
        _ => x.checked_mul(y),
    }
}

/// `x op y` beyond two ints: floats, numeric strings, `.` on non-strings,
/// through p8's operators. None: a fault was raised (a warning or an
/// error the prototype does not report as PHP does).
#[cold]
#[inline(never)]
fn arith_cold<const A: u8, const B: u8>(st: &mut State, f: &Func, w: &Window, i: &Insn, pc: u32, op: BinOp, ra: u8, rb: u8) -> Option<Value> {
    let x = rd::<A>(st, f, w, i.k0, ra, pc);
    let y = rd::<B>(st, f, w, i.k1, rb, pc);
    let mut diags = Vec::new();
    match rops::binary(op, &x, &y, &mut diags) {
        Ok(v) if diags.is_empty() => Some(v),
        Ok(_) => {
            fault(st, format!("arithmetic diagnostic not in the prototype: {diags:?}"));
            None
        }
        Err(e) => {
            fault(st, format!("arithmetic error not in the prototype: {e:?}"));
            None
        }
    }
}

/// A condition that is not a bool.
#[cold]
#[inline(never)]
fn truth_cold<const K: u8>(st: &mut State, f: &Func, pc: u32, r: u8, v: &Value) -> bool {
    if K == CV && v.is_undef() {
        undef_cv(st, f, pc, r);
        return false;
    }
    v.deref().to_bool()
}

/// The key a register names (an int or a string; anything else is a fault).
#[inline(always)]
fn key_of(st: &mut State, k: &Value) -> Option<Key> {
    if let Some(n) = k.as_int() {
        Some(Key::Int(n))
    } else if let Some(s) = k.cloned_str() {
        Some(Key::from_string(s))
    } else {
        fault_kind(st, "array key", k);
        None
    }
}

/// The element at the key operand of kind `KEY` (`k0` a literal, else the
/// register `r`), if the array has it.
#[inline(always)]
fn elem<'a, const KEY: u8>(st: &mut State, a: &'a PhpArray, w: &'a Window, k0: &Value, r: u8) -> Result<Option<&'a Value>, ()> {
    Ok(match KEY {
        CINT => a.get_int(k0.as_int().expect("an int literal key")),
        CSTR => a.get_str(k0.as_str().expect("a string literal key")),
        _ => {
            let k = &w[usize::from(r)];
            if let Some(n) = k.as_int() {
                a.get_int(n)
            } else if let Some(s) = k.as_str() {
                // (A numeric string names an int key: normalised as PHP does.)
                match Key::from_string(s.to_php_str_lossy()) {
                    Key::Int(n) => a.get_int(n),
                    Key::Str(s) => a.get_str(&s),
                }
            } else {
                fault_kind(st, "array key", k);
                return Err(());
            }
        }
    })
}

trait LossyStr {
    fn to_php_str_lossy(&self) -> PhpStr;
}

impl LossyStr for p8_runtime::string::StrHead {
    #[inline(always)]
    fn to_php_str_lossy(&self) -> PhpStr {
        PhpStr::from(self.as_bytes())
    }
}

/// isset's test: there and not null (through a reference).
#[inline(always)]
fn is_set(v: &Value) -> bool {
    if v.is_null_or_undef() {
        false
    } else if let Some(cell) = v.as_reference() {
        !cell.borrow().is_null_or_undef()
    } else {
        true
    }
}

#[cold]
#[inline(never)]
fn undefined_key(st: &mut State, f: &Func, pc: u32, key: &Value) -> Value {
    let shown = match key.as_str() {
        Some(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
        None => key.as_int().map_or_else(String::new, |i| i.to_string()),
    };
    warning(st, f, pc, &format!("Undefined array key {shown}"));
    Value::NULL
}

/// The slot a property site answers for `o` after a miss: the property
/// looked up by name, public, plain (no hooks, not readonly); the cache
/// filled for this class.
#[cold]
#[inline(never)]
fn fill_prop(f: &Func, o: &Object, cache: &Cache, this: bool) -> Result<usize, String> {
    let name = f.consts.get(usize::from(cache.name)).and_then(Value::as_str).ok_or("property name constant")?;
    let scope = if this { Some(o.class.lower.as_slice()) } else { None };
    let Some(slot) = o.class.slot(name.as_bytes(), scope) else {
        return Err(format!("dynamic or undeclared property ${} not in the prototype", String::from_utf8_lossy(name.as_bytes())));
    };
    let p = &o.class.properties[slot];
    if (p.visibility != Visibility::Public && !this) || p.hooks != 0 || p.readonly || p.set_visibility.is_some() {
        return Err(format!("property ${} is not a plain public property", String::from_utf8_lossy(name.as_bytes())));
    }
    cache.class.set(Rc::as_ptr(&o.class) as usize);
    cache.slot.set(slot as u32);
    cache.flags.set(if p.ty_decl.is_some() { TYPED } else { 0 });
    Ok(slot)
}

/// The function a method site answers for `o` after a miss.
#[cold]
#[inline(never)]
fn fill_method(f: &Func, o: &Object, cache: &Cache) -> Result<u32, String> {
    let site = f.src.sites.get(usize::from(cache.name)).ok_or("method site")?;
    let Some(m) = o.class.method(site.name.as_bytes()) else {
        return Err(format!("Call to undefined method {}::{}()", String::from_utf8_lossy(o.class.name.as_bytes()), String::from_utf8_lossy(site.display.as_bytes())));
    };
    if m.visibility != Visibility::Public || m.is_static || m.is_abstract || m.native.is_some() {
        return Err(format!("method {} is not a plain public instance method", String::from_utf8_lossy(site.display.as_bytes())));
    }
    cache.class.set(Rc::as_ptr(&o.class) as usize);
    cache.slot.set(m.func);
    Ok(m.func)
}

/// A typed property's check of the value stored (cold: the common
/// property is untyped). False: the store needs a coercion or throws.
#[cold]
#[inline(never)]
fn typed_ok(o: &Object, slot: usize, v: &Value) -> bool {
    o.class.properties[slot].ty_decl.as_ref().is_none_or(|t| t.quick(v))
}

// ---- the instructions ----------------------------------------------------

/// `a = b`: a copy into a temporary or a variable (`Move`, `LoadConst`).
#[inline(always)]
pub fn mv<'p, const DST: u8, const SRC: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let v = rd::<SRC>(st, f, w, i.k0, i.b, pc);
    wr::<DST>(w, i.a, v);
    pc + 1
}

/// `$a = b`: the variable written through a reference it holds; `TAKE`
/// moves the temporary out (`AssignTake`).
#[inline(always)]
pub fn assign<'p, const SRC: u8, const MODE: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let v = if MODE == TAKE { take(st, f, w, i.b, pc) } else { rd::<SRC>(st, f, w, i.k0, i.b, pc) };
    wr::<CV>(w, i.a, v);
    pc + 1
}

/// Release a temporary.
#[inline(always)]
pub fn free<'p>(_st: &mut State<'p>, w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    wr::<TMP>(w, i.a, Value::UNDEF);
    pc + 1
}

#[inline(always)]
pub fn jmp<'p>(_st: &mut State<'p>, _w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, _pc: u32) -> u32 {
    i.imm
}

/// `JumpIf` (WHEN = T) / `JumpIfNot` (F) on the condition in `a`.
#[inline(always)]
pub fn jmp_cond<'p, const K: u8, const WHEN: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let v = &w[usize::from(i.a)];
    let t = match v.as_bool() {
        Some(b) => b,
        None => truth_cold::<K>(st, f, pc, i.a, v),
    };
    if t == (WHEN == T) { i.imm } else { pc + 1 }
}

/// A comparison and its branch in one: `a CMP b`, jump to `imm` when the
/// result is WHEN. Two ints inline; two strings for `===` inline; the
/// rest resolved cold.
#[inline(always)]
pub fn jcmp<'p, const CMP: u8, const WHEN: u8, const A: u8, const B: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let (x, y) = (view::<A>(w, i.k0, i.a), view::<B>(w, i.k1, i.b));
    let t = if let (Some(x), Some(y)) = (x.as_int(), y.as_int()) {
        int_cmp::<CMP>(x, y)
    } else if (CMP == ID || CMP == NI)
        && let (Some(p), Some(q)) = (x.as_str(), y.as_str())
    {
        (p.as_bytes() == q.as_bytes()) == (CMP == ID)
    } else {
        cmp_cold::<A, B>(st, f, w, i, pc, CMP, i.a, i.b)
    };
    if t == (WHEN == T) { i.imm } else { pc + 1 }
}

/// `++$a` (or `--`) fused with the compare-and-jump that follows: step,
/// then jump to `imm` when `$a STEP b` holds, else past the test (pc + 2).
/// A bound that is not an int leaves the test to the next instruction.
#[inline(always)]
pub fn step_jmp<'p, const STEP: u8, const B: u8>(st: &mut State<'p>, w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let r = &mut w[usize::from(i.a)];
    let n = match r.as_int_mut() {
        Some(v) => {
            let stepped = if STEP == INC_LT || STEP == INC_LE { v.checked_add(1) } else { v.checked_sub(1) };
            match stepped {
                Some(n) => {
                    *v = n;
                    n
                }
                None => return fault(st, "++/-- overflow not in the prototype"),
            }
        }
        None => return fault_kind(st, "++/--", &w[usize::from(i.a)]),
    };
    let bound = view::<B>(w, i.k0, i.b);
    match bound.as_int() {
        Some(b) => {
            let holds = match STEP {
                INC_LT => n < b,
                INC_LE => n <= b,
                DEC_GT => n > b,
                _ => n >= b,
            };
            if holds { i.imm } else { pc + 2 }
        }
        None => pc + 1,
    }
}

/// `a = b OP c`: two ints inline (no overflow), the rest cold.
#[inline(always)]
pub fn arith<'p, const OP: u8, const DST: u8, const A: u8, const B: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let (x, y) = (view::<A>(w, i.k0, i.b), view::<B>(w, i.k1, i.c));
    let v = if let (Some(x), Some(y)) = (x.as_int(), y.as_int())
        && let Some(n) = int_op::<OP>(x, y)
    {
        Value::int(n)
    } else {
        match arith_cold::<A, B>(st, f, w, i, pc, binop::<OP>(), i.b, i.c) {
            Some(v) => v,
            None => return REFRAME,
        }
    };
    wr::<DST>(w, i.a, v);
    pc + 1
}

/// `a = b . c`: two strings inline, the rest cold (conversions).
#[inline(always)]
pub fn concat<'p, const DST: u8, const A: u8, const B: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let (x, y) = (view::<A>(w, i.k0, i.b), view::<B>(w, i.k1, i.c));
    let v = if let (Some(p), Some(q)) = (x.as_str(), y.as_str()) {
        Value::str(PhpStr::concat(p.as_bytes(), q.as_bytes()))
    } else {
        match arith_cold::<A, B>(st, f, w, i, pc, BinOp::Concat, i.b, i.c) {
            Some(v) => v,
            None => return REFRAME,
        }
    };
    wr::<DST>(w, i.a, v);
    pc + 1
}

/// `a = b CMP c` as a bool.
#[inline(always)]
pub fn compare<'p, const CMP: u8, const DST: u8, const A: u8, const B: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let (x, y) = (view::<A>(w, i.k0, i.b), view::<B>(w, i.k1, i.c));
    let t = if let (Some(x), Some(y)) = (x.as_int(), y.as_int()) {
        int_cmp::<CMP>(x, y)
    } else if (CMP == ID || CMP == NI)
        && let (Some(p), Some(q)) = (x.as_str(), y.as_str())
    {
        (p.as_bytes() == q.as_bytes()) == (CMP == ID)
    } else {
        cmp_cold::<A, B>(st, f, w, i, pc, CMP, i.b, i.c)
    };
    wr::<DST>(w, i.a, Value::bool(t));
    pc + 1
}

/// `a = b[c]` on an array; the key a literal (`k0`) or a register.
#[inline(always)]
pub fn fetch_dim<'p, const ARR: u8, const KEY: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let arr = &w[usize::from(i.b)];
    let v = if let Some(a) = arr.as_array() {
        match elem::<KEY>(st, a, w, i.k0, i.c) {
            Ok(Some(e)) => {
                if e.is_reference() {
                    deref_ref(e)
                } else {
                    e.clone()
                }
            }
            Ok(None) => {
                let key = view::<KEY>(w, i.k0, i.c);
                undefined_key(st, f, pc, key)
            }
            Err(()) => return REFRAME,
        }
    } else {
        return fault_kind(st, "$a[k]", arr);
    };
    wr::<TMP>(w, i.a, v);
    pc + 1
}

/// `a = isset(b[c])`.
#[inline(always)]
pub fn isset_dim<'p, const VAR: u8, const KEY: u8>(st: &mut State<'p>, w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let var = &w[usize::from(i.b)];
    let set = if let Some(a) = var.as_array() {
        match elem::<KEY>(st, a, w, i.k0, i.c) {
            Ok(e) => e.is_some_and(is_set),
            Err(()) => return REFRAME,
        }
    } else if var.is_null_or_undef() {
        false
    } else {
        return fault_kind(st, "isset($a[k])", var);
    };
    wr::<TMP>(w, i.a, Value::bool(set));
    pc + 1
}

/// `$a[c] = b`, the result into `d` (or freed: UNUSED skips the `Free`).
#[inline(always)]
pub fn assign_dim<'p, const KEY: u8, const VAL: u8, const RES: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let v = rd::<VAL>(st, f, w, i.k1, i.b, pc);
    let key = match KEY {
        CINT => Key::Int(i.k0.as_int().expect("an int literal key")),
        CSTR => Key::Str(i.k0.cloned_str().expect("a string literal key")),
        _ => match key_of(st, &w[usize::from(i.c)]) {
            Some(k) => k,
            None => return REFRAME,
        },
    };
    let slot = &mut w[usize::from(i.a)];
    let Some(arr) = slot.array_mut() else {
        return fault_kind(st, "$a[k] = v", &w[usize::from(i.a)]);
    };
    let e = arr.slot_mut(&key);
    let keep = if RES == USED { Some(v.clone()) } else { None };
    if e.is_reference() {
        write_ref(e, v);
    } else {
        *e = v;
    }
    if let Some(v) = keep {
        wr::<TMP>(w, i.d, v);
        pc + 1
    } else {
        pc + 2
    }
}

/// `a = b->name` (`name` the cache's constant), the slot from the cache.
#[inline(always)]
pub fn fetch_prop<'p, const OBJ: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let cache = &f.caches[i.imm as usize];
    let v = {
        let o: &Rc<Object> = if OBJ == THIS {
            match &st.this {
                Some(o) => o,
                None => return fault(st, "Using $this when not in object context"),
            }
        } else {
            match w[usize::from(i.b)].as_object() {
                Some(o) => o,
                None => return fault_kind(st, "->prop", &w[usize::from(i.b)]),
            }
        };
        let slot = if cache.class.get() == Rc::as_ptr(&o.class) as usize {
            cache.slot.get() as usize
        } else {
            match fill_prop(f, o, cache, OBJ == THIS) {
                Ok(s) => s,
                Err(e) => return fault(st, e),
            }
        };
        o.slots()[slot].peek(|v| {
            if v.is_undef() {
                None
            } else if v.is_reference() {
                Some(deref_ref(v))
            } else {
                Some(v.clone())
            }
        })
    };
    let Some(v) = v else {
        return fault(st, "read of an unset or uninitialized property not in the prototype");
    };
    wr::<TMP>(w, i.a, v);
    let _ = pc;
    pc + 1
}

/// `a->name = b`, the result into `d` (or freed).
#[inline(always)]
pub fn assign_prop<'p, const OBJ: u8, const VAL: u8, const MODE: u8, const RES: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let v = if MODE == TAKE { take(st, f, w, i.b, pc) } else { rd::<VAL>(st, f, w, i.k0, i.b, pc) };
    let cache = &f.caches[i.imm as usize];
    let keep = {
        let o: &Rc<Object> = if OBJ == THIS {
            match &st.this {
                Some(o) => o,
                None => return fault(st, "Using $this when not in object context"),
            }
        } else {
            match w[usize::from(i.a)].as_object() {
                Some(o) => o,
                None => return fault_kind(st, "->prop = v", &w[usize::from(i.a)]),
            }
        };
        let slot = if cache.class.get() == Rc::as_ptr(&o.class) as usize {
            cache.slot.get() as usize
        } else {
            match fill_prop(f, o, cache, OBJ == THIS) {
                Ok(s) => s,
                Err(e) => return fault(st, e),
            }
        };
        if cache.flags.get() & TYPED != 0 && !typed_ok(o, slot, &v) {
            return fault(st, "typed property coercion or TypeError not in the prototype");
        }
        let keep = if RES == USED { Some(v.clone()) } else { None };
        let s = &o.slots()[slot];
        let old = s.replace(v);
        if old.is_reference() {
            // (The reference back in the slot, the value into it.)
            let nv = s.replace(Value::UNDEF);
            write_ref(&old, nv);
            p8_runtime::object::put_slot(s, old);
        } else {
            drop(old);
        }
        keep
    };
    if let Some(v) = keep {
        wr::<TMP>(w, i.d, v);
        pc + 1
    } else {
        pc + 2
    }
}

/// A call of the user function `imm` with the `b` arguments at `a`, the
/// result to `d` (UNUSED: discarded, and the `Free` after skipped).
#[inline(always)]
pub fn call<'p, const RES: u8>(st: &mut State<'p>, _w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    st.event = Event::Call { func: i.imm, args: i.a, argc: i.b, dst: i.d, used: RES == USED, this: None };
    REFRAME | (pc + if RES == USED { 1 } else { 2 })
}

/// `return a` (the register's value moved out: the frame ends here).
#[inline(always)]
pub fn ret<'p, const SRC: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let v = match SRC {
        NONE => Value::NULL,
        CONST => i.k0.clone(),
        _ => {
            let v = mem::replace(&mut w[usize::from(i.a)], Value::UNDEF);
            if SRC == CV && v.is_undef() {
                undef_cv(st, f, pc, i.a)
            } else if v.is_reference() {
                deref_ref(&v)
            } else {
                v
            }
        }
    };
    st.event = Event::Return(v);
    REFRAME
}

/// `$o->m($v)`: the variable `c` as argument `d` into `b`, when the method
/// (the cache of site `imm`, filled here if needed) takes it by value.
#[inline(always)]
pub fn send_var_method<'p>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let cache = &f.caches[i.imm as usize];
    let func = {
        let Some(o) = w[usize::from(i.a)].as_object() else {
            return fault_kind(st, "method call", &w[usize::from(i.a)]);
        };
        if cache.class.get() == Rc::as_ptr(&o.class) as usize {
            cache.slot.get()
        } else {
            match fill_method(f, o, cache) {
                Ok(m) => m,
                Err(e) => return fault(st, e),
            }
        }
    };
    if st.prog.funcs[func as usize].src.param_by_ref(usize::from(i.d)) {
        return fault(st, "by-reference argument not in the prototype");
    }
    let v = rd::<CV>(st, f, w, i.k0, i.c, pc);
    wr::<TMP>(w, i.b, v);
    pc + 1
}

/// `a = b->m(...)`: the method of site `imm` on the object in `b` (or
/// `$this`), with the `d` arguments at `c`.
#[inline(always)]
pub fn method_call<'p, const OBJ: u8, const RES: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let cache = &f.caches[i.imm as usize];
    let (func, this) = {
        let o: &Rc<Object> = if OBJ == THIS {
            match &st.this {
                Some(o) => o,
                None => return fault(st, "Using $this when not in object context"),
            }
        } else {
            match w[usize::from(i.b)].as_object() {
                Some(o) => o,
                None => return fault_kind(st, "method call", &w[usize::from(i.b)]),
            }
        };
        let func = if cache.class.get() == Rc::as_ptr(&o.class) as usize {
            cache.slot.get()
        } else {
            match fill_method(f, o, cache) {
                Ok(m) => m,
                Err(e) => return fault(st, e),
            }
        };
        (func, Rc::clone(o))
    };
    let callee = &st.prog.funcs[func as usize];
    if u16::from(i.d) < callee.required || u16::from(i.d) > callee.nparams {
        return fault_args(st, i.d, &callee.name);
    }
    st.event = Event::Call { func, args: i.c, argc: i.d, dst: i.a, used: RES == USED, this: Some(this) };
    REFRAME | (pc + if RES == USED { 1 } else { 2 })
}

/// `foreach` over the array in `a`: `b` gets the array, `b + 1` the
/// position 0; an empty array jumps to `imm`.
#[inline(always)]
pub fn iter_init<'p, const SRC: u8>(st: &mut State<'p>, w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let src = &w[usize::from(i.a)];
    let Some(arr) = src.as_array() else {
        return fault_kind(st, "foreach", src);
    };
    if arr.is_empty() {
        return i.imm;
    }
    let v = src.clone();
    wr::<TMP>(w, i.b, v);
    wr::<TMP>(w, i.b.wrapping_add(1), Value::int(0));
    pc + 1
}

/// The next element of iterator `a`: its value into `b` (and key into `c`),
/// or to `imm` when done.
#[inline(always)]
pub fn iter_next<'p, const WITH_KEY: u8>(st: &mut State<'p>, w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let at = usize::from(i.a);
    let pos = w[at + 1].as_int().unwrap_or(0) as usize;
    let Some(arr) = w[at].as_array() else {
        return fault(st, "iterator is not an array");
    };
    let (p, v, k) = if WITH_KEY == KEY {
        match arr.entry_at(pos) {
            Some((p, k, v)) => (p, v.deref(), Some(k)),
            None => return i.imm,
        }
    } else {
        match arr.value_at(pos) {
            Some((p, v)) => (p, v.deref(), None),
            None => return i.imm,
        }
    };
    w[at + 1] = Value::int(p as i64 + 1);
    wr::<CV>(w, i.b, v);
    if let Some(k) = k {
        wr::<CV>(w, i.c, k.into_value());
    }
    pc + 1
}

/// `a = new C` (class `imm`): a `CTOR` class's constructor runs next
/// (`Construct`); a `PLAIN` one's `Construct` is skipped.
#[inline(always)]
pub fn new_obj<'p, const HAS: u8>(st: &mut State<'p>, w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let class = &st.prog.classes[i.imm as usize];
    let o = Object::new(Rc::clone(class), &st.ids);
    wr::<TMP>(w, i.a, Value::object(o));
    pc + if HAS == CTOR { 1 } else { 2 }
}

/// The constructor (function `imm`) of the object in `a`, with the `c`
/// arguments at `b`.
#[inline(always)]
pub fn construct<'p>(st: &mut State<'p>, w: &mut Window, _f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    if i.imm == u32::MAX {
        return fault(st, "Construct reached for a class without a constructor");
    }
    let Some(o) = w[usize::from(i.a)].cloned_object() else {
        return fault(st, "Construct on a non-object");
    };
    st.event = Event::Call { func: i.imm, args: i.b, argc: i.c, dst: 0, used: false, this: Some(o) };
    REFRAME | (pc + 1)
}

/// `echo a`.
#[inline(always)]
pub fn echo<'p, const SRC: u8>(st: &mut State<'p>, w: &mut Window, f: &'p Func<'p>, i: &Insn<'p>, pc: u32) -> u32 {
    let v = rd::<SRC>(st, f, w, i.k0, i.a, pc);
    match v.to_php_string() {
        Some(s) => st.out.extend_from_slice(s.as_bytes()),
        None => return fault(st, "echo of an array or object not in the prototype"),
    }
    if st.out.len() > 1 << 16 {
        flush(st);
    }
    pc + 1
}

#[cold]
#[inline(never)]
pub fn flush(st: &mut State) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(&st.out);
    st.out.clear();
}

/// The one instruction of a function the translator refused.
#[inline(always)]
pub fn refused<'p>(st: &mut State<'p>, _w: &mut Window, f: &'p Func<'p>, _i: &Insn<'p>, _pc: u32) -> u32 {
    fault(st, format!("refused: {}", f.refused.as_deref().unwrap_or("?")))
}

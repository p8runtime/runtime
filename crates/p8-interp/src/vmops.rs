//! The handlers of the generated instructions (`p8-bytecode/ops.def`).
//!
//! Each instruction is written once here, its operand kinds and flags as
//! `const` parameters (`p8_bytecode::insn::kinds`). The generator writes
//! one dispatch arm per live combination (`P8_FETCH_DIM_KSTR` calls
//! `fetch_dim::<{KSTR}>`), so every `if KEY == KSTR` below is a test of
//! two constants, folded per arm. A handler is a plain function of the
//! frame's view (`Hx`) and its decoded operands, which gives back what
//! the loop does next (`Ctl`): it never leaves the loop itself. A case it
//! does not take inline is `Ctl::Cold`: the loop runs the instruction's
//! cold method (`cold=` in ops.def, the `Context::cold_*` below, which
//! `step` runs for it too), out of line.
//!
//! Operand values: a register is its window index, a literal its
//! constant's index (`Hx::v` reads either by its kind), `$this` a 0.

use super::*;
use crate::fast::{Src, WINDOW, clear, copied, elem, is_set, isset_elem, plain, put, put_var, read_ref, set_int_elem, test, truth};
use crate::sites::{Plain, cached_prop, plain_slot};
use p8_bytecode::insn::kinds::*;

/// What the loop does after a handler.
pub(crate) enum Ctl {
    /// Go on at the next instruction; whether the write released memory
    /// (a destructor may be due).
    Next(bool),
    /// Go past the next instruction too (the `Free` of an UNUSED result).
    Skip(bool),
    /// Go to the target (a backward one is where an interrupt is noticed).
    Jump(u32),
    /// Run the instruction's cold method instead.
    Cold,
}

/// A handler's view of the frame: the window, the closure (constants,
/// caches), the frame itself (`$this`, scope).
pub(crate) struct Hx<'a> {
    pub(crate) w: &'a mut [Value; WINDOW],
    pub(crate) clo: &'a Closure,
    pub(crate) top: &'a Frame,
    /// A clone's re-initialization is running (readonly properties).
    pub(crate) reinit: bool,
}

#[inline(always)]
fn r(x: u32) -> usize {
    x as usize & (WINDOW - 1)
}

/// A literal kind.
const fn is_k(kind: u8) -> bool {
    kind == K || kind == KINT || kind == KSTR
}

impl Hx<'_> {
    /// A value operand of kind `KIND`: a literal read in place, or a register.
    #[inline(always)]
    fn v<const KIND: u8>(&self, x: u32) -> &Value {
        if is_k(KIND) { &self.clo.consts[x as usize] } else { &self.w[r(x)] }
    }

    /// The same as a `Src`, for the out-of-line helpers.
    #[inline(always)]
    fn src<const KIND: u8>(&self, x: u32) -> Src<'_> {
        if is_k(KIND) { Src::Const(&self.clo.consts[x as usize]) } else { Src::Reg(x as Reg) }
    }

    /// The class the frame's code is in, as a cache entry keys it.
    #[inline(always)]
    fn scope(&self) -> usize {
        self.top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize)
    }
}

/// `dst = src`: a register's copy (a reference's value read through it;
/// undefined: cold, the warning) or a literal. A write that released an
/// object stops for its destructor before the next instruction.
#[inline(always)]
pub(crate) fn mov<const SRC: u8>(hx: Hx, dst: u32, src: u32) -> Ctl {
    if is_k(SRC) {
        let v = hx.v::<SRC>(src).clone();
        return Ctl::Next(put(&mut hx.w[r(dst)], v));
    }
    let Some(v) = copied(&hx.w[r(src)]) else { return Ctl::Cold };
    let _ = put(&mut hx.w[r(dst)], v);
    Ctl::Next(true)
}

/// `$var = src`, through a reference the variable holds.
#[inline(always)]
pub(crate) fn assign<const SRC: u8>(hx: Hx, var: u32, src: u32) -> Ctl {
    let v = if is_k(SRC) {
        hx.v::<SRC>(src).clone()
    } else {
        match copied(&hx.w[r(src)]) {
            Some(v) => v,
            None => return Ctl::Cold,
        }
    };
    Ctl::Next(put_var(&mut hx.w[r(var)], v))
}

/// `$var = tmp`, the temporary read for the last time: moved out of it.
#[inline(always)]
pub(crate) fn assign_take(hx: Hx, var: u32, src: u32) -> Ctl {
    let taken = std::mem::replace(&mut hx.w[r(src)], Value::UNDEF);
    // Undefined: the warning (the register is as it was).
    if taken.is_undef() {
        return Ctl::Cold;
    }
    if let Some(cell) = taken.as_reference() {
        // A reference's value, and the temporary lets go of the reference
        // (which may have been its last holder).
        return match read_ref(cell) {
            Some(v) => {
                let _ = put_var(&mut hx.w[r(var)], v);
                drop(taken);
                Ctl::Next(true)
            }
            None => {
                hx.w[r(src)] = taken;
                Ctl::Cold
            }
        };
    }
    Ctl::Next(put_var(&mut hx.w[r(var)], taken))
}

#[inline(always)]
pub(crate) fn free(hx: Hx, reg: u32) -> Ctl {
    Ctl::Next(clear(&mut hx.w[r(reg)]))
}

/// `$this`, in a method that has one (else cold: the error).
#[inline(always)]
pub(crate) fn this(hx: Hx, dst: u32) -> Ctl {
    let Some(this) = &hx.top.this else { return Ctl::Cold };
    let this = Value::object(Rc::clone(this));
    Ctl::Next(put(&mut hx.w[r(dst)], this))
}

/// A condition: a bool inline, anything else through `truth` (what may
/// warn, NAN and an undefined variable, is cold).
#[inline(always)]
pub(crate) fn jump_cond<const WHEN: u8>(hx: Hx, cond: u32, target: u32) -> Ctl {
    let v = &hx.w[r(cond)];
    let c = match v.as_bool() {
        Some(c) => c,
        None => match truth(v) {
            Some(c) => c,
            None => return Ctl::Cold,
        },
    };
    if c == (WHEN == T) { Ctl::Jump(target) } else { Ctl::Next(false) }
}

/// A compare-and-branch: two ints inline, anything else (floats,
/// references, strings, null) through `test`; an undefined variable (its
/// warning) is cold.
#[inline(always)]
pub(crate) fn jump_cmp<const CMP: u8, const WHEN: u8, const B: u8>(hx: Hx, a: u32, b: u32, target: u32) -> Ctl {
    let cmp = match CMP {
        LT => Cmp::Lt,
        LE => Cmp::Le,
        GT => Cmp::Gt,
        GE => Cmp::Ge,
        EQ => Cmp::Eq,
        _ => Cmp::Identical,
    };
    let (x, y) = (&hx.w[r(a)], hx.v::<B>(b));
    let t = if let (Some(x), Some(y)) = (x.as_int(), y.as_int()) {
        int_test(cmp, x, y)
    } else {
        match test(cmp, x, y) {
            Some(t) => t,
            None => return Ctl::Cold,
        }
    };
    if t == (WHEN == T) { Ctl::Jump(target) } else { Ctl::Next(false) }
}

/// `$a[k]` on an array holding a plain value at that key: an int key, a
/// string literal (looked up by its hash, in place), a string in a
/// register; an array through a reference by `elem`. Anything else (a
/// missing key's warning, an object, a string's offset) is cold.
#[inline(always)]
pub(crate) fn fetch_dim<const KEY: u8>(hx: Hx, dst: u32, arr: u32, key: u32) -> Ctl {
    let kv = hx.v::<KEY>(key);
    let v = if let Some(a) = hx.w[r(arr)].as_array() {
        let e = if KEY == KSTR {
            match kv.as_str() {
                Some(s) => a.get_str(s),
                None => return Ctl::Cold,
            }
        } else if let Some(i) = kv.as_int() {
            a.get_int(i)
        } else if KEY == REG
            && let Some(s) = kv.as_str()
        {
            a.get_str(s)
        } else {
            return Ctl::Cold;
        };
        match e {
            Some(v) if !v.is_reference() => v.clone(),
            _ => return Ctl::Cold,
        }
    } else {
        match elem(&hx.w[..], arr as Reg, hx.src::<KEY>(key)) {
            Some(v) => v,
            None => return Ctl::Cold,
        }
    };
    let _ = put(&mut hx.w[r(dst)], v);
    Ctl::Next(true)
}

/// `isset($a[k])` on an array (or a reference to one), k an int or a
/// string; the rest (an object, a string's offset, a key of another type)
/// is cold.
#[inline(always)]
pub(crate) fn isset_dim<const KEY: u8>(hx: Hx, dst: u32, var: u32, key: u32) -> Ctl {
    let kv = hx.v::<KEY>(key);
    let set = if let Some(a) = hx.w[r(var)].as_array() {
        if KEY == KSTR {
            match kv.as_str() {
                Some(s) => a.get_str(s).is_some_and(is_set),
                None => return Ctl::Cold,
            }
        } else if let Some(i) = kv.as_int() {
            a.get_int(i).is_some_and(is_set)
        } else if KEY == REG
            && let Some(s) = kv.as_str()
        {
            a.get_str(s).is_some_and(is_set)
        } else {
            return Ctl::Cold;
        }
    } else {
        match isset_elem(&hx.w[..], var as Reg, hx.src::<KEY>(key)) {
            Some(set) => set,
            None => return Ctl::Cold,
        }
    };
    Ctl::Next(put(&mut hx.w[r(dst)], Value::bool(set)))
}

/// `$a[k] = v`, `$a` an array (or, for an int key, a reference to one): a
/// string key with any defined, non-reference value; an int key with a
/// plain value. The element replaced may have been an object's last
/// holder: tested as for any write. USED: the value to `dst` too; UNUSED:
/// nothing there, and the Free after is skipped.
#[inline(always)]
pub(crate) fn assign_dim<const KEY: u8, const VAL: u8, const RES: u8>(hx: Hx, var: u32, key: u32, val: u32, dst: u32) -> Ctl {
    let kv = hx.v::<KEY>(key);
    let v = if KEY == KSTR || (KEY == REG && kv.is_str()) {
        let (Some(s), Some(v)) = (kv.cloned_str(), copied(hx.v::<VAL>(val))) else { return Ctl::Cold };
        let key = Key::from_string(s);
        let slot = &mut hx.w[r(var)];
        match slot.as_array() {
            Some(a) if !a.get(&key).is_some_and(Value::is_reference) => {}
            _ => return Ctl::Cold,
        }
        let a = slot.array_mut().expect("an array");
        let (stored, result) = if RES == USED { (v.clone(), Some(v)) } else { (v, None) };
        match a.get_mut(&key) {
            // (The old element's release: tested below, as for any write.)
            Some(e) => *e = stored,
            None => a.insert(key, stored),
        }
        result
    } else {
        let (Some(k), Some(v)) = (kv.as_int(), plain(hx.v::<VAL>(val))) else { return Ctl::Cold };
        let slot = &mut hx.w[r(var)];
        let done = if slot.is_array() {
            set_int_elem(slot, k, &v)
        } else if let Some(cell) = slot.as_reference() {
            set_int_elem(&mut cell.borrow_mut(), k, &v)
        } else {
            false
        };
        if !done {
            return Ctl::Cold;
        }
        (RES == USED).then_some(v)
    };
    match v {
        Some(v) => {
            let _ = put(&mut hx.w[r(dst)], v);
            Ctl::Next(true)
        }
        None => Ctl::Skip(true),
    }
}

/// A property read the name's cache entry answers (anything else is
/// cold: a miss, an unset slot, a reference operand).
#[inline(always)]
pub(crate) fn fetch_prop<const OBJ: u8>(hx: Hx, dst: u32, obj: u32, site: u32) -> Ctl {
    let Some(entry) = hx.clo.props.get(site as usize) else { return Ctl::Cold };
    let o = if OBJ == THIS {
        let Some(o) = &hx.top.this else { return Ctl::Cold };
        o
    } else {
        let Some(o) = hx.w[r(obj)].as_object() else { return Ctl::Cold };
        o
    };
    let Some(v) = cached_prop(entry.get(), o, hx.scope()) else { return Ctl::Cold };
    Ctl::Next(put(&mut hx.w[r(dst)], v))
}

/// `$o->name = v` the name's cache entry answers, on a property with
/// nothing to check (`plain_slot`): the slots borrowed once, nothing
/// cloned but the value, and that only when the result is USED. An unset
/// slot of a class with __set and its kin, and anything `plain_slot`
/// declines, is cold. TAKE: the value's temporary is released after.
#[inline(always)]
pub(crate) fn assign_prop<const OBJ: u8, const VAL: u8, const MODE: u8, const RES: u8>(hx: Hx, obj: u32, val: u32, dst: u32, site: u32) -> Ctl {
    let Some(entry) = hx.clo.props.get(site as usize) else { return Ctl::Cold };
    let o = if OBJ == THIS {
        let Some(o) = &hx.top.this else { return Ctl::Cold };
        o
    } else {
        let Some(o) = hx.w[r(obj)].as_object() else { return Ctl::Cold };
        o
    };
    let v = hx.v::<VAL>(val);
    let Some((slot, plain)) = plain_slot(entry.get(), o, (hx.scope(), hx.top.scope.as_deref()), v, hx.reinit) else { return Ctl::Cold };
    let written = match o.slots().get(slot) {
        Some(s)
            if (o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0 && plain == Plain::Any)
                || {
                    let unset = p8_runtime::object::Slot::peek(s, Value::is_undef);
                    if plain == Plain::Unset { unset && o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0 } else { !unset }
                } =>
        {
            // The slot's value out, written as a register is, back in.
            // (Pending is tested after, whatever this released.)
            let mut cur = s.replace(Value::UNDEF);
            let _ = put_var(&mut cur, v.clone());
            p8_runtime::object::put_slot(s, cur);
            true
        }
        _ => false,
    };
    if !written {
        return Ctl::Cold;
    }
    let ctl = if RES == USED {
        let v = hx.v::<VAL>(val).clone();
        let _ = put(&mut hx.w[r(dst)], v);
        Ctl::Next(true)
    } else {
        Ctl::Skip(true)
    };
    if MODE == TAKE {
        let _ = clear(&mut hx.w[r(val)]);
    }
    ctl
}

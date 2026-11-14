//! The handlers of the generated instructions (`p8-bytecode/ops.def`).
//!
//! Each instruction is written once here. Its signature is generated
//! (p8-vmgen, from ops.def) and matched by name: the decoded operands are
//! the fields of `args::<Op>`, destructured, and the kind of each operand
//! that has several (a flag's too) is an associated type of the
//! specialisation's `spec::<Op>` impl (`S::Key`, `kind::KSTR` in
//! `P8_FETCH_DIM_R_KSTR`). The generator writes one dispatch arm per live
//! combination, so every `if S::Key::ID == KSTR` below is a test of two
//! constants, folded per arm. A handler is a plain function of the frame's
//! view (`Hx`) and its operands, which gives back what the loop does next
//! (`Ctl`): it never leaves the loop itself. A case it does not take
//! inline is `Ctl::Cold`: the loop runs the instruction's cold method
//! (`cold=` in ops.def, `Context::cold_run`, which `step` runs for it
//! too), out of line.
//!
//! Operand values: a register is its window index, a literal its
//! constant's index (`Hx::v` reads either by its kind), `$this` a 0.

use super::*;
use crate::fast::{Src, WINDOW, Wrote, clear, copied, identical_rest, is_owner, is_set, isset_elem, loose_eq_rest, owner_write, put, put_var, read_ref, ref_object, set_int_elem, test, truth, write_ref};
use crate::sites::{Plain, cached_prop, dynamic_prop, plain_slot, refill_prop};
use p8_bytecode::insn::args;
use p8_bytecode::insn::kind::Kind;
use p8_bytecode::insn::kinds::*;
use p8_bytecode::insn::spec;

/// What the loop does after a handler.
pub(crate) enum Ctl {
    /// Go on at the next instruction; whether the write released memory
    /// (a destructor may be due).
    Next(bool),
    /// Go past the next instruction too (the `Free` of an UNUSED result).
    Skip(bool),
    /// Go to the target (a backward one is where an interrupt is noticed).
    Jump(u32),
    /// Return this value from the frame (`ret!` in the loop: the caller's
    /// frame, or out of the loop's run from its bottom frame).
    Return(Value),
    /// Run the instruction's cold method instead.
    Cold,
}

/// A handler's view of the frame: the window, the closure (constants,
/// caches), the frame itself (`$this`, scope).
pub(crate) struct Hx<'a> {
    pub(crate) w: &'a mut [Value; WINDOW],
    pub(crate) clo: &'a Closure,
    pub(crate) top: &'a Frame,
    /// The frame stack (`top` is its last).
    pub(crate) frames: &'a Frames,
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
    /// A value operand of kind `K`: a literal read in place, or a register.
    #[inline(always)]
    fn v<K: Kind>(&self, x: u32) -> &Value {
        if is_k(K::ID) { &self.clo.consts[x as usize] } else { &self.w[r(x)] }
    }

    /// The same as a `Src`, for the out-of-line helpers.
    #[inline(always)]
    fn src<K: Kind>(&self, x: u32) -> Src<'_> {
        if is_k(K::ID) { Src::Const(&self.clo.consts[x as usize]) } else { Src::Reg(x as Reg) }
    }

    /// Class reference `class` of kind `C` (NAMED, STATIC, SCOPE) as a
    /// cache entry keys it (`ConstSite::key`, `Context::class_key`): a
    /// named one by its index, the called class or the scope by its
    /// address; None when there is no such class.
    #[inline(always)]
    fn class_key<C: Kind>(&self, class: u32) -> Option<usize> {
        if C::ID == NAMED {
            return Some(class as usize + 1);
        }
        let c = if C::ID == STATIC { self.top.called_class() } else { self.top.scope.as_ref() };
        Some(Rc::as_ptr(c?) as usize)
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
pub(crate) fn mov<S: spec::Mov>(hx: Hx, args::Mov { dst, src }: args::Mov) -> Ctl {
    if is_k(S::Src::ID) {
        let v = hx.v::<S::Src>(src).clone();
        return Ctl::Next(put(&mut hx.w[r(dst)], v));
    }
    let Some(v) = copied(&hx.w[r(src)]) else { return Ctl::Cold };
    let _ = put(&mut hx.w[r(dst)], v);
    Ctl::Next(true)
}

/// `$var = src`, through a reference the variable holds.
#[inline(always)]
pub(crate) fn assign<S: spec::Assign>(hx: Hx, args::Assign { var, src }: args::Assign) -> Ctl {
    let v = if is_k(S::Src::ID) {
        hx.v::<S::Src>(src).clone()
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
pub(crate) fn assign_take(hx: Hx, args::AssignTake { var, src }: args::AssignTake) -> Ctl {
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
pub(crate) fn free(hx: Hx, args::Free { reg }: args::Free) -> Ctl {
    Ctl::Next(clear(&mut hx.w[r(reg)]))
}

/// `$this`, in a method that has one (else cold: the error).
#[inline(always)]
pub(crate) fn this(hx: Hx, args::This { dst }: args::This) -> Ctl {
    let Some(this) = &hx.top.this else { return Ctl::Cold };
    let this = Value::object(Rc::clone(this));
    Ctl::Next(put(&mut hx.w[r(dst)], this))
}

/// A condition: a bool, an int and null inline (`truth_by_tag`),
/// anything else through `truth` (what may warn, NAN and an undefined
/// variable, is cold).
#[inline(always)]
pub(crate) fn jump_cond<S: spec::JumpCond>(hx: Hx, args::JumpCond { cond, target }: args::JumpCond) -> Ctl {
    let v = &hx.w[r(cond)];
    let c = match v.truth_by_tag() {
        Some(c) => c,
        None => match truth(v) {
            Some(c) => c,
            None => return Ctl::Cold,
        },
    };
    if c == (S::When::ID == T) { Ctl::Jump(target) } else { Ctl::Next(false) }
}

/// A compare-and-branch. Inline: two ints, and for `===` whatever the
/// two tags and a word decide (`identical_by_tag`: null, bools, two
/// program-memory strings, two kinds). Two strings are one call
/// (`identical_rest`, `loose_eq_rest`); anything else (floats,
/// references, arrays, objects) goes through `test`; an undefined
/// variable (its warning) is cold.
#[inline(always)]
pub(crate) fn jump_cmp<S: spec::JumpCmp>(hx: Hx, args::JumpCmp { a, b, target }: args::JumpCmp) -> Ctl {
    let cmp = match S::Cmp::ID {
        LT => Cmp::Lt,
        LE => Cmp::Le,
        GT => Cmp::Gt,
        GE => Cmp::Ge,
        EQ => Cmp::Eq,
        _ => Cmp::Identical,
    };
    let (x, y) = (&hx.w[r(a)], hx.v::<S::B>(b));
    let t = if S::Cmp::ID == ID {
        x.identical_by_tag(y).or_else(|| identical_rest(x, y))
    } else if let (Some(x), Some(y)) = (x.as_int(), y.as_int()) {
        Some(int_test(cmp, x, y))
    } else if S::Cmp::ID == EQ {
        loose_eq_rest(x, y)
    } else {
        test(cmp, x, y)
    };
    let Some(t) = t else { return Ctl::Cold };
    if t == (S::When::ID == T) { Ctl::Jump(target) } else { Ctl::Next(false) }
}

/// `$a[k]` on an array holding a plain value at that key: an int key, a
/// string literal (looked up by its hash, in place), a string in a
/// register; an array through a reference by `elem`. IS (`??`) and EMPTY
/// (empty()'s last dimension): a missing key of an array is null. Anything
/// else (a missing key's warning, an object, a string's offset) is cold.
#[inline(always)]
pub(crate) fn fetch_dim<S: spec::FetchDim>(hx: Hx, args::FetchDim { dst, arr, key }: args::FetchDim) -> Ctl {
    let kv = hx.v::<S::Key>(key);
    let v = if let Some(a) = hx.w[r(arr)].as_array() {
        let e = if S::Key::ID == KSTR {
            // (Never a canonical integer: lowering's promise.)
            match kv.as_str() {
                Some(s) => a.get_str_key(s),
                None => return Ctl::Cold,
            }
        } else if let Some(i) = kv.as_int() {
            a.get_int(i)
        } else if S::Key::ID == REG
            && let Some(s) = kv.as_str()
        {
            a.get_str(s)
        } else {
            return Ctl::Cold;
        };
        match e {
            Some(v) if !v.is_reference() => v.clone(),
            // (An element bound by reference: read through it, as a read
            // derefs; out of line.)
            Some(v) => match ref_elem(v) {
                Some(v) => v,
                None => return Ctl::Cold,
            },
            None if S::Mode::ID != R => Value::NULL,
            None => return Ctl::Cold,
        }
    } else {
        // (Out of line: the array case is the arm's straight path.)
        match dim_of(&hx.w[..], arr as Reg, hx.src::<S::Key>(key), S::Key::ID == KSTR, S::Mode::ID != R) {
            Some(v) => v,
            None => return Ctl::Cold,
        }
    };
    let _ = put(&mut hx.w[r(dst)], v);
    Ctl::Next(true)
}

/// An array element that is a reference, read through it (`read_ref`:
/// None when it holds nothing).
#[cold]
#[inline(never)]
fn ref_elem(v: &Value) -> Option<Value> {
    read_ref(v.as_reference()?)
}

/// `$c[k]` of anything but an array: a string's offset, an int key, is its
/// byte (one of the shared one-byte strings), counted from the end when
/// negative (out of range: null when `quiet`, else None, the warning); a
/// quiet read (`??`, `empty`) of null, an undefined variable or a scalar
/// is null, silently, as ZEND_FETCH_DIM_IS gives. Through a reference the
/// same of what its cell holds, and an array's element (a missing one
/// null when `quiet`). None (cold) for the rest: a string's string key
/// (`kstr`: a literal one), an object, a warning.
#[cold]
#[inline(never)]
fn dim_of(w: &[Value], arr: Reg, key: Src, kstr: bool, quiet: bool) -> Option<Value> {
    let c = &w[usize::from(arr)];
    let Some(cell) = c.as_reference() else { return dim_in(c, w, key, kstr, quiet) };
    let held = cell.borrow();
    if let Some(a) = held.as_array() {
        let k = crate::fast::src(w, key);
        let e = if let Some(i) = k.as_int() {
            a.get_int(i)
        } else if let Some(s) = k.as_str() {
            a.get_str(s)
        } else {
            return None;
        };
        return match e {
            // (An element bound by reference, to this cell too: read, a
            // second shared borrow.)
            Some(v) => match v.as_reference() {
                Some(e) => read_ref(e),
                None => Some(v.clone()),
            },
            None if quiet => Some(Value::NULL),
            None => None,
        };
    }
    dim_in(&held, w, key, kstr, quiet)
}

/// `dim_of` of a value that is not a reference.
fn dim_in(c: &Value, w: &[Value], key: Src, kstr: bool, quiet: bool) -> Option<Value> {
    if let Some(s) = c.as_str() {
        let i = if kstr { None } else { crate::fast::src(w, key).as_int() }?;
        let at = if i < 0 { i.wrapping_add(s.len() as i64) } else { i };
        return match usize::try_from(at).ok().and_then(|at| s.as_bytes().get(at)) {
            Some(&b) => Some(Value::str(PhpStr::byte(b))),
            None if quiet => Some(Value::NULL),
            None => None,
        };
    }
    if quiet && (c.is_null_or_undef() || c.is_bool() || c.is_int() || c.is_float()) {
        return Some(Value::NULL);
    }
    None
}

/// `isset($a[k])` on an array (or a reference to one), k an int or a
/// string; the rest (an object, a string's offset, a key of another type)
/// is cold.
#[inline(always)]
pub(crate) fn isset_dim<S: spec::IssetDim>(hx: Hx, args::IssetDim { dst, var, key }: args::IssetDim) -> Ctl {
    let kv = hx.v::<S::Key>(key);
    let set = if let Some(a) = hx.w[r(var)].as_array() {
        if S::Key::ID == KSTR {
            match kv.as_str() {
                Some(s) => a.get_str_key(s).is_some_and(is_set),
                None => return Ctl::Cold,
            }
        } else if let Some(i) = kv.as_int() {
            a.get_int(i).is_some_and(is_set)
        } else if S::Key::ID == REG
            && let Some(s) = kv.as_str()
        {
            a.get_str(s).is_some_and(is_set)
        } else {
            return Ctl::Cold;
        }
    } else {
        match isset_elem(&hx.w[..], var as Reg, hx.src::<S::Key>(key)) {
            Some(set) => set,
            None => match isset_scalar(&hx.w[r(var)], kv) {
                Some(set) => set,
                None => return Ctl::Cold,
            },
        }
    };
    Ctl::Next(put(&mut hx.w[r(dst)], Value::bool(set)))
}

/// `isset($c[k])` of a container that is not an array (through a
/// reference too): null, undefined, a bool or a number has no element
/// (no warning: isset is quiet); a string has the byte at an int offset
/// inside it (counted from the end when negative). None (cold) for the
/// rest: an object (ArrayAccess), a string's string key.
#[cold]
#[inline(never)]
fn isset_scalar(c: &Value, key: &Value) -> Option<bool> {
    let c = c.deref();
    if c.is_null_or_undef() || c.is_bool() || c.is_int() || c.is_float() {
        return Some(false);
    }
    let s = c.as_str()?;
    let i = key.as_int()?;
    let len = s.len() as i64;
    Some(if i < 0 { -i <= len } else { i < len })
}

/// `$a[k] = v`, `$a` an array or a reference to one, the key an int or a
/// string, the value any defined one: one lookup of the key. An element
/// bound by reference gets the value in its cell; one bound to `$a`'s own
/// cell (`$a[k] = &$a`) makes the cell the value, as PHP's assignment
/// through that reference does, after the array's borrow is over. The
/// element replaced may have been an object's last holder: tested as for
/// any write. USED: the value to `dst` too; UNUSED: nothing there, and the
/// Free after is skipped.
#[inline(always)]
pub(crate) fn assign_dim<S: spec::AssignDim>(hx: Hx, args::AssignDim { var, key, val, dst }: args::AssignDim) -> Ctl {
    let kv = hx.v::<S::Key>(key);
    let v = if S::Key::ID == KSTR || (S::Key::ID == REG && kv.is_str()) {
        // The key's value copied out of the frame (a literal or a count: no
        // string copied), so the array can be written while it is borrowed.
        let key = kv.clone();
        let (Some(s), Some(v)) = (key.as_str(), copied(hx.v::<S::Val>(val))) else { return Ctl::Cold };
        // One lookup, as zend_hash_lookup: the slot is written in place, or
        // the key added with no second search. (A shared array is separated
        // first: the cold write would separate it too.) A variable bound by
        // reference: the array its cell holds.
        let slot = &mut hx.w[r(var)];
        let cell = slot.as_reference().map(Rc::clone);
        let (stored, result) = if S::Res::ID == USED { (v.clone(), Some(v)) } else { (v, None) };
        // The value back when the element found is bound to the cell the
        // array is borrowed from (`$a['k'] = &$a`, `is_owner`): written
        // below, once that borrow is over.
        let owned = {
            let mut held;
            let root: &mut Value = match &cell {
                Some(cell) => {
                    held = cell.borrow_mut();
                    &mut held
                }
                None => slot,
            };
            let Some(a) = root.array_mut() else { return Ctl::Cold };
            // (A KSTR literal is never a canonical integer: lowering's promise.)
            let found = if S::Key::ID == KSTR { a.get_str_key_mut(s) } else { a.get_str_mut(s) };
            match found {
                Some(e) if is_owner(e, cell.as_deref()) => Some(stored),
                // An element bound by reference: the value goes into its cell.
                Some(e) if e.is_reference() => {
                    let _ = write_ref(e.as_reference().expect("a reference"), stored);
                    None
                }
                // (The old element's release: tested below, as for any write.)
                Some(e) => {
                    *e = stored;
                    None
                }
                None => {
                    let s = key.cloned_str().expect("a string");
                    a.insert_absent(if S::Key::ID == KSTR { Key::Str(s) } else { Key::from_string(s) }, stored);
                    None
                }
            }
        };
        if let (Some(v), Some(cell)) = (owned, &cell) {
            owner_write(cell, v);
        }
        result
    } else {
        // (Any defined value, copied: a string, an array shares its
        // storage until either is written.)
        let (Some(k), Some(v)) = (kv.as_int(), copied(hx.v::<S::Val>(val))) else { return Ctl::Cold };
        let slot = &mut hx.w[r(var)];
        let done = if slot.is_array() {
            set_int_elem(slot, k, &v, None)
        } else if let Some(cell) = slot.as_reference() {
            // (The borrow ends with this statement: a match on it would
            // hold it through the arms.)
            let wrote = set_int_elem(&mut cell.borrow_mut(), k, &v, Some(&**cell));
            match wrote {
                // The element bound to the variable's own cell: written
                // now that the borrow is over.
                Some(Wrote::Owner) => {
                    owner_write(cell, v.clone());
                    Some(Wrote::Done(true))
                }
                done => done,
            }
        } else {
            None
        };
        if done.is_none() {
            return Ctl::Cold;
        }
        (S::Res::ID == USED).then_some(v)
    };
    match v {
        Some(v) => {
            let _ = put(&mut hx.w[r(dst)], v);
            Ctl::Next(true)
        }
        None => {
            // (What the skipped Free does: a stale value there goes now.)
            let _ = clear(&mut hx.w[r(dst)]);
            Ctl::Skip(true)
        }
    }
}

/// `$a[] = v`, `$a` an array (separated if shared; through a reference,
/// out of line: `push_ref`): appended at its next key; an occupied next
/// key (the error), anything but an array, an undefined value is cold.
#[inline(always)]
pub(crate) fn append_dim<S: spec::AppendDim>(hx: Hx, args::AppendDim { var, val, dst }: args::AppendDim) -> Ctl {
    let v = if is_k(S::Val::ID) {
        hx.v::<S::Val>(val).clone()
    } else {
        match copied(&hx.w[r(val)]) {
            Some(v) => v,
            None => return Ctl::Cold,
        }
    };
    let kept = if S::Res::ID == USED { Some(v.clone()) } else { None };
    match hx.w[r(var)].array_mut() {
        Some(a) => {
            if a.next_key().is_none() {
                return Ctl::Cold;
            }
            let _ = a.push(v);
        }
        None => {
            if !push_ref(&hx.w[r(var)], v) {
                return Ctl::Cold;
            }
        }
    }
    match kept {
        Some(v) => {
            let _ = put(&mut hx.w[r(dst)], v);
            Ctl::Next(true)
        }
        // (What the skipped Free does: a stale value there goes now.)
        None => Ctl::Skip(clear(&mut hx.w[r(dst)])),
    }
}

/// `$a[] = v` on a variable bound by reference (`global $list; $list[] =
/// $x`): pushed onto the array its cell holds, at its next key; false
/// (cold) for anything else.
#[cold]
#[inline(never)]
fn push_ref(var: &Value, v: Value) -> bool {
    let Some(cell) = var.as_reference() else { return false };
    let mut held = cell.borrow_mut();
    let Some(a) = held.array_mut() else { return false };
    if a.next_key().is_none() {
        return false;
    }
    let _ = a.push(v);
    true
}

/// `v` written to `o`'s declared slot `slot`, as `plain_slot` found it
/// writable (`plain`): the slots borrowed once, the value out, written as
/// a register is, back in. False (nothing written) for an unset slot of a
/// class with __set and its kin, and a readonly property's slot set
/// already. (Pending destructors are tested after, whatever this
/// released.)
#[inline(always)]
pub(crate) fn write_slot(o: &p8_runtime::object::Object, slot: usize, plain: Plain, v: &Value) -> bool {
    match o.slots().get(slot) {
        Some(s)
            if (o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0 && plain == Plain::Any)
                || {
                    let unset = p8_runtime::object::Slot::peek(s, Value::is_undef);
                    if plain == Plain::Unset { unset && o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS == 0 } else { !unset }
                } =>
        {
            let mut cur = s.replace(Value::UNDEF);
            let _ = put_var(&mut cur, v.clone());
            p8_runtime::object::put_slot(s, cur);
            true
        }
        _ => false,
    }
}

/// A property read the name's cache entry answers, of an object or of
/// one a reference holds (`global $wpdb; $wpdb->posts`); anything else is
/// cold: a miss, an unset slot.
#[inline(always)]
pub(crate) fn fetch_prop<S: spec::FetchProp>(hx: Hx, args::FetchProp { dst, obj, site }: args::FetchProp) -> Ctl {
    let Some(entry) = hx.clo.props.get(site as usize) else { return Ctl::Cold };
    let held;
    let o = if S::Obj::ID == THIS {
        let Some(o) = &hx.top.this else { return Ctl::Cold };
        o
    } else {
        match hx.w[r(obj)].as_object() {
            Some(o) => o,
            // (Its own handle, out of line: the reference's borrow ends.)
            None => {
                held = ref_object(&hx.w[r(obj)]);
                let Some(o) = &held else { return Ctl::Cold };
                o
            }
        }
    };
    let v = match cached_prop(entry.get(), o, hx.scope()) {
        Some(v) => v,
        // A class or scope the entry has not seen: filled for this one
        // (`refill_prop`, out of line), then read.
        None if refill_prop(entry, o, &hx.clo.consts[site as usize], hx.top.scope.as_ref()) => match cached_prop(entry.get(), o, hx.scope()) {
            Some(v) => v,
            None => return Ctl::Cold,
        },
        // A dynamic property (`dynamic_prop`, out of line).
        None => match dynamic_prop(o, &hx.clo.consts[site as usize]) {
            Some(v) => v,
            None => return Ctl::Cold,
        },
    };
    Ctl::Next(put(&mut hx.w[r(dst)], v))
}

/// `$o->name = v` the name's cache entry answers, on a property with
/// nothing to check (`plain_slot`): the slots borrowed once, nothing
/// cloned but the value, and that only when the result is USED. An unset
/// slot of a class with __set and its kin, and anything `plain_slot`
/// declines, is cold. TAKE: the value's temporary is released after.
#[inline(always)]
pub(crate) fn assign_prop<S: spec::AssignProp>(hx: Hx, args::AssignProp { obj, val, dst, site }: args::AssignProp) -> Ctl {
    let Some(entry) = hx.clo.props.get(site as usize) else { return Ctl::Cold };
    let o = if S::Obj::ID == THIS {
        let Some(o) = &hx.top.this else { return Ctl::Cold };
        o
    } else {
        let Some(o) = hx.w[r(obj)].as_object() else { return Ctl::Cold };
        o
    };
    let v = hx.v::<S::Val>(val);
    let scope = (hx.scope(), hx.top.scope.as_deref());
    let found = match plain_slot(entry.get(), o, scope, v, hx.reinit) {
        Some(found) => Some(found),
        // A class or scope the entry has not seen: filled for this one
        // (`refill_prop`, out of line), then asked again.
        None if refill_prop(entry, o, &hx.clo.consts[site as usize], hx.top.scope.as_ref()) => plain_slot(entry.get(), o, scope, v, hx.reinit),
        None => None,
    };
    let Some((slot, plain)) = found else { return Ctl::Cold };
    if !write_slot(o, slot, plain, v) {
        return Ctl::Cold;
    }
    let ctl = if S::Res::ID == USED {
        let v = hx.v::<S::Val>(val).clone();
        let _ = put(&mut hx.w[r(dst)], v);
        Ctl::Next(true)
    } else {
        // (What the skipped Free does: a stale value there goes now.)
        let _ = clear(&mut hx.w[r(dst)]);
        Ctl::Skip(true)
    };
    if S::Mode::ID == TAKE {
        let _ = clear(&mut hx.w[r(val)]);
    }
    ctl
}

/// `return src`: the value moved out of its register when nothing reads
/// the register again (a temporary; a variable of a frame whose variables
/// are its own), else copied; a reference's value read through it. To
/// the calling frame, or out of the loop's run from its bottom frame.
/// The script's end (the main frame's return), a generator's return and
/// an undefined variable (the warning) are cold.
#[inline(always)]
pub(crate) fn ret<S: spec::Ret>(hx: Hx, args::Ret { src }: args::Ret) -> Ctl {
    let f = hx.frames;
    if f.len() == f.floor + 1 && (f.floor == 0 || hx.clo.code.is_generator) {
        return Ctl::Cold;
    }
    if is_k(S::Src::ID) {
        return Ctl::Return(hx.v::<S::Src>(src).clone());
    }
    let temp = S::Src::ID == TMP;
    let slot = &mut hx.w[r(src)];
    let v = if slot.is_undef() {
        return Ctl::Cold;
    } else if let Some(cell) = slot.as_reference() {
        let v = cell.borrow().clone();
        // A temporary lets go of the reference (none is left at exit).
        if temp {
            drop(std::mem::take(slot));
        }
        v
    } else if temp || (!hx.top.global && hx.top.extra.is_none()) {
        std::mem::take(slot)
    } else {
        slot.clone()
    };
    Ctl::Return(v)
}

/// `src instanceof C`: an object (or a reference to one) tested against
/// the class (`Class::is_a`, its parents and interfaces by name); anything
/// else is false. A named class not resolved yet (the cold path resolves
/// it, never autoloading), no called class or scope, an undefined
/// variable (the warning) are cold.
#[inline(always)]
pub(crate) fn instance_of<S: spec::InstanceOf>(hx: Hx, args::InstanceOf { dst, src, class }: args::InstanceOf) -> Ctl {
    // A class in a register (its errors, a string's class), `parent` in a
    // trait: out of line, whatever `src` is.
    if S::Cls::ID == OTHER {
        return Ctl::Cold;
    }
    let is = |o: &Object| -> Option<bool> {
        if S::Cls::ID == NAMED {
            let (p8_bytecode::ClassRef::Named(name) | p8_bytecode::ClassRef::Forward(name)) = &hx.clo.code.classes[class as usize] else { return None };
            let target = hx.clo.class_refs[class as usize].borrow();
            if std::ptr::eq(Rc::as_ptr(&o.class), target.as_ptr()) {
                return Some(true);
            }
            let target = target.upgrade()?;
            // (Through an alias, the class it names, and the name itself.)
            Some(o.class.is_a(&target.lower) || (target.lower != name.lower.as_bytes() &&o.class.is_a(name.lower.as_bytes())))
        } else {
            let c = if S::Cls::ID == STATIC { hx.top.called_class()? } else { hx.top.scope.as_ref()? };
            Some(Rc::ptr_eq(&o.class, c) || o.class.is_a(&c.lower))
        }
    };
    let v = &hx.w[r(src)];
    let t = if let Some(o) = v.as_object() {
        is(o)
    } else if let Some(cell) = v.as_reference() {
        let v = cell.borrow();
        match v.as_object() {
            Some(o) => is(o),
            None => Some(false),
        }
    } else if v.is_undef() {
        None
    } else {
        Some(false)
    };
    let Some(t) = t else { return Ctl::Cold };
    Ctl::Next(put(&mut hx.w[r(dst)], Value::bool(t)))
}

/// `C::NAME` the name's cache entry holds for this class (`ConstSite`):
/// a named class's by its reference, the called class's or a trait's
/// class's by its address; a private or protected one's for the scope it
/// was read from. A miss (the first read, another class or scope, a
/// constant that is not cached), no called class or scope, are cold.
#[inline(always)]
pub(crate) fn class_const<S: spec::ClassConst>(hx: Hx, args::ClassConst { dst, class, site }: args::ClassConst) -> Ctl {
    let Some(key) = hx.class_key::<S::Cls>(class) else { return Ctl::Cold };
    let Some(entry) = hx.clo.class_consts.get(site as usize) else { return Ctl::Cold };
    let v = {
        let e = entry.borrow();
        if e.key != key || (e.scope != 0 && e.scope != hx.scope()) {
            return Ctl::Cold;
        }
        e.value.clone()
    };
    Ctl::Next(put(&mut hx.w[r(dst)], v))
}

/// `C::$name` the name's cache entry holds the cell of for this class
/// (`StaticSite`, keyed as `class_const`'s) and this scope: its value, a
/// reference's read through it. A miss (the first read, another class or
/// scope, a class whose initializers have not run), no called class or
/// scope, an unset value (a typed property not initialized) are cold.
#[inline(always)]
pub(crate) fn fetch_static_prop<S: spec::FetchStaticProp>(hx: Hx, args::FetchStaticProp { dst, class, site }: args::FetchStaticProp) -> Ctl {
    let Some(key) = hx.class_key::<S::Cls>(class) else { return Ctl::Cold };
    let Some(entry) = hx.clo.statics.get(site as usize) else { return Ctl::Cold };
    let v = {
        let e = entry.borrow();
        let Some(cell) = &e.cell else { return Ctl::Cold };
        if e.key != key || e.scope != hx.scope() {
            return Ctl::Cold;
        }
        let v = cell.borrow().deref();
        if v.is_undef() {
            return Ctl::Cold;
        }
        v
    };
    Ctl::Next(put(&mut hx.w[r(dst)], v))
}

/// `C::$name = val`: the cell `fetch_static_prop`'s entry holds for this
/// class and scope gets the value (through a reference it holds, as
/// `store` writes it), and `dst` the value too. A miss, no called class or
/// scope, an undefined value (the warning) are cold.
#[inline(always)]
pub(crate) fn assign_static_prop<S: spec::AssignStaticProp>(hx: Hx, args::AssignStaticProp { val, dst, class, site }: args::AssignStaticProp) -> Ctl {
    let Some(key) = hx.class_key::<S::Cls>(class) else { return Ctl::Cold };
    let Some(entry) = hx.clo.statics.get(site as usize) else { return Ctl::Cold };
    let v = if is_k(S::Val::ID) {
        hx.v::<S::Val>(val).clone()
    } else {
        match copied(&hx.w[r(val)]) {
            Some(v) => v,
            None => return Ctl::Cold,
        }
    };
    let released = {
        let e = entry.borrow();
        let Some(cell) = &e.cell else { return Ctl::Cold };
        if e.key != key || e.scope != hx.scope() {
            return Ctl::Cold;
        }
        put_var(&mut cell.borrow_mut(), v.clone())
    };
    Ctl::Next(put(&mut hx.w[r(dst)], v) | released)
}

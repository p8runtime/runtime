//! The v2 value: scalars held inline, strings and arrays shared until
//! written, a PHP reference (`&`) only where a variable is one. Conversions
//! and comparisons follow zend_operators.c for PHP 8.4. Objects and
//! resources join with the class model (milestone M4).
//!
//! `Value` is a newtype over a private `Repr`: nothing outside this file
//! can name a variant, so the accessor API below is the only way in and
//! the compiler (not a grep) keeps it that way.

use crate::array::PhpArray;
use crate::object::Object;
use crate::string::{IStr, PhpStr, StrHead};
use crate::numeric::{Number, leading_numeric, numeric_string};
use crate::number::{Precision, dval_to_lval, float_to_string};
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::fmt;
use std::mem::{self, ManuallyDrop};
use std::rc::Rc;

/// A PHP value: 16 bytes, a tag and a payload, as a zval.
pub struct Value(Repr);

/// A handle inside the representation: the `Rc` (a string's, an array's,
/// a reference's, an object's) with no drop glue of its own, so that
/// `Repr` has none either and the one place a value is let go of is
/// [`Value`]'s `Drop`: a tag test inline, and only a value holding memory
/// makes the call that decrements and frees (PHP's `zval_ptr_dtor_nogc`:
/// `Z_REFCOUNTED`, then `i_zval_ptr_dtor`). Without this, every drop of a
/// value was the generic glue of a four-`Rc` enum, which the compiler
/// kept out of line (one call per value dropped, plain ones included:
/// 2.2M calls, 26M instructions, on one WordPress page).
///
/// Reading through it is free (`Deref`); consuming it is
/// `ManuallyDrop::into_inner`, which the release paths below do, and
/// nothing else may drop a heap `Repr` (it would leak): `Value` does.
type Handle<T> = ManuallyDrop<T>;

/// The representation. The values that hold nothing to release come
/// first (tags 0..=5) and the ones holding memory after, so "is there
/// anything to release" is one compare of the tag (PHP's
/// `Z_REFCOUNTED`): what `Drop` and `is_plain` test.
///
/// `repr(u64)` with explicit discriminants makes the tag a word with
/// exactly these values (the reference's primitive-representation
/// layout), so `is_plain` is `tag < 6` by definition, not by codegen.
/// A word, not a byte: the value is then two words with no padding, and
/// a copy of one is two moves. With a byte tag the seven bytes after it
/// are padding the compiler copies field by field (four more loads and
/// stores, through the stack, on every register copy).
#[derive(Debug)]
#[repr(u64)]
enum Repr {
    Null = 0,
    Bool(B) = 1,
    Int(i64) = 2,
    Float(f64) = 3,
    /// A variable never assigned (only in registers; reads warn and give null).
    Undef = 4,
    /// A string in program memory (a compiled literal): a plain pointer,
    /// no count to keep, nothing to release (PHP's interned strings,
    /// which `Z_REFCOUNTED` is false for).
    IStr(&'static IStr) = 5,
    Str(Handle<PhpStr>) = 6,
    Array(Handle<Rc<PhpArray>>) = 7,
    /// A PHP reference: the shared cell `$a = &$b` makes.
    Ref(Handle<Rc<RefCell<Value>>>) = 8,
    /// An object: a shared handle (copying the value copies the handle).
    Object(Handle<Rc<Object>>) = 9,
}

/// A bool's payload, at the payload's offset (8) as every other variant's
/// is: a copy of any value is then its tag and one word. (A plain `bool`
/// sits at offset 1, and copying a value had to move bytes 1..8 too, one
/// by one through the stack: most of what a register copy cost.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(align(8))]
struct B(bool);

impl B {
    const FALSE: B = B(false);
    const TRUE: B = B(true);

    #[inline(always)]
    const fn new(b: bool) -> B {
        B(b)
    }

    #[inline(always)]
    const fn get(self) -> bool {
        self.0
    }
}

const _: () = assert!(mem::size_of::<B>() == 8);

/// The first tag holding memory: tags below it are plain.
const FIRST_HEAP_TAG: u8 = 6;

// The layout the engine is tuned for: a zval's 16 bytes, and `Option<Value>`
// no bigger (the tag's spare values are its `None`).
const _: () = assert!(mem::size_of::<Value>() == 16);
const _: () = assert!(mem::size_of::<Option<Value>>() == 16);
// `Kind` mirrors the tags, so `kind()` is the tag itself except that both
// strings are `Kind::Str` (the program-memory one's tag, the last plain
// one): the heap tags 6..=9 keep their numbers but for Str's.
const _: () = {
    assert!(Kind::Null as u8 == 0);
    assert!(Kind::Bool as u8 == 1);
    assert!(Kind::Int as u8 == 2);
    assert!(Kind::Float as u8 == 3);
    assert!(Kind::Undef as u8 == 4);
    assert!(Kind::Str as u8 == FIRST_HEAP_TAG - 1);
    assert!(Kind::Array as u8 == FIRST_HEAP_TAG + 1);
    assert!(Kind::Ref as u8 == 8);
    assert!(Kind::Object as u8 == 9);
    assert!(mem::size_of::<Kind>() == 1);
};
// A program-memory string is a pointer: Copy, and shared by every thread.
const _: () = {
    const fn shared<T: Sync + 'static>() {}
    shared::<IStr>();
    assert!(mem::size_of::<&'static IStr>() == 8);
};

impl Default for Value {
    /// Null: what `mem::take` leaves.
    #[inline(always)]
    fn default() -> Value {
        Value::NULL
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Drop for Value {
    /// Inline, as PHP's `zval_ptr_dtor_nogc`: one compare of the tag, and
    /// nothing else for a plain value (tags 0..=5). A value holding memory
    /// makes one call, out of line, which decrements its count and frees
    /// at zero (`release_heap`: PHP's `i_zval_ptr_dtor`, `GC_DELREF` then
    /// `rc_dtor_func`). The handles have no drop glue of their own
    /// ([`Handle`]), so this is the whole drop: no generic glue jumping
    /// on the type follows it. The `Rc` count still goes the same way at
    /// the same point, so a destructor is queued where it was and the
    /// collector sees the same counts.
    ///
    /// The interpreter's register overwrite (`fast::put`) tests the tag in
    /// the slot itself and releases through [`Value::release`], whose
    /// decrement is inline (measured better there, #324's PR); this is
    /// every other drop of a value: a slot overwritten by `*slot = v`, a
    /// temporary, an array's or an object's values when it goes.
    ///
    /// Being a `Drop` type also keeps a `Value` from being destructured by
    /// move: the `into_*` take the representation with `mem::replace`.
    #[inline(always)]
    fn drop(&mut self) {
        if !self.is_plain() {
            release_heap(&mut self.0);
        }
    }
}

/// A value holding memory is let go of (its `Value` is being dropped):
/// the count's decrement, and the free when this was the last handle.
/// One out-of-line function for every kind of handle, as PHP's
/// `i_zval_ptr_dtor`; what it leaves behind is plain (Undef). The count
/// is read in place, the same way for every kind of handle, so the
/// common case (a shared handle) is the decrement and the tag's store,
/// with nothing moved; only the last handle's free takes the
/// representation out.
#[inline(never)]
fn release_heap(r: &mut Repr) {
    let count = match &*r {
        Repr::Str(s) => s.count(),
        Repr::Array(a) => Rc::strong_count(a),
        Repr::Ref(c) => Rc::strong_count(c),
        Repr::Object(o) => Rc::strong_count(o),
        _ => return,
    };
    if count == 1 {
        free_last(mem::replace(r, Repr::Undef));
    } else {
        drop_shared(mem::replace(r, Repr::Undef));
    }
}

/// Let go of a representation taken out of its value (see
/// [`Value::release`]): the count read first, the same way for every
/// kind of handle (the `Rc` count, first in its allocation); the free out
/// of line when it is 1, else the decrement alone.
#[inline(always)]
fn release_repr(r: Repr) -> bool {
    let count = match &r {
        Repr::Str(s) => s.count(),
        Repr::Array(a) => Rc::strong_count(a),
        Repr::Ref(c) => Rc::strong_count(c),
        Repr::Object(o) => Rc::strong_count(o),
        _ => return false,
    };
    if count == 1 {
        // A string's free cannot run a destructor.
        let owns = !matches!(r, Repr::Str(_));
        free_last(r);
        owns
    } else {
        drop_shared(r);
        false
    }
}

/// The last handle of a string, array, reference or object goes: its
/// free, out of line, by its kind (PHP's `rc_dtor_func`). The count is
/// known to be 1 here, so each `Rc` drop is the free. An array's values
/// go in one walk of its buffer (its `Drop`: PHP's `zend_array_destroy`),
/// each a tag test and, for one holding memory, `release_heap`; the
/// buffer then goes back to its pool. (An explicit walk releasing each
/// value with the decrement inline was measured worse on WordPress:
/// +26M instructions over this glue, #361's PR.) An object's drop
/// releases its properties and queues its destructor.
#[cold]
#[inline(never)]
fn free_last(r: Repr) {
    match r {
        Repr::Str(s) => drop(ManuallyDrop::into_inner(s)),
        Repr::Array(a) => drop(ManuallyDrop::into_inner(a)),
        Repr::Ref(c) => drop(ManuallyDrop::into_inner(c)),
        Repr::Object(o) => drop(ManuallyDrop::into_inner(o)),
        _ => {}
    }
}

/// A shared handle's drop: the count was just read and is known not to
/// reach zero, so the `Rc` drop is the decrement alone.
#[inline(always)]
fn drop_shared(r: Repr) {
    match r {
        Repr::Str(s) => drop(ManuallyDrop::into_inner(s)),
        Repr::Array(a) => drop(ManuallyDrop::into_inner(a)),
        Repr::Ref(c) => drop(ManuallyDrop::into_inner(c)),
        Repr::Object(o) => drop(ManuallyDrop::into_inner(o)),
        _ => {}
    }
}

impl Value {
    /// Let go of a value (PHP's `zval_ptr_dtor`): nothing for a plain one;
    /// for one holding memory its count's decrement, inline, and the free
    /// out of line only when this was the last handle (PHP's `GC_DELREF`,
    /// then `rc_dtor_func` at zero). Returns whether a handle that may own
    /// an object went (an array, a reference, an object): the free may
    /// have left a destructor due. A decrement that frees nothing, or a
    /// string's free, cannot have.
    ///
    /// The same as dropping the value, with the decrement inline (the
    /// interpreter's register writes are the hot case), and the answer.
    #[inline(always)]
    pub fn release(mut self) -> bool {
        // (What is left is plain: the drop of `self` that follows tests
        // its tag and does nothing.)
        release_repr(mem::replace(&mut self.0, Repr::Undef))
    }

    /// Whether `self` is `other` itself: the same scalar (a float by its
    /// bits) or the same handle (one string, array, reference or object,
    /// not equal contents). A register already holding a frame's constant
    /// this way needs no fresh copy of it.
    #[inline(always)]
    pub fn same_handle(&self, other: &Value) -> bool {
        match (&self.0, &other.0) {
            (Repr::Null, Repr::Null) | (Repr::Undef, Repr::Undef) => true,
            (Repr::Bool(a), Repr::Bool(b)) => a == b,
            (Repr::Int(a), Repr::Int(b)) => a == b,
            (Repr::Float(a), Repr::Float(b)) => a.to_bits() == b.to_bits(),
            (Repr::IStr(a), Repr::IStr(b)) => std::ptr::eq(*a, *b),
            (Repr::Str(a), Repr::Str(b)) => PhpStr::ptr_eq(a, b),
            (Repr::Array(a), Repr::Array(b)) => Rc::ptr_eq(a, b),
            (Repr::Ref(a), Repr::Ref(b)) => Rc::ptr_eq(a, b),
            (Repr::Object(a), Repr::Object(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Clone for Value {
    /// Inline: copying a value is on every register move.
    ///
    /// A jump table on the tag (#353's finding), still: the plain arms
    /// cannot fold into one copy while their payloads are of different
    /// types to the compiler (an int, a float, a bool, a pointer), as it
    /// keeps each arm's store and tag constant apart (measured in #361's
    /// PR: a tag test before the table, and a word of payload on every
    /// plain variant, folded nothing and cost WordPress +1.2%
    /// instructions). A copy with no dispatch needs every plain payload
    /// to be one word of the same type: the program-memory string by
    /// index rather than by reference, the float by its bits. Separate work.
    #[inline(always)]
    fn clone(&self) -> Self {
        Value(match &self.0 {
            Repr::Null => Repr::Null,
            Repr::Bool(b) => Repr::Bool(*b),
            Repr::Int(i) => Repr::Int(*i),
            Repr::Float(f) => Repr::Float(*f),
            Repr::IStr(s) => Repr::IStr(s),
            Repr::Str(s) => Repr::Str(s.clone()),
            Repr::Array(a) => Repr::Array(a.clone()),
            Repr::Ref(r) => Repr::Ref(r.clone()),
            Repr::Object(o) => Repr::Object(o.clone()),
            Repr::Undef => Repr::Undef,
        })
    }
}

/// The kind of a value: one per PHP type, and Undef. This is the tag code
/// outside this module dispatches on; it stays as it is when the
/// representation gains variants (a program-memory string is `Kind::Str`,
/// a program-memory array `Kind::Array`). In the representation's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    Null = 0,
    Bool = 1,
    Int = 2,
    Float = 3,
    Undef = 4,
    Str = 5,
    Array = 7,
    Ref = 8,
    Object = 9,
}

/// The accessor API: the only way code outside this module reads, writes
/// or builds a `Value` (the representation is private).
///
/// - Constructors: the consts and `bool`/`int`/`float`/`str`/`array`/
///   `object`/`reference`, and `From`.
/// - `kind()` for an exhaustive dispatch; `is_*` for one test.
/// - Borrowing views `as_*`: a scalar by value, a string as its
///   [`StrHead`], an array as the [`PhpArray`] behind its handle, an
///   object or a reference as its `Rc`. `as_int_mut` writes an int in
///   place; `array_mut` separates a shared array (`Rc::make_mut`) and
///   `str_mut`/`str_push` a shared string.
/// - `cloned_*`: an owned handle, the value kept.
/// - `into_*`: the handle out of a value being consumed (`mem::replace`
///   of the representation, which leaves Undef for `Drop`).
impl Value {
    pub const NULL: Value = Value(Repr::Null);
    pub const UNDEF: Value = Value(Repr::Undef);
    pub const TRUE: Value = Value(Repr::Bool(B::TRUE));
    pub const FALSE: Value = Value(Repr::Bool(B::FALSE));

    #[inline(always)]
    pub const fn bool(b: bool) -> Value {
        Value(Repr::Bool(B::new(b)))
    }

    #[inline(always)]
    pub const fn int(i: i64) -> Value {
        Value(Repr::Int(i))
    }

    #[inline(always)]
    pub const fn float(f: f64) -> Value {
        Value(Repr::Float(f))
    }

    pub fn str(s: impl Into<PhpStr>) -> Value {
        Value(Repr::Str(Handle::new(s.into())))
    }

    /// A program-memory string's value (a compiled literal): no count,
    /// nothing to release.
    #[inline(always)]
    pub const fn istr(s: &'static IStr) -> Value {
        Value(Repr::IStr(s))
    }

    /// The program-memory string this value holds, if it holds one.
    #[inline(always)]
    pub fn as_istr(&self) -> Option<&'static IStr> {
        match self.0 {
            Repr::IStr(s) => Some(s),
            _ => None,
        }
    }

    /// An array value: from a `PhpArray` (one allocation for the handle)
    /// or an `Rc<PhpArray>` already shared.
    #[inline(always)]
    pub fn array(a: impl Into<Rc<PhpArray>>) -> Value {
        Value(Repr::Array(Handle::new(a.into())))
    }

    #[inline(always)]
    pub fn object(o: impl Into<Rc<Object>>) -> Value {
        Value(Repr::Object(Handle::new(o.into())))
    }

    /// A PHP reference to the cell `$a = &$b` shares.
    #[inline(always)]
    pub fn reference(cell: Rc<RefCell<Value>>) -> Value {
        Value(Repr::Ref(Handle::new(cell)))
    }

    /// A PHP reference to a fresh cell holding `v`.
    pub fn new_reference(v: Value) -> Value {
        Value(Repr::Ref(Handle::new(Rc::new(RefCell::new(v)))))
    }

    #[inline(always)]
    pub fn kind(&self) -> Kind {
        match self.0 {
            Repr::Null => Kind::Null,
            Repr::Bool(_) => Kind::Bool,
            Repr::Int(_) => Kind::Int,
            Repr::Float(_) => Kind::Float,
            Repr::Undef => Kind::Undef,
            Repr::IStr(_) | Repr::Str(_) => Kind::Str,
            Repr::Array(_) => Kind::Array,
            Repr::Ref(_) => Kind::Ref,
            Repr::Object(_) => Kind::Object,
        }
    }

    #[inline(always)]
    pub fn is_null(&self) -> bool {
        matches!(self.0, Repr::Null)
    }

    #[inline(always)]
    pub fn is_undef(&self) -> bool {
        matches!(self.0, Repr::Undef)
    }

    /// Null or undefined: what reads as null (isset is false, the type
    /// name is "null").
    #[inline(always)]
    pub fn is_null_or_undef(&self) -> bool {
        matches!(self.0, Repr::Null | Repr::Undef)
    }

    #[inline(always)]
    pub fn is_bool(&self) -> bool {
        matches!(self.0, Repr::Bool(_))
    }

    #[inline(always)]
    pub fn is_int(&self) -> bool {
        matches!(self.0, Repr::Int(_))
    }

    #[inline(always)]
    pub fn is_float(&self) -> bool {
        matches!(self.0, Repr::Float(_))
    }

    /// An int or a float.
    #[inline(always)]
    pub fn is_number(&self) -> bool {
        matches!(self.0, Repr::Int(_) | Repr::Float(_))
    }

    #[inline(always)]
    pub fn is_str(&self) -> bool {
        matches!(self.0, Repr::IStr(_) | Repr::Str(_))
    }

    #[inline(always)]
    pub fn is_array(&self) -> bool {
        matches!(self.0, Repr::Array(_))
    }

    #[inline(always)]
    pub fn is_object(&self) -> bool {
        matches!(self.0, Repr::Object(_))
    }

    #[inline(always)]
    pub fn is_reference(&self) -> bool {
        matches!(self.0, Repr::Ref(_))
    }

    /// Whether the value holds nothing to release (null, a bool, an int,
    /// a float, undefined, a program-memory string): overwriting it is a
    /// store, copying it a copy. These are the representation's first
    /// tags, so this is one compare.
    #[inline(always)]
    pub fn is_plain(&self) -> bool {
        self.tag() < FIRST_HEAP_TAG
    }

    /// The representation's tag (its `repr(u8)` discriminant).
    #[inline(always)]
    fn tag(&self) -> u8 {
        match self.0 {
            Repr::Null => 0,
            Repr::Bool(_) => 1,
            Repr::Int(_) => 2,
            Repr::Float(_) => 3,
            Repr::Undef => 4,
            Repr::IStr(_) => 5,
            Repr::Str(_) => 6,
            Repr::Array(_) => 7,
            Repr::Ref(_) => 8,
            Repr::Object(_) => 9,
        }
    }

    #[inline(always)]
    pub fn as_bool(&self) -> Option<bool> {
        match self.0 {
            Repr::Bool(b) => Some(b.get()),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_int(&self) -> Option<i64> {
        match self.0 {
            Repr::Int(i) => Some(i),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_float(&self) -> Option<f64> {
        match self.0 {
            Repr::Float(f) => Some(f),
            _ => None,
        }
    }

    /// The int, for writing in place.
    #[inline(always)]
    pub fn as_int_mut(&mut self) -> Option<&mut i64> {
        match &mut self.0 {
            Repr::Int(i) => Some(i),
            _ => None,
        }
    }

    /// The float, for writing in place.
    #[inline(always)]
    pub fn as_float_mut(&mut self) -> Option<&mut f64> {
        match &mut self.0 {
            Repr::Float(f) => Some(f),
            _ => None,
        }
    }

    /// The string, read through its head (bytes, length, hash). To own
    /// it, `cloned_str`; to consume the value, `into_str`.
    #[inline(always)]
    pub fn as_str(&self) -> Option<&StrHead> {
        match &self.0 {
            Repr::IStr(s) => Some(s),
            Repr::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The array, for reading. To write, `array_mut`.
    #[inline(always)]
    pub fn as_array(&self) -> Option<&PhpArray> {
        match &self.0 {
            Repr::Array(a) => Some(a),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_object(&self) -> Option<&Rc<Object>> {
        match &self.0 {
            Repr::Object(o) => Some(o),
            _ => None,
        }
    }

    /// The cell a PHP reference shares.
    #[inline(always)]
    pub fn as_reference(&self) -> Option<&Rc<RefCell<Value>>> {
        match &self.0 {
            Repr::Ref(cell) => Some(cell),
            _ => None,
        }
    }

    /// The array, for writing: separated first if shared (`Rc::make_mut`,
    /// PHP's SEPARATE_ARRAY). A program-memory array is copied here too.
    #[inline(always)]
    pub fn array_mut(&mut self) -> Option<&mut PhpArray> {
        match &mut self.0 {
            Repr::Array(a) => Some(Rc::make_mut(a)),
            _ => None,
        }
    }

    /// The string's bytes, for writing: separated first if shared
    /// (`PhpStr::make_mut`).
    /// A program-memory string is copied out first (it is never written).
    pub fn str_mut(&mut self) -> Option<&mut Vec<u8>> {
        if let Repr::IStr(s) = self.0 {
            self.0 = Repr::Str(Handle::new(PhpStr::from(s.as_bytes())));
        }
        match &mut self.0 {
            Repr::Str(s) => Some(s.make_mut()),
            _ => None,
        }
    }

    /// Append to the string, as `.=` does (in place when it is the only
    /// holder). False, nothing done, when this is not a string.
    pub fn str_push(&mut self, bytes: &[u8]) -> bool {
        match &mut self.0 {
            Repr::Str(s) => {
                s.push_bytes(bytes);
                true
            }
            // (One new string of both, as a shared one's append.)
            Repr::IStr(s) => {
                self.0 = Repr::Str(Handle::new(PhpStr::concat(s.as_bytes(), bytes)));
                true
            }
            _ => false,
        }
    }

    /// The string as an owned handle. A program-memory string's is the
    /// thread's ordinary twin of it (a count, no copy); to read, `as_str`.
    #[inline(always)]
    pub fn cloned_str(&self) -> Option<PhpStr> {
        match &self.0 {
            Repr::Str(s) => Some(PhpStr::clone(s)),
            Repr::IStr(s) => Some(s.to_php_str()),
            _ => None,
        }
    }

    /// The array's handle (shared, not copied). Own one to write or to
    /// keep an array apart from the value; to read, `as_array`.
    #[inline(always)]
    pub fn cloned_array(&self) -> Option<Rc<PhpArray>> {
        match &self.0 {
            Repr::Array(a) => Some(Rc::clone(a)),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn cloned_object(&self) -> Option<Rc<Object>> {
        match &self.0 {
            Repr::Object(o) => Some(Rc::clone(o)),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn cloned_reference(&self) -> Option<Rc<RefCell<Value>>> {
        match &self.0 {
            Repr::Ref(cell) => Some(Rc::clone(cell)),
            _ => None,
        }
    }

    /// The string out of a value being consumed; the value back when it
    /// is not one.
    #[inline]
    pub fn into_str(mut self) -> Result<PhpStr, Value> {
        match mem::replace(&mut self.0, Repr::Undef) {
            Repr::Str(s) => Ok(ManuallyDrop::into_inner(s)),
            Repr::IStr(s) => Ok(s.to_php_str()),
            r => Err(Value(r)),
        }
    }

    #[inline]
    pub fn into_array(mut self) -> Result<Rc<PhpArray>, Value> {
        match mem::replace(&mut self.0, Repr::Undef) {
            Repr::Array(a) => Ok(ManuallyDrop::into_inner(a)),
            r => Err(Value(r)),
        }
    }

    #[inline]
    pub fn into_object(mut self) -> Result<Rc<Object>, Value> {
        match mem::replace(&mut self.0, Repr::Undef) {
            Repr::Object(o) => Ok(ManuallyDrop::into_inner(o)),
            r => Err(Value(r)),
        }
    }

    #[inline]
    pub fn into_reference(mut self) -> Result<Rc<RefCell<Value>>, Value> {
        match mem::replace(&mut self.0, Repr::Undef) {
            Repr::Ref(cell) => Ok(ManuallyDrop::into_inner(cell)),
            r => Err(Value(r)),
        }
    }
}

impl From<bool> for Value {
    #[inline(always)]
    fn from(b: bool) -> Value {
        Value(Repr::Bool(B::new(b)))
    }
}

impl From<i64> for Value {
    #[inline(always)]
    fn from(i: i64) -> Value {
        Value(Repr::Int(i))
    }
}

impl From<f64> for Value {
    #[inline(always)]
    fn from(f: f64) -> Value {
        Value(Repr::Float(f))
    }
}

impl From<PhpStr> for Value {
    #[inline(always)]
    fn from(s: PhpStr) -> Value {
        Value(Repr::Str(Handle::new(s)))
    }
}

impl From<Rc<PhpArray>> for Value {
    #[inline(always)]
    fn from(a: Rc<PhpArray>) -> Value {
        Value(Repr::Array(Handle::new(a)))
    }
}

impl From<PhpArray> for Value {
    #[inline(always)]
    fn from(a: PhpArray) -> Value {
        Value(Repr::Array(Handle::new(Rc::new(a))))
    }
}

impl From<Rc<Object>> for Value {
    #[inline(always)]
    fn from(o: Rc<Object>) -> Value {
        Value(Repr::Object(Handle::new(o)))
    }
}

impl From<Rc<RefCell<Value>>> for Value {
    #[inline(always)]
    fn from(cell: Rc<RefCell<Value>>) -> Value {
        Value(Repr::Ref(Handle::new(cell)))
    }
}

impl Value {
    /// The value itself, looking through a reference.
    #[inline]
    pub fn deref(&self) -> Value {
        match &self.0 {
            Repr::Ref(cell) => cell.borrow().clone(),
            _ => self.clone(),
        }
    }

    /// The type name PHP uses in messages (zend_zval_type_name): an
    /// object's is its class name.
    pub fn type_name(&self) -> Cow<'static, str> {
        Cow::Borrowed(match &self.0 {
            Repr::Null | Repr::Undef => "null",
            Repr::Bool(_) => "bool",
            Repr::Int(_) => "int",
            Repr::Float(_) => "float",
            Repr::IStr(_) | Repr::Str(_) => "string",
            Repr::Array(_) => "array",
            Repr::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => "resource",
            Repr::Object(o) => return Cow::Owned(String::from_utf8_lossy(o.class.name.as_bytes()).into_owned()),
            Repr::Ref(cell) => return cell.borrow().type_name(),
        })
    }

    /// zend_is_true
    pub fn to_bool(&self) -> bool {
        match &self.0 {
            Repr::Null | Repr::Undef => false,
            Repr::Bool(b) => b.get(),
            Repr::Int(i) => *i != 0,
            Repr::Float(f) => *f != 0.0,
            Repr::IStr(s) => !(s.is_empty() || s.as_bytes() == b"0"),
            Repr::Str(s) => !(s.is_empty() || s.as_bytes() == b"0"),
            Repr::Array(a) => !a.is_empty(),
            Repr::Object(o) => o.class.cast_bool.is_none_or(|cast| cast(o)),
            Repr::Ref(cell) => cell.borrow().to_bool(),
        }
    }

    /// Whether this is a NAN float (through a reference): what PHP 8.5+
    /// warns about when it is coerced to bool, string, array or object.
    #[inline]
    pub fn is_nan(&self) -> bool {
        match &self.0 {
            Repr::Float(f) => f.is_nan(),
            Repr::Ref(cell) => cell.borrow().is_nan(),
            _ => false,
        }
    }

    /// zval_get_long: floats wrap as zend_dval_to_lval does; numeric
    /// strings read their numeric prefix, saturating when out of range.
    pub fn to_int(&self) -> i64 {
        match &self.0 {
            Repr::Null | Repr::Undef => 0,
            Repr::Bool(b) => i64::from(b.get()),
            Repr::Int(i) => *i,
            Repr::Float(f) => dval_to_lval(*f),
            Repr::IStr(_) | Repr::Str(_) => match leading_numeric(self.as_str().expect("a string").as_bytes()) {
                Some((Number::Int(i), _)) => i,
                Some((Number::Float(f), _)) => dval_to_lval_cap(f),
                None => 0,
            },
            Repr::Array(a) => i64::from(!a.is_empty()),
            // A resource is its number.
            Repr::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => i64::from(o.id),
            Repr::Object(o) if o.class.cast_int.is_some() => o.class.cast_int.expect("checked")(o),
            Repr::Object(o) if o.class.cast_string.is_some() => Value::str(o.class.cast_string.expect("checked")(o)).to_int(),
            // PHP warns "could not be converted to int" (the engine reports it).
            Repr::Object(_) => 1,
            Repr::Ref(cell) => cell.borrow().to_int(),
        }
    }

    /// zval_get_double
    pub fn to_float(&self) -> f64 {
        match &self.0 {
            Repr::Null | Repr::Undef => 0.0,
            Repr::Bool(b) => f64::from(u8::from(b.get())),
            Repr::Int(i) => *i as f64,
            Repr::Float(f) => *f,
            Repr::IStr(_) | Repr::Str(_) => match leading_numeric(self.as_str().expect("a string").as_bytes()) {
                Some((Number::Int(i), _)) => i as f64,
                Some((Number::Float(f), _)) => f,
                None => 0.0,
            },
            Repr::Array(a) => f64::from(u8::from(!a.is_empty())),
            Repr::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => f64::from(o.id),
            Repr::Object(o) if o.class.cast_string.is_some() => Value::str(o.class.cast_string.expect("checked")(o)).to_float(),
            Repr::Object(_) => 1.0,
            Repr::Ref(cell) => cell.borrow().to_float(),
        }
    }

    /// The string a scalar converts to (zval_get_string); None for an array,
    /// which the engine reports ("Array to string conversion").
    pub fn to_php_string(&self) -> Option<PhpStr> {
        Some(match &self.0 {
            Repr::Null | Repr::Undef => PhpStr::empty(),
            Repr::Bool(b) => if b.get() { PhpStr::from("1") } else { PhpStr::empty() },
            Repr::Int(i) => PhpStr::from_int(*i),
            Repr::Float(f) => PhpStr::from(float_to_string(*f, Precision::ECHO, 'E')),
            Repr::Str(s) => PhpStr::clone(s),
            Repr::IStr(s) => s.to_php_str(),
            Repr::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => PhpStr::from(format!("Resource id #{}", o.id)),
            Repr::Object(o) if o.class.cast_string.is_some() => o.class.cast_string.expect("checked")(o),
            // An object's string comes from __toString, which the engine
            // calls; neither converts here.
            Repr::Array(_) | Repr::Object(_) => return None,
            Repr::Ref(cell) => return cell.borrow().to_php_string(),
        })
    }

    /// `to_php_string` to read: a string is lent as it is (no handle
    /// made, a program-memory one's included), anything else converted.
    #[inline]
    pub fn to_str_view(&self) -> Option<crate::string::StrView<'_>> {
        match self.as_str() {
            Some(s) => Some(crate::string::StrView::Borrowed(s)),
            None => self.to_php_string().map(crate::string::StrView::Owned),
        }
    }

    /// `===` (zend_is_identical).
    pub fn identical(&self, other: &Value) -> bool {
        match (&self.0, &other.0) {
            // One program-memory string per bytes: the same bytes are
            // the same pointer.
            (Repr::IStr(a), Repr::IStr(b)) => std::ptr::eq(*a, *b),
            (Repr::Str(a), Repr::Str(b)) => a == b,
            (Repr::Ref(a), _) => a.borrow().identical(other),
            (_, Repr::Ref(b)) => self.identical(&b.borrow()),
            (Repr::Null | Repr::Undef, Repr::Null | Repr::Undef) => true,
            (Repr::Bool(a), Repr::Bool(b)) => a == b,
            (Repr::Int(a), Repr::Int(b)) => a == b,
            (Repr::Float(a), Repr::Float(b)) => a == b,
            (Repr::IStr(_) | Repr::Str(_), Repr::IStr(_) | Repr::Str(_)) => self.as_str() == other.as_str(),
            (Repr::Array(a), Repr::Array(b)) => Rc::ptr_eq(a, b) || a.identical(b),
            (Repr::Object(a), Repr::Object(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// `==` (zend_compare == 0, with strings compared as zend_fast_equal_strings).
    pub fn loose_eq(&self, other: &Value) -> bool {
        match (self.as_str(), other.as_str()) {
            (Some(a), Some(b)) => {
                std::ptr::eq(a, b)
                    || match (numeric_string(a.as_bytes()), numeric_string(b.as_bytes())) {
                        (Some(x), Some(y)) => compare_numbers(x, y) == Ordering::Equal,
                        _ => a.as_bytes() == b.as_bytes(),
                    }
            }
            _ => self.compare(other) == Some(Ordering::Equal),
        }
    }

    /// `<=>` (zend_compare). None for arrays that cannot be compared (a key
    /// of the left missing on the right), which PHP treats as greater (1)
    /// and unequal.
    pub fn compare(&self, other: &Value) -> Option<Ordering> {
        use Repr::*;
        Some(match (&self.0, &other.0) {
            (Ref(a), _) => return a.borrow().compare(other),
            (_, Ref(b)) => return self.compare(&b.borrow()),
            // An object with a scalar cast against a scalar: compared as its string.
            (Object(o), x) if !matches!(x, Object(_) | Array(_) | Null | Undef | Bool(_)) && o.class.cast_string.is_some() => {
                return Value::str(o.class.cast_string.expect("checked")(o)).compare(other);
            }
            (x, Object(o)) if !matches!(x, Object(_) | Array(_) | Null | Undef | Bool(_)) && o.class.cast_string.is_some() => {
                return self.compare(&Value::str(o.class.cast_string.expect("checked")(o)));
            }
            (Undef, _) => return Value::NULL.compare(other),
            (_, Undef) => return self.compare(&Value::NULL),
            (Int(a), Int(b)) => a.cmp(b),
            (Int(a), Float(b)) => float_cmp(*a as f64, *b),
            (Float(a), Int(b)) => float_cmp(*a, *b as f64),
            (Float(a), Float(b)) => float_cmp(*a, *b),
            (Array(a), Array(b)) => return a.compare(b),
            // zend_std_compare_objects: the same object, or the same class
            // compared property by property; other classes can't be compared.
            (Object(a), Object(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Some(Ordering::Equal);
                }
                // Other classes, and enum cases, can't be compared.
                if !Rc::ptr_eq(&a.class, &b.class) || a.class.flags & crate::object::CLASS_ENUM != 0 {
                    return None;
                }
                // (Copies: a property may lead back to either object.)
                for (p, q) in a.slots().iter().zip(b.slots().iter()) {
                    use crate::object::Slot;
                    match p.cloned().compare(&q.cloned())? {
                        Ordering::Equal => {}
                        o => return Some(o),
                    }
                }
                return a.dynamic.borrow().compare(&b.dynamic.borrow());
            }
            (Null, Null) => Ordering::Equal,
            (Null, Bool(b)) => false.cmp(&b.get()),
            (Bool(a), Null) => a.get().cmp(&false),
            (Bool(a), Bool(b)) => a.get().cmp(&b.get()),
            (Null, IStr(_) | Str(_)) => if other.as_str().expect("a string").is_empty() { Ordering::Equal } else { Ordering::Less },
            (IStr(_) | Str(_), Null) => if self.as_str().expect("a string").is_empty() { Ordering::Equal } else { Ordering::Greater },
            (IStr(_) | Str(_), IStr(_) | Str(_)) => smart_strcmp(self.as_str().expect("a string"), other.as_str().expect("a string")),
            (Bool(a), _) => a.get().cmp(&other.to_bool()),
            (_, Bool(b)) => self.to_bool().cmp(&b.get()),
            (Null, _) => false.cmp(&other.to_bool()),
            (_, Null) => self.to_bool().cmp(&false),
            // NAN against a string is "greater" either way round (zend_compare)
            (Float(f), IStr(_) | Str(_)) | (IStr(_) | Str(_), Float(f)) if f.is_nan() => Ordering::Greater,
            (Int(_) | Float(_), IStr(_) | Str(_)) => compare_number_to_string(self, other.as_str().expect("a string")),
            (IStr(_) | Str(_), Int(_) | Float(_)) => compare_number_to_string(other, self.as_str().expect("a string")).reverse(),
            (Array(_), _) => Ordering::Greater,
            (_, Array(_)) => Ordering::Less,
            (Object(_), _) => Ordering::Greater,
            (_, Object(_)) => Ordering::Less,
        })
    }
}

/// zend_dval_to_lval_cap: a float from a numeric string saturates.
fn dval_to_lval_cap(d: f64) -> i64 {
    if !d.is_finite() {
        0
    } else if d >= i64::MAX as f64 {
        i64::MAX
    } else if d < i64::MIN as f64 {
        i64::MIN
    } else {
        d as i64
    }
}

/// ZEND_THREEWAY_COMPARE on doubles: NAN compares as "greater" both ways.
fn float_cmp(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Greater)
}

fn compare_numbers(a: Number, b: Number) -> Ordering {
    match (a, b) {
        (Number::Int(x), Number::Int(y)) => x.cmp(&y),
        (Number::Int(x), Number::Float(y)) => float_cmp(x as f64, y),
        (Number::Float(x), Number::Int(y)) => float_cmp(x, y as f64),
        (Number::Float(x), Number::Float(y)) => float_cmp(x, y),
    }
}

/// zendi_smart_strcmp: two numeric strings compare as numbers, anything
/// else byte by byte (then by length).
fn smart_strcmp(a: &StrHead, b: &StrHead) -> Ordering {
    match (numeric_string(a.as_bytes()), numeric_string(b.as_bytes())) {
        (Some(x), Some(y)) => compare_numbers(x, y),
        _ => a.as_bytes().cmp(b.as_bytes()),
    }
}

/// compare_longs_to_string / compare_doubles_to_string (PHP 8): a
/// numeric string compares as a number; otherwise the number is compared
/// as a string.
fn compare_number_to_string(number: &Value, s: &StrHead) -> Ordering {
    let n = match number.0 {
        Repr::Int(i) => Number::Int(i),
        Repr::Float(f) => Number::Float(f),
        _ => unreachable!("only numbers are compared to strings here"),
    };
    match numeric_string(s.as_bytes()) {
        Some(m) => compare_numbers(n, m),
        None => {
            let text = number.to_php_string().unwrap_or_else(PhpStr::empty);
            text.as_bytes().cmp(s.as_bytes())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_memory_string_is_a_plain_string() {
        let lit = Value::istr(IStr::intern(b"10 program-memory apples").expect("room"));
        let same = Value::str("10 program-memory apples");
        // Nothing to release, yet a string to everything that asks.
        assert!(lit.is_plain() && !same.is_plain());
        assert_eq!((lit.kind(), lit.is_str(), lit.type_name()), (Kind::Str, true, Cow::Borrowed("string")));
        assert!(lit.identical(&same) && same.identical(&lit) && lit.loose_eq(&same));
        assert_eq!(lit.compare(&same), Some(Ordering::Equal));
        assert_eq!((lit.to_int(), lit.to_float(), lit.to_bool()), (10, 10.0, true));
        // (Not numeric: 9 compares as "9".)
        assert_eq!((lit.compare(&Value::int(9)), Value::int(9).compare(&lit)), (Some(Ordering::Less), Some(Ordering::Greater)));
        assert!(!lit.identical(&Value::istr(IStr::intern(b"other").expect("room"))));
        // A copy is the same pointer; owned handles are the thread's twin.
        let copy = lit.clone();
        assert!(std::ptr::eq(copy.as_str().expect("a string"), lit.as_str().expect("a string")));
        assert!(PhpStr::ptr_eq(&lit.cloned_str().expect("a string"), &copy.into_str().expect("a string")));
        // Writing one makes an ordinary string; program memory is unchanged.
        let mut w = lit.clone();
        assert!(w.str_push(b"!"));
        assert_eq!((w.as_str().expect("a string").as_bytes(), w.is_plain()), (&b"10 program-memory apples!"[..], false));
        let mut m = lit.clone();
        m.str_mut().expect("a string").clear();
        assert!(m.as_str().expect("a string").is_empty());
        assert_eq!(lit.as_str().expect("a string").as_bytes(), b"10 program-memory apples");
    }
}

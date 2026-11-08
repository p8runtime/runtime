//! The v2 value: scalars held inline, strings and arrays shared until
//! written, a PHP reference (`&`) only where a variable is one. Conversions
//! and comparisons follow zend_operators.c for PHP 8.4. Objects and
//! resources join with the class model (milestone M4).
//!
//! `Value` is a newtype over a private `Repr`: nothing outside this file
//! can name a variant, so the accessor API below is the only way in and
//! the compiler (not a grep) keeps it that way.
//!
//! #391 (prototype): every plain payload is one word of one type (`i64`:
//! an int, a float's bits, a bool, a program-memory string's id), so
//! copying a plain value is a copy of two words whatever its kind, and
//! the value is a scalar pair (two registers) to the compiler.

use crate::array::PhpArray;
use crate::object::Object;
use crate::string::{IStr, PhpStr, StrHead};
use crate::numeric::{Number, leading_numeric, numeric_string};
use crate::number::{Precision, dval_to_lval, float_to_string};
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::fmt;
use crate::heap::{HRc, Heap};
use std::mem::{self, ManuallyDrop};
use std::rc::Rc;

/// A PHP value: 16 bytes, a tag and a payload, as a zval.
pub struct Value(Repr);

/// A handle inside the representation: the `Rc` with no drop glue of its
/// own, so that the one place a value is let go of is [`Value`]'s `Drop`
/// (one tag test inline; PHP's `zval_ptr_dtor_nogc`).
type Handle<T> = ManuallyDrop<T>;

/// The representation: a tag word and one payload word in every variant,
/// the plain ones first (tags 0..=5), each holding an `i64` (an int, a
/// float's bits, a bool, a program-memory string's id), the ones holding
/// memory after. One word in every variant at the same offset is what
/// makes the value a scalar pair to rustc: passed and returned in two
/// registers, never through a stack slot (#391's layout experiment: an
/// enum whose variants hold two fields, or a niche-encoded one with a
/// pointer variant, is an aggregate, returned through memory).
#[derive(Debug)]
#[repr(u64)]
enum Repr {
    Null(i64) = 0,
    Bool(i64) = 1,
    Int(i64) = 2,
    Float(i64) = 3,
    /// A variable never assigned (only in registers; reads warn and give null).
    Undef(i64) = 4,
    /// A string in program memory (a compiled literal), by its id
    /// as a plain pointer: no count, nothing to release. (By id through a
    /// table, measured: +37 instructions per iteration on `identical`.
    /// A pointer is one word too, and rustc's scalar pair unifies it
    /// with the others' `i64`: the copy still folds.)
    IStr(&'static IStr) = 5,
    /// Every value holding memory (a string, an array, a reference, an
    /// object): ONE pointer type, its kind a byte behind it ([`Heap`]).
    Heap(Handle<Rc<Heap>>) = 6,
}

/// The typed handles the engine names.
pub type ArrayRc = HRc<PhpArray>;
pub type RefRc = HRc<RefCell<Value>>;
pub type ObjectRc = HRc<Object>;

// The layout the engine is tuned for: a zval's 16 bytes, and `Option<Value>`
// no bigger (the tag's spare values are its `None`).
const _: () = assert!(mem::size_of::<Value>() == 16);
const _: () = assert!(mem::size_of::<Option<Value>>() == 16);
const _: () = {
    assert!(Kind::Null as u8 == 0);
    assert!(Kind::Bool as u8 == 1);
    assert!(Kind::Int as u8 == 2);
    assert!(Kind::Float as u8 == 3);
    assert!(Kind::Undef as u8 == 4);
    assert!(Kind::Str as u8 == 5);
    assert!(Kind::Array as u8 == 7);
    assert!(Kind::Ref as u8 == 8);
    assert!(Kind::Object as u8 == 9);
    assert!(mem::size_of::<Kind>() == 1);
};

/// A value seen as its kind and payload, for the code that dispatches on
/// both (conversions, comparisons): the representation stays private to
/// the few functions that copy, release and build.
enum V<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Undef,
    IStr(&'static IStr),
    Str(&'a StrHead),
    Array(&'a PhpArray),
    Ref(&'a RefCell<Value>),
    Object(&'a Object),
}

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
    /// nothing else for a plain value. A value holding memory makes one
    /// call, out of line (`release_heap`).
    #[inline(always)]
    fn drop(&mut self) {
        if !self.is_plain() {
            release_heap(&mut self.0);
        }
    }
}

const UNDEF_REPR: Repr = Repr::Undef(0);

/// A value holding memory is let go of: the count's decrement, and the
/// free when this was the last handle (PHP's `i_zval_ptr_dtor`).
#[inline(never)]
fn release_heap(r: &mut Repr) {
    let Repr::Heap(h) = &*r else { return };
    if Rc::strong_count(h) == 1 {
        free_last(mem::replace(r, UNDEF_REPR));
    } else {
        drop_shared(mem::replace(r, UNDEF_REPR));
    }
}

/// Let go of a representation taken out of its value (see
/// [`Value::release`]): one count for every kind of heap value.
#[inline(always)]
fn release_repr(r: Repr) -> bool {
    let Repr::Heap(h) = &r else { return false };
    if Rc::strong_count(h) == 1 {
        // A string's free cannot run a destructor.
        let owns = !matches!(***h, Heap::Str(_));
        free_last(r);
        owns
    } else {
        drop_shared(r);
        false
    }
}

/// The last handle goes: its free, out of line (the `Heap`'s drop is the
/// dispatch on its kind: PHP's `rc_dtor_func`).
#[cold]
#[inline(never)]
fn free_last(r: Repr) {
    if let Repr::Heap(h) = r {
        drop(ManuallyDrop::into_inner(h));
    }
}

/// A shared handle's drop: the decrement alone.
#[inline(always)]
fn drop_shared(r: Repr) {
    if let Repr::Heap(h) = r {
        drop(ManuallyDrop::into_inner(h));
    }
}

impl Value {
    /// Let go of a value (PHP's `zval_ptr_dtor`). Returns whether a handle
    /// that may own an object went (an array, a reference, an object).
    #[inline(always)]
    pub fn release(mut self) -> bool {
        release_repr(mem::replace(&mut self.0, UNDEF_REPR))
    }

    /// Whether `self` is `other` itself: the same scalar (a float by its
    /// bits) or the same handle.
    #[inline(always)]
    pub fn same_handle(&self, other: &Value) -> bool {
        match (&self.0, &other.0) {
            (Repr::Heap(a), Repr::Heap(b)) => Rc::ptr_eq(a, b),
            _ => match (self.plain_word(), other.plain_word()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }

    /// A plain value's tag and payload word.
    #[inline(always)]
    fn plain_word(&self) -> Option<(Kind, i64)> {
        match self.0 {
            Repr::Null(w) => Some((Kind::Null, w)),
            Repr::Bool(w) => Some((Kind::Bool, w)),
            Repr::Int(w) => Some((Kind::Int, w)),
            Repr::Float(w) => Some((Kind::Float, w)),
            Repr::Undef(w) => Some((Kind::Undef, w)),
            Repr::IStr(s) => Some((Kind::Str, s.id() as i64)),
            _ => None,
        }
    }

    #[inline(always)]
    fn view(&self) -> V<'_> {
        match &self.0 {
            Repr::Null(_) => V::Null,
            Repr::Bool(w) => V::Bool(*w != 0),
            Repr::Int(w) => V::Int(*w),
            Repr::Float(w) => V::Float(f64::from_bits(*w as u64)),
            Repr::Undef(_) => V::Undef,
            Repr::IStr(s) => V::IStr(s),
            Repr::Heap(h) => match &***h {
                Heap::Str(s) => V::Str(s),
                Heap::Array(a) => V::Array(a),
                Heap::Ref(c) => V::Ref(c),
                Heap::Object(o) => V::Object(o),
            },
        }
    }

    /// How many values share this one's heap value (0 for a plain value).
    #[inline(always)]
    pub fn heap_count(&self) -> usize {
        match &self.0 {
            Repr::Heap(h) => Rc::strong_count(h),
            _ => 0,
        }
    }

    /// The heap value behind the pointer, if this holds one.
    #[inline(always)]
    fn heap(&self) -> Option<&Heap> {
        match &self.0 {
            Repr::Heap(h) => Some(h),
            _ => None,
        }
    }
}

impl Clone for Value {
    /// Inline: copying a value is on every register move. One compare of
    /// the tag; a plain value is then a copy of both words (the plain arms
    /// fold: every payload is an `i64`), a heap value one increment (one
    /// pointer type: no dispatch on its kind).
    #[inline(always)]
    fn clone(&self) -> Self {
        Value(match &self.0 {
            Repr::Null(w) => Repr::Null(*w),
            Repr::Bool(w) => Repr::Bool(*w),
            Repr::Int(w) => Repr::Int(*w),
            Repr::Float(w) => Repr::Float(*w),
            Repr::Undef(w) => Repr::Undef(*w),
            Repr::IStr(s) => Repr::IStr(s),
            Repr::Heap(h) => Repr::Heap(h.clone()),
        })
    }
}

/// The kind of a value: one per PHP type, and Undef. This is the tag code
/// outside this module dispatches on.
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
impl Value {
    pub const NULL: Value = Value(Repr::Null(0));
    pub const UNDEF: Value = Value(UNDEF_REPR);
    pub const TRUE: Value = Value(Repr::Bool(1));
    pub const FALSE: Value = Value(Repr::Bool(0));

    #[inline(always)]
    pub const fn bool(b: bool) -> Value {
        Value(Repr::Bool(b as i64))
    }

    #[inline(always)]
    pub const fn int(i: i64) -> Value {
        Value(Repr::Int(i))
    }

    #[inline(always)]
    pub const fn float(f: f64) -> Value {
        Value(Repr::Float(f.to_bits() as i64))
    }

    pub fn str(s: impl Into<PhpStr>) -> Value {
        Value::of_heap(s.into().into_heap())
    }

    /// A program-memory string's value (a compiled literal): a pointer, no
    /// count, nothing to release.
    #[inline(always)]
    pub fn istr(s: &'static IStr) -> Value {
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

    #[inline(always)]
    fn of_heap(h: Rc<Heap>) -> Value {
        Value(Repr::Heap(Handle::new(h)))
    }

    #[inline(always)]
    pub fn array(a: impl Into<ArrayRc>) -> Value {
        Value::of_heap(HRc::into_heap(a.into()))
    }

    #[inline(always)]
    pub fn object(o: impl Into<ObjectRc>) -> Value {
        Value::of_heap(HRc::into_heap(o.into()))
    }

    /// A PHP reference to the cell `$a = &$b` shares.
    #[inline(always)]
    pub fn reference(cell: RefRc) -> Value {
        Value::of_heap(HRc::into_heap(cell))
    }

    /// A PHP reference to a fresh cell holding `v`.
    pub fn new_reference(v: Value) -> Value {
        Value::reference(HRc::new(RefCell::new(v)))
    }

    #[inline(always)]
    pub fn kind(&self) -> Kind {
        match self.0 {
            Repr::Null(_) => Kind::Null,
            Repr::Bool(_) => Kind::Bool,
            Repr::Int(_) => Kind::Int,
            Repr::Float(_) => Kind::Float,
            Repr::Undef(_) => Kind::Undef,
            Repr::IStr(_) => Kind::Str,
            // (The kind behind the pointer: a byte next to the count.)
            Repr::Heap(ref h) => match ***h {
                Heap::Str(_) => Kind::Str,
                Heap::Array(_) => Kind::Array,
                Heap::Ref(_) => Kind::Ref,
                Heap::Object(_) => Kind::Object,
            },
        }
    }

    #[inline(always)]
    pub fn is_null(&self) -> bool {
        matches!(self.0, Repr::Null(_))
    }

    #[inline(always)]
    pub fn is_undef(&self) -> bool {
        matches!(self.0, Repr::Undef(_))
    }

    /// Null or undefined: what reads as null.
    #[inline(always)]
    pub fn is_null_or_undef(&self) -> bool {
        matches!(self.0, Repr::Null(_) | Repr::Undef(_))
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
        matches!(self.0, Repr::IStr(_)) || matches!(self.heap(), Some(Heap::Str(_)))
    }

    #[inline(always)]
    pub fn is_array(&self) -> bool {
        matches!(self.heap(), Some(Heap::Array(_)))
    }

    #[inline(always)]
    pub fn is_object(&self) -> bool {
        matches!(self.heap(), Some(Heap::Object(_)))
    }

    #[inline(always)]
    pub fn is_reference(&self) -> bool {
        matches!(self.heap(), Some(Heap::Ref(_)))
    }

    /// Whether the value holds nothing to release: one compare of the tag.
    #[inline(always)]
    pub fn is_plain(&self) -> bool {
        matches!(self.0, Repr::Null(_) | Repr::Bool(_) | Repr::Int(_) | Repr::Float(_) | Repr::Undef(_) | Repr::IStr(_))
    }

    #[inline(always)]
    pub fn as_bool(&self) -> Option<bool> {
        match self.0 {
            Repr::Bool(w) => Some(w != 0),
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
            Repr::Float(w) => Some(f64::from_bits(w as u64)),
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

    /// The string, read through its head (bytes, length, hash).
    #[inline(always)]
    pub fn as_str(&self) -> Option<&StrHead> {
        match &self.0 {
            Repr::IStr(s) => Some(s),
            Repr::Heap(h) => match &***h {
                Heap::Str(s) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    /// The array, for reading. To write, `array_mut`.
    #[inline(always)]
    pub fn as_array(&self) -> Option<&PhpArray> {
        match self.heap() {
            Some(Heap::Array(a)) => Some(a),
            _ => None,
        }
    }

    /// The object, for reading (#391: a borrow of the body; an owned
    /// handle is `cloned_object`).
    #[inline(always)]
    pub fn as_object(&self) -> Option<&Object> {
        match self.heap() {
            Some(Heap::Object(o)) => Some(o),
            _ => None,
        }
    }

    /// The cell a PHP reference shares.
    #[inline(always)]
    pub fn as_reference(&self) -> Option<&RefCell<Value>> {
        match self.heap() {
            Some(Heap::Ref(c)) => Some(c),
            _ => None,
        }
    }

    /// The heap handle, for writing in place, if it holds a `T`.
    #[inline(always)]
    fn heap_mut<T: crate::heap::HeapKind>(&mut self) -> Option<&mut Rc<Heap>> {
        match &mut self.0 {
            Repr::Heap(h) if T::get(h).is_some() => Some(h),
            _ => None,
        }
    }

    /// The array, for writing: separated first if shared (`Rc::make_mut`,
    /// PHP's SEPARATE_ARRAY).
    #[inline(always)]
    pub fn array_mut(&mut self) -> Option<&mut PhpArray> {
        let h = self.heap_mut::<PhpArray>()?;
        if Rc::get_mut(h).is_none() {
            separate_array(h);
        }
        match Rc::get_mut(h) {
            Some(Heap::Array(a)) => Some(a),
            _ => None,
        }
    }

    /// The string's bytes, for writing: separated first if shared.
    /// A program-memory string is copied out first (it is never written).
    pub fn str_mut(&mut self) -> Option<&mut Vec<u8>> {
        if let Some(s) = self.as_istr() {
            *self = Value::str(PhpStr::from(s.as_bytes()));
        }
        self.heap_mut::<StrHead>().map(|h| crate::string::writable_heap(h, 0))
    }

    /// Append to the string, as `.=` does (in place when it is the only
    /// holder). False, nothing done, when this is not a string.
    pub fn str_push(&mut self, bytes: &[u8]) -> bool {
        if let Some(s) = self.as_istr() {
            // (One new string of both, as a shared one's append.)
            *self = Value::str(PhpStr::concat(s.as_bytes(), bytes));
            return true;
        }
        match self.heap_mut::<StrHead>() {
            Some(h) => {
                crate::string::writable_heap(h, bytes.len()).extend_from_slice(bytes);
                true
            }
            None => false,
        }
    }

    /// The heap handle (shared), if it holds a `T`.
    #[inline(always)]
    fn cloned_heap<T: crate::heap::HeapKind>(&self) -> Option<HRc<T>> {
        match &self.0 {
            Repr::Heap(h) => HRc::from_heap(Rc::clone(h)).ok(),
            _ => None,
        }
    }

    /// The string as an owned handle.
    #[inline(always)]
    pub fn cloned_str(&self) -> Option<PhpStr> {
        match &self.0 {
            Repr::IStr(s) => Some(s.to_php_str()),
            Repr::Heap(h) if matches!(***h, Heap::Str(_)) => PhpStr::from_heap(Rc::clone(h)).ok(),
            _ => None,
        }
    }

    /// The array's handle (shared, not copied).
    #[inline(always)]
    pub fn cloned_array(&self) -> Option<ArrayRc> {
        self.cloned_heap()
    }

    #[inline(always)]
    pub fn cloned_object(&self) -> Option<ObjectRc> {
        self.cloned_heap()
    }

    #[inline(always)]
    pub fn cloned_reference(&self) -> Option<RefRc> {
        self.cloned_heap()
    }

    /// The heap handle out of a value being consumed, if it holds a `T`.
    #[inline]
    fn into_heap<T: crate::heap::HeapKind>(mut self) -> Result<HRc<T>, Value> {
        match mem::replace(&mut self.0, UNDEF_REPR) {
            Repr::Heap(h) => HRc::from_heap(ManuallyDrop::into_inner(h)).map_err(Value::of_heap),
            r => Err(Value(r)),
        }
    }

    /// The string out of a value being consumed; the value back when it
    /// is not one.
    #[inline]
    pub fn into_str(mut self) -> Result<PhpStr, Value> {
        match mem::replace(&mut self.0, UNDEF_REPR) {
            Repr::IStr(s) => Ok(s.to_php_str()),
            Repr::Heap(h) => PhpStr::from_heap(ManuallyDrop::into_inner(h)).map_err(Value::of_heap),
            r => Err(Value(r)),
        }
    }

    #[inline]
    pub fn into_array(self) -> Result<ArrayRc, Value> {
        self.into_heap()
    }

    #[inline]
    pub fn into_object(self) -> Result<ObjectRc, Value> {
        self.into_heap()
    }

    #[inline]
    pub fn into_reference(self) -> Result<RefRc, Value> {
        self.into_heap()
    }
}

impl From<bool> for Value {
    #[inline(always)]
    fn from(b: bool) -> Value {
        Value::bool(b)
    }
}

impl From<i64> for Value {
    #[inline(always)]
    fn from(i: i64) -> Value {
        Value::int(i)
    }
}

impl From<f64> for Value {
    #[inline(always)]
    fn from(f: f64) -> Value {
        Value::float(f)
    }
}

impl From<PhpStr> for Value {
    #[inline(always)]
    fn from(s: PhpStr) -> Value {
        Value::str(s)
    }
}

impl From<ArrayRc> for Value {
    #[inline(always)]
    fn from(a: ArrayRc) -> Value {
        Value::array(a)
    }
}

impl From<PhpArray> for Value {
    #[inline(always)]
    fn from(a: PhpArray) -> Value {
        Value::array(HRc::new(a))
    }
}

impl From<ObjectRc> for Value {
    #[inline(always)]
    fn from(o: ObjectRc) -> Value {
        Value::object(o)
    }
}

impl From<RefRc> for Value {
    #[inline(always)]
    fn from(cell: RefRc) -> Value {
        Value::reference(cell)
    }
}

/// A shared array's separation (PHP's SEPARATE_ARRAY): a copy of it, in a
/// heap value of its own. Out of line: the write in place is the hot case.
#[inline(never)]
fn separate_array(h: &mut Rc<Heap>) {
    let Heap::Array(a) = &**h else { return };
    *h = Rc::new(Heap::Array(a.clone()));
}

impl Value {
    /// The value itself, looking through a reference.
    #[inline]
    pub fn deref(&self) -> Value {
        match self.as_reference() {
            Some(cell) => cell.borrow().clone(),
            None => self.clone(),
        }
    }

    /// The type name PHP uses in messages (zend_zval_type_name): an
    /// object's is its class name.
    pub fn type_name(&self) -> Cow<'static, str> {
        Cow::Borrowed(match self.view() {
            V::Null | V::Undef => "null",
            V::Bool(_) => "bool",
            V::Int(_) => "int",
            V::Float(_) => "float",
            V::IStr(_) | V::Str(_) => "string",
            V::Array(_) => "array",
            V::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => "resource",
            V::Object(o) => return Cow::Owned(String::from_utf8_lossy(o.class.name.as_bytes()).into_owned()),
            V::Ref(cell) => return cell.borrow().type_name(),
        })
    }

    /// zend_is_true
    pub fn to_bool(&self) -> bool {
        match self.view() {
            V::Null | V::Undef => false,
            V::Bool(b) => b,
            V::Int(i) => i != 0,
            V::Float(f) => f != 0.0,
            V::IStr(s) => !(s.is_empty() || s.as_bytes() == b"0"),
            V::Str(s) => !(s.is_empty() || s.as_bytes() == b"0"),
            V::Array(a) => !a.is_empty(),
            V::Object(o) => o.class.cast_bool.is_none_or(|cast| cast(o)),
            V::Ref(cell) => cell.borrow().to_bool(),
        }
    }

    /// Whether this is a NAN float (through a reference).
    #[inline]
    pub fn is_nan(&self) -> bool {
        match self.view() {
            V::Float(f) => f.is_nan(),
            V::Ref(cell) => cell.borrow().is_nan(),
            _ => false,
        }
    }

    /// zval_get_long: floats wrap as zend_dval_to_lval does; numeric
    /// strings read their numeric prefix, saturating when out of range.
    pub fn to_int(&self) -> i64 {
        match self.view() {
            V::Null | V::Undef => 0,
            V::Bool(b) => i64::from(b),
            V::Int(i) => i,
            V::Float(f) => dval_to_lval(f),
            V::IStr(_) | V::Str(_) => match leading_numeric(self.as_str().expect("a string").as_bytes()) {
                Some((Number::Int(i), _)) => i,
                Some((Number::Float(f), _)) => dval_to_lval_cap(f),
                None => 0,
            },
            V::Array(a) => i64::from(!a.is_empty()),
            // A resource is its number.
            V::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => i64::from(o.id),
            V::Object(o) if o.class.cast_int.is_some() => o.class.cast_int.expect("checked")(o),
            V::Object(o) if o.class.cast_string.is_some() => Value::str(o.class.cast_string.expect("checked")(o)).to_int(),
            // PHP warns "could not be converted to int" (the engine reports it).
            V::Object(_) => 1,
            V::Ref(cell) => cell.borrow().to_int(),
        }
    }

    /// zval_get_double
    pub fn to_float(&self) -> f64 {
        match self.view() {
            V::Null | V::Undef => 0.0,
            V::Bool(b) => f64::from(u8::from(b)),
            V::Int(i) => i as f64,
            V::Float(f) => f,
            V::IStr(_) | V::Str(_) => match leading_numeric(self.as_str().expect("a string").as_bytes()) {
                Some((Number::Int(i), _)) => i as f64,
                Some((Number::Float(f), _)) => f,
                None => 0.0,
            },
            V::Array(a) => f64::from(u8::from(!a.is_empty())),
            V::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => f64::from(o.id),
            V::Object(o) if o.class.cast_string.is_some() => Value::str(o.class.cast_string.expect("checked")(o)).to_float(),
            V::Object(_) => 1.0,
            V::Ref(cell) => cell.borrow().to_float(),
        }
    }

    /// The string a scalar converts to (zval_get_string); None for an array,
    /// which the engine reports ("Array to string conversion").
    pub fn to_php_string(&self) -> Option<PhpStr> {
        Some(match self.view() {
            V::Null | V::Undef => PhpStr::empty(),
            V::Bool(b) => if b { PhpStr::from("1") } else { PhpStr::empty() },
            V::Int(i) => PhpStr::from_int(i),
            V::Float(f) => PhpStr::from(float_to_string(f, Precision::ECHO, 'E')),
            V::Str(_) => self.cloned_str().expect("a string"),
            V::IStr(s) => s.to_php_str(),
            V::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => PhpStr::from(format!("Resource id #{}", o.id)),
            V::Object(o) if o.class.cast_string.is_some() => o.class.cast_string.expect("checked")(o),
            // An object's string comes from __toString, which the engine
            // calls; neither converts here.
            V::Array(_) | V::Object(_) => return None,
            V::Ref(cell) => return cell.borrow().to_php_string(),
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
        match (self.view(), other.view()) {
            // One program-memory string per bytes: the same bytes are
            // the same pointer.
            (V::IStr(a), V::IStr(b)) => std::ptr::eq(a, b),
            (V::Str(a), V::Str(b)) => a == b,
            (V::Ref(a), _) => a.borrow().identical(other),
            (_, V::Ref(b)) => self.identical(&b.borrow()),
            (V::Null | V::Undef, V::Null | V::Undef) => true,
            (V::Bool(a), V::Bool(b)) => a == b,
            (V::Int(a), V::Int(b)) => a == b,
            (V::Float(a), V::Float(b)) => a == b,
            (V::IStr(_) | V::Str(_), V::IStr(_) | V::Str(_)) => self.as_str() == other.as_str(),
            (V::Array(a), V::Array(b)) => std::ptr::eq(a, b) || a.identical(b),
            (V::Object(a), V::Object(b)) => std::ptr::eq(a, b),
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
        use V::*;
        Some(match (self.view(), other.view()) {
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
            (Int(a), Int(b)) => a.cmp(&b),
            (Int(a), Float(b)) => float_cmp(a as f64, b),
            (Float(a), Int(b)) => float_cmp(a, b as f64),
            (Float(a), Float(b)) => float_cmp(a, b),
            (Array(a), Array(b)) => return a.compare(b),
            // zend_std_compare_objects: the same object, or the same class
            // compared property by property; other classes can't be compared.
            (Object(a), Object(b)) => {
                if std::ptr::eq(a, b) {
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
            (Null, Bool(b)) => false.cmp(&b),
            (Bool(a), Null) => a.cmp(&false),
            (Bool(a), Bool(b)) => a.cmp(&b),
            (Null, IStr(_) | Str(_)) => if other.as_str().expect("a string").is_empty() { Ordering::Equal } else { Ordering::Less },
            (IStr(_) | Str(_), Null) => if self.as_str().expect("a string").is_empty() { Ordering::Equal } else { Ordering::Greater },
            (IStr(_) | Str(_), IStr(_) | Str(_)) => smart_strcmp(self.as_str().expect("a string"), other.as_str().expect("a string")),
            (Bool(a), _) => a.cmp(&other.to_bool()),
            (_, Bool(b)) => self.to_bool().cmp(&b),
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
    let n = match number.view() {
        V::Int(i) => Number::Int(i),
        V::Float(f) => Number::Float(f),
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

    #[test]
    fn plain_payloads_round_trip_through_one_word() {
        for f in [0.0, -0.0, 1.5, f64::NAN, f64::INFINITY, f64::MIN_POSITIVE] {
            assert_eq!(Value::float(f).as_float().map(f64::to_bits), Some(f.to_bits()));
        }
        assert_eq!((Value::TRUE.as_bool(), Value::FALSE.as_bool(), Value::int(-7).as_int()), (Some(true), Some(false), Some(-7)));
        // (Same payload word, other kinds: never the same value.)
        assert!(!Value::int(0).same_handle(&Value::NULL) && !Value::int(1).identical(&Value::TRUE));
        assert!(Value::NULL.is_plain() && Value::UNDEF.is_undef() && !Value::NULL.is_undef());
    }
}

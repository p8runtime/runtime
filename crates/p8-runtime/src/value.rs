//! The v2 value: scalars held inline, strings and arrays shared until
//! written, a PHP reference (`&`) only where a variable is one. Conversions
//! and comparisons follow zend_operators.c for PHP 8.4. Objects and
//! resources join with the class model (milestone M4).

use crate::array::PhpArray;
use crate::object::Object;
use crate::string::{PhpStr, StrHead};
use crate::numeric::{Number, leading_numeric, numeric_string};
use crate::number::{Precision, dval_to_lval, float_to_string};
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

#[derive(Debug, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(PhpStr),
    Array(Rc<PhpArray>),
    /// A PHP reference: the shared cell `$a = &$b` makes.
    Ref(Rc<RefCell<Value>>),
    /// An object: a shared handle (copying the value copies the handle).
    Object(Rc<Object>),
    /// A variable never assigned (only in registers; reads warn and give null).
    Undef,
}

impl Clone for Value {
    /// Inline: copying a value is on every register move.
    #[inline(always)]
    fn clone(&self) -> Self {
        match self {
            Value::Null => Value::Null,
            Value::Bool(b) => Value::Bool(*b),
            Value::Int(i) => Value::Int(*i),
            Value::Float(f) => Value::Float(*f),
            Value::Str(s) => Value::Str(s.clone()),
            Value::Array(a) => Value::Array(Rc::clone(a)),
            Value::Ref(r) => Value::Ref(Rc::clone(r)),
            Value::Object(o) => Value::Object(Rc::clone(o)),
            Value::Undef => Value::Undef,
        }
    }
}

/// The kind of a value: one per PHP type, and Undef. This is the tag code
/// outside this module dispatches on; it stays as it is when the
/// representation gains variants (a program-memory string is `Kind::Str`,
/// a program-memory array `Kind::Array`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Null,
    Bool,
    Int,
    Float,
    Str,
    Array,
    Ref,
    Object,
    Undef,
}

/// The accessor API: the only way code outside this module reads, writes
/// or builds a `Value`. Nothing here names a variant to its callers, so
/// the representation can change underneath (`IStr`/`IArr` variants, a
/// `Drop` impl, a newtype over a private enum) without touching them.
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
/// - `into_*`: the handle out of a value being consumed. These never
///   destructure a `Value` by move, so they stay legal under a `Drop`
///   impl (there: `mem::replace` on the inner enum, then `mem::forget`).
impl Value {
    pub const NULL: Value = Value::Null;
    pub const UNDEF: Value = Value::Undef;
    pub const TRUE: Value = Value::Bool(true);
    pub const FALSE: Value = Value::Bool(false);

    #[inline(always)]
    pub const fn bool(b: bool) -> Value {
        Value::Bool(b)
    }

    #[inline(always)]
    pub const fn int(i: i64) -> Value {
        Value::Int(i)
    }

    #[inline(always)]
    pub const fn float(f: f64) -> Value {
        Value::Float(f)
    }

    pub fn str(s: impl Into<PhpStr>) -> Value {
        Value::Str(s.into())
    }

    /// An array value: from a `PhpArray` (one allocation for the handle)
    /// or an `Rc<PhpArray>` already shared.
    #[inline(always)]
    pub fn array(a: impl Into<Rc<PhpArray>>) -> Value {
        Value::Array(a.into())
    }

    #[inline(always)]
    pub fn object(o: impl Into<Rc<Object>>) -> Value {
        Value::Object(o.into())
    }

    /// A PHP reference to the cell `$a = &$b` shares.
    #[inline(always)]
    pub fn reference(cell: Rc<RefCell<Value>>) -> Value {
        Value::Ref(cell)
    }

    /// A PHP reference to a fresh cell holding `v`.
    pub fn new_reference(v: Value) -> Value {
        Value::Ref(Rc::new(RefCell::new(v)))
    }

    #[inline(always)]
    pub fn kind(&self) -> Kind {
        match self {
            Value::Null => Kind::Null,
            Value::Bool(_) => Kind::Bool,
            Value::Int(_) => Kind::Int,
            Value::Float(_) => Kind::Float,
            Value::Str(_) => Kind::Str,
            Value::Array(_) => Kind::Array,
            Value::Ref(_) => Kind::Ref,
            Value::Object(_) => Kind::Object,
            Value::Undef => Kind::Undef,
        }
    }

    #[inline(always)]
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    #[inline(always)]
    pub fn is_undef(&self) -> bool {
        matches!(self, Value::Undef)
    }

    /// Null or undefined: what reads as null (isset is false, the type
    /// name is "null").
    #[inline(always)]
    pub fn is_null_or_undef(&self) -> bool {
        matches!(self, Value::Null | Value::Undef)
    }

    #[inline(always)]
    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    #[inline(always)]
    pub fn is_int(&self) -> bool {
        matches!(self, Value::Int(_))
    }

    #[inline(always)]
    pub fn is_float(&self) -> bool {
        matches!(self, Value::Float(_))
    }

    /// An int or a float.
    #[inline(always)]
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Float(_))
    }

    #[inline(always)]
    pub fn is_str(&self) -> bool {
        matches!(self, Value::Str(_))
    }

    #[inline(always)]
    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    #[inline(always)]
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    #[inline(always)]
    pub fn is_reference(&self) -> bool {
        matches!(self, Value::Ref(_))
    }

    /// Whether the value holds nothing to release (null, a bool, an int,
    /// a float, undefined): overwriting it is a store. The scalars are
    /// listed, so any value holding memory is not plain.
    #[inline(always)]
    pub fn is_plain(&self) -> bool {
        matches!(self, Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Undef)
    }

    #[inline(always)]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// The int, for writing in place.
    #[inline(always)]
    pub fn as_int_mut(&mut self) -> Option<&mut i64> {
        match self {
            Value::Int(i) => Some(i),
            _ => None,
        }
    }

    /// The float, for writing in place.
    #[inline(always)]
    pub fn as_float_mut(&mut self) -> Option<&mut f64> {
        match self {
            Value::Float(f) => Some(f),
            _ => None,
        }
    }

    /// The string, read through its head (bytes, length, hash). To own
    /// it, `cloned_str`; to consume the value, `into_str`.
    #[inline(always)]
    pub fn as_str(&self) -> Option<&StrHead> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The array, for reading. To write, `array_mut`.
    #[inline(always)]
    pub fn as_array(&self) -> Option<&PhpArray> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_object(&self) -> Option<&Rc<Object>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// The cell a PHP reference shares.
    #[inline(always)]
    pub fn as_reference(&self) -> Option<&Rc<RefCell<Value>>> {
        match self {
            Value::Ref(cell) => Some(cell),
            _ => None,
        }
    }

    /// The array, for writing: separated first if shared (`Rc::make_mut`,
    /// PHP's SEPARATE_ARRAY). A program-memory array is copied here too.
    #[inline(always)]
    pub fn array_mut(&mut self) -> Option<&mut PhpArray> {
        match self {
            Value::Array(a) => Some(Rc::make_mut(a)),
            _ => None,
        }
    }

    /// The string's bytes, for writing: separated first if shared
    /// (`PhpStr::make_mut`).
    pub fn str_mut(&mut self) -> Option<&mut Vec<u8>> {
        match self {
            Value::Str(s) => Some(s.make_mut()),
            _ => None,
        }
    }

    /// Append to the string, as `.=` does (in place when it is the only
    /// holder). False, nothing done, when this is not a string.
    pub fn str_push(&mut self, bytes: &[u8]) -> bool {
        match self {
            Value::Str(s) => {
                s.push_bytes(bytes);
                true
            }
            _ => false,
        }
    }

    #[inline(always)]
    pub fn cloned_str(&self) -> Option<PhpStr> {
        match self {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// The array's handle (shared, not copied). Own one to write or to
    /// keep an array apart from the value; to read, `as_array`.
    #[inline(always)]
    pub fn cloned_array(&self) -> Option<Rc<PhpArray>> {
        match self {
            Value::Array(a) => Some(Rc::clone(a)),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn cloned_object(&self) -> Option<Rc<Object>> {
        match self {
            Value::Object(o) => Some(Rc::clone(o)),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn cloned_reference(&self) -> Option<Rc<RefCell<Value>>> {
        match self {
            Value::Ref(cell) => Some(Rc::clone(cell)),
            _ => None,
        }
    }

    /// The string out of a value being consumed; the value back when it
    /// is not one.
    #[inline]
    pub fn into_str(self) -> Result<PhpStr, Value> {
        match self {
            Value::Str(s) => Ok(s),
            v => Err(v),
        }
    }

    #[inline]
    pub fn into_array(self) -> Result<Rc<PhpArray>, Value> {
        match self {
            Value::Array(a) => Ok(a),
            v => Err(v),
        }
    }

    #[inline]
    pub fn into_object(self) -> Result<Rc<Object>, Value> {
        match self {
            Value::Object(o) => Ok(o),
            v => Err(v),
        }
    }

    #[inline]
    pub fn into_reference(self) -> Result<Rc<RefCell<Value>>, Value> {
        match self {
            Value::Ref(cell) => Ok(cell),
            v => Err(v),
        }
    }
}

impl From<bool> for Value {
    #[inline(always)]
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    #[inline(always)]
    fn from(i: i64) -> Value {
        Value::Int(i)
    }
}

impl From<f64> for Value {
    #[inline(always)]
    fn from(f: f64) -> Value {
        Value::Float(f)
    }
}

impl From<PhpStr> for Value {
    #[inline(always)]
    fn from(s: PhpStr) -> Value {
        Value::Str(s)
    }
}

impl From<Rc<PhpArray>> for Value {
    #[inline(always)]
    fn from(a: Rc<PhpArray>) -> Value {
        Value::Array(a)
    }
}

impl From<PhpArray> for Value {
    #[inline(always)]
    fn from(a: PhpArray) -> Value {
        Value::Array(Rc::new(a))
    }
}

impl From<Rc<Object>> for Value {
    #[inline(always)]
    fn from(o: Rc<Object>) -> Value {
        Value::Object(o)
    }
}

impl From<Rc<RefCell<Value>>> for Value {
    #[inline(always)]
    fn from(cell: Rc<RefCell<Value>>) -> Value {
        Value::Ref(cell)
    }
}

impl Value {
    /// The value itself, looking through a reference.
    #[inline]
    pub fn deref(&self) -> Value {
        match self {
            Value::Ref(cell) => cell.borrow().clone(),
            other => other.clone(),
        }
    }

    /// The type name PHP uses in messages (zend_zval_type_name): an
    /// object's is its class name.
    pub fn type_name(&self) -> Cow<'static, str> {
        Cow::Borrowed(match self {
            Value::Null | Value::Undef => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => "resource",
            Value::Object(o) => return Cow::Owned(String::from_utf8_lossy(o.class.name.as_bytes()).into_owned()),
            Value::Ref(cell) => return cell.borrow().type_name(),
        })
    }

    /// zend_is_true
    pub fn to_bool(&self) -> bool {
        match self {
            Value::Null | Value::Undef => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !(s.is_empty() || s.as_bytes() == b"0"),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => o.class.cast_bool.is_none_or(|cast| cast(o)),
            Value::Ref(cell) => cell.borrow().to_bool(),
        }
    }

    /// Whether this is a NAN float (through a reference): what PHP 8.5+
    /// warns about when it is coerced to bool, string, array or object.
    #[inline]
    pub fn is_nan(&self) -> bool {
        match self {
            Value::Float(f) => f.is_nan(),
            Value::Ref(cell) => cell.borrow().is_nan(),
            _ => false,
        }
    }

    /// zval_get_long: floats wrap as zend_dval_to_lval does; numeric
    /// strings read their numeric prefix, saturating when out of range.
    pub fn to_int(&self) -> i64 {
        match self {
            Value::Null | Value::Undef => 0,
            Value::Bool(b) => i64::from(*b),
            Value::Int(i) => *i,
            Value::Float(f) => dval_to_lval(*f),
            Value::Str(s) => match leading_numeric(s.as_bytes()) {
                Some((Number::Int(i), _)) => i,
                Some((Number::Float(f), _)) => dval_to_lval_cap(f),
                None => 0,
            },
            Value::Array(a) => i64::from(!a.is_empty()),
            // A resource is its number.
            Value::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => i64::from(o.id),
            Value::Object(o) if o.class.cast_int.is_some() => o.class.cast_int.expect("checked")(o),
            Value::Object(o) if o.class.cast_string.is_some() => Value::Str(o.class.cast_string.expect("checked")(o)).to_int(),
            // PHP warns "could not be converted to int" (the engine reports it).
            Value::Object(_) => 1,
            Value::Ref(cell) => cell.borrow().to_int(),
        }
    }

    /// zval_get_double
    pub fn to_float(&self) -> f64 {
        match self {
            Value::Null | Value::Undef => 0.0,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Int(i) => *i as f64,
            Value::Float(f) => *f,
            Value::Str(s) => match leading_numeric(s.as_bytes()) {
                Some((Number::Int(i), _)) => i as f64,
                Some((Number::Float(f), _)) => f,
                None => 0.0,
            },
            Value::Array(a) => f64::from(u8::from(!a.is_empty())),
            Value::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => f64::from(o.id),
            Value::Object(o) if o.class.cast_string.is_some() => Value::Str(o.class.cast_string.expect("checked")(o)).to_float(),
            Value::Object(_) => 1.0,
            Value::Ref(cell) => cell.borrow().to_float(),
        }
    }

    /// The string a scalar converts to (zval_get_string); None for an array,
    /// which the engine reports ("Array to string conversion").
    pub fn to_php_string(&self) -> Option<PhpStr> {
        Some(match self {
            Value::Null | Value::Undef | Value::Bool(false) => PhpStr::empty(),
            Value::Bool(true) => PhpStr::from("1"),
            Value::Int(i) => PhpStr::from_int(*i),
            Value::Float(f) => PhpStr::from(float_to_string(*f, Precision::ECHO, 'E')),
            Value::Str(s) => s.clone(),
            Value::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => PhpStr::from(format!("Resource id #{}", o.id)),
            Value::Object(o) if o.class.cast_string.is_some() => o.class.cast_string.expect("checked")(o),
            // An object's string comes from __toString, which the engine
            // calls; neither converts here.
            Value::Array(_) | Value::Object(_) => return None,
            Value::Ref(cell) => return cell.borrow().to_php_string(),
        })
    }

    /// `===` (zend_is_identical).
    pub fn identical(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Ref(a), _) => a.borrow().identical(other),
            (_, Value::Ref(b)) => self.identical(&b.borrow()),
            (Value::Null | Value::Undef, Value::Null | Value::Undef) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b) || a.identical(b),
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// `==` (zend_compare == 0, with strings compared as zend_fast_equal_strings).
    pub fn loose_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => {
                PhpStr::ptr_eq(a, b)
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
        use Value::*;
        Some(match (self, other) {
            (Ref(a), _) => return a.borrow().compare(other),
            (_, Ref(b)) => return self.compare(&b.borrow()),
            // An object with a scalar cast against a scalar: compared as its string.
            (Object(o), x) if !matches!(x, Object(_) | Array(_) | Null | Undef | Bool(_)) && o.class.cast_string.is_some() => {
                return Str(o.class.cast_string.expect("checked")(o)).compare(other);
            }
            (x, Object(o)) if !matches!(x, Object(_) | Array(_) | Null | Undef | Bool(_)) && o.class.cast_string.is_some() => {
                return self.compare(&Str(o.class.cast_string.expect("checked")(o)));
            }
            (Undef, _) => return Null.compare(other),
            (_, Undef) => return self.compare(&Null),
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
                let (x, y) = (a.slots.borrow(), b.slots.borrow());
                for (p, q) in x.iter().zip(y.iter()) {
                    match p.compare(q)? {
                        Ordering::Equal => {}
                        o => return Some(o),
                    }
                }
                return a.dynamic.borrow().compare(&b.dynamic.borrow());
            }
            (Null, Null) => Ordering::Equal,
            (Null, Bool(b)) | (Bool(false), Bool(b)) => false.cmp(b),
            (Bool(a), Null) => a.cmp(&false),
            (Bool(true), Bool(b)) => true.cmp(b),
            (Null, Str(s)) => if s.is_empty() { Ordering::Equal } else { Ordering::Less },
            (Str(s), Null) => if s.is_empty() { Ordering::Equal } else { Ordering::Greater },
            (Str(a), Str(b)) => smart_strcmp(a, b),
            (Bool(a), x) => a.cmp(&x.to_bool()),
            (x, Bool(b)) => x.to_bool().cmp(b),
            (Null, x) => false.cmp(&x.to_bool()),
            (x, Null) => x.to_bool().cmp(&false),
            // NAN against a string is "greater" either way round (zend_compare)
            (Float(f), Str(_)) | (Str(_), Float(f)) if f.is_nan() => Ordering::Greater,
            (Int(_) | Float(_), Str(s)) => compare_number_to_string(self, s),
            (Str(s), Int(_) | Float(_)) => compare_number_to_string(other, s).reverse(),
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
fn smart_strcmp(a: &PhpStr, b: &PhpStr) -> Ordering {
    match (numeric_string(a.as_bytes()), numeric_string(b.as_bytes())) {
        (Some(x), Some(y)) => compare_numbers(x, y),
        _ => a.as_bytes().cmp(b.as_bytes()),
    }
}

/// compare_longs_to_string / compare_doubles_to_string (PHP 8): a
/// numeric string compares as a number; otherwise the number is compared
/// as a string.
fn compare_number_to_string(number: &Value, s: &PhpStr) -> Ordering {
    let n = match number {
        Value::Int(i) => Number::Int(*i),
        Value::Float(f) => Number::Float(*f),
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

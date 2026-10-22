//! The v2 value: scalars held inline, strings and arrays shared until
//! written, a PHP reference (`&`) only where a variable is one. Conversions
//! and comparisons follow zend_operators.c for PHP 8.4. Objects and
//! resources join with the class model (milestone M4).

use super::array::PhpArray;
use super::string::PhpStr;
use crate::core::numeric::{Number, leading_numeric, numeric_string};
use crate::core::value::{FloatPrecision, dval_to_lval, php_float_to_string};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(PhpStr),
    Array(Rc<PhpArray>),
    /// A PHP reference: the shared cell `$a = &$b` makes.
    Ref(Rc<RefCell<Value>>),
}

impl Value {
    pub fn str(s: impl Into<PhpStr>) -> Value {
        Value::Str(s.into())
    }

    /// The value itself, looking through a reference.
    pub fn deref(&self) -> Value {
        match self {
            Value::Ref(cell) => cell.borrow().clone(),
            other => other.clone(),
        }
    }

    /// The type name PHP uses in messages (zend_zval_type_name).
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Ref(cell) => cell.borrow().type_name(),
        }
    }

    /// zend_is_true
    pub fn to_bool(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !(s.is_empty() || s.as_bytes() == b"0"),
            Value::Array(a) => !a.is_empty(),
            Value::Ref(cell) => cell.borrow().to_bool(),
        }
    }

    /// zval_get_long: floats wrap as zend_dval_to_lval does; numeric
    /// strings read their numeric prefix, saturating when out of range.
    pub fn to_int(&self) -> i64 {
        match self {
            Value::Null => 0,
            Value::Bool(b) => i64::from(*b),
            Value::Int(i) => *i,
            Value::Float(f) => dval_to_lval(*f),
            Value::Str(s) => match leading_numeric(s.as_bytes()) {
                Some((Number::Int(i), _)) => i,
                Some((Number::Float(f), _)) => dval_to_lval_cap(f),
                None => 0,
            },
            Value::Array(a) => i64::from(!a.is_empty()),
            Value::Ref(cell) => cell.borrow().to_int(),
        }
    }

    /// zval_get_double
    pub fn to_float(&self) -> f64 {
        match self {
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Int(i) => *i as f64,
            Value::Float(f) => *f,
            Value::Str(s) => match leading_numeric(s.as_bytes()) {
                Some((Number::Int(i), _)) => i as f64,
                Some((Number::Float(f), _)) => f,
                None => 0.0,
            },
            Value::Array(a) => f64::from(u8::from(!a.is_empty())),
            Value::Ref(cell) => cell.borrow().to_float(),
        }
    }

    /// The string a scalar converts to (zval_get_string); None for an array,
    /// which the engine reports ("Array to string conversion").
    pub fn to_php_string(&self) -> Option<PhpStr> {
        Some(match self {
            Value::Null | Value::Bool(false) => PhpStr::empty(),
            Value::Bool(true) => PhpStr::from("1"),
            Value::Int(i) => PhpStr::from(i.to_string()),
            Value::Float(f) => PhpStr::from(php_float_to_string(*f, FloatPrecision::echo(), 'E')),
            Value::Str(s) => s.clone(),
            Value::Array(_) => return None,
            Value::Ref(cell) => return cell.borrow().to_php_string(),
        })
    }

    /// `===` (zend_is_identical).
    pub fn identical(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Ref(a), _) => a.borrow().identical(other),
            (_, Value::Ref(b)) => self.identical(&b.borrow()),
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b) || a.identical(b),
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
            (Int(a), Int(b)) => a.cmp(b),
            (Int(a), Float(b)) => float_cmp(*a as f64, *b),
            (Float(a), Int(b)) => float_cmp(*a, *b as f64),
            (Float(a), Float(b)) => float_cmp(*a, *b),
            (Array(a), Array(b)) => return a.compare(b),
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
        })
    }
}

/// zend_dval_to_lval_cap: a float from a numeric string saturates.
fn dval_to_lval_cap(d: f64) -> i64 {
    if !d.is_finite() {
        0
    } else if d >= 9.223_372_036_854_775_807e18 {
        i64::MAX
    } else if d < -9.223_372_036_854_775_808e18 {
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

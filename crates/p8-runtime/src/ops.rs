//! PHP 8.4's operators on v2 values (zend_operators.c): arithmetic with
//! int overflow to float, the numeric-string rules, string concatenation,
//! bitwise operators (bytewise on two strings), shifts, ++ and --.
//!
//! Operators report warnings and deprecations into `Diags`, which the
//! interpreter shows at the current line; errors are returned as `OpError`
//! for it to throw.

use crate::array::PhpArray;
use crate::string::PhpStr;
use crate::value::Value;
use crate::numeric::{Number, increment_alphanumeric, leading_numeric, numeric_string, push_int};
use crate::number::{Precision, dval_to_lval, float_to_string};
use std::rc::Rc;

/// A diagnostic an operator raised (shown with the current location).
#[derive(Debug, Clone, PartialEq)]
pub enum Diag {
    Warning(String),
    Deprecated(String),
}

pub type Diags = Vec<Diag>;

/// What zend_dval_to_lval reports for `f` (8.5+): NAN, the infinities and
/// floats outside the int range are not representable (a warning; the int
/// is then 0 or the value modulo 2^64, as `dval_to_lval` gives).
pub fn float_to_int_cast_diag(f: f64) -> Option<Diag> {
    let fits = (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f);
    (!fits).then(|| {
        Diag::Warning(format!(
            "The float {} is not representable as an int, cast occurred",
            float_to_string(f, Precision::Shortest, 'E')
        ))
    })
}

/// What zend_dval_to_lval_safe reports for `f` where PHP needs an int
/// (array keys, string offsets, %, bitwise and shift operands): the
/// warning above for a float that isn't representable, then, for one
/// ZEND_DOUBLE_FITS_LONG lets through (a fraction, or NAN, which compares
/// false both ways) the precision deprecation.
pub fn float_to_int_diags(f: f64, diags: &mut Diags) {
    diags.extend(float_to_int_cast_diag(f));
    // NAN fits too (ZEND_DOUBLE_FITS_LONG compares false both ways), which
    // Range::contains would not let through.
    #[allow(clippy::manual_range_contains)] // NAN must pass, see above
    let fits_long = !(f >= 9.223_372_036_854_776e18 || f < -9.223_372_036_854_776e18);
    if fits_long && dval_to_lval(f) as f64 != f {
        diags.push(Diag::Deprecated(format!(
            "Implicit conversion from float {} to int loses precision",
            float_to_string(f, Precision::Shortest, 'E')
        )));
    }
}

/// Whether `float_to_int_diags` reports anything for `f`.
pub fn float_is_int(f: f64) -> bool {
    f.is_finite() && (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f) && f.fract() == 0.0
}

/// zend_nan_coerced_to_type_warning (8.5+): NAN used as a bool, a string,
/// an array or an object (`type` as zend_get_type_by_const names it).
pub fn nan_coerced(f: f64, type_name: &str, diags: &mut Diags) {
    if f.is_nan() {
        diags.push(Diag::Warning(format!("unexpected NAN value was coerced to {type_name}")));
    }
}

/// An error an operator throws.
#[derive(Debug, Clone, PartialEq)]
pub enum OpError {
    /// TypeError
    Type(String),
    /// DivisionByZeroError
    DivisionByZero(&'static str),
    /// ArithmeticError
    Arithmetic(&'static str),
    /// Error
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Concat => ".",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }
}

/// `a op b`.
pub fn binary(op: BinOp, a: &Value, b: &Value, diags: &mut Diags) -> Result<Value, OpError> {
    if let Value::Ref(cell) = a {
        return binary(op, &cell.borrow(), b, diags);
    }
    if let Value::Ref(cell) = b {
        return binary(op, a, &cell.borrow(), diags);
    }
    // An object with a scalar cast (SimpleXML) takes part as its string.
    if let Value::Object(x) = a
        && let Some(cast) = x.class.cast_string
    {
        return binary(op, &Value::Str(cast(x)), b, diags);
    }
    if let Value::Object(y) = b
        && let Some(cast) = y.class.cast_string
    {
        return binary(op, a, &Value::Str(cast(y)), diags);
    }
    match op {
        BinOp::Concat => {
            if let Some(e) = object_string_error(a).or_else(|| object_string_error(b)) {
                return Err(e);
            }
            Ok(concat(a, b, diags))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => arithmetic(op, a, b, diags),
        BinOp::Mod => {
            let (x, y) = (int_operand(op, a, b, a, diags)?, int_operand(op, a, b, b, diags)?);
            match y {
                0 => Err(OpError::DivisionByZero("Modulo by zero")),
                -1 => Ok(Value::Int(0)),
                _ => Ok(Value::Int(x % y)),
            }
        }
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
            if let (Value::Str(x), Value::Str(y)) = (a, b) {
                return Ok(Value::Str(bytewise(op, x.as_bytes(), y.as_bytes())));
            }
            let (x, y) = (int_operand(op, a, b, a, diags)?, int_operand(op, a, b, b, diags)?);
            Ok(Value::Int(match op {
                BinOp::BitAnd => x & y,
                BinOp::BitOr => x | y,
                _ => x ^ y,
            }))
        }
        BinOp::Shl | BinOp::Shr => {
            let (x, y) = (int_operand(op, a, b, a, diags)?, int_operand(op, a, b, b, diags)?);
            if y < 0 {
                return Err(OpError::Arithmetic("Bit shift by negative number"));
            }
            Ok(Value::Int(match (op, y >= 64) {
                (BinOp::Shl, true) => 0,
                (BinOp::Shl, false) => x.wrapping_shl(y as u32),
                (_, true) => if x < 0 { -1 } else { 0 },
                (_, false) => x >> y,
            }))
        }
    }
}

fn unsupported(op: BinOp, a: &Value, b: &Value) -> OpError {
    OpError::Type(format!("Unsupported operand types: {} {} {}", a.type_name(), op.symbol(), b.type_name()))
}

/// The number an arithmetic operand stands for (zendi_try_convert_scalar_to_number).
fn number_operand(op: BinOp, a: &Value, b: &Value, v: &Value, diags: &mut Diags) -> Result<Number, OpError> {
    Ok(match v {
        Value::Int(i) => Number::Int(*i),
        Value::Float(f) => Number::Float(*f),
        Value::Null | Value::Undef => Number::Int(0),
        Value::Bool(x) => Number::Int(i64::from(*x)),
        Value::Str(s) => match numeric_string(s.as_bytes()) {
            Some(n) => n,
            None => match leading_numeric(s.as_bytes()) {
                Some((n, _)) => {
                    diags.push(Diag::Warning("A non-numeric value encountered".into()));
                    n
                }
                None => return Err(unsupported(op, a, b)),
            },
        },
        Value::Array(_) | Value::Ref(_) | Value::Object(_) => return Err(unsupported(op, a, b)),
    })
}

/// The int an operand of %, bitwise or shift operators stands for; a
/// fractional float (or float string) is deprecated.
fn int_operand(op: BinOp, a: &Value, b: &Value, v: &Value, diags: &mut Diags) -> Result<i64, OpError> {
    let n = number_operand(op, a, b, v, diags)?;
    Ok(match n {
        Number::Int(i) => i,
        // zendi_try_get_long: a float string saturates and loses precision
        // (deprecated) when it isn't an int; a float goes through
        // zend_dval_to_lval_safe.
        Number::Float(f) => match v {
            Value::Str(s) => {
                // zend_dval_to_lval_cap: not finite is 0, out of range saturates
                let i = if f.is_finite() { f as i64 } else { 0 };
                if i as f64 != f {
                    diags.push(Diag::Deprecated(format!(
                        "Implicit conversion from float-string \"{}\" to int loses precision",
                        String::from_utf8_lossy(s.as_bytes())
                    )));
                }
                i
            }
            _ => {
                float_to_int_diags(f, diags);
                dval_to_lval(f)
            }
        },
    })
}

fn arithmetic(op: BinOp, a: &Value, b: &Value, diags: &mut Diags) -> Result<Value, OpError> {
    if op == BinOp::Add
        && let (Value::Array(x), Value::Array(y)) = (a, b)
    {
        return Ok(Value::Array(array_union(x, y)));
    }
    // Operands convert left to right; an array operand stops there.
    if matches!(a, Value::Array(_)) {
        return Err(unsupported(op, a, b));
    }
    let x = number_operand(op, a, b, a, diags)?;
    if matches!(b, Value::Array(_)) {
        return Err(unsupported(op, a, b));
    }
    let y = number_operand(op, a, b, b, diags)?;
    if op == BinOp::Pow && as_f64(x) == 0.0 && as_f64(y) < 0.0 {
        diags.push(Diag::Deprecated("Power of base 0 and negative exponent is deprecated".into()));
    }
    Ok(match (x, y) {
        (Number::Int(x), Number::Int(y)) => match op {
            BinOp::Add => x.checked_add(y).map_or(Value::Float(x as f64 + y as f64), Value::Int),
            BinOp::Sub => x.checked_sub(y).map_or(Value::Float(x as f64 - y as f64), Value::Int),
            BinOp::Mul => x.checked_mul(y).map_or(Value::Float(x as f64 * y as f64), Value::Int),
            BinOp::Div => {
                if y == 0 {
                    return Err(OpError::DivisionByZero("Division by zero"));
                }
                if !(x == i64::MIN && y == -1) && x % y == 0 {
                    Value::Int(x / y)
                } else {
                    Value::Float(x as f64 / y as f64)
                }
            }
            _ => int_pow(x, y),
        },
        (x, y) => {
            let (x, y) = (as_f64(x), as_f64(y));
            Value::Float(match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div => {
                    if y == 0.0 {
                        return Err(OpError::DivisionByZero("Division by zero"));
                    }
                    x / y
                }
                _ => x.powf(y),
            })
        }
    })
}

fn as_f64(n: Number) -> f64 {
    match n {
        Number::Int(i) => i as f64,
        Number::Float(f) => f,
    }
}

/// pow_function_base: square-and-multiply in ints; on overflow the rest
/// is computed in doubles from where it stopped, as PHP does (so the
/// rounding matches).
fn int_pow(base: i64, exp: i64) -> Value {
    if exp < 0 {
        return Value::Float((base as f64).powf(exp as f64));
    }
    if exp == 0 {
        return Value::Int(1);
    }
    if base == 0 {
        return Value::Int(0);
    }
    let (mut l1, mut l2, mut i) = (1i64, base, exp);
    while i >= 1 {
        if i % 2 == 1 {
            i -= 1;
            match l1.checked_mul(l2) {
                Some(v) => l1 = v,
                None => return Value::Float((l1 as f64 * l2 as f64) * (l2 as f64).powf(i as f64)),
            }
        } else {
            i /= 2;
            match l2.checked_mul(l2) {
                Some(v) => l2 = v,
                None => return Value::Float(l1 as f64 * (l2 as f64 * l2 as f64).powf(i as f64)),
            }
        }
    }
    Value::Int(l1)
}

/// `$a + $b` on arrays: the left, plus the right's keys it lacks.
fn array_union(a: &Rc<PhpArray>, b: &Rc<PhpArray>) -> Rc<PhpArray> {
    if b.is_empty() {
        return a.clone();
    }
    let mut out = a.clone();
    for (key, value) in b.iter() {
        if !out.contains_key(&key) {
            Rc::make_mut(&mut out).insert(key, value.clone());
        }
    }
    out
}

fn bytewise(op: BinOp, x: &[u8], y: &[u8]) -> PhpStr {
    // & and ^ use the shorter length; | the longer, keeping the tail
    let (short, long) = if x.len() <= y.len() { (x, y) } else { (y, x) };
    let mut out: Vec<u8> = short
        .iter()
        .zip(long)
        .map(|(a, b)| match op {
            BinOp::BitAnd => a & b,
            BinOp::BitOr => a | b,
            _ => a ^ b,
        })
        .collect();
    if op == BinOp::BitOr {
        out.extend_from_slice(&long[short.len()..]);
    }
    PhpStr::new(out)
}

/// A value as `.` and echo see it; arrays warn and are "Array".
/// The Error converting an object to a string raises (until __toString
/// is called by the engine: an object without it can't be converted).
pub fn object_string_error(v: &Value) -> Option<OpError> {
    match v {
        Value::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 || o.class.cast_string.is_some() => None,
        Value::Object(o) => Some(OpError::Error(format!(
            "Object of class {} could not be converted to string",
            String::from_utf8_lossy(o.class.name.as_bytes())
        ))),
        Value::Ref(cell) => object_string_error(&cell.borrow()),
        _ => None,
    }
}

pub fn to_string_lossy(v: &Value, diags: &mut Diags) -> PhpStr {
    // zend_double_to_str (8.5+)
    if v.is_nan() {
        nan_coerced(f64::NAN, "string", diags);
    }
    match v.to_php_string() {
        Some(s) => s,
        None => {
            diags.push(Diag::Warning("Array to string conversion".into()));
            PhpStr::from("Array")
        }
    }
}

fn concat(a: &Value, b: &Value, diags: &mut Diags) -> Value {
    if let (Value::Str(x), Value::Str(y)) = (a, b) {
        return Value::Str(PhpStr::concat(x.as_bytes(), y.as_bytes()));
    }
    let mut bytes = PhpStr::scratch(string_len_hint(a) + string_len_hint(b));
    push_string(&mut bytes, a, diags);
    push_string(&mut bytes, b, diags);
    Value::Str(PhpStr::from_scratch(bytes))
}

/// Append `v` as a string (what `.` and interpolation do), the same text
/// as `to_string_lossy` gives, without building a string for a number.
pub fn push_string(out: &mut Vec<u8>, v: &Value, diags: &mut Diags) {
    match v {
        Value::Str(s) => out.extend_from_slice(s.as_bytes()),
        Value::Int(i) => push_int(out, *i),
        Value::Null | Value::Undef | Value::Bool(false) => {}
        Value::Bool(true) => out.push(b'1'),
        other => out.extend_from_slice(to_string_lossy(other, diags).as_bytes()),
    }
}

/// Bytes `v` probably takes as a string (for reserving room).
pub fn string_len_hint(v: &Value) -> usize {
    match v {
        Value::Str(s) => s.len(),
        Value::Int(_) | Value::Float(_) => 20,
        _ => 1,
    }
}



/// `-$a` (as `$a * -1`, keeping -0.0 and int overflow rules).
pub fn negate(a: &Value, diags: &mut Diags) -> Result<Value, OpError> {
    match a {
        Value::Int(i) => Ok(i.checked_neg().map_or(Value::Float(-(*i as f64)), Value::Int)),
        Value::Float(f) => Ok(Value::Float(negate_float(*f))),
        _ => binary(BinOp::Mul, a, &Value::Int(-1), diags),
    }
}

/// `-$f` as PHP computes it: `$f * -1` (zend_compile_unary_pm). The same
/// as `-f` except for a NaN: a multiply passes a NaN operand through
/// (quieted) with its sign, where `-f` flips the sign (visible to
/// `pack()`). LLVM may turn `f * -1.0` into `-f`, so that case is written
/// out.
pub fn negate_float(f: f64) -> f64 {
    if f.is_nan() { f64::from_bits(f.to_bits() | 0x0008_0000_0000_0000) } else { -f }
}

/// `~$a` (bitwise_not_function: a float through zend_dval_to_lval_safe)
pub fn bit_not(a: &Value, diags: &mut Diags) -> Result<Value, OpError> {
    match a {
        Value::Int(i) => Ok(Value::Int(!i)),
        Value::Float(f) => {
            float_to_int_diags(*f, diags);
            Ok(Value::Int(!dval_to_lval(*f)))
        }
        Value::Str(s) => Ok(Value::Str(PhpStr::new(s.as_bytes().iter().map(|b| !b).collect()))),
        Value::Ref(cell) => bit_not(&cell.borrow(), diags),
        other => Err(OpError::Type(format!("Cannot perform bitwise not on {}", other.type_name()))),
    }
}

const INCREMENT_NON_NUMERIC: &str = "Increment on non-numeric string is deprecated, use str_increment() instead";

/// `++$a` (increment_function).
pub fn increment(a: &Value, diags: &mut Diags) -> Result<Value, OpError> {
    Ok(match a {
        Value::Int(i) => i.checked_add(1).map_or(Value::Float(*i as f64 + 1.0), Value::Int),
        Value::Float(f) => Value::Float(f + 1.0),
        Value::Null | Value::Undef => Value::Int(1),
        Value::Bool(_) => {
            diags.push(Diag::Warning(
                "Increment on type bool has no effect, this will change in the next major version of PHP".into(),
            ));
            a.clone()
        }
        // increment_string(): every non-numeric string, the empty one included,
        // is deprecated since 8.5 (8.3 only deprecated non-alphanumeric ones).
        Value::Str(s) if s.is_empty() => {
            diags.push(Diag::Deprecated(INCREMENT_NON_NUMERIC.into()));
            Value::str("1")
        }
        Value::Str(s) => match numeric_string(s.as_bytes()) {
            Some(Number::Int(i)) => i.checked_add(1).map_or(Value::Float(i as f64 + 1.0), Value::Int),
            Some(Number::Float(f)) => Value::Float(f + 1.0),
            None => {
                diags.push(Diag::Deprecated(INCREMENT_NON_NUMERIC.into()));
                let mut bytes = s.as_bytes().to_vec();
                increment_alphanumeric(&mut bytes);
                Value::Str(PhpStr::new(bytes))
            }
        },
        Value::Array(_) => return Err(OpError::Type("Cannot increment array".into())),
        Value::Object(_) => return Err(OpError::Type(format!("Cannot increment {}", a.type_name()))),
        Value::Ref(cell) => return increment(&cell.borrow(), diags),
    })
}

/// `--$a` (decrement_function).
pub fn decrement(a: &Value, diags: &mut Diags) -> Result<Value, OpError> {
    Ok(match a {
        Value::Int(i) => i.checked_sub(1).map_or(Value::Float(*i as f64 - 1.0), Value::Int),
        Value::Float(f) => Value::Float(f - 1.0),
        Value::Null | Value::Undef => {
            diags.push(Diag::Warning(
                "Decrement on type null has no effect, this will change in the next major version of PHP".into(),
            ));
            Value::Null
        }
        Value::Bool(_) => {
            diags.push(Diag::Warning(
                "Decrement on type bool has no effect, this will change in the next major version of PHP".into(),
            ));
            a.clone()
        }
        Value::Str(s) if s.is_empty() => {
            diags.push(Diag::Deprecated("Decrement on empty string is deprecated as non-numeric".into()));
            Value::Int(-1)
        }
        Value::Str(s) => match numeric_string(s.as_bytes()) {
            Some(Number::Int(i)) => i.checked_sub(1).map_or(Value::Float(i as f64 - 1.0), Value::Int),
            Some(Number::Float(f)) => Value::Float(f - 1.0),
            None => {
                diags.push(Diag::Deprecated("Decrement on non-numeric string has no effect and is deprecated".into()));
                a.clone()
            }
        },
        Value::Array(_) => return Err(OpError::Type("Cannot decrement array".into())),
        Value::Object(_) => return Err(OpError::Type(format!("Cannot decrement {}", a.type_name()))),
        Value::Ref(cell) => return decrement(&cell.borrow(), diags),
    })
}

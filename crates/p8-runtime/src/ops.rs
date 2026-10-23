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
use crate::numeric::{Number, increment_alphanumeric, leading_numeric, numeric_string};
use crate::number::{Precision, dval_to_lval, float_to_string};
use std::rc::Rc;

/// A diagnostic an operator raised (shown with the current location).
#[derive(Debug, Clone, PartialEq)]
pub enum Diag {
    Warning(String),
    Deprecated(String),
}

pub type Diags = Vec<Diag>;

/// An error an operator throws.
#[derive(Debug, Clone, PartialEq)]
pub enum OpError {
    /// TypeError
    Type(String),
    /// DivisionByZeroError
    DivisionByZero(&'static str),
    /// ArithmeticError
    Arithmetic(&'static str),
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
    match op {
        BinOp::Concat => Ok(concat(a, b, diags)),
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
        Value::Array(_) | Value::Ref(_) => return Err(unsupported(op, a, b)),
    })
}

/// The int an operand of %, bitwise or shift operators stands for; a
/// fractional float (or float string) is deprecated.
fn int_operand(op: BinOp, a: &Value, b: &Value, v: &Value, diags: &mut Diags) -> Result<i64, OpError> {
    let n = number_operand(op, a, b, v, diags)?;
    Ok(match n {
        Number::Int(i) => i,
        Number::Float(f) => {
            let i = dval_to_lval(f);
            if !f.is_finite() || i as f64 != f {
                let message = match v {
                    Value::Str(s) => format!(
                        "Implicit conversion from float-string \"{}\" to int loses precision",
                        String::from_utf8_lossy(s.as_bytes())
                    ),
                    _ => format!(
                        "Implicit conversion from float {} to int loses precision",
                        float_to_string(f, Precision::Shortest, 'E')
                    ),
                };
                diags.push(Diag::Deprecated(message));
            }
            i
        }
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
pub fn to_string_lossy(v: &Value, diags: &mut Diags) -> PhpStr {
    match v.to_php_string() {
        Some(s) => s,
        None => {
            diags.push(Diag::Warning("Array to string conversion".into()));
            PhpStr::from("Array")
        }
    }
}

fn concat(a: &Value, b: &Value, diags: &mut Diags) -> Value {
    let mut bytes = Vec::with_capacity(string_len_hint(a) + string_len_hint(b));
    push_string(&mut bytes, a, diags);
    push_string(&mut bytes, b, diags);
    Value::Str(PhpStr::new(bytes))
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

/// An int's decimal digits, appended.
fn push_int(out: &mut Vec<u8>, i: i64) {
    let mut buf = [0u8; 20];
    let mut at = buf.len();
    let mut n = i.unsigned_abs();
    loop {
        at -= 1;
        buf[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if i < 0 {
        out.push(b'-');
    }
    out.extend_from_slice(&buf[at..]);
}

/// `-$a` (as `$a * -1`, keeping -0.0 and int overflow rules).
pub fn negate(a: &Value, diags: &mut Diags) -> Result<Value, OpError> {
    match a {
        Value::Int(i) => Ok(i.checked_neg().map_or(Value::Float(-(*i as f64)), Value::Int)),
        Value::Float(f) => Ok(Value::Float(-f)),
        _ => binary(BinOp::Mul, a, &Value::Int(-1), diags),
    }
}

/// `~$a`
pub fn bit_not(a: &Value) -> Result<Value, OpError> {
    match a {
        Value::Int(i) => Ok(Value::Int(!i)),
        Value::Float(f) => Ok(Value::Int(!dval_to_lval(*f))),
        Value::Str(s) => Ok(Value::Str(PhpStr::new(s.as_bytes().iter().map(|b| !b).collect()))),
        Value::Ref(cell) => bit_not(&cell.borrow()),
        other => Err(OpError::Type(format!("Cannot perform bitwise not on {}", other.type_name()))),
    }
}

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
        Value::Str(s) if s.is_empty() => {
            diags.push(Diag::Deprecated("Increment on non-alphanumeric string is deprecated".into()));
            Value::str("1")
        }
        Value::Str(s) => match numeric_string(s.as_bytes()) {
            Some(Number::Int(i)) => i.checked_add(1).map_or(Value::Float(i as f64 + 1.0), Value::Int),
            Some(Number::Float(f)) => Value::Float(f + 1.0),
            None => {
                if !s.as_bytes().iter().all(u8::is_ascii_alphanumeric) {
                    diags.push(Diag::Deprecated("Increment on non-alphanumeric string is deprecated".into()));
                }
                let mut bytes = s.as_bytes().to_vec();
                increment_alphanumeric(&mut bytes);
                Value::Str(PhpStr::new(bytes))
            }
        },
        Value::Array(_) => return Err(OpError::Type("Cannot increment array".into())),
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
        Value::Ref(cell) => return decrement(&cell.borrow(), diags),
    })
}

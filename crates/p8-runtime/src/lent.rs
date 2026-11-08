//! Reading the arguments of a builtin on the borrowed ABI
//! ([`crate::ext::Fast`]): the caller's registers, lent. Whichever path
//! called it, a typed parameter's argument is of the declared type when
//! the builtin runs (the frameless call site found it so, or the checked
//! path converted it), so each reader is one tag test on its common
//! path; the conversion after it is for untyped and union parameters.
//! An optional parameter the call left out reads as its stub default
//! would for the common cases (empty, 0, false, null): a builtin whose
//! default is anything else tests `args.len()` itself.

use crate::array::PhpArray;
use crate::string::{PhpStr, StrHead};
use crate::value::Value;

/// Argument `i`, if passed (null when it was left out).
#[inline(always)]
pub fn at(args: &[Value], i: usize) -> &Value {
    const NULL: &Value = &Value::NULL;
    args.get(i).unwrap_or(NULL)
}

/// Argument `i` with a reference looked through (`mixed` parameters):
/// borrowed, unless it is a reference, whose value is copied out.
#[inline(always)]
pub fn value(args: &[Value], i: usize) -> std::borrow::Cow<'_, Value> {
    let v = at(args, i);
    match v.as_reference() {
        Some(cell) => std::borrow::Cow::Owned(cell.borrow().clone()),
        None => std::borrow::Cow::Borrowed(v),
    }
}

/// Argument `i` as an int (`int` parameters).
#[inline(always)]
pub fn int(args: &[Value], i: usize) -> i64 {
    match args.get(i) {
        Some(v) => match v.as_int() {
            Some(n) => n,
            None => v.deref().to_int(),
        },
        None => 0,
    }
}

/// Argument `i` as a float (`float` parameters, which take ints).
#[inline(always)]
pub fn float(args: &[Value], i: usize) -> f64 {
    match args.get(i) {
        Some(v) => match v.as_float() {
            Some(f) => f,
            None => v.deref().to_float(),
        },
        None => 0.0,
    }
}

/// Argument `i` as a bool (`bool` parameters).
#[inline(always)]
pub fn bool(args: &[Value], i: usize) -> bool {
    match args.get(i) {
        Some(v) => match v.as_bool() {
            Some(b) => b,
            None => v.deref().to_bool(),
        },
        None => false,
    }
}

/// Argument `i`'s bytes (`string` parameters), read in place: empty when
/// it was left out. (Anything that is not a string here is an untyped
/// argument: [`string`] converts it.)
#[inline(always)]
pub fn bytes(args: &[Value], i: usize) -> &[u8] {
    match args.get(i).and_then(Value::as_str) {
        Some(s) => s.as_bytes(),
        None => b"",
    }
}

/// Argument `i` as a string, read in place when it is one.
#[inline(always)]
pub fn str(args: &[Value], i: usize) -> Option<&StrHead> {
    args.get(i).and_then(Value::as_str)
}

/// Argument `i` converted to a string (an owned handle: a string argument
/// is shared, not copied). Objects and arrays are the caller's to have
/// refused: they give the empty string.
#[inline]
pub fn string(args: &[Value], i: usize) -> PhpStr {
    match args.get(i) {
        Some(v) => v.deref().to_php_string().unwrap_or_else(PhpStr::empty),
        None => PhpStr::empty(),
    }
}

/// Argument `i`'s array (`array` parameters), read in place.
#[inline(always)]
pub fn array(args: &[Value], i: usize) -> Option<&PhpArray> {
    args.get(i).and_then(Value::as_array)
}

/// Argument `i` unless it was left out or is null (`?T $x = null`).
#[inline(always)]
pub fn given(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i).filter(|v| !v.is_null_or_undef())
}

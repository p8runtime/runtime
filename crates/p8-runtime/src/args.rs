//! The argument checks PHP makes before a builtin runs, from the builtin's
//! stub signature (p8-stubs): ArgumentCountError for a wrong count,
//! TypeError for a value its parameter can't take, the deprecations for
//! null and for fractional floats passed to scalar parameters, and the
//! weak-mode conversions that give the builtin the declared type.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_API.c - zend_wrong_parameters_count_error,
//! zend_parse_arg_*_weak, zend_null_arg_deprecated, zend_zval_value_name

use crate::number::{Precision, float_to_string};
use crate::numeric::{Number, numeric_string};
use crate::ops::{Diag, Diags};
use crate::string::PhpStr;
use crate::value::Value;
use p8_stubs::{
    Param, Sig, T_ARRAY, T_BOOL, T_CALLABLE, T_FALSE, T_FLOAT, T_INT, T_MIXED, T_NULL, T_STRING, T_TRUE,
    type_to_string,
};

/// An error a builtin call raises before it runs: the class and message.
pub type Refused = (&'static str, String);

/// Check `args` (as passed) against `sig`, converting each accepted
/// scalar argument to its parameter's type in place, as PHP does in
/// non-strict mode. Deprecations go to `diags`.
/// (A function PHP deprecates as a whole is reported by the caller, from
/// `p8_stubs::deprecation`, looked up once.)
pub fn check(sig: &Sig, args: &mut [Value], diags: &mut Diags) -> Result<(), Refused> {
    let params = sig.params;
    let variadic = params.last().is_some_and(|p| p.variadic);
    let required = params.iter().rposition(|p| !p.optional()).map_or(0, |i| i + 1);
    let max = if variadic { usize::MAX } else { params.len() };
    if args.len() < required || args.len() > max {
        let (word, n) = if required == max {
            ("exactly", required)
        } else if args.len() < required {
            ("at least", required)
        } else {
            ("at most", max)
        };
        let plural = if n == 1 { "" } else { "s" };
        return Err(("ArgumentCountError", format!("{}() expects {word} {n} argument{plural}, {} given", sig.name, args.len())));
    }
    for (i, arg) in args.iter_mut().enumerate() {
        let Some(param) = params.get(i).or_else(|| params.last().filter(|p| p.variadic)) else { break };
        if param.by_ref {
            continue;
        }
        match accepts(param, arg) {
            Accept::Yes => {}
            Accept::NullDeprecated => diags.push(Diag::Deprecated(format!(
                "{}(): Passing null to parameter #{}{} of type {} is deprecated",
                sig.name,
                i + 1,
                label(param),
                type_to_string(param.mask, param.classes)
            ))),
            Accept::Deprecated(message) => diags.push(Diag::Deprecated(message)),
            Accept::No => {
                return Err((
                    "TypeError",
                    format!(
                        "{}(): Argument #{}{} must be of type {}, {} given",
                        sig.name,
                        i + 1,
                        label(param),
                        type_to_string(param.mask, param.classes),
                        value_name(arg)
                    ),
                ));
            }
        }
        if let Some(converted) = convert(param.mask, arg) {
            *arg = converted;
        }
    }
    Ok(())
}

/// " ($name)" after "Argument #N"; PHP names no variadic argument.
fn label(param: &Param) -> String {
    if param.variadic { String::new() } else { format!(" (${})", param.name) }
}

/// How a TypeError names a value (zend_zval_value_name).
pub fn value_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::Ref(cell) => value_name(&cell.borrow()),
        other => other.type_name(),
    }
}

/// Whether a float converts to int without overflow (ZEND_DOUBLE_FITS_LONG).
fn fits_long(f: f64) -> bool {
    (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f)
}

enum Accept {
    Yes,
    /// Null to a scalar parameter: accepted, deprecated.
    NullDeprecated,
    /// Accepted with this deprecation (a fractional float to an int).
    Deprecated(String),
    No,
}

fn accepts(param: &Param, value: &Value) -> Accept {
    let mask = param.mask;
    if param.untyped() || mask & T_MIXED != 0 {
        return Accept::Yes;
    }
    let scalar = mask & (T_BOOL | T_TRUE | T_FALSE | T_INT | T_FLOAT | T_STRING);
    match value {
        Value::Null | Value::Undef => {
            if mask & T_NULL != 0 {
                Accept::Yes
            } else if scalar != 0 {
                Accept::NullDeprecated
            } else {
                Accept::No
            }
        }
        Value::Bool(b) => {
            let exact = mask & T_BOOL != 0 || mask & if *b { T_TRUE } else { T_FALSE } != 0;
            if exact || mask & (T_INT | T_FLOAT | T_STRING) != 0 { Accept::Yes } else { Accept::No }
        }
        Value::Int(_) => {
            if mask & (T_INT | T_FLOAT | T_STRING | T_BOOL) != 0 { Accept::Yes } else { Accept::No }
        }
        Value::Float(f) => {
            if mask & T_FLOAT != 0 {
                Accept::Yes
            } else if mask & T_INT != 0 {
                if !fits_long(*f) {
                    if mask & (T_STRING | T_BOOL) != 0 { Accept::Yes } else { Accept::No }
                } else if f.fract() != 0.0 && mask & T_STRING == 0 {
                    Accept::Deprecated(format!(
                        "Implicit conversion from float {} to int loses precision",
                        float_to_string(*f, Precision::SERIALIZE, 'E')
                    ))
                } else {
                    Accept::Yes
                }
            } else if mask & (T_STRING | T_BOOL) != 0 {
                Accept::Yes
            } else {
                Accept::No
            }
        }
        Value::Str(s) => {
            if mask & (T_STRING | T_CALLABLE) != 0 {
                return Accept::Yes;
            }
            if mask & (T_INT | T_FLOAT) != 0 {
                return match numeric_string(s.as_bytes()) {
                    Some(Number::Float(f)) if mask & T_FLOAT == 0 => {
                        if !fits_long(f) {
                            Accept::No
                        } else if f.fract() != 0.0 {
                            Accept::Deprecated(format!(
                                "Implicit conversion from float-string \"{}\" to int loses precision",
                                String::from_utf8_lossy(s.as_bytes())
                            ))
                        } else {
                            Accept::Yes
                        }
                    }
                    Some(_) => Accept::Yes,
                    None if mask & T_BOOL != 0 => Accept::Yes,
                    None => Accept::No,
                };
            }
            if mask & T_BOOL != 0 { Accept::Yes } else { Accept::No }
        }
        Value::Array(_) => {
            if mask & (T_ARRAY | T_CALLABLE) != 0 { Accept::Yes } else { Accept::No }
        }
        Value::Ref(cell) => accepts(param, &cell.borrow()),
    }
}

/// The value the builtin receives for an accepted argument when it differs
/// from the one passed: weak-mode scalar conversion to the declared type.
fn convert(mask: u32, value: &Value) -> Option<Value> {
    if mask == 0 || mask & (T_MIXED | T_CALLABLE) != 0 {
        return None;
    }
    if let Value::Ref(cell) = value {
        return convert(mask, &cell.borrow()).or_else(|| Some(cell.borrow().clone()));
    }
    let accepts = |bit: u32| mask & bit != 0;
    let already = match value {
        Value::Null | Value::Undef => accepts(T_NULL),
        Value::Bool(true) => accepts(T_BOOL | T_TRUE),
        Value::Bool(false) => accepts(T_BOOL | T_FALSE),
        Value::Int(_) => accepts(T_INT),
        Value::Float(_) => accepts(T_FLOAT),
        Value::Str(_) => accepts(T_STRING),
        Value::Array(_) | Value::Ref(_) => true,
    };
    if already {
        return None;
    }
    Some(match value {
        Value::Null | Value::Undef | Value::Bool(_) => {
            let b = value.to_bool();
            if accepts(T_INT) {
                Value::Int(i64::from(b))
            } else if accepts(T_FLOAT) {
                Value::Float(if b { 1.0 } else { 0.0 })
            } else if accepts(T_STRING) {
                Value::Str(PhpStr::new(if b { b"1".to_vec() } else { Vec::new() }))
            } else if accepts(T_BOOL | T_TRUE | T_FALSE) {
                Value::Bool(b)
            } else {
                return None;
            }
        }
        Value::Int(i) => {
            if accepts(T_FLOAT) {
                Value::Float(*i as f64)
            } else if accepts(T_STRING) {
                Value::Str(value.to_php_string()?)
            } else {
                Value::Bool(*i != 0)
            }
        }
        Value::Float(f) => {
            if accepts(T_INT) && fits_long(*f) && (f.fract() == 0.0 || !accepts(T_STRING)) {
                Value::Int(*f as i64)
            } else if accepts(T_STRING) {
                Value::Str(value.to_php_string()?)
            } else {
                Value::Bool(*f != 0.0)
            }
        }
        Value::Str(s) => match numeric_string(s.as_bytes()) {
            Some(Number::Int(i)) if accepts(T_INT) => Value::Int(i),
            Some(Number::Int(i)) if accepts(T_FLOAT) => Value::Float(i as f64),
            Some(Number::Float(f)) if accepts(T_FLOAT) => Value::Float(f),
            Some(Number::Float(f)) if accepts(T_INT) => Value::Int(f as i64),
            _ => Value::Bool(value.to_bool()),
        },
        Value::Array(_) | Value::Ref(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Value {
        Value::Str(PhpStr::from(text))
    }

    fn run(name: &str, mut args: Vec<Value>) -> (Result<(), Refused>, Vec<Value>, Vec<String>) {
        let sig = p8_stubs::lookup(name.as_bytes()).expect("a stub signature");
        let mut diags = Vec::new();
        let r = check(sig, &mut args, &mut diags);
        let diags = diags
            .into_iter()
            .map(|d| match d {
                Diag::Warning(m) | Diag::Deprecated(m) => m,
            })
            .collect();
        (r, args, diags)
    }

    /// Messages and conversions as php:8.4-cli gives them.
    #[test]
    fn checks_follow_php() {
        let (r, ..) = run("strlen", vec![]);
        assert_eq!(r, Err(("ArgumentCountError", "strlen() expects exactly 1 argument, 0 given".into())));
        let (r, ..) = run("str_repeat", vec![s("a")]);
        assert_eq!(r, Err(("ArgumentCountError", "str_repeat() expects exactly 2 arguments, 1 given".into())));
        let (r, ..) = run("strlen", vec![Value::Array(Default::default())]);
        assert_eq!(r, Err(("TypeError", "strlen(): Argument #1 ($string) must be of type string, array given".into())));
        let (r, args, _) = run("strlen", vec![Value::Int(123)]);
        assert_eq!((r, args[0].identical(&s("123"))), (Ok(()), true));
        let (r, args, diags) = run("strlen", vec![Value::Null]);
        assert_eq!((r, args[0].identical(&s(""))), (Ok(()), true));
        assert_eq!(diags, ["strlen(): Passing null to parameter #1 ($string) of type string is deprecated"]);
        let (r, args, diags) = run("str_repeat", vec![s("a"), Value::Float(2.5)]);
        assert_eq!((r, args[1].identical(&Value::Int(2))), (Ok(()), true));
        assert_eq!(diags, ["Implicit conversion from float 2.5 to int loses precision"]);
        let (r, ..) = run("str_repeat", vec![s("a"), s("x")]);
        assert_eq!(r, Err(("TypeError", "str_repeat(): Argument #2 ($times) must be of type int, string given".into())));
        let (r, ..) = run("str_repeat", vec![s("a"), Value::Bool(true), Value::Int(1)]);
        assert_eq!(r, Err(("ArgumentCountError", "str_repeat() expects exactly 2 arguments, 3 given".into())));
    }
}

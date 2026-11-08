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
use crate::value::{Kind, Value};
use p8_stubs::{
    Param, Sig, T_ARRAY, T_BOOL, T_CALLABLE, T_FALSE, T_FLOAT, T_INT, T_MIXED, T_NULL, T_OBJECT, T_STRING, T_TRUE,
    type_to_string,
};

/// An error a builtin call raises before it runs: the class and message.
pub type Refused = (&'static str, String);

/// zend_is_callable_ex()'s error for a value that is not callable (the
/// engine's answer, [`crate::ext::Ctx::why_not_callable`]).
pub fn callable_error(ctx: &mut dyn crate::ext::Ctx, v: &Value) -> String {
    ctx.why_not_callable(v).unwrap_or_default()
}

/// Check `args` (as passed) against `sig`, converting each accepted
/// scalar argument to its parameter's type in place, as PHP does in
/// non-strict mode. Deprecations go to `diags`.
/// (A function PHP deprecates as a whole is reported by the caller, from
/// `p8_stubs::deprecation`, looked up once.)
pub fn check(sig: &Sig, args: &mut [Value], diags: &mut Diags) -> Result<(), Refused> {
    check_counted(sig, counts(sig), args, diags)
}

/// The argument counts `sig` takes: the required ones, and at most
/// (usize::MAX: variadic). The registry works them out once per builtin.
pub fn counts(sig: &Sig) -> (usize, usize) {
    let params = sig.params;
    let variadic = params.last().is_some_and(|p| p.variadic);
    let required = params.iter().rposition(|p| !p.optional()).map_or(0, |i| i + 1);
    (required, if variadic { usize::MAX } else { params.len() })
}

/// Whether a frameless call can skip [`check_counted`]: as many arguments
/// as `sig` takes, each plainly of its parameter's type (nothing to
/// convert, warn about or refuse), none a reference or undefined, and
/// none an object unless `objects` (the builtin is known to run no PHP
/// code on one: an object may have a __toString() or __debugInfo(), or
/// be a Countable, which the checked path runs with the builtin's frame
/// pushed and the registers lent). (ZEND_FRAMELESS_ICALL's operands are
/// type-tested the same way before the handler runs.)
#[inline(always)]
pub fn plain_call(sig: &Sig, (required, max): (usize, usize), objects: bool, args: &[Value]) -> bool {
    if args.len() < required || args.len() > max {
        return false;
    }
    let params = sig.params;
    for (i, arg) in args.iter().enumerate() {
        let param = match params.get(i) {
            Some(p) => p,
            None => match params.last() {
                Some(p) if p.variadic => p,
                _ => return false,
            },
        };
        let shut = match arg.kind() {
            Kind::Ref | Kind::Undef => true,
            Kind::Object => !objects,
            _ => false,
        };
        if shut || !plainly(param, arg) {
            return false;
        }
    }
    true
}

/// [`check`], with `sig`'s [`counts`] already known.
#[inline]
pub fn check_counted(sig: &Sig, (required, max): (usize, usize), args: &mut [Value], diags: &mut Diags) -> Result<(), Refused> {
    let params = sig.params;
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
        // By reference, or plainly of a type the parameter takes: nothing
        // to check or convert (accepts would say Yes, convert would keep it).
        // A plain callable is the engine's to check ([`callback_error`]).
        if param.by_ref || pure_callable(param) || plainly(param, arg) {
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

/// Whether `value` is of a kind `param` takes as it is: untyped or mixed,
/// or its kind's bit set; a string or an array to a callable, and an
/// object to `object` or a callable, which are checked when called
/// (`accepts` says Yes and `convert` keeps them). A reference, and an
/// object to a class type: the full check.
#[inline]
fn plainly(param: &Param, value: &Value) -> bool {
    let mask = param.mask;
    if mask & T_MIXED != 0 || param.untyped() {
        return true;
    }
    mask & match value.kind() {
        Kind::Null => T_NULL,
        Kind::Bool => T_BOOL | if value.as_bool().expect("a bool") { T_TRUE } else { T_FALSE },
        Kind::Int => T_INT,
        Kind::Float => T_FLOAT,
        Kind::Str => T_STRING | T_CALLABLE,
        Kind::Array => T_ARRAY | T_CALLABLE,
        Kind::Object => T_OBJECT | T_CALLABLE,
        Kind::Undef | Kind::Ref => 0,
    } != 0
}

/// Whether argument `value` to `param` (a `callable` one) is for the
/// engine to check as a callable: not null where null is taken, nor of
/// another kind the parameter takes as it is.
pub fn wants_callable_check(param: &Param, value: &Value) -> bool {
    let mask = param.mask;
    pure_callable(param)
        && !param.by_ref
        && match value.kind() {
            Kind::Null | Kind::Undef => mask & T_NULL == 0,
            Kind::Str => mask & T_STRING == 0,
            Kind::Array => mask & T_ARRAY == 0,
            Kind::Object => mask & T_OBJECT == 0,
            Kind::Ref => wants_callable_check(param, &value.as_reference().expect("a reference").borrow()),
            _ => true,
        }
}

/// Whether `param` is `callable` or `?callable` alone: ZPP's `f` (a
/// "valid callback" error). A union with other types (`callable|string|null`)
/// is parsed as a union, and its builtin checks the callable itself.
fn pure_callable(param: &Param) -> bool {
    param.mask & T_CALLABLE != 0 && param.mask & !(T_CALLABLE | T_NULL) == 0 && param.classes.is_empty()
}

/// The TypeError for argument `i` (0-based) of `sig`, a callback that is
/// not callable for `why` (zend_is_callable's reason): "f(): Argument #1
/// ($callback) must be a valid callback[ or null], why".
pub fn callback_error(sig: &Sig, i: usize, why: &str) -> Refused {
    let param = sig.params.get(i).or_else(|| sig.params.last().filter(|p| p.variadic)).expect("a callable parameter");
    let or_null = if param.mask & T_NULL != 0 { " or null" } else { "" };
    ("TypeError", format!("{}(): Argument #{}{} must be a valid callback{or_null}, {why}", sig.name, i + 1, label(param)))
}

/// " ($name)" after "Argument #N"; PHP names no variadic argument.
fn label(param: &Param) -> String {
    if param.variadic { String::new() } else { format!(" (${})", param.name) }
}

/// How a TypeError names a value (zend_zval_value_name).
pub fn value_name(v: &Value) -> std::borrow::Cow<'static, str> {
    if let Some(b) = v.as_bool() {
        if b { "true".into() } else { "false".into() }
    } else if let Some(cell) = v.as_reference() {
        value_name(&cell.borrow())
    } else {
        v.type_name()
    }
}

/// Whether a float converts to int without overflow (ZEND_DOUBLE_FITS_LONG).
fn fits_long(f: f64) -> bool {
    (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f)
}

pub(crate) enum Accept {
    Yes,
    /// Null to a scalar parameter: accepted, deprecated.
    NullDeprecated,
    /// Accepted with this deprecation (a fractional float to an int).
    Deprecated(String),
    No,
}

pub(crate) fn accepts(param: &Param, value: &Value) -> Accept {
    let mask = param.mask;
    if param.untyped() || mask & T_MIXED != 0 {
        return Accept::Yes;
    }
    let scalar = mask & (T_BOOL | T_TRUE | T_FALSE | T_INT | T_FLOAT | T_STRING);
    match value.kind() {
        Kind::Null | Kind::Undef => {
            if mask & T_NULL != 0 {
                Accept::Yes
            } else if scalar != 0 {
                Accept::NullDeprecated
            } else {
                Accept::No
            }
        }
        Kind::Bool => {
            let b = value.as_bool().expect("a bool");
            let exact = mask & T_BOOL != 0 || mask & if b { T_TRUE } else { T_FALSE } != 0;
            if exact || mask & (T_INT | T_FLOAT | T_STRING) != 0 { Accept::Yes } else { Accept::No }
        }
        Kind::Int => {
            if mask & (T_INT | T_FLOAT | T_STRING | T_BOOL) != 0 { Accept::Yes } else { Accept::No }
        }
        Kind::Float => {
            let f = value.as_float().expect("a float");
            if mask & T_FLOAT != 0 {
                Accept::Yes
            } else if mask & T_INT != 0 {
                if !fits_long(f) {
                    if mask & (T_STRING | T_BOOL) != 0 { Accept::Yes } else { Accept::No }
                } else if f.fract() != 0.0 && mask & T_STRING == 0 {
                    Accept::Deprecated(format!(
                        "Implicit conversion from float {} to int loses precision",
                        float_to_string(f, Precision::SERIALIZE, 'E')
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
        Kind::Str => {
            let s = value.as_str().expect("a string");
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
        Kind::Array => {
            if mask & (T_ARRAY | T_CALLABLE) != 0 { Accept::Yes } else { Accept::No }
        }
        Kind::Ref => accepts(param, &value.as_reference().expect("a reference").borrow()),
        Kind::Object => {
            let o = value.as_object().expect("an object");
            let named = param.classes.iter().any(|c| o.class.is_a(c.to_ascii_lowercase().as_bytes()));
            // A callable object (a Closure, an invokable) is checked when called.
            if mask & (T_OBJECT | T_CALLABLE) != 0 || named { Accept::Yes } else { Accept::No }
        }
    }
}

/// The value the builtin receives for an accepted argument when it differs
/// from the one passed: weak-mode scalar conversion to the declared type.
pub(crate) fn convert(mask: u32, value: &Value) -> Option<Value> {
    if mask == 0 || mask & (T_MIXED | T_CALLABLE) != 0 {
        return None;
    }
    if let Some(cell) = value.as_reference() {
        return convert(mask, &cell.borrow()).or_else(|| Some(cell.borrow().clone()));
    }
    let accepts = |bit: u32| mask & bit != 0;
    let already = match value.kind() {
        Kind::Null | Kind::Undef => accepts(T_NULL),
        Kind::Bool => accepts(T_BOOL | if value.as_bool().expect("a bool") { T_TRUE } else { T_FALSE }),
        Kind::Int => accepts(T_INT),
        Kind::Float => accepts(T_FLOAT),
        Kind::Str => accepts(T_STRING),
        Kind::Array | Kind::Ref | Kind::Object => true,
    };
    if already {
        return None;
    }
    Some(match value.kind() {
        Kind::Null | Kind::Undef | Kind::Bool => {
            let b = value.to_bool();
            if accepts(T_INT) {
                Value::int(i64::from(b))
            } else if accepts(T_FLOAT) {
                Value::float(if b { 1.0 } else { 0.0 })
            } else if accepts(T_STRING) {
                Value::str(if b { "1" } else { "" })
            } else if accepts(T_BOOL | T_TRUE | T_FALSE) {
                Value::bool(b)
            } else {
                return None;
            }
        }
        Kind::Int => {
            let i = value.as_int().expect("an int");
            if accepts(T_FLOAT) {
                Value::float(i as f64)
            } else if accepts(T_STRING) {
                Value::str(value.to_php_string()?)
            } else {
                Value::bool(i != 0)
            }
        }
        Kind::Float => {
            let f = value.as_float().expect("a float");
            if accepts(T_INT) && fits_long(f) && (f.fract() == 0.0 || !accepts(T_STRING)) {
                Value::int(f as i64)
            } else if accepts(T_STRING) {
                Value::str(value.to_php_string()?)
            } else {
                Value::bool(f != 0.0)
            }
        }
        Kind::Str => match numeric_string(value.as_str().expect("a string").as_bytes()) {
            Some(Number::Int(i)) if accepts(T_INT) => Value::int(i),
            Some(Number::Int(i)) if accepts(T_FLOAT) => Value::float(i as f64),
            Some(Number::Float(f)) if accepts(T_FLOAT) => Value::float(f),
            Some(Number::Float(f)) if accepts(T_INT) => Value::int(f as i64),
            _ => Value::bool(value.to_bool()),
        },
        Kind::Array | Kind::Ref | Kind::Object => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::array::PhpArray;
    use crate::string::PhpStr;

    fn s(text: &str) -> Value {
        Value::str(PhpStr::from(text))
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
        let (r, ..) = run("strlen", vec![Value::array(PhpArray::default())]);
        assert_eq!(r, Err(("TypeError", "strlen(): Argument #1 ($string) must be of type string, array given".into())));
        let (r, args, _) = run("strlen", vec![Value::int(123)]);
        assert_eq!((r, args[0].identical(&s("123"))), (Ok(()), true));
        let (r, args, diags) = run("strlen", vec![Value::NULL]);
        assert_eq!((r, args[0].identical(&s(""))), (Ok(()), true));
        assert_eq!(diags, ["strlen(): Passing null to parameter #1 ($string) of type string is deprecated"]);
        let (r, args, diags) = run("str_repeat", vec![s("a"), Value::float(2.5)]);
        assert_eq!((r, args[1].identical(&Value::int(2))), (Ok(()), true));
        assert_eq!(diags, ["Implicit conversion from float 2.5 to int loses precision"]);
        let (r, ..) = run("str_repeat", vec![s("a"), s("x")]);
        assert_eq!(r, Err(("TypeError", "str_repeat(): Argument #2 ($times) must be of type int, string given".into())));
        let (r, ..) = run("str_repeat", vec![s("a"), Value::TRUE, Value::int(1)]);
        assert_eq!(r, Err(("ArgumentCountError", "str_repeat() expects exactly 2 arguments, 3 given".into())));
    }
}

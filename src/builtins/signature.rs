//! Builtin function signatures (generated from php-src's stubs, see
//! `signatures.rs`) and the argument checks PHP derives from them before a
//! builtin runs: ArgumentCountError, TypeError, and the deprecations for null
//! and fractional floats passed to scalar parameters.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_API.c - zend_parse_arg_*_weak,
//! zend_wrong_parameters_count_error, zend_null_arg_deprecated;
//! $PHP_SRC_PATH/Zend/zend_compile.c - zend_type_to_string

use crate::core::value::{Handle, Val};
use crate::vm::engine::{ErrorLevel, VM, VmError};

// The signatures and their types live in p8-stubs, shared with p8.
pub(crate) use p8_stubs::{
    Param, SIG_ABSTRACT, SIG_FINAL, SIG_PRIVATE, SIG_PROTECTED, SIG_STATIC, Sig, T_ARRAY, T_BOOL, T_CALLABLE, T_FALSE,
    T_FLOAT, T_INT, T_MIXED, T_NEVER, T_NULL, T_OBJECT, T_STATIC, T_STRING, T_TRUE, T_VOID, all, deprecation, lookup,
    type_to_string,
};

/// How PHP names a value in a TypeError ("int", "true", "array", "Foo").
/// Reference: $PHP_SRC_PATH/Zend/zend_API.c - zend_zval_value_name
pub(crate) fn value_name(vm: &VM, handle: Handle) -> String {
    match &vm.arena.get(handle).value {
        Val::Object(payload) => vm.describe_object_class(*payload),
        other => crate::vm::error_formatting::scalar_value_name(other).to_string(),
    }
}

/// What a numeric string holds: None when it is not numeric (leading or
/// trailing whitespace is allowed, trailing data is not).
/// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - _is_numeric_string_ex
pub(crate) fn numeric_string(s: &[u8]) -> Option<Val> {
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c');
    let start = s.iter().position(|c| !ws(c))?;
    let end = s.iter().rposition(|c| !ws(c))? + 1;
    let text = std::str::from_utf8(&s[start..end]).ok()?;
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let valid = body.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        && body
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !valid {
        return None;
    }
    if body.bytes().all(|c| c.is_ascii_digit())
        && let Ok(i) = text.parse::<i64>()
    {
        return Some(Val::Int(i));
    }
    text.parse::<f64>().ok().map(Val::Float)
}

impl VM {
    /// The class of an object payload.
    fn object_class(&self, payload: Handle) -> Option<crate::core::value::Symbol> {
        match &self.arena.get(payload).value {
            Val::ObjPayload(data) => Some(data.class),
            _ => None,
        }
    }

    /// `class instanceof name`, for a class name written in a signature.
    fn class_is_a(&self, class: crate::core::value::Symbol, name: &[u8]) -> bool {
        self.context
            .interner
            .find(name)
            .is_some_and(|target| self.is_subclass_of(class, target))
    }

    /// Whether objects of `class` convert to string (Stringable).
    fn class_has_to_string(&self, class: crate::core::value::Symbol) -> bool {
        self.context.interner.find(b"__toString").is_some_and(|m| {
            self.find_method(class, m).is_some() || self.find_native_method(class, m).is_some()
        })
    }
}

/// Whether a float converts to int without overflow (ZEND_DOUBLE_FITS_LONG).
fn fits_long(f: f64) -> bool {
    (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f)
}

/// Outcome of checking one argument against its parameter type.
enum Check {
    Ok,
    /// Accepted with a deprecation (null or a fractional float to a scalar).
    Deprecated(String),
    TypeError,
}

fn check_value(vm: &VM, param: &Param, handle: Handle, strict: bool) -> Check {
    let mask = param.mask;
    if param.untyped() || mask & T_MIXED != 0 {
        return Check::Ok;
    }
    let scalar = mask & (T_BOOL | T_TRUE | T_FALSE | T_INT | T_FLOAT | T_STRING);
    match &vm.arena.get(handle).value {
        Val::Null => {
            if mask & T_NULL != 0 {
                Check::Ok
            } else if scalar != 0 && !strict {
                Check::Deprecated(String::new())
            } else {
                Check::TypeError
            }
        }
        Val::Bool(b) => {
            let exact = mask & T_BOOL != 0 || mask & if *b { T_TRUE } else { T_FALSE } != 0;
            if exact || (!strict && mask & (T_INT | T_FLOAT | T_STRING) != 0) {
                Check::Ok
            } else {
                Check::TypeError
            }
        }
        Val::Int(_) => {
            if mask & (T_INT | T_FLOAT) != 0 || (!strict && mask & (T_STRING | T_BOOL) != 0) {
                Check::Ok
            } else {
                Check::TypeError
            }
        }
        Val::Float(f) => {
            if mask & T_FLOAT != 0 {
                Check::Ok
            } else if strict {
                Check::TypeError
            } else if mask & T_INT != 0 {
                if !fits_long(*f) {
                    if mask & (T_STRING | T_BOOL) != 0 {
                        Check::Ok
                    } else {
                        Check::TypeError
                    }
                } else if f.fract() != 0.0 && mask & T_STRING == 0 {
                    Check::Deprecated(format!(
                        "Implicit conversion from float {} to int loses precision",
                        crate::core::value::php_float_to_string(
                            *f,
                            crate::core::value::FloatPrecision::Shortest,
                            'E'
                        )
                    ))
                } else {
                    Check::Ok
                }
            } else if mask & (T_STRING | T_BOOL) != 0 {
                Check::Ok
            } else {
                Check::TypeError
            }
        }
        Val::String(s) => {
            if mask & T_STRING != 0 || mask & T_CALLABLE != 0 {
                return Check::Ok;
            }
            if strict {
                return Check::TypeError;
            }
            if mask & (T_INT | T_FLOAT) != 0 {
                return match numeric_string(s) {
                    Some(Val::Float(f)) if mask & T_FLOAT == 0 => {
                        if !fits_long(f) {
                            Check::TypeError
                        } else if f.fract() != 0.0 {
                            Check::Deprecated(format!(
                                "Implicit conversion from float-string \"{}\" to int loses precision",
                                String::from_utf8_lossy(s)
                            ))
                        } else {
                            Check::Ok
                        }
                    }
                    Some(_) => Check::Ok,
                    None if mask & T_BOOL != 0 => Check::Ok,
                    None => Check::TypeError,
                };
            }
            if mask & T_BOOL != 0 {
                Check::Ok
            } else {
                Check::TypeError
            }
        }
        Val::Array(_) => {
            if mask & (T_ARRAY | T_CALLABLE) != 0 {
                Check::Ok
            } else {
                Check::TypeError
            }
        }
        Val::Object(payload) => {
            if mask & (T_OBJECT | T_CALLABLE) != 0 {
                return Check::Ok;
            }
            let class = vm.object_class(*payload);
            if let Some(class) = class
                && param
                    .classes
                    .iter()
                    .any(|c| vm.class_is_a(class, c.as_bytes()))
            {
                return Check::Ok;
            }
            if !strict && mask & T_STRING != 0 && class.is_some_and(|c| vm.class_has_to_string(c)) {
                return Check::Ok;
            }
            Check::TypeError
        }
        // Resources and engine-internal values are not checked here.
        _ => Check::Ok,
    }
}

/// Check a builtin call's arguments against PHP's signature for it. Raises
/// ArgumentCountError/TypeError as PHP does before the function runs, and
/// reports the deprecations PHP reports while still calling it.
pub(crate) fn check_builtin_call(
    vm: &mut VM,
    sig: &'static Sig,
    args: &mut [Handle],
    strict: bool,
) -> Result<(), VmError> {
    if let Some(message) = deprecation(sig) {
        vm.report_error(ErrorLevel::Deprecated, &message);
    }
    let params = sig.params;
    let variadic = params.last().is_some_and(|p| p.variadic);
    let required = params
        .iter()
        .rposition(|p| !p.optional())
        .map_or(0, |i| i + 1);
    let max = if variadic { usize::MAX } else { params.len() };
    if args.len() < required || args.len() > max {
        let (word, n) = if required == max {
            ("exactly", required)
        } else if args.len() < required {
            ("at least", required)
        } else {
            ("at most", max)
        };
        let msg = format!(
            "{}() expects {} {} argument{}, {} given",
            sig.name,
            word,
            n,
            if n == 1 { "" } else { "s" },
            args.len()
        );
        return Err(vm.throw_error(b"ArgumentCountError", &msg));
    }
    for i in 0..args.len() {
        let handle = args[i];
        let Some(param) = params
            .get(i)
            .or_else(|| params.last().filter(|p| p.variadic))
        else {
            break;
        };
        match check_value(vm, param, handle, strict) {
            Check::Ok => {}
            Check::Deprecated(msg) if msg.is_empty() => {
                let msg = format!(
                    "{}(): Passing null to parameter #{}{} of type {} is deprecated",
                    sig.name,
                    i + 1,
                    arg_label(param),
                    type_to_string(param.mask, param.classes)
                );
                vm.report_error(ErrorLevel::Deprecated, &msg);
            }
            Check::Deprecated(msg) => vm.report_error(ErrorLevel::Deprecated, &msg),
            Check::TypeError => {
                let msg = format!(
                    "{}(): Argument #{}{} must be of type {}, {} given",
                    sig.name,
                    i + 1,
                    arg_label(param),
                    type_to_string(param.mask, param.classes),
                    value_name(vm, handle)
                );
                return Err(vm.throw_error(b"TypeError", &msg));
            }
        }
        if !param.by_ref
            && let Some(coerced) = coerce(vm, param.mask, handle)?
        {
            args[i] = vm.arena.alloc(coerced);
        }
    }
    Ok(())
}

/// " ($name)" after "Argument #N"; PHP names no variadic argument.
/// Reference: $PHP_SRC_PATH/Zend/zend_API.c - get_active_function_arg_name
fn arg_label(param: &Param) -> String {
    if param.variadic {
        String::new()
    } else {
        format!(" (${})", param.name)
    }
}

/// The value a builtin receives for an accepted argument, when it differs
/// from the one passed: weak-mode scalar conversion, preferring int, then
/// float, then string, then bool, as zend_verify_weak_scalar_type_hint does.
/// zend_verify_scalar_type_hint for a user function's argument (or a
/// typed property or return value): `mask` holds the type's scalar T_* bits.
/// Returns the coerced value, or None when the value is not accepted (a
/// TypeError). Null is never coerced for user code; strict mode only
/// widens int to float.
pub(crate) fn coerce_user_scalar(
    vm: &mut VM,
    mask: u32,
    handle: Handle,
    strict: bool,
) -> Result<Option<Val>, VmError> {
    let value = vm.arena.get(handle).value.clone();
    if mask & (T_BOOL | T_TRUE | T_FALSE | T_INT | T_FLOAT | T_STRING) == 0
        || matches!(value, Val::Null)
    {
        return Ok(None);
    }
    if strict {
        return Ok(match value {
            Val::Int(i) if mask & T_FLOAT != 0 => Some(Val::Float(i as f64)),
            _ => None,
        });
    }
    let float_text = |f: f64| crate::core::value::php_float_to_string(f, crate::core::value::FloatPrecision::Shortest, 'E');
    // Type preference order: int -> float -> string -> bool
    if mask & T_INT != 0 {
        match &value {
            Val::String(s) if mask & T_FLOAT != 0 => {
                if let Some(n) = numeric_string(s) {
                    return Ok(Some(n));
                }
            }
            // zend_parse_arg_long_weak
            Val::Float(f) if f.is_finite() && fits_long(*f) => {
                if f.fract() != 0.0 {
                    let message = format!("Implicit conversion from float {} to int loses precision", float_text(*f));
                    vm.report_error(ErrorLevel::Deprecated, &message);
                }
                return Ok(Some(Val::Int(*f as i64)));
            }
            Val::String(s) => match numeric_string(s) {
                Some(Val::Int(i)) => return Ok(Some(Val::Int(i))),
                Some(Val::Float(f)) if fits_long(f) => {
                    if f.fract() != 0.0 {
                        let message = format!(
                            "Implicit conversion from float-string \"{}\" to int loses precision",
                            String::from_utf8_lossy(s)
                        );
                        vm.report_error(ErrorLevel::Deprecated, &message);
                    }
                    return Ok(Some(Val::Int(f as i64)));
                }
                _ => {}
            },
            Val::Bool(b) => return Ok(Some(Val::Int(*b as i64))),
            _ => {}
        }
    }
    if mask & T_FLOAT != 0 {
        match &value {
            Val::Int(i) => return Ok(Some(Val::Float(*i as f64))),
            Val::String(s) => {
                if let Some(n) = numeric_string(s) {
                    return Ok(Some(Val::Float(n.to_float())));
                }
            }
            Val::Bool(b) => return Ok(Some(Val::Float(*b as i64 as f64))),
            _ => {}
        }
    }
    if mask & T_STRING != 0 {
        match &value {
            Val::Int(_) | Val::Float(_) | Val::Bool(_) => {
                return Ok(Some(Val::String(value.to_php_string_bytes().into())));
            }
            Val::Object(payload) => {
                let class = match &vm.arena.get(*payload).value {
                    Val::ObjPayload(obj) => Some(obj.class),
                    _ => None,
                };
                if class.is_some_and(|c| vm.class_has_to_string(c))
                    && let Some(text) = coerce(vm, T_STRING, handle)?
                {
                    return Ok(Some(text));
                }
            }
            _ => {}
        }
    }
    if mask & T_BOOL != 0 && matches!(value, Val::Int(_) | Val::Float(_) | Val::String(_)) {
        return Ok(Some(Val::Bool(value.to_bool())));
    }
    Ok(None)
}

fn coerce(vm: &mut VM, mask: u32, handle: Handle) -> Result<Option<Val>, VmError> {
    // Callables (names, arrays, closures) and untyped parameters pass as is.
    if mask == 0 || mask & (T_MIXED | T_CALLABLE) != 0 {
        return Ok(None);
    }
    let val = vm.arena.get(handle).value.clone();
    let accepts = |bit: u32| mask & bit != 0;
    let already = match &val {
        Val::Null => accepts(T_NULL),
        Val::Bool(true) => accepts(T_BOOL | T_TRUE),
        Val::Bool(false) => accepts(T_BOOL | T_FALSE),
        Val::Int(_) => accepts(T_INT),
        Val::Float(_) => accepts(T_FLOAT),
        Val::String(_) => accepts(T_STRING),
        Val::Object(_) => mask & (T_STRING | T_INT | T_FLOAT | T_BOOL) == 0 || accepts(T_OBJECT),
        _ => true,
    };
    if already {
        return Ok(None);
    }
    let out = match &val {
        Val::Null | Val::Bool(_) => {
            let b = val.to_bool();
            if accepts(T_INT) {
                Val::Int(b as i64)
            } else if accepts(T_FLOAT) {
                Val::Float(b as i64 as f64)
            } else if accepts(T_STRING) {
                Val::String(if b { b"1".to_vec() } else { Vec::new() }.into())
            } else if accepts(T_BOOL | T_TRUE | T_FALSE) {
                Val::Bool(b)
            } else {
                return Ok(None);
            }
        }
        Val::Int(i) => {
            if accepts(T_FLOAT) {
                Val::Float(*i as f64)
            } else if accepts(T_STRING) {
                Val::String(i.to_string().into_bytes().into())
            } else {
                Val::Bool(*i != 0)
            }
        }
        Val::Float(f) => {
            if accepts(T_INT) && fits_long(*f) && (f.fract() == 0.0 || !accepts(T_STRING)) {
                Val::Int(*f as i64)
            } else if accepts(T_STRING) {
                Val::String(val.to_php_string_bytes().into())
            } else {
                Val::Bool(*f != 0.0)
            }
        }
        Val::String(s) => match numeric_string(s) {
            Some(Val::Int(i)) if accepts(T_INT) => Val::Int(i),
            Some(Val::Int(i)) if accepts(T_FLOAT) => Val::Float(i as f64),
            Some(Val::Float(f)) if accepts(T_FLOAT) => Val::Float(f),
            Some(Val::Float(f)) if accepts(T_INT) => Val::Int(f as i64),
            _ => Val::Bool(val.to_bool()),
        },
        Val::Object(_) => Val::String(vm.convert_to_string(handle)?.into()),
        _ => return Ok(None),
    };
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_print_as_php_prints_them() {
        assert_eq!(type_to_string(T_STRING | T_ARRAY, &[]), "array|string");
        assert_eq!(type_to_string(T_INT | T_NULL, &[]), "?int");
        assert_eq!(
            type_to_string(T_INT | T_STRING | T_NULL, &[]),
            "string|int|null"
        );
        assert_eq!(
            type_to_string(T_ARRAY, &["Traversable"]),
            "Traversable|array"
        );
        assert_eq!(type_to_string(T_FALSE | T_TRUE, &[]), "bool");
    }

    #[test]
    fn numeric_strings_follow_php_8() {
        assert_eq!(numeric_string(b" 12"), Some(Val::Int(12)));
        assert_eq!(numeric_string(b"12 "), Some(Val::Int(12)));
        assert_eq!(numeric_string(b"1.5"), Some(Val::Float(1.5)));
        assert_eq!(numeric_string(b"1e3"), Some(Val::Float(1000.0)));
        assert_eq!(numeric_string(b"12abc"), None);
        assert_eq!(numeric_string(b""), None);
        assert_eq!(numeric_string(b"abc"), None);
    }

    #[test]
    fn signatures_come_from_the_stubs() {
        let sig = lookup(b"STRLEN").expect("strlen");
        assert_eq!(sig.params.len(), 1);
        assert_eq!(sig.params[0].name, "string");
        assert_eq!(sig.params[0].mask, T_STRING);
        let sort = lookup(b"sort").expect("sort");
        assert!(sort.params[0].by_ref);
        assert_eq!(sort.params[1].default, Some("SORT_REGULAR"));
    }
}

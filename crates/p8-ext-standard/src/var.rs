//! ext/standard's variable functions: types, conversions, print_r and
//! var_export. Arguments arrive already checked and converted to their
//! declared types (p8_runtime::args).
//!
//! Reference: $PHP_SRC_PATH/ext/standard/type.c, var.c;
//! $PHP_SRC_PATH/Zend/zend.c - print_hash, zend_print_zval_r_to_buf

use crate::{arg, str_arg};
use p8_runtime::array::Key;
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::numeric::numeric_string;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "gettype", run: gettype, by_ref: &[] },
    Builtin { name: "get_debug_type", run: get_debug_type, by_ref: &[] },
    Builtin { name: "is_null", run: is_null, by_ref: &[] },
    Builtin { name: "is_bool", run: is_bool, by_ref: &[] },
    Builtin { name: "is_int", run: is_int, by_ref: &[] },
    Builtin { name: "is_integer", run: is_int, by_ref: &[] },
    Builtin { name: "is_long", run: is_int, by_ref: &[] },
    Builtin { name: "is_float", run: is_float, by_ref: &[] },
    Builtin { name: "is_double", run: is_float, by_ref: &[] },
    Builtin { name: "is_string", run: is_string, by_ref: &[] },
    Builtin { name: "is_array", run: is_array, by_ref: &[] },
    Builtin { name: "is_scalar", run: is_scalar, by_ref: &[] },
    Builtin { name: "is_numeric", run: is_numeric, by_ref: &[] },
    Builtin { name: "is_iterable", run: is_array, by_ref: &[] },
    Builtin { name: "is_countable", run: is_array, by_ref: &[] },
    Builtin { name: "intval", run: intval, by_ref: &[] },
    Builtin { name: "floatval", run: floatval, by_ref: &[] },
    Builtin { name: "doubleval", run: floatval, by_ref: &[] },
    Builtin { name: "strval", run: strval, by_ref: &[] },
    Builtin { name: "boolval", run: boolval, by_ref: &[] },
    Builtin { name: "print_r", run: print_r, by_ref: &[] },
    Builtin { name: "var_export", run: var_export, by_ref: &[] },
];

fn gettype(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str(match arg(args, 0) {
        Value::Null | Value::Undef => "NULL",
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Float(_) => "double",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Ref(_) => unreachable!("arg() looks through references"),
    }))
}

fn get_debug_type(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str(match arg(args, 0) {
        Value::Null | Value::Undef => "null",
        Value::Bool(_) => "bool",
        other => other.type_name(),
    }))
}

fn is_null(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Null | Value::Undef)))
}

fn is_bool(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Bool(_))))
}

fn is_int(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Int(_))))
}

fn is_float(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Float(_))))
}

fn is_string(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Str(_))))
}

fn is_array(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Array(_))))
}

fn is_scalar(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_))))
}

/// An int or float, or a string that is entirely a number (whitespace
/// allowed around it, as of PHP 8).
fn is_numeric(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(match arg(args, 0) {
        Value::Int(_) | Value::Float(_) => true,
        Value::Str(s) => numeric_string(s.as_bytes()).is_some(),
        _ => false,
    }))
}

/// intval($value, $base = 10): a string in another base is read as
/// strtol does (prefixes 0x, 0o, 0b and a leading 0 for base 0 / 16 / 8 / 2).
fn intval(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let base = if args.len() > 1 { arg(args, 1).to_int() } else { 10 };
    let Value::Str(s) = &v else { return Ok(Value::Int(v.to_int())) };
    if base == 10 {
        return Ok(Value::Int(v.to_int()));
    }
    Ok(Value::Int(strtol(s.as_bytes(), base)))
}

/// C's strtol, saturating at the int range, as ZEND_STRTOL.
fn strtol(s: &[u8], base: i64) -> i64 {
    let mut i = s.iter().take_while(|c| c.is_ascii_whitespace()).count();
    let negative = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let rest = &s[i..];
    let prefixed = |p: u8| rest.len() > 2 && rest[0] == b'0' && rest[1].eq_ignore_ascii_case(&p);
    let (base, mut digits) = match base {
        16 if prefixed(b'x') => (16, &rest[2..]),
        8 if prefixed(b'o') => (8, &rest[2..]),
        2 if prefixed(b'b') => (2, &rest[2..]),
        0 if prefixed(b'x') => (16, &rest[2..]),
        0 if prefixed(b'o') => (8, &rest[2..]),
        0 if prefixed(b'b') => (2, &rest[2..]),
        0 if rest.first() == Some(&b'0') => (8, &rest[1..]),
        0 => (10, rest),
        b => (b, rest),
    };
    if !(2..=36).contains(&base) {
        return 0;
    }
    let mut n: i64 = 0;
    let mut overflow = false;
    while let Some((&c, tail)) = digits.split_first() {
        let d = match c {
            b'0'..=b'9' => i64::from(c - b'0'),
            b'a'..=b'z' => i64::from(c - b'a') + 10,
            b'A'..=b'Z' => i64::from(c - b'A') + 10,
            _ => break,
        };
        if d >= base {
            break;
        }
        match n.checked_mul(base).and_then(|n| if negative { n.checked_sub(d) } else { n.checked_add(d) }) {
            Some(next) => n = next,
            None => overflow = true,
        }
        digits = tail;
    }
    if overflow {
        return if negative { i64::MIN } else { i64::MAX };
    }
    n
}

fn floatval(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Float(arg(args, 0).to_float()))
}

fn strval(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Str(str_arg(args, 0)))
}

fn boolval(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(arg(args, 0).to_bool()))
}

/// print_r($value, $return = false)
fn print_r(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    print_r_to(&arg(args, 0), 0, &mut out);
    if args.len() > 1 && arg(args, 1).to_bool() {
        return Ok(Value::Str(PhpStr::new(out)));
    }
    ctx.echo(&out);
    Ok(Value::Bool(true))
}

fn print_r_to(v: &Value, indent: usize, out: &mut Vec<u8>) {
    match v {
        Value::Array(a) => {
            out.extend_from_slice(b"Array\n");
            out.extend(std::iter::repeat_n(b' ', indent));
            out.extend_from_slice(b"(\n");
            for (k, v) in a.iter() {
                out.extend(std::iter::repeat_n(b' ', indent + 4));
                out.push(b'[');
                match k {
                    Key::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
                    Key::Str(s) => out.extend_from_slice(s.as_bytes()),
                }
                out.extend_from_slice(b"] => ");
                print_r_to(v, indent + 8, out);
                out.push(b'\n');
            }
            out.extend(std::iter::repeat_n(b' ', indent));
            out.extend_from_slice(b")\n");
        }
        Value::Ref(cell) => print_r_to(&cell.borrow(), indent, out),
        other => {
            if let Some(s) = other.to_php_string() {
                out.extend_from_slice(s.as_bytes());
            }
        }
    }
}

/// var_export($value, $return = false)
fn var_export(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    export(&arg(args, 0), 1, &mut out);
    if args.len() > 1 && arg(args, 1).to_bool() {
        return Ok(Value::Str(PhpStr::new(out)));
    }
    ctx.echo(&out);
    Ok(Value::Null)
}

/// php_var_export_ex
fn export(v: &Value, level: usize, out: &mut Vec<u8>) {
    match v {
        Value::Null | Value::Undef => out.extend_from_slice(b"NULL"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        // PHP_INT_MIN has no literal: written as an expression.
        Value::Int(i64::MIN) => out.extend_from_slice(b"-9223372036854775807-1"),
        Value::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Value::Float(f) => {
            let text = float_to_string(*f, Precision::SERIALIZE, 'E');
            out.extend_from_slice(text.as_bytes());
            if f.is_finite() && !text.contains(['.', 'E', 'e']) {
                out.extend_from_slice(b".0");
            }
        }
        Value::Str(s) => export_string(s.as_bytes(), out),
        Value::Array(a) => {
            if level > 1 {
                out.push(b'\n');
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.extend_from_slice(b"array (\n");
            for (k, v) in a.iter() {
                out.extend(std::iter::repeat_n(b' ', level + 1));
                match k {
                    Key::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
                    Key::Str(s) => export_string(s.as_bytes(), out),
                }
                out.extend_from_slice(b" => ");
                export(v, level + 2, out);
                out.extend_from_slice(b",\n");
            }
            if level > 1 {
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.push(b')');
        }
        Value::Ref(cell) => export(&cell.borrow(), level, out),
    }
}

/// A string as var_export writes it: single-quoted with \ and ' escaped,
/// and NUL bytes spliced in as `' . "\0" . '`.
fn export_string(s: &[u8], out: &mut Vec<u8>) {
    out.push(b'\'');
    for &c in s {
        match c {
            b'\'' | b'\\' => {
                out.push(b'\\');
                out.push(c);
            }
            0 => out.extend_from_slice(b"' . \"\\0\" . '"),
            c => out.push(c),
        }
    }
    out.push(b'\'');
}

#[cfg(test)]
mod tests {
    use super::strtol;

    #[test]
    fn strtol_reads_prefixes_and_saturates() {
        assert_eq!(strtol(b"0x1A", 16), 26);
        assert_eq!(strtol(b"0x1A", 0), 26);
        assert_eq!(strtol(b"012", 0), 10);
        assert_eq!(strtol(b"  -101", 2), -5);
        assert_eq!(strtol(b"zz", 36), 1295);
        assert_eq!(strtol(b"99999999999999999999", 10), i64::MAX);
        assert_eq!(strtol(b"12abc", 16), 0x12abc);
    }
}

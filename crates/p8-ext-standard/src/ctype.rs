//! ctype: the C library's character classes in the C locale, over a
//! string's bytes. An int is (deprecated) taken as a character code
//! (-128..255) or else as its digits.
//!
//! Reference: $PHP_SRC_PATH/ext/ctype/ctype.c

use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::value::Value;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "ctype_alnum", run: ctype_alnum, by_ref: &[] },
    Builtin { name: "ctype_alpha", run: ctype_alpha, by_ref: &[] },
    Builtin { name: "ctype_cntrl", run: ctype_cntrl, by_ref: &[] },
    Builtin { name: "ctype_digit", run: ctype_digit, by_ref: &[] },
    Builtin { name: "ctype_lower", run: ctype_lower, by_ref: &[] },
    Builtin { name: "ctype_graph", run: ctype_graph, by_ref: &[] },
    Builtin { name: "ctype_print", run: ctype_print, by_ref: &[] },
    Builtin { name: "ctype_punct", run: ctype_punct, by_ref: &[] },
    Builtin { name: "ctype_space", run: ctype_space, by_ref: &[] },
    Builtin { name: "ctype_upper", run: ctype_upper, by_ref: &[] },
    Builtin { name: "ctype_xdigit", run: ctype_xdigit, by_ref: &[] },
];

/// Whether every byte of the string argument is in the class; for an
/// int, the deprecated fallback (`digits`/`minus`: the answer for a
/// number outside -128..255, positive or negative).
fn ctype(ctx: &mut dyn Ctx, func: &str, args: &[Value], is: fn(u8) -> bool, digits: bool, minus: bool) -> Result<Value, Throw> {
    let v = args.first().map(Value::deref).unwrap_or(Value::NULL);
    if let Some(s) = v.as_str() {
        return Ok(Value::bool(!s.is_empty() && s.as_bytes().iter().all(|&c| is(c))));
    }
    ctx.report(8192, &format!("{func}(): Argument of type {} will be interpreted as string in the future", v.type_name()));
    Ok(Value::bool(match v.as_int() {
        Some(n @ 0..=255) => is(n as u8),
        Some(n @ -128..=-1) => is((n + 256) as u8),
        Some(n) if n >= 0 => digits,
        Some(_) => minus,
        None => false,
    }))
}

fn ctype_alnum(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_alnum", args, |c| c.is_ascii_alphanumeric(), true, false)
}

fn ctype_alpha(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_alpha", args, |c| c.is_ascii_alphabetic(), false, false)
}

fn ctype_cntrl(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_cntrl", args, |c| c.is_ascii_control(), false, false)
}

fn ctype_digit(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_digit", args, |c| c.is_ascii_digit(), true, false)
}

fn ctype_lower(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_lower", args, |c| c.is_ascii_lowercase(), false, false)
}

fn ctype_graph(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_graph", args, |c| c.is_ascii_graphic(), true, true)
}

fn ctype_print(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_print", args, |c| c.is_ascii_graphic() || c == b' ', true, true)
}

fn ctype_punct(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_punct", args, |c| c.is_ascii_punctuation(), false, false)
}

fn ctype_space(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    // C's isspace: space, \t \n \v \f \r.
    ctype(ctx, "ctype_space", args, |c| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'), false, false)
}

fn ctype_upper(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_upper", args, |c| c.is_ascii_uppercase(), false, false)
}

fn ctype_xdigit(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctype(ctx, "ctype_xdigit", args, |c| c.is_ascii_hexdigit(), true, false)
}

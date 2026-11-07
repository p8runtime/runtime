//! The printf family's formatter, as php_formatted_print(): argument
//! numbers (`%2$s`), flags (`-+ 0` and `'c` padding), width and precision
//! (also from arguments, `*`), and the s d u c e E f F g G h H o x X b
//! conversions, with PHP's errors for what it rejects.
//!
//! Floats: f/F and e/E are zend_dtoa's correctly rounded digits (Rust's
//! formatting gives the same ones), laid out as php_conv_fp() does; g/G
//! and h/H are zend_gcvt() (number::float_to_string).
//!
//! Reference: $PHP_SRC_PATH/ext/standard/formatted_print.c,
//! $PHP_SRC_PATH/main/snprintf.c (php_conv_fp)

use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

const FLOAT_PRECISION: usize = 6;
const MAX_FLOAT_PRECISION: usize = 53;
const INT_MAX: i64 = i32::MAX as i64;

/// How missing arguments are reported: an ArgumentCountError counting the
/// function's parameters before the values (`Count(n)`), or, for the v*
/// functions whose values come in an array, a ValueError (`Array`).
#[derive(Clone, Copy)]
pub enum Missing {
    Count(usize),
    Array,
}

/// Where a field goes and how it is padded.
struct Field {
    width: usize,
    padding: u8,
    left: bool,
    always_sign: bool,
}

/// php_sprintf_appendstring(): `s` (cut to `max` when a precision was
/// given) padded to the field's width; a sign stays in front of zeros.
fn append(out: &mut Vec<u8>, s: &[u8], f: &Field, max: Option<usize>, neg: bool) {
    let copy = max.map_or(s.len(), |m| m.min(s.len()));
    let npad = f.width.saturating_sub(copy);
    let mut s = &s[..copy];
    if !f.left {
        if (neg || f.always_sign) && f.padding == b'0' && !s.is_empty() {
            out.push(if neg { b'-' } else { b'+' });
            s = &s[1..];
        }
        out.extend(std::iter::repeat_n(f.padding, npad));
        out.extend_from_slice(s);
    } else {
        out.extend_from_slice(s);
        out.extend(std::iter::repeat_n(f.padding, npad));
    }
}

/// php_sprintf_appendint() and appenduint(): zeros never pad on the right,
/// and an unsigned number has no sign (`+` is ignored).
fn append_int(out: &mut Vec<u8>, n: i64, f: &Field, unsigned: bool) {
    let f = Field {
        padding: if f.left && f.padding == b'0' { b' ' } else { f.padding },
        always_sign: f.always_sign && !unsigned,
        ..*f
    };
    let (text, neg) = if unsigned {
        ((n as u64).to_string(), false)
    } else if n < 0 {
        (n.to_string(), true)
    } else if f.always_sign {
        (format!("+{n}"), false)
    } else {
        (n.to_string(), false)
    };
    append(out, text.as_bytes(), &f, None, neg);
}

/// php_sprintf_append2n(): binary, octal or hex digits of the bits.
fn append_2n(out: &mut Vec<u8>, n: i64, f: &Field, conv: u8) {
    let n = n as u64;
    let text = match conv {
        b'b' => format!("{n:b}"),
        b'o' => format!("{n:o}"),
        b'x' => format!("{n:x}"),
        _ => format!("{n:X}"),
    };
    append(out, text.as_bytes(), f, None, false);
}

/// php_sprintf_appenddouble().
fn append_double(ctx: &mut dyn Ctx, fname: &str, out: &mut Vec<u8>, x: f64, f: &Field, precision: Option<i64>, conv: u8) {
    let mut precision = match precision {
        None => FLOAT_PRECISION as i64,
        Some(p) if p > MAX_FLOAT_PRECISION as i64 => {
            ctx.notice(&format!("{fname}(): Requested precision of {p} digits was truncated to PHP maximum of {MAX_FLOAT_PRECISION} digits"));
            MAX_FLOAT_PRECISION as i64
        }
        Some(p) => p,
    };
    if x.is_nan() {
        append(out, b"NaN", &Field { width: 3, ..*f }, None, false);
        return;
    }
    if x.is_infinite() {
        let s: &[u8] = if x < 0.0 { b"-INF" } else { b"INF" };
        append(out, s, &Field { width: s.len(), ..*f }, None, x < 0.0);
        return;
    }
    let (text, neg) = match conv {
        b'e' | b'E' | b'f' | b'F' => {
            let p = precision as usize;
            // (A zero has no sign here: __cvt() clears it.)
            let neg = x < 0.0;
            let body = if conv == b'f' || conv == b'F' {
                format!("{:.*}", p, x.abs())
            } else {
                // php_conv_fp: d.ddd, the exponent signed and unpadded.
                let s = format!("{:.*e}", p, x.abs());
                let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
                let exp: i32 = exp.parse().unwrap_or(0);
                let e = if conv == b'E' { 'E' } else { 'e' };
                format!("{mantissa}{e}{}{}", if exp < 0 { '-' } else { '+' }, exp.abs())
            };
            if neg {
                (format!("-{body}"), true)
            } else if f.always_sign {
                (format!("+{body}"), false)
            } else {
                (body, false)
            }
        }
        _ => {
            // g G h H: zend_gcvt(); precision 0 means 1, -1 the shortest.
            if precision == 0 {
                precision = 1;
            }
            let exp = if conv == b'G' || conv == b'H' { 'E' } else { 'e' };
            let digits = if precision < 0 { Precision::Shortest } else { Precision::Digits(precision as usize) };
            let s = float_to_string(x, digits, exp);
            match s.strip_prefix('-') {
                Some(_) => (s, true),
                None if f.always_sign => (format!("+{s}"), false),
                None => (s, false),
            }
        }
    };
    append(out, text.as_bytes(), f, None, neg);
}

/// php_sprintf_get_argnum(): a "n$" argument number (0-based), None when
/// there is none; an error for 0 or too large.
fn argnum(ctx: &mut dyn Ctx, fmt: &[u8], i: &mut usize) -> Result<Option<usize>, Throw> {
    let digits = fmt[*i..].iter().take_while(|c| c.is_ascii_digit()).count();
    if fmt.get(*i + digits) != Some(&b'$') {
        return Ok(None);
    }
    let n = number(fmt, i);
    if n <= 0 {
        return Err(ctx.throw("ValueError", format!("Argument number specifier must be greater than zero and less than {INT_MAX}")));
    }
    *i += 1; // the '$'
    Ok(Some(n as usize - 1))
}

/// php_sprintf_getnumber(): the digits at `i` (-1 when out of int range).
fn number(fmt: &[u8], i: &mut usize) -> i64 {
    let start = *i;
    while fmt.get(*i).is_some_and(u8::is_ascii_digit) {
        *i += 1;
    }
    let n = std::str::from_utf8(&fmt[start..*i]).ok().and_then(|t| t.parse::<i64>().ok()).unwrap_or(i64::MAX);
    if !(0..INT_MAX).contains(&n) { -1 } else { n }
}

/// The values, the next one a specifier without a number takes, and the
/// highest number asked for past the end (for the error at the end).
struct Args<'a> {
    values: &'a [Value],
    next: usize,
    missing: Option<usize>,
}

impl<'a> Args<'a> {
    /// Argument `n` (None: the next one); None when there is no such
    /// argument, which is then counted as missing.
    fn take(&mut self, n: Option<usize>) -> Option<&'a Value> {
        let n = n.unwrap_or_else(|| {
            self.next += 1;
            self.next - 1
        });
        let v = self.values.get(n);
        if v.is_none() {
            self.missing = Some(self.missing.map_or(n, |m| m.max(n)));
        }
        v
    }
}

/// A `*` width or precision: the integer argument it takes. Ok(None): the
/// argument is missing.
fn star(ctx: &mut dyn Ctx, fmt: &[u8], i: &mut usize, args: &mut Args, what: &str, min: i64) -> Result<Option<i64>, Throw> {
    let n = argnum(ctx, fmt, i)?;
    let Some(v) = args.take(n) else { return Ok(None) };
    let Some(v) = v.deref().as_int() else {
        return Err(ctx.throw("ValueError", format!("{what} must be an integer")));
    };
    if v < min || v > INT_MAX {
        return Err(ctx.throw("ValueError", format!("{what} must be between {min} and {INT_MAX}")));
    }
    Ok(Some(v))
}

/// zval_get_tmp_string(): a string, an object's __toString(), or "Array"
/// with a warning.
pub(crate) fn to_str(ctx: &mut dyn Ctx, v: &Value) -> Result<PhpStr, Throw> {
    let v = v.deref();
    if v.is_array() {
        ctx.warn("Array to string conversion");
        return Ok(PhpStr::from("Array"));
    }
    if let Some(o) = v.as_object() {
        if o.class.method(b"__tostring").is_none() {
            return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(o.class.name.as_bytes()))));
        }
        let method = Value::array(PhpArray::from_list(vec![Value::object(o.clone()), Value::str("__toString")]));
        return Ok(ctx.call(&method, &mut [])?.to_php_string().unwrap_or_else(PhpStr::empty));
    }
    Ok(v.to_php_string().unwrap_or_else(PhpStr::empty))
}

/// php_formatted_print(): `format` with `args`; `fname` for messages.
pub fn formatted(ctx: &mut dyn Ctx, fname: &str, format: &[u8], args: &[Value], missing_as: Missing) -> Result<Vec<u8>, Throw> {
    let fmt = format;
    let mut out = Vec::with_capacity(fmt.len() + 16);
    let mut i = 0;
    let mut args = Args { values: args, next: 0, missing: None };
    while i < fmt.len() {
        let Some(pct) = fmt[i..].iter().position(|&c| c == b'%') else {
            out.extend_from_slice(&fmt[i..]);
            break;
        };
        out.extend_from_slice(&fmt[i..i + pct]);
        i += pct + 1;
        if fmt.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        let mut f = Field { width: 0, padding: b' ', left: false, always_sign: false };
        let mut precision: Option<i64> = None;
        let mut n = None;
        if !fmt.get(i).is_some_and(u8::is_ascii_alphabetic) {
            n = argnum(ctx, fmt, &mut i)?;
            loop {
                match fmt.get(i) {
                    Some(&c @ (b' ' | b'0')) => f.padding = c,
                    Some(b'-') => f.left = true,
                    Some(b'+') => f.always_sign = true,
                    Some(b'\'') => {
                        if fmt.len() - i > 1 {
                            i += 1;
                            f.padding = fmt[i];
                        } else {
                            return Err(ctx.throw("ValueError", "Missing padding character".into()));
                        }
                    }
                    _ => break,
                }
                i += 1;
            }
            if fmt.get(i) == Some(&b'*') {
                i += 1;
                match star(ctx, fmt, &mut i, &mut args, "Width", 0)? {
                    Some(w) => f.width = w as usize,
                    None => continue,
                }
            } else if fmt.get(i).is_some_and(u8::is_ascii_digit) {
                let w = number(fmt, &mut i);
                if w < 0 {
                    return Err(ctx.throw("ValueError", format!("Width must be between 0 and {INT_MAX}")));
                }
                f.width = w as usize;
            }
            if fmt.get(i) == Some(&b'.') {
                i += 1;
                if fmt.get(i) == Some(&b'*') {
                    i += 1;
                    match star(ctx, fmt, &mut i, &mut args, "Precision", -1)? {
                        Some(p) => precision = Some(p),
                        None => continue,
                    }
                } else if fmt.get(i).is_some_and(u8::is_ascii_digit) {
                    let p = number(fmt, &mut i);
                    if p < 0 {
                        return Err(ctx.throw("ValueError", format!("Precision must be between 0 and {INT_MAX}")));
                    }
                    precision = Some(p);
                } else {
                    precision = Some(0);
                }
            }
        }
        if fmt.get(i) == Some(&b'l') {
            i += 1;
        }
        let Some(v) = args.take(n) else { continue };
        let conv = fmt.get(i).copied();
        if precision == Some(-1) && !matches!(conv, Some(b'g' | b'G' | b'h' | b'H')) {
            return Err(ctx.throw("ValueError", "Precision -1 is only supported for %g, %G, %h and %H".into()));
        }
        match conv {
            Some(b's') => {
                let s = to_str(ctx, v)?;
                append(&mut out, s.as_bytes(), &f, precision.map(|p| p as usize), false);
            }
            Some(b'd') => append_int(&mut out, v.to_int(), &f, false),
            Some(b'u') => append_int(&mut out, v.to_int(), &f, true),
            Some(c @ (b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'h' | b'H')) => append_double(ctx, fname, &mut out, v.to_float(), &f, precision, c),
            Some(b'c') => out.push(v.to_int() as u8),
            Some(c @ (b'o' | b'x' | b'X' | b'b')) => {
                // A precision cuts the digits (expprec), as for strings.
                let mut digits = Vec::new();
                append_2n(&mut digits, v.to_int(), &Field { width: 0, ..f }, c);
                append(&mut out, &digits, &f, precision.map(|p| p as usize), false);
            }
            Some(b'%') => out.push(b'%'),
            None => return Err(ctx.throw("ValueError", "Missing format specifier at end of string".into())),
            Some(c) => return Err(ctx.throw("ValueError", format!("Unknown format specifier \"{}\"", c as char))),
        }
        i += 1;
    }
    if let Some(m) = args.missing {
        let given = args.values.len();
        return Err(match missing_as {
            Missing::Array => ctx.throw("ValueError", format!("The arguments array must contain {} items, {given} given", m + 1)),
            Missing::Count(before) => ctx.throw("ArgumentCountError", format!("{} arguments are required, {} given", m + before + 1, given + before)),
        });
    }
    Ok(out)
}

/// The values of a v*printf() array, in order.
pub fn array_values(v: &Value) -> Vec<Value> {
    match v.as_array() {
        Some(a) => a.values().cloned().collect(),
        None => Vec::new(),
    }
}

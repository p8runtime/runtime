//! The filter extension: filter_var, filter_var_array, filter_input,
//! filter_input_array, filter_has_var, filter_list, filter_id, with the
//! validating filters (int, bool, float, regexp, domain, url, email, ip,
//! mac), the sanitizing ones and FILTER_CALLBACK.
//!
//! The regular expressions (FILTER_VALIDATE_REGEXP, the e-mail pattern)
//! run through preg_match, a URL is split by parse_url, and
//! FILTER_SANITIZE_FULL_SPECIAL_CHARS is htmlentities: the same code PHP
//! calls. FILTER_SANITIZE_STRING (deprecated) strips tags with
//! strip_tags(), which reads "< b" as text where PHP's filter reads a tag.
//!
//! As in the CLI, there is no request input: INPUT_GET, INPUT_POST and
//! INPUT_COOKIE have no variables; INPUT_SERVER and INPUT_ENV are $_SERVER
//! and $_ENV.
//!
//! Reference: $PHP_SRC_PATH/ext/filter

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::numeric::{Number, numeric_string};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "filter_var", run: filter_var, by_ref: &[] },
    Builtin { name: "filter_var_array", run: filter_var_array, by_ref: &[] },
    Builtin { name: "filter_input", run: filter_input, by_ref: &[] },
    Builtin { name: "filter_input_array", run: filter_input_array, by_ref: &[] },
    Builtin { name: "filter_has_var", run: filter_has_var, by_ref: &[] },
    Builtin { name: "filter_list", run: filter_list, by_ref: &[] },
    Builtin { name: "filter_id", run: filter_id, by_ref: &[] },
];

/// The builtins that call PHP code (callbacks, __toString, preg_match) or
/// read the globals.
pub const REENTERING: &[&str] = &["filter_var", "filter_var_array", "filter_input", "filter_input_array", "filter_has_var"];

const REQUIRE_ARRAY: i64 = 0x0100_0000;
const REQUIRE_SCALAR: i64 = 0x0200_0000;
const FORCE_ARRAY: i64 = 0x0400_0000;
const NULL_ON_FAILURE: i64 = 0x0800_0000;
const ALLOW_OCTAL: i64 = 0x0001;
const ALLOW_HEX: i64 = 0x0002;
const STRIP_LOW: i64 = 0x0004;
const STRIP_HIGH: i64 = 0x0008;
const ENCODE_LOW: i64 = 0x0010;
const ENCODE_HIGH: i64 = 0x0020;
const ENCODE_AMP: i64 = 0x0040;
const NO_ENCODE_QUOTES: i64 = 0x0080;
const EMPTY_STRING_NULL: i64 = 0x0100;
const STRIP_BACKTICK: i64 = 0x0200;
const ALLOW_FRACTION: i64 = 0x1000;
const ALLOW_THOUSAND: i64 = 0x2000;
const ALLOW_SCIENTIFIC: i64 = 0x4000;
const PATH_REQUIRED: i64 = 0x04_0000;
const QUERY_REQUIRED: i64 = 0x08_0000;
const FLAG_IPV4: i64 = 0x0010_0000;
const FLAG_IPV6: i64 = 0x0020_0000;
const NO_RES_RANGE: i64 = 0x0040_0000;
const NO_PRIV_RANGE: i64 = 0x0080_0000;
// (0x1000_0000 until 8.5, which gave that bit to FILTER_THROW_ON_FAILURE)
const GLOBAL_RANGE: i64 = 0x2000_0000;
const FLAG_HOSTNAME: i64 = 0x10_0000;
const EMAIL_UNICODE: i64 = 0x10_0000;

const VALIDATE_INT: i64 = 0x0101;
const VALIDATE_BOOL: i64 = 0x0102;
const VALIDATE_FLOAT: i64 = 0x0103;
const VALIDATE_REGEXP: i64 = 0x0110;
const VALIDATE_URL: i64 = 0x0111;
const VALIDATE_EMAIL: i64 = 0x0112;
const VALIDATE_IP: i64 = 0x0113;
const VALIDATE_MAC: i64 = 0x0114;
const VALIDATE_DOMAIN: i64 = 0x0115;
const DEFAULT: i64 = 0x0204;
const SANITIZE_STRING: i64 = 0x0201;
const SANITIZE_ENCODED: i64 = 0x0202;
const SANITIZE_SPECIAL_CHARS: i64 = 0x0203;
const SANITIZE_EMAIL: i64 = 0x0205;
const SANITIZE_URL: i64 = 0x0206;
const SANITIZE_NUMBER_INT: i64 = 0x0207;
const SANITIZE_NUMBER_FLOAT: i64 = 0x0208;
const SANITIZE_FULL_SPECIAL_CHARS: i64 = 0x020a;
const SANITIZE_ADD_SLASHES: i64 = 0x020b;
const CALLBACK: i64 = 0x0400;

/// filter_list(), in PHP's order: name and id.
const FILTERS: [(&str, i64); 21] = [
    ("int", VALIDATE_INT),
    ("boolean", VALIDATE_BOOL),
    ("float", VALIDATE_FLOAT),
    ("validate_regexp", VALIDATE_REGEXP),
    ("validate_domain", VALIDATE_DOMAIN),
    ("validate_url", VALIDATE_URL),
    ("validate_email", VALIDATE_EMAIL),
    ("validate_ip", VALIDATE_IP),
    ("validate_mac", VALIDATE_MAC),
    ("string", SANITIZE_STRING),
    ("stripped", SANITIZE_STRING),
    ("encoded", SANITIZE_ENCODED),
    ("special_chars", SANITIZE_SPECIAL_CHARS),
    ("full_special_chars", SANITIZE_FULL_SPECIAL_CHARS),
    ("unsafe_raw", DEFAULT),
    ("email", SANITIZE_EMAIL),
    ("url", SANITIZE_URL),
    ("number_int", SANITIZE_NUMBER_INT),
    ("number_float", SANITIZE_NUMBER_FLOAT),
    ("add_slashes", SANITIZE_ADD_SLASHES),
    ("callback", CALLBACK),
];

/// PHP_FILTER_ID_EXISTS.
fn exists(id: i64) -> bool {
    (0x0200..=SANITIZE_ADD_SLASHES).contains(&id) || (0x0100..=VALIDATE_DOMAIN).contains(&id) || id == CALLBACK
}

/// What a failed validation leaves: false, or null with FILTER_NULL_ON_FAILURE.
fn failed(flags: i64) -> Value {
    if flags & NULL_ON_FAILURE != 0 { Value::NULL } else { Value::FALSE }
}

/// An option from the options array.
fn option(options: Option<&Value>, name: &str) -> Option<Value> {
    options?.as_array()?.get(&Key::from_string(PhpStr::from(name))).map(Value::deref)
}

fn string_option(options: Option<&Value>, name: &str) -> Option<Vec<u8>> {
    option(options, name)?.as_str().map(|s| s.as_bytes().to_vec())
}

/// PHP_FILTER_TRIM_DEFAULT: spaces, tabs, CR, LF and VT off both ends.
fn trim(s: &[u8]) -> &[u8] {
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\r' | b'\x0b' | b'\n');
    let start = s.iter().position(|c| !ws(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|c| !ws(c)).map_or(start, |i| i + 1);
    &s[start..end]
}

/// php_zval_filter: `value` (a scalar or object) through filter `id`, then
/// the "default" option when it failed.
fn apply(ctx: &mut dyn Ctx, value: Value, id: i64, flags: i64, options: Option<&Value>) -> Result<Value, Throw> {
    let id = if FILTERS.iter().any(|(_, f)| *f == id) { id } else { DEFAULT };
    let result = match value {
        v if v.as_object().is_some_and(|o| o.class.method(b"__tostring").is_none()) => failed(flags),
        v => {
            let s = crate::printf::to_str(ctx, &v)?;
            run(ctx, id, s, flags, options)?
        }
    };
    let failure = if flags & NULL_ON_FAILURE != 0 { result.is_null() } else { result.as_bool() == Some(false) };
    if failure
        && options.is_some_and(Value::is_array)
        && let Some(default) = option(options, "default")
    {
        return Ok(default);
    }
    Ok(result)
}

fn run(ctx: &mut dyn Ctx, id: i64, s: PhpStr, flags: i64, options: Option<&Value>) -> Result<Value, Throw> {
    let b = s.as_bytes();
    Ok(match id {
        VALIDATE_INT => validate_int(b, flags, options),
        VALIDATE_BOOL => validate_bool(b, flags),
        VALIDATE_FLOAT => validate_float(ctx, b, flags, options)?,
        VALIDATE_REGEXP => {
            let Some(re) = option(options, "regexp").and_then(|v| v.into_str().ok()) else {
                let name = ctx.builtin_name();
                return Err(ctx.throw("ValueError", format!("{name}(): \"regexp\" option is missing")));
            };
            if matches(ctx, Value::str(re), &s)? { Value::str(s) } else { failed(flags) }
        }
        VALIDATE_DOMAIN => {
            if valid_domain(b, flags) {
                Value::str(s)
            } else {
                failed(flags)
            }
        }
        VALIDATE_URL => validate_url(ctx, s, flags)?,
        VALIDATE_EMAIL => {
            let pattern = if flags & EMAIL_UNICODE != 0 { EMAIL_UNICODE_RE } else { EMAIL_RE };
            if b.len() <= 320 && matches(ctx, Value::str(pattern), &s)? { Value::str(s) } else { failed(flags) }
        }
        VALIDATE_IP => {
            if valid_ip(b, flags) {
                Value::str(s)
            } else {
                failed(flags)
            }
        }
        VALIDATE_MAC => validate_mac(ctx, s, flags, options)?,
        SANITIZE_STRING => {
            let mut enc = [false; 256];
            if flags & NO_ENCODE_QUOTES == 0 {
                enc[b'\'' as usize] = true;
                enc[b'"' as usize] = true;
            }
            encode_flags(&mut enc, flags);
            let stripped = encode_html(&strip(b, flags), &enc);
            let out = ctx.call(&Value::str("strip_tags"), &mut [Value::str(PhpStr::new(stripped))])?;
            match out {
                t if t.as_str().is_some_and(|t| t.is_empty()) && flags & EMPTY_STRING_NULL != 0 => Value::NULL,
                other => other,
            }
        }
        SANITIZE_ENCODED => Value::str(PhpStr::new(encode_url(&strip(b, flags)))),
        SANITIZE_SPECIAL_CHARS => {
            let mut enc = [false; 256];
            for &c in b"'\"<>&" {
                enc[c as usize] = true;
            }
            enc[..32].fill(true);
            if flags & ENCODE_HIGH != 0 {
                enc[127..].fill(true);
            }
            Value::str(PhpStr::new(encode_html(&strip(b, flags), &enc)))
        }
        SANITIZE_FULL_SPECIAL_CHARS => {
            let quotes = if flags & NO_ENCODE_QUOTES == 0 { 3 } else { 0 };
            ctx.call(&Value::str("htmlentities"), &mut [Value::str(s), Value::int(quotes), Value::NULL, Value::FALSE])?
        }
        SANITIZE_EMAIL => keep(b, |c| c.is_ascii_alphanumeric() || b"!#$%&'*+-=?^_`{|}~@.[]".contains(&c)),
        SANITIZE_URL => keep(b, url_char),
        SANITIZE_NUMBER_INT => keep(b, |c| c.is_ascii_digit() || c == b'+' || c == b'-'),
        SANITIZE_NUMBER_FLOAT => keep(b, |c| {
            c.is_ascii_digit()
                || c == b'+'
                || c == b'-'
                || (c == b'.' && flags & ALLOW_FRACTION != 0)
                || (c == b',' && flags & ALLOW_THOUSAND != 0)
                || ((c == b'e' || c == b'E') && flags & ALLOW_SCIENTIFIC != 0)
        }),
        SANITIZE_ADD_SLASHES => ctx.call(&Value::str("addslashes"), &mut [Value::str(s)])?,
        CALLBACK => {
            let callable = options.cloned().unwrap_or(Value::NULL);
            if options.is_none() || !ctx.is_callable(&callable) {
                let name = ctx.builtin_name();
                return Err(ctx.throw("TypeError", format!("{name}(): Option must be a valid callback")));
            }
            ctx.call(&callable, &mut [Value::str(s)])?
        }
        // FILTER_UNSAFE_RAW (FILTER_DEFAULT).
        _ => {
            if flags != 0 && !b.is_empty() {
                let mut enc = [false; 256];
                encode_flags(&mut enc, flags);
                Value::str(PhpStr::new(encode_html(&strip(b, flags), &enc)))
            } else if flags & EMPTY_STRING_NULL != 0 && b.is_empty() {
                Value::NULL
            } else {
                Value::str(s)
            }
        }
    })
}

/// Whether preg_match finds `pattern` in `s`.
fn matches(ctx: &mut dyn Ctx, pattern: Value, s: &PhpStr) -> Result<bool, Throw> {
    let r = ctx.call(&Value::str("preg_match"), &mut [pattern, Value::str(s.clone())])?;
    Ok(r.as_int().is_some_and(|n| n > 0))
}

/// php_filter_int.
fn validate_int(s: &[u8], flags: i64, options: Option<&Value>) -> Value {
    let min = option(options, "min_range").map(|v| v.to_int());
    let max = option(options, "max_range").map(|v| v.to_int());
    let p = trim(s);
    if p.is_empty() {
        return failed(flags);
    }
    let parsed = if let Some(rest) = p.strip_prefix(b"0") {
        if flags & ALLOW_HEX != 0 && matches!(rest.first(), Some(b'x' | b'X')) {
            if rest.len() == 1 {
                return failed(flags);
            }
            radix(&rest[1..], 16)
        } else if flags & ALLOW_OCTAL != 0 {
            let digits = match rest.first() {
                Some(b'o' | b'O') if rest.len() == 1 => return failed(flags),
                Some(b'o' | b'O') => &rest[1..],
                _ => rest,
            };
            radix(digits, 8)
        } else if rest.is_empty() {
            Some(0)
        } else {
            None
        }
    } else {
        parse_int(p)
    };
    match parsed {
        Some(n) if min.is_none_or(|m| n >= m) && max.is_none_or(|m| n <= m) => Value::int(n),
        _ => failed(flags),
    }
}

/// php_filter_parse_int: an optional sign, then 1-9 and digits (or a lone
/// 0), within the integer range.
fn parse_int(s: &[u8]) -> Option<i64> {
    let (negative, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if digits == b"0" {
        return Some(0);
    }
    if !matches!(digits.first(), Some(b'1'..=b'9')) || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut n: i64 = 0;
    for &d in digits {
        let d = i64::from(d - b'0');
        n = n.checked_mul(10)?;
        n = if negative { n.checked_sub(d)? } else { n.checked_add(d)? };
    }
    Some(n)
}

/// Hex or octal digits as an unsigned number (wrapping to a negative int
/// past the signed range, as PHP's cast does).
fn radix(s: &[u8], base: u64) -> Option<i64> {
    let mut n: u64 = 0;
    for &c in s {
        let d = (c as char).to_digit(base as u32)?;
        n = n.checked_mul(base)?.checked_add(u64::from(d))?;
    }
    Some(n as i64)
}

/// php_filter_boolean: 1/true/on/yes and 0/false/off/no/"", else a failure.
fn validate_bool(s: &[u8], flags: i64) -> Value {
    match trim(s).to_ascii_lowercase().as_slice() {
        b"1" | b"true" | b"on" | b"yes" => Value::TRUE,
        b"0" | b"false" | b"off" | b"no" | b"" => Value::FALSE,
        _ => failed(flags),
    }
}

/// php_filter_float: digits with the decimal separator, an exponent and
/// (with FILTER_FLAG_ALLOW_THOUSAND) groups of three.
fn validate_float(ctx: &mut dyn Ctx, s: &[u8], flags: i64, options: Option<&Value>) -> Result<Value, Throw> {
    let str = trim(s);
    if str.is_empty() {
        return Ok(failed(flags));
    }
    let mut dec = b'.';
    if let Some(d) = string_option(options, "decimal") {
        if d.len() != 1 {
            let name = ctx.builtin_name();
            return Err(ctx.throw("ValueError", format!("{name}(): \"decimal\" option must be one character long")));
        }
        dec = d[0];
    }
    let thousand = match string_option(options, "thousand") {
        Some(t) if t.is_empty() => {
            let name = ctx.builtin_name();
            return Err(ctx.throw("ValueError", format!("{name}(): \"thousand\" option must not be empty")));
        }
        Some(t) => t,
        None => b"',.".to_vec(),
    };
    let min = option(options, "min_range").map(|v| v.to_float());
    let max = option(options, "max_range").map(|v| v.to_float());
    let at = |i: usize| str.get(i).copied().unwrap_or(0);
    let mut num = Vec::with_capacity(str.len());
    let mut i = 0;
    if matches!(at(0), b'+' | b'-') {
        num.push(str[0]);
        i = 1;
    }
    let mut first = true;
    loop {
        let mut n = 0;
        while at(i).is_ascii_digit() && i < str.len() {
            n += 1;
            num.push(str[i]);
            i += 1;
        }
        if i == str.len() || at(i) == dec || at(i) == b'e' || at(i) == b'E' {
            if !first && n != 3 {
                return Ok(failed(flags));
            }
            if i < str.len() && at(i) == dec {
                num.push(b'.');
                i += 1;
                while i < str.len() && at(i).is_ascii_digit() {
                    num.push(str[i]);
                    i += 1;
                }
            }
            if i < str.len() && (at(i) == b'e' || at(i) == b'E') {
                num.push(str[i]);
                i += 1;
                if i < str.len() && matches!(at(i), b'+' | b'-') {
                    num.push(str[i]);
                    i += 1;
                }
                while i < str.len() && at(i).is_ascii_digit() {
                    num.push(str[i]);
                    i += 1;
                }
            }
            break;
        }
        if flags & ALLOW_THOUSAND != 0 && thousand.contains(&at(i)) {
            if if first { !(1..=3).contains(&n) } else { n != 3 } {
                return Ok(failed(flags));
            }
            first = false;
            i += 1;
        } else {
            return Ok(failed(flags));
        }
    }
    if i != str.len() {
        return Ok(failed(flags));
    }
    // is_numeric_string: the whole of it, no surrounding whitespace.
    let value = match numeric_string(&num) {
        Some(_) if num.first().is_some_and(u8::is_ascii_whitespace) => None,
        Some(Number::Int(n)) => Some((n as f64, false)),
        Some(Number::Float(f)) => Some((f, true)),
        None => None,
    };
    let Some((f, is_double)) = value else { return Ok(failed(flags)) };
    if is_double && ((f == 0.0 && num.len() > 1 && num.iter().any(|c| (b'1'..=b'9').contains(c))) || !f.is_finite()) {
        return Ok(failed(flags));
    }
    if min.is_some_and(|m| f < m) || max.is_some_and(|m| f > m) {
        return Ok(failed(flags));
    }
    Ok(Value::float(f))
}

/// _php_filter_validate_domain: labels of up to 63 characters, 253 in
/// all; with FILTER_FLAG_HOSTNAME, letters, digits and inner hyphens.
fn valid_domain(d: &[u8], flags: i64) -> bool {
    let hostname = flags & FLAG_HOSTNAME != 0;
    let d = d.strip_suffix(b".").unwrap_or(d);
    if d.len() > 253 {
        return false;
    }
    let at = |i: usize| d.get(i).copied().unwrap_or(0);
    if at(0) == b'.' || (hostname && !at(0).is_ascii_alphanumeric()) {
        return false;
    }
    let mut label = 1u32;
    for i in 0..d.len() {
        if d[i] == b'.' {
            let before = if i == 0 { 0 } else { d[i - 1] };
            if at(i + 1) == b'.' || (hostname && (!before.is_ascii_alphanumeric() || !at(i + 1).is_ascii_alphanumeric())) {
                return false;
            }
            label = 1;
        } else {
            if label > 63 || (hostname && (d[i] != b'-' || at(i + 1) == 0) && !d[i].is_ascii_alphanumeric()) {
                return false;
            }
            label += 1;
        }
    }
    true
}

/// php_filter_validate_url: unchanged by the URL sanitizer, parsed by
/// parse_url, with a scheme, a valid host for http(s), the parts the
/// flags require, and clean user info.
fn validate_url(ctx: &mut dyn Ctx, s: PhpStr, flags: i64) -> Result<Value, Throw> {
    let b = s.as_bytes();
    if !b.iter().all(|&c| url_char(c)) {
        return Ok(failed(flags));
    }
    let Ok(url) = ctx.call(&Value::str("parse_url"), &mut [Value::str(s.clone())])?.into_array() else {
        return Ok(failed(flags));
    };
    let part = |name: &str| {
        let v = url.get(&Key::from_string(PhpStr::from(name)))?;
        if let Some(v) = v.as_str() {
            Some(v.as_bytes().to_vec())
        } else {
            v.as_int().map(|n| n.to_string().into_bytes())
        }
    };
    let scheme = part("scheme");
    let host = part("host");
    if let Some(sc) = &scheme
        && (sc.eq_ignore_ascii_case(b"http") || sc.eq_ignore_ascii_case(b"https"))
    {
        let Some(h) = &host else { return Ok(failed(flags)) };
        let ipv6 = h.len() >= 2 && h[0] == b'[' && h[h.len() - 1] == b']' && ipv6_blocks(&h[1..h.len() - 1]).is_some();
        if !ipv6 && !valid_domain(h, FLAG_HOSTNAME) {
            return Ok(failed(flags));
        }
    }
    let Some(scheme) = scheme else { return Ok(failed(flags)) };
    if (host.is_none() && !matches!(scheme.as_slice(), b"mailto" | b"news" | b"file"))
        || (flags & PATH_REQUIRED != 0 && part("path").is_none())
        || (flags & QUERY_REQUIRED != 0 && part("query").is_none())
    {
        return Ok(failed(flags));
    }
    if part("user").is_some_and(|u| !userinfo_valid(&u)) || part("pass").is_some_and(|p| !userinfo_valid(&p)) {
        return Ok(failed(flags));
    }
    Ok(Value::str(s))
}

fn userinfo_valid(s: &[u8]) -> bool {
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:".contains(&c) {
            i += 1;
        } else if c == b'%' && i + 3 <= s.len() && s[i + 1].is_ascii_digit() && s[i + 2].is_ascii_hexdigit() {
            i += 3;
        } else {
            return false;
        }
    }
    true
}

/// The characters RFC 1738's section 5 allows in a URL.
fn url_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"$-_.+!*'(),{}|\\^~[]`<>#%\";/?:@&=".contains(&c)
}

/// _php_filter_validate_ipv4: four dotted numbers to 255, no leading zeros.
fn ipv4(s: &[u8]) -> Option<[i32; 4]> {
    let mut ip = [0; 4];
    let mut n = 0;
    let mut i = 0;
    while i < s.len() {
        if !s[i].is_ascii_digit() {
            return None;
        }
        let leading_zero = s[i] == b'0';
        let mut digits = 1;
        let mut num = i32::from(s[i] - b'0');
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            num = num * 10 + i32::from(s[i] - b'0');
            i += 1;
            digits += 1;
            if num > 255 || digits > 3 {
                return None;
            }
        }
        if leading_zero && (num != 0 || digits > 1) {
            return None;
        }
        ip[n] = num;
        n += 1;
        if n == 4 {
            return (i == s.len()).then_some(ip);
        }
        if i >= s.len() || s[i] != b'.' {
            return None;
        }
        i += 1;
    }
    None
}

/// _php_filter_validate_ipv6: the eight 16-bit blocks, `::` expanded and
/// a trailing IPv4 address mapped.
fn ipv6_blocks(s: &[u8]) -> Option<[i32; 8]> {
    if !s.contains(&b':') {
        return None;
    }
    let mut ip = [0i32; 8];
    let mut blocks = 0usize;
    let mut compressed: Option<usize> = None;
    let mut len = s.len();
    let mut v4 = None;
    if let Some(dot) = s.iter().position(|&c| c == b'.') {
        let mut start = dot;
        while start > 0 && s[start - 1] != b':' {
            start -= 1;
        }
        v4 = Some(ipv4(&s[start..])?);
        len = start;
        if len < 2 {
            return None;
        }
        if s[start - 2] != b':' {
            len -= 1;
        }
        blocks = 2;
    }
    let s = &s[..len];
    let mut i = 0;
    while i < s.len() {
        if s[i] == b':' {
            i += 1;
            if i >= s.len() {
                return None;
            }
            if s[i] == b':' {
                if compressed.is_some() {
                    return None;
                }
                if blocks < 8 {
                    ip[blocks] = -1;
                }
                compressed = Some(blocks);
                blocks += 1;
                i += 1;
                if i == s.len() {
                    if blocks > 8 {
                        return None;
                    }
                    break;
                }
            } else if i - 1 == 0 {
                return None;
            }
        }
        let mut num = 0i32;
        let mut n = 0;
        while i < s.len() && s[i].is_ascii_hexdigit() {
            num = num * 16 + (s[i] as char).to_digit(16).unwrap_or(0) as i32;
            n += 1;
            i += 1;
        }
        if blocks < 8 {
            ip[blocks] = num;
        }
        if !(1..=4).contains(&n) {
            return None;
        }
        blocks += 1;
        if blocks > 8 {
            return None;
        }
    }
    if let Some(v4) = v4 {
        ip = [0, 0, 0, 0, 0, 0xffff, 256 * v4[0] + v4[1], 256 * v4[2] + v4[3]];
    } else if let Some(at) = compressed
        && blocks <= 8
    {
        let offset = 8 - blocks;
        let mut i = 7;
        while i > at + offset {
            ip[i] = ip[i - offset];
            i -= 1;
        }
        for slot in &mut ip[at..=at + offset] {
            *slot = 0;
        }
    }
    (compressed.is_some() && blocks <= 8 || blocks == 8).then_some(ip)
}

/// RFC 6890's special-purpose ranges: (global, reserved, private), or
/// None for an ordinary address.
fn ipv4_status(ip: [i32; 4]) -> Option<(bool, bool, bool)> {
    Some(match ip {
        [0, ..] => (false, true, false),
        [10, ..] => (false, false, true),
        [100, b, ..] if (64..=127).contains(&b) => (false, false, false),
        [127, ..] => (false, true, false),
        [169, 254, ..] => (false, true, false),
        [172, b, ..] if (16..=31).contains(&b) => (false, false, true),
        [192, 0, 0, _] | [192, 0, 2, _] => (false, false, false),
        [192, 88, 99, _] => (true, false, false),
        [192, 168, ..] => (false, false, true),
        [198, b, ..] if (18..=19).contains(&b) => (false, false, false),
        [198, 51, 100, _] | [203, 0, 113, _] => (false, false, false),
        [a, b, ..] if a >= 240 && b <= 255 => (false, true, false),
        _ => return None,
    })
}

fn ipv6_status(ip: [i32; 8]) -> Option<(bool, bool, bool)> {
    Some(match ip {
        [0, 0, 0, 0, 0, 0, 0, 0] | [0, 0, 0, 0, 0, 0, 0, 1] => (false, true, false),
        [0x64, 0xff9b, ..] => (true, false, false),
        [0, 0, 0, 0, 0, 0xffff, ..] => (false, true, false),
        [0x100, 0, 0, 0, ..] => (false, false, false),
        [0x2001, b, ..] if b <= 0x1ff => (false, false, false),
        [0x2001, 2, 0, ..] | [0x2001, 0xdb8, ..] => (false, false, false),
        [0x2001, b, ..] if (0x10..=0x1f).contains(&b) => (false, false, false),
        [0x2002, ..] => (false, false, false),
        [a, ..] if (0xfc00..=0xfdff).contains(&a) => (false, false, true),
        [a, ..] if (0xfe80..=0xfebf).contains(&a) => (false, true, false),
        _ => return None,
    })
}

/// php_filter_validate_ip: IPv4 or IPv6 (by the first ':' or '.'), as the
/// flags allow, outside the ranges they exclude.
fn valid_ip(s: &[u8], flags: i64) -> bool {
    let v6 = s.contains(&b':');
    if !v6 && !s.contains(&b'.') {
        return false;
    }
    let (want4, want6) = (flags & FLAG_IPV4 != 0, flags & FLAG_IPV6 != 0);
    if want4 != want6 && ((want4 && v6) || (want6 && !v6)) {
        return false;
    }
    let status = if v6 {
        let Some(ip) = ipv6_blocks(s) else { return false };
        ipv6_status(ip)
    } else {
        let Some(ip) = ipv4(s) else { return false };
        ipv4_status(ip)
    };
    let Some((global, reserved, private)) = status else { return true };
    !((flags & GLOBAL_RANGE != 0 && !global) || (flags & NO_PRIV_RANGE != 0 && private) || (flags & NO_RES_RANGE != 0 && reserved))
}

/// php_filter_validate_mac: six hex pairs (':' or '-') or three hex
/// quads ('.').
fn validate_mac(ctx: &mut dyn Ctx, s: PhpStr, flags: i64, options: Option<&Value>) -> Result<Value, Throw> {
    let expected = string_option(options, "separator");
    if expected.as_ref().is_some_and(|e| e.len() != 1) {
        let name = ctx.builtin_name();
        return Err(ctx.throw("ValueError", format!("{name}(): \"separator\" option must be one character long")));
    }
    let b = s.as_bytes();
    let (tokens, length, sep) = match (b.len(), b.get(2)) {
        (14, _) => (3, 4, b'.'),
        (17, Some(b'-')) => (6, 2, b'-'),
        (17, Some(b':')) => (6, 2, b':'),
        _ => return Ok(failed(flags)),
    };
    if expected.is_some_and(|e| e[0] != sep) {
        return Ok(failed(flags));
    }
    for i in 0..tokens {
        let at = i * (length + 1);
        if i < tokens - 1 && b[at + length] != sep {
            return Ok(failed(flags));
        }
        if !b[at..at + length].iter().all(u8::is_ascii_hexdigit) {
            return Ok(failed(flags));
        }
    }
    Ok(Value::str(s))
}

/// php_filter_strip: the flags' low, high and backtick bytes dropped.
fn strip(s: &[u8], flags: i64) -> Vec<u8> {
    s.iter()
        .copied()
        .filter(|&c| !((c >= 127 && flags & STRIP_HIGH != 0) || (c < 32 && flags & STRIP_LOW != 0) || (c == b'`' && flags & STRIP_BACKTICK != 0)))
        .collect()
}

/// The bytes FILTER_FLAG_ENCODE_AMP, _LOW and _HIGH add to `enc`.
fn encode_flags(enc: &mut [bool; 256], flags: i64) {
    if flags & ENCODE_AMP != 0 {
        enc[b'&' as usize] = true;
    }
    if flags & ENCODE_LOW != 0 {
        enc[..32].fill(true);
    }
    if flags & ENCODE_HIGH != 0 {
        enc[127..].fill(true);
    }
}

/// php_filter_encode_html: the marked bytes as `&#N;`.
fn encode_html(s: &[u8], enc: &[bool; 256]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        if enc[c as usize] {
            out.extend_from_slice(format!("&#{c};").as_bytes());
        } else {
            out.push(c);
        }
    }
    out
}

/// php_filter_encode_url: all but letters, digits and "-._" as %XX.
fn encode_url(s: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(s.len() * 3);
    for &c in s {
        if c.is_ascii_alphanumeric() || b"-._".contains(&c) {
            out.push(c);
        } else {
            out.extend_from_slice(&[b'%', HEX[(c >> 4) as usize], HEX[(c & 15) as usize]]);
        }
    }
    out
}

fn keep(s: &[u8], allowed: impl Fn(u8) -> bool) -> Value {
    Value::str(PhpStr::new(s.iter().copied().filter(|&c| allowed(c)).collect::<Vec<u8>>()))
}

/// php_filter_call: the filter, flags and options from `args` (an options
/// array, or the flags), applied to `value` or, as the flags ask, to each
/// element of an array.
fn call(ctx: &mut dyn Ctx, value: Value, mut filter: i64, args: Option<&Value>, mut flags: i64) -> Result<Value, Throw> {
    let mut options: Option<Value> = None;
    let args = args.map(Value::deref);
    match args.as_ref().and_then(Value::as_array) {
        Some(a) => {
            let get = |k: &str| a.get(&Key::from_string(PhpStr::from(k))).map(Value::deref);
            if let Some(f) = get("filter") {
                filter = f.to_int();
            }
            if let Some(o) = get("options") {
                if filter != CALLBACK {
                    if o.is_array() {
                        options = Some(o);
                    }
                } else {
                    options = Some(o);
                    flags = 0;
                }
            }
            if let Some(f) = get("flags") {
                flags = f.to_int();
                if flags & (REQUIRE_ARRAY | FORCE_ARRAY) == 0 {
                    flags |= REQUIRE_SCALAR;
                }
            }
        }
        None => {
            let n = args.as_ref().map_or(0, |v| v.to_int());
            if filter != -1 {
                flags = n;
                if flags & (REQUIRE_ARRAY | FORCE_ARRAY) == 0 {
                    flags |= REQUIRE_SCALAR;
                }
            } else {
                filter = n;
            }
        }
    }
    let options = options.as_ref();
    if let Some(a) = value.as_array() {
        if flags & REQUIRE_SCALAR != 0 {
            return Ok(failed(flags));
        }
        return recurse(ctx, a, filter, flags, options).map(Value::array);
    }
    if flags & REQUIRE_ARRAY != 0 {
        return Ok(failed(flags));
    }
    let v = apply(ctx, value, filter, flags, options)?;
    Ok(if flags & FORCE_ARRAY != 0 { Value::array(PhpArray::from_list(vec![v])) } else { v })
}

/// php_zval_filter_recursive: every element of `a` and its arrays.
fn recurse(ctx: &mut dyn Ctx, a: &PhpArray, filter: i64, flags: i64, options: Option<&Value>) -> Result<PhpArray, Throw> {
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let v = v.deref();
        let v = match v.as_array() {
            Some(inner) => Value::array(recurse(ctx, inner, filter, flags, options)?),
            None => apply(ctx, v, filter, flags, options)?,
        };
        out.insert(k, v);
    }
    Ok(out)
}

/// The filter id argument, warning (and None) for one that does not exist.
fn filter_arg(ctx: &mut dyn Ctx, args: &[Value], i: usize) -> Option<i64> {
    let id = if args.len() > i { arg(args, i).to_int() } else { DEFAULT };
    if exists(id) {
        return Some(id);
    }
    let name = ctx.builtin_name();
    ctx.warn(&format!("{name}(): Unknown filter with ID {id}"));
    None
}

/// filter_var($value, $filter = FILTER_DEFAULT, $options = 0).
fn filter_var(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(filter) = filter_arg(ctx, args, 1) else { return Ok(Value::FALSE) };
    call(ctx, arg(args, 0), filter, args.get(2), REQUIRE_SCALAR)
}

/// php_filter_array_handler: `input` through one filter (applied to every
/// element), or through the definition's filter for each named key.
fn array_handler(ctx: &mut dyn Ctx, input: &PhpArray, definition: Option<&Value>, add_empty: bool) -> Result<Value, Throw> {
    let name = ctx.builtin_name();
    let definition = definition.map(Value::deref);
    match definition.as_ref().and_then(Value::as_array) {
        Some(def) => {
            let mut out = PhpArray::new();
            for (k, spec) in def.iter() {
                let Key::Str(key) = &k else {
                    return Err(ctx.throw("TypeError", format!("{name}(): Argument #2 ($options) must contain only string keys")));
                };
                if key.is_empty() {
                    return Err(ctx.throw("ValueError", format!("{name}(): Argument #2 ($options) cannot contain empty keys")));
                }
                match input.get(&k) {
                    None if add_empty => out.insert(k.clone(), Value::NULL),
                    None => {}
                    Some(v) => {
                        let spec = spec.deref();
                        let (args, n) = if spec.is_array() { (Some(spec.clone()), 0) } else { (None, spec.to_int()) };
                        let filtered = match args {
                            Some(a) => call(ctx, v.deref(), -1, Some(&a), REQUIRE_SCALAR)?,
                            None => call(ctx, v.deref(), -1, Some(&Value::int(n)), REQUIRE_SCALAR)?,
                        };
                        out.insert(k.clone(), filtered);
                    }
                }
            }
            Ok(Value::array(out))
        }
        None => {
            let n = definition.as_ref().map_or(DEFAULT, |v| v.to_int());
            call(ctx, Value::array(input.clone()), -1, Some(&Value::int(n)), REQUIRE_ARRAY)
        }
    }
}

/// The definition argument of the *_array functions: checked when it is a filter id.
fn definition_ok(ctx: &mut dyn Ctx, args: &[Value], i: usize) -> bool {
    match args.get(i).map(Value::deref) {
        Some(v) if !v.is_array() => {
            let id = v.to_int();
            if exists(id) {
                return true;
            }
            let name = ctx.builtin_name();
            ctx.warn(&format!("{name}(): Unknown filter with ID {id}"));
            false
        }
        _ => true,
    }
}

/// filter_var_array($array, $options = FILTER_DEFAULT, $add_empty = true).
fn filter_var_array(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    if !definition_ok(ctx, args, 1) {
        return Ok(Value::FALSE);
    }
    let Ok(input) = arg(args, 0).into_array() else { return Ok(Value::NULL) };
    let add_empty = args.len() < 3 || arg(args, 2).to_bool();
    array_handler(ctx, &input, args.get(1), add_empty)
}

/// The variables an INPUT_* type reads: none for GET, POST and COOKIE (a
/// command line has no request), $_SERVER and $_ENV.
fn storage(ctx: &mut dyn Ctx, kind: i64) -> Result<Option<PhpArray>, Throw> {
    let global: &[u8] = match kind {
        0..=2 => return Ok(None),
        4 => b"_ENV",
        5 => b"_SERVER",
        _ => {
            let name = ctx.builtin_name();
            return Err(ctx.throw("ValueError", format!("{name}(): Argument #1 ($type) must be an INPUT_* constant")));
        }
    };
    Ok(ctx.global_value(global).map(|v| v.deref()).and_then(|v| v.as_array().cloned()))
}

/// filter_input($type, $var_name, $filter = FILTER_DEFAULT, $options = 0).
fn filter_input(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(filter) = filter_arg(ctx, args, 2) else { return Ok(Value::FALSE) };
    let input = storage(ctx, arg(args, 0).to_int())?;
    let name = arg(args, 1).to_php_string().unwrap_or_else(PhpStr::empty);
    let found = input.as_ref().and_then(|a| a.get(&Key::from_string(name)).map(Value::deref));
    let Some(value) = found else {
        let mut flags = 0;
        match args.get(3).map(Value::deref) {
            Some(v) if v.is_array() => {
                let a = v.as_array().expect("an array");
                if let Some(f) = a.get(&Key::from_string(PhpStr::from("flags"))) {
                    flags = f.to_int();
                }
                if let Some(o) = a.get(&Key::from_string(PhpStr::from("options"))).map(Value::deref)
                    && let Some(o) = o.as_array()
                    && let Some(d) = o.get(&Key::from_string(PhpStr::from("default")))
                {
                    return Ok(d.deref());
                }
            }
            Some(v) => flags = v.to_int(),
            None => {}
        }
        // A missing variable is null; with FILTER_NULL_ON_FAILURE, false.
        return Ok(if flags & NULL_ON_FAILURE != 0 { Value::FALSE } else { Value::NULL });
    };
    call(ctx, value, filter, args.get(3), REQUIRE_SCALAR)
}

/// filter_input_array($type, $options = FILTER_DEFAULT, $add_empty = true).
fn filter_input_array(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    if !definition_ok(ctx, args, 1) {
        return Ok(Value::FALSE);
    }
    let Some(input) = storage(ctx, arg(args, 0).to_int())? else { return Ok(Value::NULL) };
    let add_empty = args.len() < 3 || arg(args, 2).to_bool();
    array_handler(ctx, &input, args.get(1), add_empty)
}

/// filter_has_var($input_type, $var_name).
fn filter_has_var(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let input = storage(ctx, arg(args, 0).to_int())?;
    let name = arg(args, 1).to_php_string().unwrap_or_else(PhpStr::empty);
    Ok(Value::bool(input.is_some_and(|a| a.get(&Key::from_string(name)).is_some())))
}

fn filter_list(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::array(PhpArray::from_list(FILTERS.iter().map(|(n, _)| Value::str(*n)).collect())))
}

fn filter_id(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    Ok(FILTERS.iter().find(|(n, _)| n.as_bytes() == name.as_bytes()).map_or(Value::FALSE, |(_, id)| Value::int(*id)))
}

/// The domain half of the e-mail patterns: a routable name or an address literal.
macro_rules! email_domain {
    () => {
        concat!(
            "@(?:(?:(?!.*[^.]{64,})(?:(?:(?:xn--)?[a-z0-9]+(?:-+[a-z0-9]+)*\\.){1,126}){1,}(?:(?:[a-z][a-z0-9]*)|(?:(?:xn--)[a-z0-9]+))(?:-+[a-z0-9]+)*)",
            "|(?:\\[(?:(?:IPv6:(?:(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){7})|(?:(?!(?:.*[a-f0-9][:\\]]){7,})(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,5})?::(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,5})?)))",
            "|(?:(?:IPv6:(?:(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){5}:)|(?:(?!(?:.*[a-f0-9]:){5,})(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,3})?::(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,3}:)?)))?",
            "(?:(?:25[0-5])|(?:2[0-4][0-9])|(?:1[0-9]{2})|(?:[1-9]?[0-9]))(?:\\.(?:(?:25[0-5])|(?:2[0-4][0-9])|(?:1[0-9]{2})|(?:[1-9]?[0-9]))){3}))\\]))$"
        )
    };
}

/// The e-mail patterns of php_filter_validate_email (Michael Rushton's,
/// routable addresses only); the first allows Unicode letters and digits.
const EMAIL_UNICODE_RE: &str = concat!(
    "/^(?!(?:(?:\\x22?\\x5C[\\x00-\\x7E]\\x22?)|(?:\\x22?[^\\x5C\\x22]\\x22?)){255,})(?!(?:(?:\\x22?\\x5C[\\x00-\\x7E]\\x22?)|(?:\\x22?[^\\x5C\\x22]\\x22?)){65,}@)",
    "(?:(?:[\\x21\\x23-\\x27\\x2A\\x2B\\x2D\\x2F-\\x39\\x3D\\x3F\\x5E-\\x7E\\pL\\pN]+)|(?:\\x22(?:[\\x01-\\x08\\x0B\\x0C\\x0E-\\x1F\\x21\\x23-\\x5B\\x5D-\\x7F\\pL\\pN]|(?:\\x5C[\\x00-\\x7F]))*\\x22))",
    "(?:\\.(?:(?:[\\x21\\x23-\\x27\\x2A\\x2B\\x2D\\x2F-\\x39\\x3D\\x3F\\x5E-\\x7E\\pL\\pN]+)|(?:\\x22(?:[\\x01-\\x08\\x0B\\x0C\\x0E-\\x1F\\x21\\x23-\\x5B\\x5D-\\x7F\\pL\\pN]|(?:\\x5C[\\x00-\\x7F]))*\\x22)))*",
    email_domain!(),
    "/iDu",
);
const EMAIL_RE: &str = concat!(
    "/^(?!(?:(?:\\x22?\\x5C[\\x00-\\x7E]\\x22?)|(?:\\x22?[^\\x5C\\x22]\\x22?)){255,})(?!(?:(?:\\x22?\\x5C[\\x00-\\x7E]\\x22?)|(?:\\x22?[^\\x5C\\x22]\\x22?)){65,}@)",
    "(?:(?:[\\x21\\x23-\\x27\\x2A\\x2B\\x2D\\x2F-\\x39\\x3D\\x3F\\x5E-\\x7E]+)|(?:\\x22(?:[\\x01-\\x08\\x0B\\x0C\\x0E-\\x1F\\x21\\x23-\\x5B\\x5D-\\x7F]|(?:\\x5C[\\x00-\\x7F]))*\\x22))",
    "(?:\\.(?:(?:[\\x21\\x23-\\x27\\x2A\\x2B\\x2D\\x2F-\\x39\\x3D\\x3F\\x5E-\\x7E]+)|(?:\\x22(?:[\\x01-\\x08\\x0B\\x0C\\x0E-\\x1F\\x21\\x23-\\x5B\\x5D-\\x7F]|(?:\\x5C[\\x00-\\x7F]))*\\x22)))*",
    email_domain!(),
    "/iD",
);

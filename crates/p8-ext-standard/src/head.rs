//! HTTP headers as the CLI has them: header(), setcookie() and the rest
//! check what they are given and keep the response code, but the CLI
//! sends no headers, so headers_list() is always empty. The headers count
//! as sent once output reaches the host.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/head.c, sapi_header_op() in
//! $PHP_SRC_PATH/main/SAPI.c

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "header", run: header, by_ref: &[] },
    Builtin { name: "header_remove", run: header_remove, by_ref: &[] },
    Builtin { name: "headers_sent", run: headers_sent, by_ref: &[0, 1] },
    Builtin { name: "headers_list", run: headers_list, by_ref: &[] },
    Builtin { name: "http_response_code", run: http_response_code, by_ref: &[] },
    Builtin { name: "setcookie", run: setcookie, by_ref: &[] },
    Builtin { name: "setrawcookie", run: setrawcookie, by_ref: &[] },
];

const RESPONSE_CODE: &str = "http_response_code";

fn response_code(ctx: &mut dyn Ctx) -> i64 {
    ctx.state(RESPONSE_CODE).to_int()
}

fn set_response_code(ctx: &mut dyn Ctx, code: i64) {
    *ctx.state(RESPONSE_CODE) = Value::Int(code);
}

/// Where output started, as PHP words it: " (output started at f:l)", or
/// nothing when no code was running then.
fn started_at(file: &str, line: u32) -> String {
    if file.is_empty() { String::new() } else { format!(" (output started at {file}:{line})") }
}

/// sapi_header_op()'s first check: once the headers are sent, a warning
/// and nothing changes.
fn too_late(ctx: &mut dyn Ctx) -> bool {
    match ctx.output_started() {
        Some((file, line)) => {
            let at = started_at(&file, line);
            let by = if at.is_empty() { String::new() } else { format!(" by{at}") };
            ctx.warn(&format!("Cannot modify header information - headers already sent{by}"));
            true
        }
        None => false,
    }
}

/// The header line without trailing whitespace (C's isspace).
fn trim_end(line: &[u8]) -> &[u8] {
    let end = line.iter().rposition(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')).map_or(0, |i| i + 1);
    &line[..end]
}

/// sapi_header_op() for adding or replacing a header: true when it is
/// taken (the CLI keeps only what it does to the response code).
fn header_op(ctx: &mut dyn Ctx, line: &[u8], code: i64) -> bool {
    if too_late(ctx) || line.is_empty() {
        return false;
    }
    let line = trim_end(line);
    if line.iter().any(|&c| c == b'\n' || c == b'\r') {
        ctx.warn("Header may not contain more than a single header, new line detected");
        return false;
    }
    if line.contains(&0) {
        ctx.warn("Header may not contain NUL bytes");
        return false;
    }
    if line.len() >= 5 && line[..5].eq_ignore_ascii_case(b"HTTP/") {
        // The code follows the first space that has no space after it.
        let code = line.windows(2).position(|w| w[0] == b' ' && w[1] != b' ').map_or(200, |i| atoi(&line[i + 1..]));
        set_response_code(ctx, code);
        return true;
    }
    if let Some(colon) = line.iter().position(|&c| c == b':') {
        let name = &line[..colon];
        if name.eq_ignore_ascii_case(b"Location") {
            let current = response_code(ctx);
            // A redirect is 302 Found unless a redirect code was set (the
            // CLI has no request method, so never 303).
            if !(300..=399).contains(&current) && current != 201 {
                set_response_code(ctx, if code != 0 { code } else { 302 });
            }
        } else if name.eq_ignore_ascii_case(b"WWW-Authenticate") {
            set_response_code(ctx, 401);
        }
    }
    if code != 0 {
        set_response_code(ctx, code);
    }
    true
}

/// C's atoi(): optional whitespace and sign, then digits (as an int).
fn atoi(s: &[u8]) -> i64 {
    let s = &s[s.iter().position(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')).unwrap_or(s.len())..];
    let (negative, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits.iter().take_while(|c| c.is_ascii_digit()).fold(0i64, |n, &c| n.wrapping_mul(10).wrapping_add((c - b'0') as i64)) as i32;
    (if negative { n.wrapping_neg() } else { n }) as i64
}

fn header(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let code = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    header_op(ctx, str_arg(args, 0).as_bytes(), code);
    Ok(Value::Null)
}

fn header_remove(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    if too_late(ctx) {
        return Ok(Value::Null);
    }
    if let Some(name) = args.first().map(Value::deref).filter(|v| !matches!(v, Value::Null)) {
        let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
        if !name.is_empty() && trim_end(name.as_bytes()).contains(&b':') {
            ctx.warn("Header to delete may not contain colon.");
        }
    }
    Ok(Value::Null)
}

fn headers_sent(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let started = ctx.output_started();
    let (file, line) = started.clone().unwrap_or_else(|| (Rc::from(""), 0));
    if let Some(Value::Ref(r)) = args.first() {
        *r.borrow_mut() = Value::Str(PhpStr::from(&*file));
    }
    if let Some(Value::Ref(r)) = args.get(1) {
        *r.borrow_mut() = Value::Int(line as i64);
    }
    Ok(Value::Bool(started.is_some()))
}

fn headers_list(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Array(Rc::new(PhpArray::new())))
}

fn http_response_code(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let code = if args.is_empty() { 0 } else { arg(args, 0).to_int() };
    let old = response_code(ctx);
    if code == 0 {
        return Ok(if old == 0 { Value::Bool(false) } else { Value::Int(old) });
    }
    if let Some((file, line)) = ctx.output_started() {
        ctx.warn(&format!("http_response_code(): Cannot set response code - headers already sent{}", started_at(&file, line)));
        return Ok(Value::Bool(false));
    }
    set_response_code(ctx, code);
    Ok(if old == 0 { Value::Bool(true) } else { Value::Int(old) })
}

fn setcookie(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    cookie(ctx, args, "setcookie", false)
}

fn setrawcookie(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    cookie(ctx, args, "setrawcookie", true)
}

/// The characters no cookie name, raw value, path or domain may contain.
const ILLEGAL: &[u8] = b",; \t\r\n\x0b\x0c";
const ILLEGAL_LIST: &str = r#"",", ";", " ", "\t", "\r", "\n", "\013", or "\014""#;

/// What C's strpbrk() sees: the bytes up to the first NUL.
fn c_bytes(s: &PhpStr) -> &[u8] {
    let b = s.as_bytes();
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

/// setcookie() and setrawcookie(): the name, value and options checked as
/// php_setcookie() does.
fn cookie(ctx: &mut dyn Ctx, args: &mut [Value], func: &str, raw: bool) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    let value = str_arg(args, 1);
    let mut expires = 0;
    let (mut path, mut domain, mut samesite) = (None, None, None);
    match args.get(2).map(Value::deref) {
        Some(Value::Array(options)) => {
            if args.len() > 3 {
                return Err(ctx.throw("ArgumentCountError", format!("{func}(): Expects exactly 3 arguments when argument #3 ($expires_or_options) is an array")));
            }
            for (key, v) in options.iter() {
                let Key::Str(key) = key else {
                    return Err(ctx.throw("ValueError", format!("{func}(): option array cannot have numeric keys")));
                };
                let string = || v.deref().to_php_string().unwrap_or_else(PhpStr::empty);
                match key.as_bytes().to_ascii_lowercase().as_slice() {
                    b"expires" => expires = v.deref().to_int(),
                    b"path" => path = Some(string()),
                    b"domain" => domain = Some(string()),
                    b"samesite" => samesite = Some(string()),
                    // Only flags in the header.
                    b"secure" | b"httponly" => {}
                    _ => return Err(ctx.throw("ValueError", format!("{func}(): option \"{}\" is invalid", String::from_utf8_lossy(key.as_bytes())))),
                }
            }
        }
        Some(v) => expires = v.to_int(),
        None => {}
    }
    if args.len() > 3 {
        path = Some(str_arg(args, 3));
    }
    if args.len() > 4 {
        domain = Some(str_arg(args, 4));
    }
    if name.is_empty() {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($name) must not be empty")));
    }
    if c_bytes(&name).iter().any(|c| *c == b'=' || ILLEGAL.contains(c)) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($name) cannot contain \"=\", {ILLEGAL_LIST}")));
    }
    if raw && c_bytes(&value).iter().any(|c| ILLEGAL.contains(c)) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #2 ($value) cannot contain {ILLEGAL_LIST}")));
    }
    for (option, v) in [("path", &path), ("domain", &domain)] {
        if v.as_ref().is_some_and(|v| c_bytes(v).iter().any(|c| ILLEGAL.contains(c))) {
            return Err(ctx.throw("ValueError", format!("{func}(): \"{option}\" option cannot contain {ILLEGAL_LIST}")));
        }
    }
    if expires >= 253_402_300_800 {
        return Err(ctx.throw("ValueError", format!("{func}(): \"expires\" option cannot have a year greater than 9999")));
    }
    // The header goes through sapi_header_op()'s checks (a NUL byte after
    // the part strpbrk() saw); the CLI then drops it. An encoded value and
    // the dates and flags cannot fail them, so they are left out.
    let mut line = b"Set-Cookie: ".to_vec();
    line.extend_from_slice(name.as_bytes());
    line.push(b'=');
    if raw {
        line.extend_from_slice(value.as_bytes());
    }
    for (part, v) in [("; path=", &path), ("; domain=", &domain), ("; SameSite=", &samesite)] {
        if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
            line.extend_from_slice(part.as_bytes());
            line.extend_from_slice(v.as_bytes());
        }
    }
    Ok(Value::Bool(header_op(ctx, &line, 0)))
}

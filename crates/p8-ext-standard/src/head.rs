//! HTTP headers: header(), setcookie() and the rest check what they are
//! given and keep the response code. Under the CLI the host sends no
//! headers, so headers_list() is always empty; under a web host
//! ([`p8_runtime::ext::Response::server`], PHP's cli-server SAPI) the
//! header lines are kept for it to send. The headers count as sent once
//! output reaches the host.
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
    Builtin { name: "header_register_callback", run: header_register_callback, by_ref: &[] },
    Builtin { name: "http_response_code", run: http_response_code, by_ref: &[] },
    Builtin { name: "setcookie", run: setcookie, by_ref: &[] },
    Builtin { name: "setrawcookie", run: setrawcookie, by_ref: &[] },
];

fn response_code(ctx: &mut dyn Ctx) -> i64 {
    ctx.response().status
}

/// sapi_update_response_code(): a new code drops the status line.
fn set_response_code(ctx: &mut dyn Ctx, code: i64) {
    let response = ctx.response();
    if response.status != code {
        response.status_line = None;
        response.status = code;
    }
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

/// C's isspace.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// The header line without trailing whitespace (C's isspace).
fn trim_end(line: &[u8]) -> &[u8] {
    let end = line.iter().rposition(|&c| !is_space(c)).map_or(0, |i| i + 1);
    &line[..end]
}

/// sapi_remove_header(): every header named `name` (any case) goes.
fn remove_header(headers: &mut Vec<Vec<u8>>, name: &[u8]) {
    headers.retain(|h| !(h.len() > name.len() && h[name.len()] == b':' && h[..name.len()].eq_ignore_ascii_case(name)));
}

/// sapi_header_op() for adding or replacing a header: true when it is
/// taken. (A host that sends no headers keeps only what the header does to
/// the response code.)
fn header_op(ctx: &mut dyn Ctx, line: &[u8], code: i64, replace: bool) -> bool {
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
        ctx.response().status_line = Some(line.to_vec());
        return true;
    }
    let mut line = line.to_vec();
    if let Some(colon) = line.iter().position(|&c| c == b':') {
        let name = line[..colon].to_vec();
        if name.eq_ignore_ascii_case(b"Content-Type") {
            let mut value = &line[colon + 1..];
            while let [b' ', rest @ ..] = value {
                value = rest;
            }
            let charset = ctx.response().default_charset.clone();
            // sapi_apply_default_charset(): a text/* type gets the charset.
            if !charset.is_empty() && value.starts_with(b"text/") && !value.windows(8).any(|w| w == b"charset=") {
                let mut rewritten = b"Content-type: ".to_vec();
                rewritten.extend_from_slice(value);
                rewritten.extend_from_slice(b";charset=");
                rewritten.extend_from_slice(&charset);
                line = rewritten;
            }
            ctx.response().default_content_type = None;
        } else if name.eq_ignore_ascii_case(b"Location") {
            let current = response_code(ctx);
            if !(300..=399).contains(&current) && current != 201 {
                let see_other = ctx.response().redirect_see_other;
                set_response_code(ctx, if code != 0 { code } else if see_other { 303 } else { 302 });
            }
        } else if name.eq_ignore_ascii_case(b"WWW-Authenticate") {
            set_response_code(ctx, 401);
        }
    }
    if code != 0 {
        set_response_code(ctx, code);
    }
    let response = ctx.response();
    if response.server {
        if replace && let Some(colon) = line.iter().position(|&c| c == b':') {
            let name = line[..colon].to_vec();
            remove_header(&mut response.headers, &name);
        }
        response.headers.push(line);
    }
    true
}

/// A header line added as header($line, $replace) adds it (the checks, the
/// response code it implies, and the line itself under a host that sends
/// headers): whether it was taken. The session extension sends its cookie
/// and cache headers this way.
pub fn header_line(ctx: &mut dyn Ctx, line: &[u8], replace: bool) -> bool {
    header_op(ctx, line, 0, replace)
}

/// C's atoi(): optional whitespace and sign, then digits (as an int).
fn atoi(s: &[u8]) -> i64 {
    let s = &s[s.iter().position(|&c| !is_space(c)).unwrap_or(s.len())..];
    let (negative, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits.iter().take_while(|c| c.is_ascii_digit()).fold(0i64, |n, &c| n.wrapping_mul(10).wrapping_add((c - b'0') as i64)) as i32;
    (if negative { n.wrapping_neg() } else { n }) as i64
}

fn header(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let replace = args.len() < 2 || arg(args, 1).to_bool();
    let code = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    header_op(ctx, str_arg(args, 0).as_bytes(), code, replace);
    Ok(Value::Null)
}

fn header_remove(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    if too_late(ctx) {
        return Ok(Value::Null);
    }
    match args.first().map(Value::deref).filter(|v| !matches!(v, Value::Null)) {
        // header_remove(): every header goes.
        None => ctx.response().headers.clear(),
        Some(name) => {
            let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
            if name.is_empty() {
                return Ok(Value::Null);
            }
            let name = trim_end(name.as_bytes());
            if name.contains(&b':') {
                ctx.warn("Header to delete may not contain colon.");
                return Ok(Value::Null);
            }
            remove_header(&mut ctx.response().headers, name);
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

/// header_register_callback(): the engine calls it just before the
/// headers go out (a later registration replaces it).
fn header_register_callback(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let callback = arg(args, 0);
    if !ctx.is_callable(&callback) {
        let why = p8_runtime::args::callable_error(ctx, &callback);
        return Err(ctx.throw("TypeError", format!("header_register_callback(): Argument #1 ($callback) must be a valid callback, {why}")));
    }
    *ctx.state("header_callback") = callback;
    Ok(Value::Bool(true))
}

fn headers_list(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let mut list = PhpArray::new();
    for h in ctx.response().headers.iter().filter(|h| !h.is_empty()) {
        let _ = list.push(Value::Str(PhpStr::from(h.as_slice())));
    }
    Ok(Value::Array(Rc::new(list)))
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
    // Set directly: a status line the script gave stays.
    ctx.response().status = code;
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

/// `D, d M Y H:i:s \G\M\T` for a timestamp (UTC).
pub fn cookie_date(ts: i64) -> String {
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400);
    // Civil date from days since 1970-01-01 (Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[days.rem_euclid(7) as usize],
        day,
        MONTHS[(month - 1) as usize],
        year,
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// setcookie() and setrawcookie(): php_setcookie(), the checks and the
/// header line it builds.
fn cookie(ctx: &mut dyn Ctx, args: &mut [Value], func: &str, raw: bool) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    let value = str_arg(args, 1);
    let mut expires = 0;
    let (mut path, mut domain, mut samesite) = (None, None, None);
    let (mut secure, mut httponly) = (false, false);
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
                    b"secure" => secure = v.deref().to_bool(),
                    b"httponly" => httponly = v.deref().to_bool(),
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
    if args.len() > 5 {
        secure = arg(args, 5).to_bool();
    }
    if args.len() > 6 {
        httponly = arg(args, 6).to_bool();
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
    let mut line = b"Set-Cookie: ".to_vec();
    line.extend_from_slice(name.as_bytes());
    if value.is_empty() {
        // MSIE does not delete a cookie set to nothing: expire it in the past.
        line.extend_from_slice(format!("=deleted; expires={}; Max-Age=0", cookie_date(1)).as_bytes());
    } else {
        line.push(b'=');
        if raw {
            line.extend_from_slice(value.as_bytes());
        } else {
            line.extend_from_slice(&crate::string2::url_encode(value.as_bytes(), true));
        }
        if expires > 0 {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            line.extend_from_slice(format!("; expires={}; Max-Age={}", cookie_date(expires), (expires - now).max(0)).as_bytes());
        }
    }
    for (part, v) in [("; path=", &path), ("; domain=", &domain)] {
        if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
            line.extend_from_slice(part.as_bytes());
            line.extend_from_slice(v.as_bytes());
        }
    }
    if secure {
        line.extend_from_slice(b"; secure");
    }
    if httponly {
        line.extend_from_slice(b"; HttpOnly");
    }
    if let Some(v) = samesite.as_ref().filter(|v| !v.is_empty()) {
        line.extend_from_slice(b"; SameSite=");
        line.extend_from_slice(v.as_bytes());
    }
    Ok(Value::Bool(header_op(ctx, &line, 0, false)))
}

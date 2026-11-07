//! mail(): the message handed to the program sendmail_path names (through
//! the shell, as popen() runs it), written as PHP writes it: To, Subject,
//! the additional headers, a blank line and the message.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/mail.c

use crate::arg;
use p8_runtime::args::value_name;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::{Kind, Value};
use std::io::Write;

pub static BUILTINS: &[Builtin] = &[Builtin { name: "mail", run: mail, by_ref: &[] }];

/// The exit statuses sendmail succeeds with (EX_OK, EX_TEMPFAIL).
const EX_OK: i32 = 0;
const EX_TEMPFAIL: i32 = 75;

/// mail($to, $subject, $message, $additional_headers = [], $additional_params = "").
fn mail(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = |ctx: &mut dyn Ctx, i: usize, name: &str| -> Result<Vec<u8>, Throw> {
        let v = arg(args, i).to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec();
        if v.contains(&0) {
            return Err(ctx.throw("ValueError", format!("mail(): Argument #{} (${name}) must not contain any null bytes", i + 1)));
        }
        Ok(v)
    };
    let to = path(ctx, 0, "to")?;
    let subject = path(ctx, 1, "subject")?;
    let message = path(ctx, 2, "message")?;
    let extra = if args.len() > 4 { Some(path(ctx, 4, "additional_params")?) } else { None };
    let headers = match args.get(3).map(Value::deref) {
        None => Vec::new(),
        Some(v) if v.is_null() => Vec::new(),
        Some(v) if v.is_array() => build_headers(ctx, v.as_array().expect("an array"))?,
        Some(other) => {
            let h = other.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec();
            if h.contains(&0) {
                return Err(ctx.throw("ValueError", "mail(): Argument #4 ($additional_headers) must not contain any null bytes".into()));
            }
            let end = h.iter().rposition(|c| !b" \t\n\r\x0b\0".contains(c)).map_or(0, |i| i + 1);
            h[..end].to_vec()
        }
    };
    let to = header_line(&to);
    let subject = header_line(&subject);
    let force = ctx.ini_get(b"mail.force_extra_parameters").and_then(|v| v.to_php_string()).filter(|v| !v.is_empty());
    let extra = force.map(|f| f.as_bytes().to_vec()).or(extra).map(|e| crate::string2::escape_shell_cmd(&e));

    let sep: &[u8] = if ini_on(ctx, b"mail.mixed_lf_and_crlf") { b"\n" } else { b"\r\n" };
    if !headers.is_empty() && malformed_newlines(&headers) {
        ctx.warn("mail(): Multiple or malformed newlines found in additional_header");
        return Ok(Value::FALSE);
    }
    let Some(sendmail) = ctx.ini_get(b"sendmail_path").and_then(|v| v.to_php_string()) else {
        return Ok(Value::FALSE);
    };
    let mut command = sendmail.as_bytes().to_vec();
    if let Some(extra) = &extra {
        command.push(b' ');
        command.extend_from_slice(extra);
    }
    let mut body = Vec::new();
    for part in [&b"To: "[..], &to, sep, b"Subject: ", &subject, sep] {
        body.extend_from_slice(part);
    }
    if !headers.is_empty() {
        body.extend_from_slice(&headers);
        body.extend_from_slice(sep);
    }
    for part in [sep, &message, sep] {
        body.extend_from_slice(part);
    }

    let mut cmd = crate::process::shell(ctx, &command);
    cmd.stdin(std::process::Stdio::piped());
    let Ok(mut child) = cmd.spawn() else {
        let path = String::from_utf8_lossy(sendmail.as_bytes()).into_owned();
        ctx.warn(&format!("mail(): Could not execute mail delivery program '{path}'"));
        return Ok(Value::FALSE);
    };
    // A program that stops reading early still gets its status read.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&body);
    }
    let status = child.wait().ok().and_then(|s| s.code());
    Ok(Value::bool(matches!(status, Some(EX_OK | EX_TEMPFAIL))))
}

fn ini_on(ctx: &mut dyn Ctx, name: &[u8]) -> bool {
    ctx.ini_get(name).is_some_and(|v| v.to_bool())
}

/// To or Subject made one header line: trailing whitespace dropped, and
/// control characters made spaces, except a fold (CRLF and whitespace).
fn header_line(v: &[u8]) -> Vec<u8> {
    let end = v.iter().rposition(|c| !b" \t\n\r\x0b\x0c".contains(c)).map_or(0, |i| i + 1);
    let mut out = v[..end].to_vec();
    let mut i = 0;
    while i < out.len() {
        let c = out[i];
        if c < 32 || c == 127 {
            if c == b'\r' && out.get(i + 1) == Some(&b'\n') && matches!(out.get(i + 2), Some(b' ' | b'\t')) {
                i += 2;
                while matches!(out.get(i + 1), Some(b' ' | b'\t')) {
                    i += 1;
                }
                i += 1;
                continue;
            }
            out[i] = b' ';
        }
        i += 1;
    }
    out
}

/// php_mail_detect_multiple_crlf: a header block starting badly or with an
/// empty line (or a bare CR) in it.
fn malformed_newlines(h: &[u8]) -> bool {
    if h[0] < 33 || h[0] > 126 || h[0] == b':' {
        return true;
    }
    let at = |i: usize| h.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < h.len() {
        match h[i] {
            b'\r' => {
                if matches!(at(i + 1), 0 | b'\r') || (at(i + 1) == b'\n' && matches!(at(i + 2), 0 | b'\n' | b'\r')) {
                    return true;
                }
                i += 2;
            }
            b'\n' => {
                if matches!(at(i + 1), 0 | b'\r' | b'\n') {
                    return true;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    false
}

/// php_mail_build_headers: name => value (or a list of values) as
/// "Name: value" lines.
fn build_headers(ctx: &mut dyn Ctx, headers: &PhpArray) -> Result<Vec<u8>, Throw> {
    const SINGLE: [&str; 9] = ["orig-date", "from", "sender", "reply-to", "cc", "bcc", "message-id", "references", "in-reply-to"];
    let mut out = Vec::new();
    for (k, v) in headers.iter() {
        let name = match k {
            Key::Int(i) => return Err(ctx.throw("TypeError", format!("Header name cannot be numeric, {i} given"))),
            Key::Str(s) => s.clone(),
        };
        let lower = String::from_utf8_lossy(name.as_bytes()).to_ascii_lowercase();
        let shown = String::from_utf8_lossy(name.as_bytes()).into_owned();
        match lower.as_str() {
            "to" => return Err(ctx.throw("ValueError", "The additional headers cannot contain the \"To\" header".into())),
            "subject" => return Err(ctx.throw("ValueError", "The additional headers cannot contain the \"Subject\" header".into())),
            _ => {}
        }
        let v = v.deref();
        match v.kind() {
            Kind::Str => header(ctx, &mut out, &shown, v.as_str().expect("a string").as_bytes())?,
            Kind::Array => {
                let list = v.as_array().expect("an array");
                if let Some(single) = SINGLE.iter().find(|n| **n == lower) {
                    return Err(ctx.throw("TypeError", format!("Header \"{single}\" must be of type string, array given")));
                }
                for (k, v) in list.iter() {
                    if let Key::Str(s) = k {
                        let key = String::from_utf8_lossy(s.as_bytes()).into_owned();
                        return Err(ctx.throw("TypeError", format!("Header \"{shown}\" must only contain numeric keys, \"{key}\" found")));
                    }
                    let Some(s) = v.deref().cloned_str() else {
                        return Err(ctx.throw("TypeError", format!("Header \"{shown}\" must only contain values of type string, {} found", value_name(v))));
                    };
                    header(ctx, &mut out, &shown, s.as_bytes())?;
                }
            }
            _ => return Err(ctx.throw("TypeError", format!("Header \"{shown}\" must be of type array|string, {} given", value_name(&v)))),
        }
    }
    // The last line's CRLF goes.
    out.truncate(out.len().saturating_sub(2));
    Ok(out)
}

/// One "Name: value\r\n", the name and value checked as RFC 2822 has them.
fn header(ctx: &mut dyn Ctx, out: &mut Vec<u8>, name: &str, value: &[u8]) -> Result<(), Throw> {
    if name.bytes().any(|c| !(33..=126).contains(&c) || c == b':') {
        return Err(ctx.throw("ValueError", format!("Header name \"{name}\" contains invalid characters")));
    }
    let mut i = 0;
    while i < value.len() {
        let problem = match value[i] {
            b'\r' if value.get(i + 1) != Some(&b'\n') => Some("CR character that is not allowed in the header"),
            b'\r' if matches!(value.get(i + 2), Some(b' ' | b'\t')) => {
                i += 3;
                continue;
            }
            b'\r' => Some("CRLF characters that are used as a line separator and are not allowed in the header"),
            b'\n' if matches!(value.get(i + 1), Some(b' ' | b'\t')) => {
                i += 2;
                continue;
            }
            b'\n' => Some("LF character that is not allowed in the header"),
            0 => Some("NULL character that is not allowed in the header"),
            _ => None,
        };
        if let Some(problem) = problem {
            return Err(ctx.throw("ValueError", format!("Header \"{name}\" contains {problem}")));
        }
        i += 1;
    }
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(value);
    out.extend_from_slice(b"\r\n");
    Ok(())
}

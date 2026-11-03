//! CSV: str_getcsv, fgetcsv, fputcsv. The parser is PHP's php_fgetcsv,
//! byte for byte (in the C locale every character is one byte), quirks
//! included: an enclosed field runs on across lines, text after its
//! closing enclosure is kept, and an escape character only stops the
//! next enclosure from closing the field.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/file.c (php_fgetcsv, php_fputcsv)

use crate::{arg, str_arg};
use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::stream::resource;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "str_getcsv", run: str_getcsv, by_ref: &[] },
    Builtin { name: "fgetcsv", run: fgetcsv, by_ref: &[] },
    Builtin { name: "fputcsv", run: fputcsv, by_ref: &[] },
];

/// The separator, enclosure and escape a CSV function was given.
pub(crate) struct Format {
    pub(crate) delimiter: u8,
    pub(crate) enclosure: u8,
    pub(crate) escape: Option<u8>,
}

impl Format {
    /// From the arguments at `at` (separator, enclosure, escape), checked
    /// as PHP checks them.
    pub(crate) fn from_args(ctx: &mut dyn Ctx, func: &str, args: &[Value], at: usize) -> Result<Format, Throw> {
        let one = |i: usize, name: &str, default: u8| -> Result<u8, Throw> {
            if args.len() <= i {
                return Ok(default);
            }
            let s = str_arg(args, i);
            match s.as_bytes() {
                [c] => Ok(*c),
                _ => Err(ctx.throw("ValueError", format!("{func}(): Argument #{} (${name}) must be a single character", i + 1))),
            }
        };
        let delimiter = one(at, "separator", b',')?;
        let enclosure = one(at + 1, "enclosure", b'"')?;
        let escape = if args.len() > at + 2 {
            match str_arg(args, at + 2).as_bytes() {
                [] => None,
                [c] => Some(*c),
                _ => return Err(ctx.throw("ValueError", format!("{func}(): Argument #{} ($escape) must be empty or a single character", at + 3))),
            }
        } else {
            ctx.report(8192, &format!("{func}(): the $escape parameter must be provided as its default value will change"));
            Some(b'\\')
        };
        Ok(Format { delimiter, enclosure, escape })
    }
}

/// Where a line's ending ("\r\n", "\n" or "\r") starts.
fn line_end(buf: &[u8]) -> usize {
    match buf {
        [.., b'\r', b'\n'] => buf.len() - 2,
        [.., b'\n' | b'\r'] => buf.len() - 1,
        _ => buf.len(),
    }
}

/// C's isspace.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// One CSV record from `first` (a line with its ending), reading more
/// lines from `more` while an enclosed field is open. None for a blank line.
pub(crate) fn parse(first: Vec<u8>, more: &mut dyn FnMut() -> Option<Vec<u8>>, fmt: &Format) -> Option<Vec<Vec<u8>>> {
    let (delimiter, enclosure, escape) = (fmt.delimiter, fmt.enclosure, fmt.escape);
    let mut buf = first;
    // The byte at i, as C reads it (the string's terminating NUL past its end).
    let at = |buf: &[u8], i: usize| buf.get(i).copied().unwrap_or(0);
    let slice = |buf: &[u8], from: usize, to: usize| -> Vec<u8> {
        let to = to.min(buf.len());
        if from >= to { Vec::new() } else { buf[from..to].to_vec() }
    };
    let mut limit = line_end(&buf);
    let mut end = limit;
    let mut bptr = 0usize;
    let mut values = Vec::new();
    let mut first_field = true;
    loop {
        let mut field: Vec<u8> = Vec::new();
        let mut inc = usize::from(bptr < limit);
        if inc == 1 {
            // Spaces before an enclosure are skipped.
            let mut tmp = bptr;
            while at(&buf, tmp) != delimiter && is_space(at(&buf, tmp)) {
                tmp += 1;
            }
            if at(&buf, tmp) == enclosure && tmp < limit {
                bptr = tmp;
            }
        }
        if first_field && bptr == limit {
            return None;
        }
        first_field = false;
        if inc != 0 && at(&buf, bptr) == enclosure {
            let mut state = 0;
            bptr += 1;
            let mut hunk = bptr;
            // An enclosed field.
            'field: loop {
                if inc == 0 {
                    match state {
                        2 => {
                            field.extend(slice(&buf, hunk, bptr - 1));
                            hunk = bptr;
                            break 'field;
                        }
                        _ => {
                            if state == 1 {
                                field.extend(slice(&buf, hunk, bptr));
                                hunk = bptr;
                            }
                            if hunk != limit {
                                field.extend(slice(&buf, hunk, bptr));
                                hunk = bptr;
                            }
                            // The line's ending is part of the field.
                            field.extend(slice(&buf, limit, limit + end));
                            match more() {
                                Some(next) => {
                                    buf = next;
                                    bptr = 0;
                                    hunk = 0;
                                    limit = line_end(&buf);
                                    end = buf.len() - limit;
                                    state = 0;
                                }
                                None => {
                                    // Unterminated: the rest is the field.
                                    if bptr > limit {
                                        if hunk == bptr {
                                            hunk -= 1;
                                        }
                                        bptr -= 1;
                                    }
                                    break 'field;
                                }
                            }
                        }
                    }
                } else {
                    let c = at(&buf, bptr);
                    match state {
                        1 => {
                            bptr += 1;
                            state = 0;
                        }
                        2 => {
                            if c != enclosure {
                                // The closing enclosure.
                                field.extend(slice(&buf, hunk, bptr - 1));
                                hunk = bptr;
                                break 'field;
                            }
                            // A doubled enclosure is one.
                            field.extend(slice(&buf, hunk, bptr));
                            bptr += 1;
                            hunk = bptr;
                            state = 0;
                        }
                        _ => {
                            if c == enclosure {
                                state = 2;
                            } else if escape == Some(c) {
                                state = 1;
                            }
                            bptr += 1;
                        }
                    }
                }
                inc = usize::from(bptr < limit);
            }
            // What follows the enclosure, up to the delimiter, is kept.
            while inc != 0 && at(&buf, bptr) != delimiter {
                bptr += 1;
                inc = usize::from(bptr < limit);
            }
            field.extend(slice(&buf, hunk, bptr));
            bptr += inc;
        } else {
            let hunk = bptr;
            while inc != 0 && at(&buf, bptr) != delimiter {
                bptr += 1;
                inc = usize::from(bptr < limit);
            }
            field.extend(slice(&buf, hunk, bptr));
            let keep = line_end(&field);
            field.truncate(keep);
            if at(&buf, bptr) == delimiter {
                bptr += 1;
            }
        }
        values.push(field);
        if inc == 0 {
            return Some(values);
        }
    }
}

/// The record as PHP returns it ([null] for a blank line).
fn record(fields: Option<Vec<Vec<u8>>>) -> Value {
    let mut out = PhpArray::new();
    match fields {
        Some(fields) => {
            for f in fields {
                let _ = out.push(Value::Str(PhpStr::new(f)));
            }
        }
        None => {
            let _ = out.push(Value::Null);
        }
    }
    Value::Array(Rc::new(out))
}

/// str_getcsv(string $string, string $separator = ",", string $enclosure = "\"", string $escape = "\\")
fn str_getcsv(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let fmt = Format::from_args(ctx, "str_getcsv", args, 1)?;
    Ok(record(parse(s.as_bytes().to_vec(), &mut || None, &fmt)))
}

/// fgetcsv(resource $stream, ?int $length = null, string $separator = ",", ...)
fn fgetcsv(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = crate::file::stream_arg(ctx, "fgetcsv", args, 0)?;
    let len = match args.get(1).map(Value::deref) {
        None | Some(Value::Null) => 0,
        Some(v) => v.to_int(),
    };
    if len < 0 {
        return Err(ctx.throw("ValueError", format!("fgetcsv(): Argument #2 ($length) must be between 0 and {}", i64::MAX - 1)));
    }
    let fmt = Format::from_args(ctx, "fgetcsv", args, 2)?;
    let r = resource(&o).expect("a stream resource");
    let limit = (len > 0).then(|| len as usize + 1);
    // A user-space wrapper's code reaching for its own stream finds it busy.
    let Ok(mut s) = r.stream.try_borrow_mut() else { return Ok(Value::Bool(false)) };
    let Some(first) = s.gets_in(ctx, limit)? else { return Ok(Value::Bool(false)) };
    // A line more for a quoted field going on; what a wrapper throws
    // ends the record and goes on up.
    let mut thrown = None;
    let mut more = || s.gets_in(ctx, None).unwrap_or_else(|t| {
        thrown.get_or_insert(t);
        None
    });
    let fields = record(parse(first, &mut more, &fmt));
    match thrown {
        Some(t) => Err(t),
        None => Ok(fields),
    }
}

/// One CSV line for `fields`, as fputcsv writes it.
pub(crate) fn line(fields: &PhpArray, fmt: &Format, eol: &[u8], diags: &mut Vec<p8_runtime::ops::Diag>) -> Vec<u8> {
    let mut out = Vec::new();
    let count = fields.len();
    for (i, (_, v)) in fields.iter().enumerate() {
        let mut s = Vec::new();
        p8_runtime::ops::push_string(&mut s, v, diags);
        let needs = s.iter().any(|&c| {
            c == fmt.delimiter || c == fmt.enclosure || Some(c) == fmt.escape || matches!(c, b'\n' | b'\r' | b'\t' | b' ')
        });
        if needs {
            out.push(fmt.enclosure);
            let mut escaped = false;
            for &c in &s {
                if Some(c) == fmt.escape {
                    escaped = true;
                } else if !escaped && c == fmt.enclosure {
                    out.push(fmt.enclosure);
                } else {
                    escaped = false;
                }
                out.push(c);
            }
            out.push(fmt.enclosure);
        } else {
            out.extend_from_slice(&s);
        }
        if i + 1 != count {
            out.push(fmt.delimiter);
        }
    }
    out.extend_from_slice(eol);
    out
}

/// fputcsv(resource $stream, array $fields, string $separator = ",", string $enclosure = "\"", string $escape = "\\", string $eol = "\n")
fn fputcsv(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = crate::file::stream_arg(ctx, "fputcsv", args, 0)?;
    let fields = match arg(args, 1) {
        Value::Array(a) => a,
        _ => Rc::new(PhpArray::new()),
    };
    let fmt = Format::from_args(ctx, "fputcsv", args, 2)?;
    let eol = match args.get(5).map(Value::deref) {
        None | Some(Value::Null) => b"\n".to_vec(),
        Some(v) => v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec(),
    };
    let mut diags = Vec::new();
    let bytes = line(&fields, &fmt, &eol, &mut diags);
    if !diags.is_empty() {
        ctx.warn("Array to string conversion");
    }
    let written = crate::file::with_user(ctx, &o, None, |ctx, s| s.write(ctx, &bytes))?;
    Ok(written.map_or(Value::Bool(false), |n| Value::Int(n as i64)))
}

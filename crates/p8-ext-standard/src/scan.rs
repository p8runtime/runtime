//! sscanf and fscanf; pack and unpack; parse_ini_string and
//! parse_ini_file; assert.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/scanf.c, pack.c,
//! $PHP_SRC_PATH/Zend/zend_ini_scanner.l, zend_ini_parser.y,
//! $PHP_SRC_PATH/ext/standard/assert.c

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::stream::{resource, resource_of};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "sscanf", run: sscanf, by_ref: &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23] },
    Builtin { name: "fscanf", run: fscanf, by_ref: &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23] },
    Builtin { name: "pack", run: pack, by_ref: &[] },
    Builtin { name: "unpack", run: unpack, by_ref: &[] },
    Builtin { name: "parse_ini_string", run: parse_ini_string, by_ref: &[] },
    Builtin { name: "parse_ini_file", run: parse_ini_file, by_ref: &[] },
    Builtin { name: "assert", run: assert, by_ref: &[] },
];

/// One conversion's result: a value, or nothing (the input ended).
enum Scanned {
    Value(Value),
    Skipped,
}

/// php_sscanf_internal: the values `format` reads from `input`, in
/// order (None where the input ran out), or None when the input was
/// empty before the first conversion.
fn scan(input: &[u8], format: &[u8]) -> Option<Vec<Option<Value>>> {
    let mut values: Vec<Option<Value>> = Vec::new();
    let mut positioned: Vec<(usize, Value)> = Vec::new();
    let (mut i, mut f) = (0usize, 0usize);
    let mut failed = false;
    let mut any = false;
    let mut slots = 0usize;
    while f < format.len() {
        let c = format[f];
        if c.is_ascii_whitespace() {
            while i < input.len() && input[i].is_ascii_whitespace() {
                i += 1;
            }
            f += 1;
            continue;
        }
        if c != b'%' {
            if !failed && input.get(i) == Some(&c) {
                i += 1;
            } else {
                failed = true;
            }
            f += 1;
            continue;
        }
        f += 1;
        if format.get(f) == Some(&b'%') {
            if !failed && input.get(i) == Some(&b'%') {
                i += 1;
            } else {
                failed = true;
            }
            f += 1;
            continue;
        }
        let suppress = format.get(f) == Some(&b'*');
        if suppress {
            f += 1;
        }
        // n$ (an explicit position) or a width.
        let num_start = f;
        while f < format.len() && format[f].is_ascii_digit() {
            f += 1;
        }
        let mut position = None;
        let mut width = None;
        if format.get(f) == Some(&b'$') && f > num_start {
            position = std::str::from_utf8(&format[num_start..f]).ok().and_then(|n| n.parse::<usize>().ok());
            f += 1;
            let w = f;
            while f < format.len() && format[f].is_ascii_digit() {
                f += 1;
            }
            if f > w {
                width = std::str::from_utf8(&format[w..f]).ok().and_then(|n| n.parse::<usize>().ok());
            }
        } else if f > num_start {
            width = std::str::from_utf8(&format[num_start..f]).ok().and_then(|n| n.parse::<usize>().ok());
        }
        while matches!(format.get(f), Some(b'h' | b'l' | b'L')) {
            f += 1;
        }
        let Some(&conv) = format.get(f) else { break };
        f += 1;
        let mut set: Option<(bool, Vec<bool>)> = None;
        if conv == b'[' {
            let negate = format.get(f) == Some(&b'^');
            if negate {
                f += 1;
            }
            let mut members = vec![false; 256];
            let start = f;
            while f < format.len() && (format[f] != b']' || f == start) {
                if f + 2 < format.len() && format[f + 1] == b'-' && format[f + 2] != b']' {
                    for x in format[f]..=format[f + 2] {
                        members[x as usize] = true;
                    }
                    f += 3;
                } else {
                    members[format[f] as usize] = true;
                    f += 1;
                }
            }
            f += 1;
            set = Some((negate, members));
        }
        let result = if failed {
            Scanned::Skipped
        } else {
            if conv != b'c' && conv != b'[' && conv != b'n' {
                while i < input.len() && input[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            let max = width.unwrap_or(usize::MAX);
            let take = |i: &mut usize, ok: &dyn Fn(u8, usize) -> bool| -> Vec<u8> {
                let start = *i;
                while *i < input.len() && *i - start < max && ok(input[*i], *i - start) {
                    *i += 1;
                }
                input[start..*i].to_vec()
            };
            match conv {
                b'n' => Scanned::Value(Value::int(i as i64)),
                _ if i >= input.len() => Scanned::Skipped,
                b'd' | b'u' | b'i' | b'x' | b'X' | b'o' => {
                    let radix = match conv {
                        b'x' | b'X' => 16,
                        b'o' => 8,
                        _ => 10,
                    };
                    let mut text = take(&mut i, &|ch, at| {
                        (at == 0 && (ch == b'-' || ch == b'+'))
                            || ch.is_ascii_hexdigit() && (radix == 16 || ch.is_ascii_digit())
                            || (radix == 16 && (ch == b'x' || ch == b'X') && at <= 2)
                    });
                    let neg = text.first() == Some(&b'-');
                    if matches!(text.first(), Some(b'-' | b'+')) {
                        text.remove(0);
                    }
                    let digits = if radix == 16 { text.strip_prefix(b"0x").or_else(|| text.strip_prefix(b"0X")).unwrap_or(&text) } else { &text };
                    match std::str::from_utf8(digits).ok().and_then(|d| i64::from_str_radix(d, radix).ok()) {
                        Some(n) => Scanned::Value(Value::int(if neg { -n } else { n })),
                        None => {
                            failed = true;
                            Scanned::Skipped
                        }
                    }
                }
                b'f' | b'e' | b'E' | b'g' => {
                    let text = take(&mut i, &|ch, at| ch.is_ascii_digit() || ch == b'.' || ch == b'e' || ch == b'E' || ((ch == b'-' || ch == b'+') && at == 0) || ch == b'-' || ch == b'+');
                    match std::str::from_utf8(&text).ok().and_then(|t| t.parse::<f64>().ok()) {
                        Some(x) => Scanned::Value(Value::float(x)),
                        None => {
                            failed = true;
                            Scanned::Skipped
                        }
                    }
                }
                b's' => Scanned::Value(Value::str(PhpStr::new(take(&mut i, &|ch, _| !ch.is_ascii_whitespace())))),
                b'c' => {
                    let n = width.unwrap_or(1);
                    let end = (i + n).min(input.len());
                    let out = input[i..end].to_vec();
                    i = end;
                    Scanned::Value(Value::str(PhpStr::new(out)))
                }
                b'[' => {
                    let (negate, members) = set.clone().unwrap_or((false, vec![false; 256]));
                    let text = take(&mut i, &|ch, _| members[ch as usize] != negate);
                    if text.is_empty() {
                        failed = true;
                        Scanned::Skipped
                    } else {
                        Scanned::Value(Value::str(PhpStr::new(text)))
                    }
                }
                _ => Scanned::Skipped,
            }
        };
        if matches!(result, Scanned::Value(_)) {
            any = true;
        }
        if suppress {
            continue;
        }
        let v = match result {
            Scanned::Value(v) => Some(v),
            Scanned::Skipped => None,
        };
        match position {
            Some(p) => {
                if let Some(v) = v {
                    positioned.push((p, v));
                }
                slots = slots.max(p);
            }
            None => {
                values.push(v);
                slots = slots.max(values.len());
            }
        }
    }
    if !positioned.is_empty() {
        values = vec![None; slots];
        for (p, v) in positioned {
            if p >= 1 && p <= values.len() {
                values[p - 1] = Some(v);
            }
        }
    }
    if input.is_empty() && !any {
        return None;
    }
    Some(values)
}

fn finish(ctx: &mut dyn Ctx, func: &str, scanned: Option<Vec<Option<Value>>>, vars: &[Value]) -> Result<Value, Throw> {
    if vars.is_empty() {
        return Ok(match scanned {
            None => Value::NULL,
            Some(values) => Value::array(PhpArray::from_list(values.into_iter().map(|v| v.unwrap_or(Value::NULL)).collect())),
        });
    }
    let Some(values) = scanned else { return Ok(Value::int(-1)) };
    if values.len() != vars.len() {
        let _ = func;
        return Err(ctx.throw("ValueError", "Different numbers of variable names and field specifiers".into()));
    }
    let mut n = 0;
    for (v, var) in values.into_iter().zip(vars) {
        if let (Some(v), Some(cell)) = (v, var.as_reference()) {
            *cell.borrow_mut() = v;
            n += 1;
        }
    }
    Ok(Value::int(n))
}

/// sscanf(string $string, string $format, mixed &...$vars)
fn sscanf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let input = str_arg(args, 0);
    let format = str_arg(args, 1);
    let scanned = scan(input.as_bytes(), format.as_bytes());
    let vars: Vec<Value> = args[2.min(args.len())..].to_vec();
    finish(ctx, "sscanf", scanned, &vars)
}

/// fscanf(resource $stream, string $format, mixed &...$vars): a line.
fn fscanf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let Some(o) = resource_of(&v) else {
        return Err(ctx.throw("TypeError", format!("fscanf(): Argument #1 ($stream) must be of type resource, {} given", v.type_name())));
    };
    let Some(r) = resource(&o) else { return Ok(Value::FALSE) };
    let line = r.stream.borrow_mut().gets(None);
    let Some(line) = line else { return Ok(Value::FALSE) };
    let format = str_arg(args, 1);
    let scanned = scan(&line, format.as_bytes());
    let vars: Vec<Value> = args[2.min(args.len())..].to_vec();
    finish(ctx, "fscanf", scanned, &vars)
}

/// The count after a pack/unpack code: None for none (1), Some(None) for '*'.
fn repeat(f: &[u8], at: &mut usize) -> Option<Option<usize>> {
    if f.get(*at) == Some(&b'*') {
        *at += 1;
        return Some(None);
    }
    let start = *at;
    while *at < f.len() && f[*at].is_ascii_digit() {
        *at += 1;
    }
    if *at > start { std::str::from_utf8(&f[start..*at]).ok()?.parse().ok().map(Some) } else { None }
}

/// Size and byte order of a fixed-width code (bytes, little endian?).
fn int_code(c: u8) -> Option<(usize, bool, bool)> {
    // (size, little endian, signed)
    let native_le = cfg!(target_endian = "little");
    Some(match c {
        b'c' => (1, true, true),
        b'C' => (1, true, false),
        b's' => (2, native_le, true),
        b'S' => (2, native_le, false),
        b'n' => (2, false, false),
        b'v' => (2, true, false),
        b'i' | b'l' => (4, native_le, true),
        b'I' | b'L' => (4, native_le, false),
        b'N' => (4, false, false),
        b'V' => (4, true, false),
        b'q' => (8, native_le, true),
        b'Q' | b'P' | b'J' => (8, c == b'P' || (c != b'J' && native_le), false),
        _ => return None,
    })
}

/// pack(string $format, mixed ...$values)
fn pack(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let format = str_arg(args, 0);
    let f = format.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut next = 1;
    let mut at = 0;
    while at < f.len() {
        let code = f[at];
        at += 1;
        let count = repeat(f, &mut at);
        match code {
            b'a' | b'A' | b'Z' | b'h' | b'H' => {
                if next >= args.len() {
                    return Err(ctx.throw("ValueError", format!("Type {}: not enough arguments", code as char)));
                }
                let s = arg(args, next).to_php_string().unwrap_or_else(PhpStr::empty);
                next += 1;
                let b = s.as_bytes();
                match code {
                    b'h' | b'H' => {
                        let n = count.flatten().unwrap_or(if count.is_some() { b.len() } else { 1 }).min(b.len());
                        let nibbles: Vec<u8> = b[..n].iter().map(|&c| (c as char).to_digit(16).unwrap_or(0) as u8).collect();
                        for pair in nibbles.chunks(2) {
                            let (hi, lo) = (pair[0], *pair.get(1).unwrap_or(&0));
                            out.push(if code == b'H' { (hi << 4) | lo } else { (lo << 4) | hi });
                        }
                    }
                    _ => {
                        let n = match count {
                            Some(None) => b.len() + usize::from(code == b'Z'),
                            Some(Some(n)) => n,
                            None => 1,
                        };
                        let pad = if code == b'A' { b' ' } else { 0 };
                        let mut field: Vec<u8> = b.iter().copied().take(n).collect();
                        if code == b'Z' && n > 0 && field.len() == n {
                            field[n - 1] = 0;
                        }
                        field.resize(n, pad);
                        out.extend(field);
                    }
                }
            }
            b'x' => out.extend(std::iter::repeat_n(0, count.flatten().unwrap_or(1))),
            b'X' => {
                let n = count.flatten().unwrap_or(1).min(out.len());
                out.truncate(out.len() - n);
            }
            b'@' => {
                let n = count.flatten().unwrap_or(0);
                out.resize(n, 0);
            }
            b'f' | b'g' | b'G' | b'd' | b'e' | b'E' => {
                let n = match count {
                    Some(None) => args.len() - next,
                    Some(Some(n)) => n,
                    None => 1,
                };
                for _ in 0..n {
                    let x = arg(args, next).to_float();
                    next += 1;
                    match code {
                        b'f' => out.extend(if cfg!(target_endian = "little") { (x as f32).to_le_bytes() } else { (x as f32).to_be_bytes() }),
                        b'g' => out.extend((x as f32).to_le_bytes()),
                        b'G' => out.extend((x as f32).to_be_bytes()),
                        b'd' => out.extend(if cfg!(target_endian = "little") { x.to_le_bytes() } else { x.to_be_bytes() }),
                        b'e' => out.extend(x.to_le_bytes()),
                        _ => out.extend(x.to_be_bytes()),
                    }
                }
            }
            c => {
                let Some((size, le, _)) = int_code(c) else {
                    return Err(ctx.throw("ValueError", format!("Type {}: unknown format code", c as char)));
                };
                let n = match count {
                    Some(None) => args.len() - next,
                    Some(Some(n)) => n,
                    None => 1,
                };
                if next + n > args.len() {
                    return Err(ctx.throw("ValueError", format!("Type {}: too few arguments", c as char)));
                }
                for _ in 0..n {
                    let v = arg(args, next).to_int() as u64;
                    next += 1;
                    let bytes = if le { v.to_le_bytes()[..size].to_vec() } else { v.to_be_bytes()[8 - size..].to_vec() };
                    out.extend(bytes);
                }
            }
        }
    }
    if next < args.len() {
        ctx.warn(&format!("pack(): {} arguments unused", args.len() - next));
    }
    Ok(Value::str(PhpStr::new(out)))
}

/// unpack(string $format, string $string, int $offset = 0)
fn unpack(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let format = str_arg(args, 0);
    let data = str_arg(args, 1);
    let offset = if args.len() > 2 { arg(args, 2).to_int().max(0) as usize } else { 0 };
    let d = data.as_bytes();
    let mut pos = offset.min(d.len());
    let mut out = PhpArray::new();
    for part in format.as_bytes().split(|&c| c == b'/') {
        if part.is_empty() {
            continue;
        }
        let code = part[0];
        let mut at = 1;
        let count = repeat(part, &mut at);
        let name = &part[at..];
        let key = |i: usize, n: usize, star: bool| -> Key {
            let base = String::from_utf8_lossy(name).into_owned();
            if (n > 1 || star) || (base.is_empty()) {
                let k = format!("{base}{}", i + 1);
                Key::from_string(PhpStr::from(k))
            } else {
                Key::from_string(PhpStr::from(base))
            }
        };
        match code {
            b'a' | b'A' | b'Z' | b'h' | b'H' => {
                let len = match count {
                    Some(None) => d.len() - pos,
                    Some(Some(n)) => if code == b'h' || code == b'H' { n.div_ceil(2) } else { n },
                    None => 1,
                };
                if pos + len > d.len() {
                    ctx.warn(&format!("unpack(): Type {}: not enough input values, need {len} values but only {} were provided", code as char, d.len() - pos));
                    return Ok(Value::FALSE);
                }
                let field = &d[pos..pos + len];
                pos += len;
                let v: Vec<u8> = match code {
                    b'a' => field.to_vec(),
                    b'A' => {
                        let end = field.iter().rposition(|c| !matches!(c, b' ' | 0 | b'\t' | b'\r' | b'\n')).map_or(0, |e| e + 1);
                        field[..end].to_vec()
                    }
                    b'Z' => field.iter().take_while(|&&c| c != 0).copied().collect(),
                    _ => {
                        let digits: Vec<u8> = field
                            .iter()
                            .flat_map(|&b| {
                                let (hi, lo) = (b >> 4, b & 15);
                                if code == b'H' { [hi, lo] } else { [lo, hi] }
                            })
                            .map(|n| b"0123456789abcdef"[n as usize])
                            .collect();
                        let n = match count {
                            Some(Some(n)) => n.min(digits.len()),
                            _ => digits.len(),
                        };
                        digits[..n].to_vec()
                    }
                };
                let k = if name.is_empty() { Key::Int(1) } else { Key::from_string(PhpStr::from(name)) };
                out.insert(k, Value::str(PhpStr::new(v)));
            }
            b'x' => pos += count.flatten().unwrap_or(1),
            b'X' => pos = pos.saturating_sub(count.flatten().unwrap_or(1)),
            b'@' => pos = offset + count.flatten().unwrap_or(0),
            c => {
                let (size, le, signed, float) = match c {
                    b'f' | b'g' | b'G' => (4, c == b'g' || (c == b'f' && cfg!(target_endian = "little")), true, true),
                    b'd' | b'e' | b'E' => (8, c == b'e' || (c == b'd' && cfg!(target_endian = "little")), true, true),
                    _ => match int_code(c) {
                        Some((size, le, signed)) => (size, le, signed, false),
                        None => return Err(ctx.throw("ValueError", format!("Type {}: unknown format code", c as char))),
                    },
                };
                let (n, star) = match count {
                    Some(None) => ((d.len() - pos) / size, true),
                    Some(Some(n)) => (n, false),
                    None => (1, false),
                };
                for i in 0..n {
                    if pos + size > d.len() {
                        ctx.warn(&format!("unpack(): Type {}: not enough input values, need {size} values but only {} were provided", c as char, d.len() - pos));
                        return Ok(Value::FALSE);
                    }
                    let b = &d[pos..pos + size];
                    pos += size;
                    let mut buf = [0u8; 8];
                    let v = if float {
                        if size == 4 {
                            let arr: [u8; 4] = b.try_into().expect("4 bytes");
                            Value::float(f64::from(if le { f32::from_le_bytes(arr) } else { f32::from_be_bytes(arr) }))
                        } else {
                            let arr: [u8; 8] = b.try_into().expect("8 bytes");
                            Value::float(if le { f64::from_le_bytes(arr) } else { f64::from_be_bytes(arr) })
                        }
                    } else {
                        if le {
                            buf[..size].copy_from_slice(b);
                        } else {
                            buf[8 - size..].copy_from_slice(b);
                        }
                        let raw = if le { u64::from_le_bytes(buf) } else { u64::from_be_bytes(buf) };
                        let v = if signed && size < 8 {
                            let shift = 64 - size * 8;
                            ((raw << shift) as i64) >> shift
                        } else {
                            raw as i64
                        };
                        Value::int(v)
                    };
                    out.insert(key(i, n, star), v);
                }
            }
        }
    }
    Ok(Value::array(out))
}

const INI_SCANNER_RAW: i64 = 1;
const INI_SCANNER_TYPED: i64 = 2;

/// An INI value as PHP gives it: quoted strings as written; bare words
/// true/on/yes "1", false/off/no/none and null ""; typed mode converts.
fn ini_value(ctx: &mut dyn Ctx, raw: &[u8], mode: i64) -> Value {
    let t = raw.trim_ascii();
    if let Some(q) = t.strip_prefix(b"\"").and_then(|r| r.strip_suffix(b"\"")) {
        return Value::str(PhpStr::from(q));
    }
    if let Some(q) = t.strip_prefix(b"'").and_then(|r| r.strip_suffix(b"'")) {
        return Value::str(PhpStr::from(q));
    }
    // An inline comment ends the value.
    let t = match t.iter().position(|&c| c == b';') {
        Some(p) => t[..p].trim_ascii(),
        None => t,
    };
    if mode == INI_SCANNER_RAW {
        return Value::str(PhpStr::from(t));
    }
    let lower = t.to_ascii_lowercase();
    match lower.as_slice() {
        b"true" | b"on" | b"yes" => return if mode == INI_SCANNER_TYPED { Value::TRUE } else { Value::str("1") },
        b"false" | b"off" | b"no" | b"none" => return if mode == INI_SCANNER_TYPED { Value::FALSE } else { Value::str("") },
        b"null" => return if mode == INI_SCANNER_TYPED { Value::NULL } else { Value::str("") },
        _ => {}
    }
    if mode == INI_SCANNER_TYPED
        && let Some(n) = p8_runtime::numeric::numeric_string(t)
    {
        return match n {
            p8_runtime::numeric::Number::Int(i) if !t.contains(&b'.') => Value::int(i),
            p8_runtime::numeric::Number::Int(i) => Value::float(i as f64),
            p8_runtime::numeric::Number::Float(f) => Value::float(f),
        };
    }
    // A constant's name stands for its value.
    if t.iter().all(|c| c.is_ascii_uppercase() || *c == b'_' || c.is_ascii_digit()) && t.first().is_some_and(u8::is_ascii_uppercase)
        && let Some(v) = ctx.constant(t)
    {
        return Value::str(v.to_php_string().unwrap_or_else(PhpStr::empty));
    }
    Value::str(PhpStr::from(t))
}

fn parse_ini(ctx: &mut dyn Ctx, text: &[u8], sections: bool, mode: i64) -> Value {
    let mut root = PhpArray::new();
    let mut section: Option<(Key, PhpArray)> = None;
    let flush = |root: &mut PhpArray, section: &mut Option<(Key, PhpArray)>| {
        if let Some((k, a)) = section.take() {
            root.insert(k, Value::array(a));
        }
    };
    for line in text.split(|&c| c == b'\n') {
        let line = line.trim_ascii();
        if line.is_empty() || line[0] == b';' || line[0] == b'#' {
            continue;
        }
        if line[0] == b'[' {
            if let Some(end) = line.iter().position(|&c| c == b']') {
                let name = line[1..end].trim_ascii();
                if sections {
                    flush(&mut root, &mut section);
                    section = Some((Key::from_string(PhpStr::from(name)), PhpArray::new()));
                }
            }
            continue;
        }
        let (k, v) = match line.iter().position(|&c| c == b'=') {
            Some(e) => (line[..e].trim_ascii(), ini_value(ctx, &line[e + 1..], mode)),
            None => (line, if mode == INI_SCANNER_TYPED { Value::NULL } else { Value::str("") }),
        };
        let target = match &mut section {
            Some((_, a)) => a,
            None => &mut root,
        };
        // key[] and key[sub]: arrays.
        if let Some(open) = k.iter().position(|&c| c == b'[')
            && k.ends_with(b"]")
        {
            let base = Key::from_string(PhpStr::from(&k[..open]));
            let sub = &k[open + 1..k.len() - 1];
            let mut arr = match target.get(&base).and_then(Value::as_array) {
                Some(a) => a.clone(),
                None => PhpArray::new(),
            };
            if sub.is_empty() {
                let _ = arr.push(v);
            } else {
                arr.insert(Key::from_string(PhpStr::from(sub)), v);
            }
            target.insert(base, Value::array(arr));
        } else {
            target.insert(Key::from_string(PhpStr::from(k)), v);
        }
    }
    flush(&mut root, &mut section);
    Value::array(root)
}

/// parse_ini_string(string $ini_string, bool $process_sections = false, int $scanner_mode = INI_SCANNER_NORMAL)
fn parse_ini_string(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let text = str_arg(args, 0);
    let sections = args.len() > 1 && arg(args, 1).to_bool();
    let mode = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    Ok(parse_ini(ctx, text.as_bytes(), sections, mode))
}

fn parse_ini_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    if path.is_empty() {
        return Err(ctx.throw("ValueError", "parse_ini_file(): Argument #1 ($filename) cannot be empty".into()));
    }
    let Ok(text) = std::fs::read(&path) else { return Ok(Value::FALSE) };
    let sections = args.len() > 1 && arg(args, 1).to_bool();
    let mode = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    Ok(parse_ini(ctx, &text, sections, mode))
}

/// assert(mixed $assertion, Throwable|string|null $description = null):
/// the compiler passes the assertion's source as a third argument.
/// The assert.* settings (assert_options()) apply: off, it passes; failing,
/// assert.callback is called (file, line, null, description), then an
/// AssertionError is thrown (assert.exception) or a warning shown
/// (assert.warning); assert.bail ends the script.
fn assert(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let setting = |ctx: &mut dyn Ctx, name: &[u8]| match ctx.ini_get(name) {
        Some(v) if v.is_str() => {
            let t = v.as_str().expect("a string").as_bytes().to_ascii_lowercase();
            matches!(t.as_slice(), b"true" | b"yes" | b"on") || v.to_int() != 0
        }
        Some(v) => v.to_bool(),
        None => false,
    };
    // A holding assertion is true whatever assert.active says: checked
    // first, so the common case reads no setting.
    if arg(args, 0).to_bool() || !setting(ctx, b"assert.active") {
        return Ok(Value::TRUE);
    }
    let message = match arg(args, 1) {
        v if v.is_null() && args.len() > 2 => arg(args, 2).to_php_string().unwrap_or_else(PhpStr::empty),
        // Called by value (`$f = 'assert'`, a callable, a pipe): no
        // source from the compiler, and php_assert throws with no message.
        v if v.is_null() => PhpStr::empty(),
        v if v.as_object().is_some_and(|o| o.class.is_a(b"throwable")) => return Err(Throw::Object(v.cloned_object().expect("an object"))),
        v => v.to_php_string().unwrap_or_else(PhpStr::empty),
    };
    let message = String::from_utf8_lossy(message.as_bytes()).into_owned();
    match ctx.ini_get(b"assert.callback") {
        None => {}
        Some(v) if v.is_null() => {}
        Some(v) if v.as_str().is_some_and(|s| s.is_empty()) => {}
        Some(callback) => {
            let Throw::Error { file, line, .. } = ctx.throw("Error", String::new()) else { unreachable!("an engine error") };
            let mut call = vec![Value::str(PhpStr::from(&*file)), Value::int(i64::from(line)), Value::NULL];
            // The description: given, or the assertion's source the compiler passes.
            if args.len() > 2 || (args.len() > 1 && !arg(args, 1).is_null()) {
                call.push(Value::str(PhpStr::from(message.as_str())));
            }
            ctx.call(&callback, &mut call)?;
        }
    }
    let bail = setting(ctx, b"assert.bail");
    if setting(ctx, b"assert.exception") {
        return Err(ctx.throw("AssertionError", message));
    }
    if setting(ctx, b"assert.warning") {
        ctx.warn(&format!("assert(): {message} failed"));
    }
    if bail {
        return Err(Throw::Exit(255));
    }
    Ok(Value::FALSE)
}

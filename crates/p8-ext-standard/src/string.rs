//! ext/standard's string functions. Arguments arrive already checked and
//! converted to their declared types (p8_runtime::args); an optional
//! argument the call left out is not in `args`.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/string.c

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "chr", run: chr, by_ref: &[] },
    Builtin { name: "ord", run: ord, by_ref: &[] },
    Builtin { name: "strtolower", run: strtolower, by_ref: &[] },
    Builtin { name: "strtoupper", run: strtoupper, by_ref: &[] },
    Builtin { name: "ucfirst", run: ucfirst, by_ref: &[] },
    Builtin { name: "lcfirst", run: lcfirst, by_ref: &[] },
    Builtin { name: "ucwords", run: ucwords, by_ref: &[] },
    Builtin { name: "trim", run: trim, by_ref: &[] },
    Builtin { name: "ltrim", run: ltrim, by_ref: &[] },
    Builtin { name: "rtrim", run: rtrim, by_ref: &[] },
    Builtin { name: "chop", run: rtrim, by_ref: &[] },
    Builtin { name: "str_contains", run: str_contains, by_ref: &[] },
    Builtin { name: "str_starts_with", run: str_starts_with, by_ref: &[] },
    Builtin { name: "str_ends_with", run: str_ends_with, by_ref: &[] },
    Builtin { name: "strpos", run: strpos, by_ref: &[] },
    Builtin { name: "stripos", run: stripos, by_ref: &[] },
    Builtin { name: "strrpos", run: strrpos, by_ref: &[] },
    Builtin { name: "strripos", run: strripos, by_ref: &[] },
    Builtin { name: "substr", run: substr, by_ref: &[] },
    Builtin { name: "substr_count", run: substr_count, by_ref: &[] },
    Builtin { name: "str_pad", run: str_pad, by_ref: &[] },
    Builtin { name: "str_split", run: str_split, by_ref: &[] },
    Builtin { name: "implode", run: implode, by_ref: &[] },
    Builtin { name: "join", run: implode, by_ref: &[] },
    Builtin { name: "explode", run: explode, by_ref: &[] },
    Builtin { name: "strrev", run: strrev, by_ref: &[] },
    Builtin { name: "strcmp", run: strcmp, by_ref: &[] },
    Builtin { name: "strcasecmp", run: strcasecmp, by_ref: &[] },
    Builtin { name: "strncmp", run: strncmp, by_ref: &[] },
    Builtin { name: "strncasecmp", run: strncasecmp, by_ref: &[] },
    Builtin { name: "str_replace", run: str_replace, by_ref: &[3] },
    Builtin { name: "str_ireplace", run: str_ireplace, by_ref: &[3] },
    Builtin { name: "strtr", run: strtr, by_ref: &[] },
    Builtin { name: "strstr", run: strstr, by_ref: &[] },
    Builtin { name: "stristr", run: stristr, by_ref: &[] },
    Builtin { name: "strrchr", run: strrchr, by_ref: &[] },
    Builtin { name: "substr_replace", run: substr_replace, by_ref: &[] },
    Builtin { name: "nl2br", run: nl2br, by_ref: &[] },
    Builtin { name: "htmlspecialchars", run: htmlspecialchars, by_ref: &[] },
    Builtin { name: "addslashes", run: addslashes, by_ref: &[] },
    Builtin { name: "stripslashes", run: stripslashes, by_ref: &[] },
    Builtin { name: "bin2hex", run: bin2hex, by_ref: &[] },
    Builtin { name: "hex2bin", run: hex2bin, by_ref: &[] },
    Builtin { name: "basename", run: basename, by_ref: &[] },
    Builtin { name: "dirname", run: dirname, by_ref: &[] },
    Builtin { name: "vsprintf", run: vsprintf, by_ref: &[] },
    Builtin { name: "vprintf", run: vprintf, by_ref: &[] },
];

fn string(bytes: Vec<u8>) -> Result<Value, Throw> {
    Ok(Value::Str(PhpStr::new(bytes)))
}

/// Optional argument `i`: None when left out or passed as null.
fn opt(args: &[Value], i: usize) -> Option<Value> {
    match args.get(i).map(Value::deref) {
        None | Some(Value::Null) => None,
        v => v,
    }
}

fn chr(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    string(vec![arg(args, 0).to_int().rem_euclid(256) as u8])
}

fn ord(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(i64::from(str_arg(args, 0).as_bytes().first().copied().unwrap_or(0))))
}

fn strtolower(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    string(str_arg(args, 0).as_bytes().to_ascii_lowercase())
}

fn strtoupper(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    string(str_arg(args, 0).as_bytes().to_ascii_uppercase())
}

fn ucfirst(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut s = str_arg(args, 0).as_bytes().to_vec();
    if let Some(c) = s.first_mut() {
        c.make_ascii_uppercase();
    }
    string(s)
}

fn lcfirst(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut s = str_arg(args, 0).as_bytes().to_vec();
    if let Some(c) = s.first_mut() {
        c.make_ascii_lowercase();
    }
    string(s)
}

/// ucwords($string, $separators = " \t\r\n\f\v")
fn ucwords(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut s = str_arg(args, 0).as_bytes().to_vec();
    let seps = if args.len() > 1 { char_mask(str_arg(args, 1).as_bytes()) } else { char_mask(b" \t\r\n\x0c\x0b") };
    let mut start = true;
    for c in &mut s {
        if start {
            c.make_ascii_uppercase();
        }
        start = seps[*c as usize];
    }
    string(s)
}

/// The bytes a character list names, with `a..z` ranges (php_charmask).
fn char_mask(list: &[u8]) -> [bool; 256] {
    let mut mask = [false; 256];
    let mut i = 0;
    while i < list.len() {
        let c = list[i];
        if i + 3 < list.len() && list[i + 1] == b'.' && list[i + 2] == b'.' && list[i + 3] >= c {
            for b in c..=list[i + 3] {
                mask[b as usize] = true;
            }
            i += 4;
        } else {
            mask[c as usize] = true;
            i += 1;
        }
    }
    mask
}

fn trim_with(args: &[Value], left: bool, right: bool) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let mask = if args.len() > 1 { char_mask(str_arg(args, 1).as_bytes()) } else { char_mask(b" \n\r\t\x0b\0") };
    let mut bytes = s.as_bytes();
    if left {
        while let Some((&c, rest)) = bytes.split_first() {
            if !mask[c as usize] {
                break;
            }
            bytes = rest;
        }
    }
    if right {
        while let Some((&c, rest)) = bytes.split_last() {
            if !mask[c as usize] {
                break;
            }
            bytes = rest;
        }
    }
    if bytes.len() == s.len() {
        return Ok(Value::Str(s));
    }
    string(bytes.to_vec())
}

fn trim(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    trim_with(args, true, true)
}

fn ltrim(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    trim_with(args, true, false)
}

fn rtrim(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    trim_with(args, false, true)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(haystack.len());
    }
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

fn str_contains(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(find(str_arg(args, 0).as_bytes(), str_arg(args, 1).as_bytes()).is_some()))
}

fn str_starts_with(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(str_arg(args, 0).as_bytes().starts_with(str_arg(args, 1).as_bytes())))
}

fn str_ends_with(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(str_arg(args, 0).as_bytes().ends_with(str_arg(args, 1).as_bytes())))
}

/// The haystack offset an `$offset` argument names, or the ValueError PHP
/// raises when it is outside the haystack.
fn offset(ctx: &dyn Ctx, func: &str, args: &[Value], len: usize) -> Result<usize, Throw> {
    let off = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    let at = if off < 0 { len as i64 + off } else { off };
    if at < 0 || at > len as i64 {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)")));
    }
    Ok(at as usize)
}

fn position(ctx: &dyn Ctx, func: &str, args: &[Value], fold: bool) -> Result<Value, Throw> {
    let (h, n) = (str_arg(args, 0), str_arg(args, 1));
    let (h, n) = if fold {
        (h.as_bytes().to_ascii_lowercase(), n.as_bytes().to_ascii_lowercase())
    } else {
        (h.as_bytes().to_vec(), n.as_bytes().to_vec())
    };
    let at = offset(ctx, func, args, h.len())?;
    Ok(find(&h[at..], &n).map_or(Value::Bool(false), |i| Value::Int((at + i) as i64)))
}

fn strpos(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    position(ctx, "strpos", args, false)
}

fn stripos(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    position(ctx, "stripos", args, true)
}

/// strrpos: a negative offset ends the search that far from the end
/// (the needle may start there); a positive one starts it.
fn last_position(ctx: &dyn Ctx, func: &str, args: &[Value], fold: bool) -> Result<Value, Throw> {
    let (h, n) = (str_arg(args, 0), str_arg(args, 1));
    let (h, n) = if fold {
        (h.as_bytes().to_ascii_lowercase(), n.as_bytes().to_ascii_lowercase())
    } else {
        (h.as_bytes().to_vec(), n.as_bytes().to_vec())
    };
    let off = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    let len = h.len() as i64;
    if off > len || off < -len {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)")));
    }
    let found = if off >= 0 {
        rfind(&h[off as usize..], &n).map(|i| i + off as usize)
    } else {
        // The needle may start at most at len + off.
        let end = ((len + off) as usize + n.len()).min(h.len());
        rfind(&h[..end], &n)
    };
    Ok(found.map_or(Value::Bool(false), |i| Value::Int(i as i64)))
}

fn strrpos(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    last_position(ctx, "strrpos", args, false)
}

fn strripos(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    last_position(ctx, "strripos", args, true)
}

/// The byte range substr($string, $offset, $length) takes (PHP 8: an
/// offset past the end gives "").
fn sub_range(len: usize, offset: i64, length: Option<i64>) -> (usize, usize) {
    let len = len as i64;
    let start = if offset < 0 { (len + offset).max(0) } else { offset.min(len) };
    let end = match length {
        None => len,
        Some(l) if l < 0 => (len + l).max(start),
        Some(l) => (start + l).min(len),
    };
    (start as usize, end.max(start) as usize)
}

fn substr(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let (a, b) = sub_range(s.len(), arg(args, 1).to_int(), opt(args, 2).map(|v| v.to_int()));
    string(s.as_bytes()[a..b].to_vec())
}

fn substr_count(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (h, n) = (str_arg(args, 0), str_arg(args, 1));
    if n.is_empty() {
        return Err(ctx.throw("ValueError", "substr_count(): Argument #2 ($needle) cannot be empty".into()));
    }
    let len = h.len() as i64;
    let off = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    let start = if off < 0 { len + off } else { off };
    if start < 0 || start > len {
        return Err(ctx.throw("ValueError", "substr_count(): Argument #3 ($offset) must be contained in argument #1 ($haystack)".into()));
    }
    let end = match opt(args, 3).map(|v| v.to_int()) {
        None => len,
        Some(l) => {
            let end = if l < 0 { len + l } else { start + l };
            if end < start || end > len {
                return Err(ctx.throw("ValueError", "substr_count(): Argument #4 ($length) must be contained in argument #1 ($haystack)".into()));
            }
            end
        }
    };
    let (hay, needle) = (&h.as_bytes()[start as usize..end as usize], n.as_bytes());
    let (mut count, mut i) = (0, 0);
    while let Some(at) = find(&hay[i..], needle) {
        count += 1;
        i += at + needle.len();
    }
    Ok(Value::Int(count))
}

/// str_pad($string, $length, $pad_string = " ", $pad_type = STR_PAD_RIGHT)
fn str_pad(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let length = arg(args, 1).to_int();
    let pad = if args.len() > 2 { str_arg(args, 2) } else { PhpStr::from(" ") };
    let kind = if args.len() > 3 { arg(args, 3).to_int() } else { 1 };
    if length < 0 || length as usize <= s.len() {
        return Ok(Value::Str(s));
    }
    if pad.is_empty() {
        return Err(ctx.throw("ValueError", "str_pad(): Argument #3 ($pad_string) must be a non-empty string".into()));
    }
    if !(0..=2).contains(&kind) {
        return Err(ctx.throw("ValueError", "str_pad(): Argument #4 ($pad_type) must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH".into()));
    }
    let total = length as usize - s.len();
    let (left, right) = match kind {
        0 => (total, 0),
        2 => (total / 2, total - total / 2),
        _ => (0, total),
    };
    let fill = |n: usize| pad.as_bytes().iter().copied().cycle().take(n).collect::<Vec<u8>>();
    string([fill(left), s.as_bytes().to_vec(), fill(right)].concat())
}

fn str_split(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let n = if args.len() > 1 { arg(args, 1).to_int() } else { 1 };
    if n < 1 {
        return Err(ctx.throw("ValueError", "str_split(): Argument #2 ($length) must be greater than 0".into()));
    }
    let parts = s.as_bytes().chunks(n as usize).map(|c| Value::Str(PhpStr::from(c))).collect();
    Ok(Value::Array(Rc::new(PhpArray::from_list(parts))))
}

/// implode(string $separator, array $array) or implode(array $array).
fn implode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (sep, list) = match (arg(args, 0), opt(args, 1)) {
        (Value::Array(a), None) => (PhpStr::empty(), a),
        (Value::Array(_), Some(_)) => {
            return Err(ctx.throw("TypeError", "implode(): Argument #1 ($separator) must be of type string, array given".into()));
        }
        (_, None) => {
            return Err(ctx.throw("TypeError", "implode(): Argument #2 ($array) must be of type ?array, null given".into()));
        }
        (sep, Some(Value::Array(a))) => (sep.to_php_string().unwrap_or_else(PhpStr::empty), a),
        (_, Some(other)) => {
            return Err(ctx.throw("TypeError", format!("implode(): Argument #2 ($array) must be of type ?array, {} given", other.type_name())));
        }
    };
    let mut out = Vec::new();
    let mut diags = Vec::new();
    for (i, (_, v)) in list.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(sep.as_bytes());
        }
        // An object: its __toString().
        if let Value::Object(o) = v.deref() {
            if o.class.method(b"__tostring").is_none() {
                return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(o.class.name.as_bytes()))));
            }
            let method = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(o), Value::Str(PhpStr::from("__toString"))])));
            let s = ctx.call(&method, &mut [])?;
            out.extend_from_slice(s.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes());
            continue;
        }
        p8_runtime::ops::push_string(&mut out, v, &mut diags);
    }
    if !diags.is_empty() {
        ctx.warn("Array to string conversion");
    }
    string(out)
}

/// explode($separator, $string, $limit = PHP_INT_MAX)
fn explode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (sep, s) = (str_arg(args, 0), str_arg(args, 1));
    // A limit of 0 means 1.
    let limit = if args.len() > 2 { arg(args, 2).to_int().max(i64::MIN + 1) } else { i64::MAX };
    let limit = if limit == 0 { 1 } else { limit };
    if sep.is_empty() {
        return Err(ctx.throw("ValueError", "explode(): Argument #1 ($separator) cannot be empty".into()));
    }
    let (s, sep) = (s.as_bytes(), sep.as_bytes());
    let mut parts: Vec<&[u8]> = Vec::new();
    let mut rest = s;
    while let Some(at) = find(rest, sep) {
        if limit > 0 && parts.len() as i64 == limit - 1 {
            break;
        }
        parts.push(&rest[..at]);
        rest = &rest[at + sep.len()..];
    }
    parts.push(rest);
    if limit < 0 {
        let keep = parts.len() as i64 + limit;
        parts.truncate(keep.max(0) as usize);
    }
    let values = parts.into_iter().map(|p| Value::Str(PhpStr::from(p))).collect();
    Ok(Value::Array(Rc::new(PhpArray::from_list(values))))
}

fn strrev(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut s = str_arg(args, 0).as_bytes().to_vec();
    s.reverse();
    string(s)
}

/// -1, 0 or 1, as PHP 8.2 normalizes string comparisons.
fn sign(o: std::cmp::Ordering) -> Result<Value, Throw> {
    Ok(Value::Int(o as i64))
}

fn strcmp(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    sign(str_arg(args, 0).as_bytes().cmp(str_arg(args, 1).as_bytes()))
}

fn strcasecmp(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    sign(str_arg(args, 0).as_bytes().to_ascii_lowercase().cmp(&str_arg(args, 1).as_bytes().to_ascii_lowercase()))
}

fn prefix(ctx: &dyn Ctx, func: &str, args: &[Value]) -> Result<usize, Throw> {
    let n = arg(args, 2).to_int();
    if n < 0 {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #3 ($length) must be greater than or equal to 0")));
    }
    Ok(n as usize)
}

fn strncmp(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = prefix(ctx, "strncmp", args)?;
    let (a, b) = (str_arg(args, 0), str_arg(args, 1));
    sign(a.as_bytes()[..n.min(a.len())].cmp(&b.as_bytes()[..n.min(b.len())]))
}

fn strncasecmp(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = prefix(ctx, "strncasecmp", args)?;
    let (a, b) = (str_arg(args, 0).as_bytes().to_ascii_lowercase(), str_arg(args, 1).as_bytes().to_ascii_lowercase());
    sign(a[..n.min(a.len())].cmp(&b[..n.min(b.len())]))
}

/// Replace every `search` in `subject` (case-insensitively if `fold`),
/// counting replacements.
fn replace_one(subject: &[u8], search: &[u8], replace: &[u8], fold: bool, count: &mut i64) -> Vec<u8> {
    if search.is_empty() {
        return subject.to_vec();
    }
    let hay = if fold { subject.to_ascii_lowercase() } else { subject.to_vec() };
    let needle = if fold { search.to_ascii_lowercase() } else { search.to_vec() };
    let mut out = Vec::with_capacity(subject.len());
    let mut i = 0;
    while let Some(at) = find(&hay[i..], &needle) {
        out.extend_from_slice(&subject[i..i + at]);
        out.extend_from_slice(replace);
        i += at + needle.len();
        *count += 1;
    }
    out.extend_from_slice(&subject[i..]);
    out
}

fn replace_in(subject: &Value, search: &Value, replace: &Value, fold: bool, count: &mut i64, diags: &mut Vec<p8_runtime::ops::Diag>) -> Value {
    let mut text = Vec::new();
    p8_runtime::ops::push_string(&mut text, subject, diags);
    match search {
        Value::Array(searches) => {
            let replaces: Vec<Value> = match replace {
                Value::Array(r) => r.values().cloned().collect(),
                other => vec![other.clone(); searches.len()],
            };
            let scalar = !matches!(replace, Value::Array(_));
            for (i, s) in searches.values().enumerate() {
                let mut needle = Vec::new();
                p8_runtime::ops::push_string(&mut needle, s, diags);
                let mut with = Vec::new();
                match replaces.get(i) {
                    Some(r) => p8_runtime::ops::push_string(&mut with, r, diags),
                    None if scalar => {}
                    None => {}
                }
                text = replace_one(&text, &needle, &with, fold, count);
            }
            Value::Str(PhpStr::new(text))
        }
        _ => {
            let mut needle = Vec::new();
            p8_runtime::ops::push_string(&mut needle, search, diags);
            let mut with = Vec::new();
            p8_runtime::ops::push_string(&mut with, replace, diags);
            Value::Str(PhpStr::new(replace_one(&text, &needle, &with, fold, count)))
        }
    }
}

fn replace(ctx: &mut dyn Ctx, args: &mut [Value], fold: bool) -> Result<Value, Throw> {
    let (search, with, subject) = (arg(args, 0), arg(args, 1), arg(args, 2));
    if let (Value::Str(_), Value::Array(_)) = (&search, &with) {
        let name = if fold { "str_ireplace" } else { "str_replace" };
        return Err(ctx.throw("TypeError", format!("{name}(): Argument #2 ($replace) must be of type string when argument #1 ($search) is a string")));
    }
    let mut count = 0;
    let mut diags = Vec::new();
    let result = match &subject {
        Value::Array(items) => {
            let mut out = PhpArray::new();
            for (k, v) in items.iter() {
                let replaced = match v {
                    Value::Array(_) => v.clone(),
                    v => replace_in(v, &search, &with, fold, &mut count, &mut diags),
                };
                out.insert(k, replaced);
            }
            Value::Array(Rc::new(out))
        }
        s => replace_in(s, &search, &with, fold, &mut count, &mut diags),
    };
    if !diags.is_empty() {
        ctx.warn("Array to string conversion");
    }
    if let Some(Value::Ref(cell)) = args.get(3) {
        *cell.borrow_mut() = Value::Int(count);
    }
    Ok(result)
}

fn str_replace(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    replace(ctx, args, false)
}

fn str_ireplace(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    replace(ctx, args, true)
}

/// strtr($string, $from, $to) maps bytes; strtr($string, $pairs) replaces
/// substrings, longest match first, never rescanning replaced text.
fn strtr(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    if args.len() == 2 {
        let Value::Array(pairs) = arg(args, 1) else {
            return Err(ctx.throw("TypeError", format!("strtr(): Argument #2 ($from) must be of type array, {} given", arg(args, 1).type_name())));
        };
        let mut map: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut diags = Vec::new();
        for (k, v) in pairs.iter() {
            let key = match k {
                Key::Int(i) => i.to_string().into_bytes(),
                Key::Str(s) => s.as_bytes().to_vec(),
            };
            if key.is_empty() {
                continue;
            }
            let mut with = Vec::new();
            p8_runtime::ops::push_string(&mut with, v, &mut diags);
            map.push((key, with));
        }
        if !diags.is_empty() {
            ctx.warn("Array to string conversion");
        }
        map.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        'scan: while i < bytes.len() {
            for (from, to) in &map {
                if bytes[i..].starts_with(from) {
                    out.extend_from_slice(to);
                    i += from.len();
                    continue 'scan;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        return string(out);
    }
    if args.len() < 3 {
        return Err(ctx.throw("ArgumentCountError", "strtr() expects exactly 3 arguments, 2 given".into()));
    }
    let (from, to) = (str_arg(args, 1), str_arg(args, 2));
    let n = from.len().min(to.len());
    let mut table: [u8; 256] = std::array::from_fn(|i| i as u8);
    for i in 0..n {
        table[from.as_bytes()[i] as usize] = to.as_bytes()[i];
    }
    string(s.as_bytes().iter().map(|&c| table[c as usize]).collect())
}

fn strstr_with(args: &[Value], fold: bool) -> Result<Value, Throw> {
    let (h, n) = (str_arg(args, 0), str_arg(args, 1));
    let before = args.len() > 2 && arg(args, 2).to_bool();
    let found = if fold {
        find(&h.as_bytes().to_ascii_lowercase(), &n.as_bytes().to_ascii_lowercase())
    } else {
        find(h.as_bytes(), n.as_bytes())
    };
    match found {
        Some(at) if before => string(h.as_bytes()[..at].to_vec()),
        Some(at) => string(h.as_bytes()[at..].to_vec()),
        None => Ok(Value::Bool(false)),
    }
}

fn strstr(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    strstr_with(args, false)
}

fn stristr(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    strstr_with(args, true)
}

/// strrchr($haystack, $needle, $before_needle = false): from the last
/// occurrence of the needle's first byte.
fn strrchr(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (h, n) = (str_arg(args, 0), str_arg(args, 1));
    let before = args.len() > 2 && arg(args, 2).to_bool();
    let c = n.as_bytes().first().copied().unwrap_or(0);
    match h.as_bytes().iter().rposition(|&b| b == c) {
        Some(at) if before => string(h.as_bytes()[..at].to_vec()),
        Some(at) => string(h.as_bytes()[at..].to_vec()),
        None => Ok(Value::Bool(false)),
    }
}

/// substr_replace for string arguments (the array forms come with the
/// array batch).
fn substr_replace(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (s, with) = (str_arg(args, 0), str_arg(args, 1));
    let (a, b) = sub_range(s.len(), arg(args, 2).to_int(), opt(args, 3).map(|v| v.to_int()));
    string([&s.as_bytes()[..a], with.as_bytes(), &s.as_bytes()[b..]].concat())
}

/// nl2br($string, $use_xhtml = true): before each \r\n, \n\r, \n or \r.
fn nl2br(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let tag: &[u8] = if args.len() < 2 || arg(args, 1).to_bool() { b"<br />" } else { b"<br>" };
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\r' || c == b'\n' {
            out.extend_from_slice(tag);
            out.push(c);
            if let Some(&next) = bytes.get(i + 1)
                && (next == b'\r' || next == b'\n')
                && next != c
            {
                out.push(next);
                i += 1;
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    string(out)
}

/// htmlspecialchars with PHP 8.1's default flags (ENT_QUOTES |
/// ENT_SUBSTITUTE | ENT_HTML401) and `$double_encode`.
fn htmlspecialchars(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 3 | 8 };
    let double = args.len() < 4 || arg(args, 3).to_bool();
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'&' if !double && entity_at(&bytes[i..]).is_some() => {
                let n = entity_at(&bytes[i..]).unwrap_or(1);
                out.extend_from_slice(&bytes[i..i + n]);
                i += n;
                continue;
            }
            b'&' => out.extend_from_slice(b"&amp;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'"' if flags & 2 != 0 => out.extend_from_slice(b"&quot;"),
            b'\'' if flags & 3 == 3 => out.extend_from_slice(b"&#039;"),
            c => out.push(c),
        }
        i += 1;
    }
    string(out)
}

/// The length of an entity (`&name;`, `&#123;`, `&#x1F;`) at the start.
fn entity_at(s: &[u8]) -> Option<usize> {
    let end = s.iter().take(40).position(|&c| c == b';')?;
    let body = &s[1..end];
    let ok = match body {
        [b'#', b'x' | b'X', hex @ ..] => !hex.is_empty() && hex.iter().all(u8::is_ascii_hexdigit),
        [b'#', dec @ ..] => !dec.is_empty() && dec.iter().all(u8::is_ascii_digit),
        name => !name.is_empty() && name.iter().all(u8::is_ascii_alphanumeric),
    };
    ok.then_some(end + 1)
}

fn addslashes(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let mut out = Vec::with_capacity(s.len());
    for &c in s.as_bytes() {
        match c {
            b'\'' | b'"' | b'\\' => {
                out.push(b'\\');
                out.push(c);
            }
            0 => out.extend_from_slice(b"\\0"),
            c => out.push(c),
        }
    }
    string(out)
}

fn stripslashes(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            match bytes.get(i + 1) {
                Some(b'0') => out.push(0),
                Some(&c) => out.push(c),
                None => {}
            }
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    string(out)
}

/// hex2bin(): false (with a warning) for an odd length or a non-hex digit.
fn hex2bin(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let s = s.as_bytes();
    if !s.len().is_multiple_of(2) {
        ctx.warn("hex2bin(): Hexadecimal input string must have an even length");
        return Ok(Value::Bool(false));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.chunks(2) {
        match ((pair[0] as char).to_digit(16), (pair[1] as char).to_digit(16)) {
            (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
            _ => {
                ctx.warn("hex2bin(): Input string must be hexadecimal string");
                return Ok(Value::Bool(false));
            }
        }
    }
    string(out)
}

fn bin2hex(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let mut out = Vec::with_capacity(s.len() * 2);
    for &c in s.as_bytes() {
        out.push(b"0123456789abcdef"[(c >> 4) as usize]);
        out.push(b"0123456789abcdef"[(c & 15) as usize]);
    }
    string(out)
}

/// basename($path, $suffix = ""), for '/' separators (php_basename).
fn basename(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let mut path = s.as_bytes();
    while path.len() > 1 && path.ends_with(b"/") {
        path = &path[..path.len() - 1];
    }
    if path == b"/" {
        return string(Vec::new());
    }
    let mut base = match path.iter().rposition(|&c| c == b'/') {
        Some(i) => &path[i + 1..],
        None => path,
    };
    if args.len() > 1 {
        let suffix = str_arg(args, 1);
        if !suffix.is_empty() && base.len() > suffix.len() && base.ends_with(suffix.as_bytes()) {
            base = &base[..base.len() - suffix.len()];
        }
    }
    string(base.to_vec())
}

/// dirname($path, $levels = 1), for '/' separators (zend_dirname).
fn dirname(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let levels = if args.len() > 1 { arg(args, 1).to_int() } else { 1 };
    if levels < 1 {
        return Err(ctx.throw("ValueError", "dirname(): Argument #2 ($levels) must be greater than or equal to 1".into()));
    }
    let mut path = s.as_bytes().to_vec();
    for _ in 0..levels {
        let before = path.clone();
        path = parent(&path);
        if path == before {
            break;
        }
    }
    string(path)
}

fn parent(path: &[u8]) -> Vec<u8> {
    if path.is_empty() {
        return Vec::new();
    }
    let mut end = path.len();
    while end > 1 && path[end - 1] == b'/' {
        end -= 1;
    }
    if end == 1 && path[0] == b'/' {
        return b"/".to_vec();
    }
    match path[..end].iter().rposition(|&c| c == b'/') {
        None => b".".to_vec(),
        Some(mut i) => {
            while i > 0 && path[i - 1] == b'/' {
                i -= 1;
            }
            if i == 0 { b"/".to_vec() } else { path[..i].to_vec() }
        }
    }
}

/// vsprintf($format, $values): sprintf with its arguments in an array.
fn vsprintf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let values = crate::printf::array_values(&arg(args, 1));
    let out = crate::printf::formatted(ctx, "vsprintf", str_arg(args, 0).as_bytes(), &values, crate::printf::Missing::Array)?;
    string(out)
}

fn vprintf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let values = crate::printf::array_values(&arg(args, 1));
    let out = crate::printf::formatted(ctx, "vprintf", str_arg(args, 0).as_bytes(), &values, crate::printf::Missing::Array)?;
    ctx.echo(&out);
    Ok(Value::Int(out.len() as i64))
}

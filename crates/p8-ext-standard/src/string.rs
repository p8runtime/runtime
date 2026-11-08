//! ext/standard's string functions. Arguments arrive already checked and
//! converted to their declared types (p8_runtime::args); an optional
//! argument the call left out is not in `args`.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/string.c

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Fault, FastBuiltin, OrFault, Throw};
use p8_runtime::lent;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

/// The string functions on the borrowed ABI: their strings are read in
/// place (no handle made, no program-memory literal copied).
pub static FAST: &[FastBuiltin] = &[
    FastBuiltin { name: "chr", run: chr },
    FastBuiltin { name: "ord", run: ord },
    FastBuiltin { name: "strtolower", run: strtolower },
    FastBuiltin { name: "strtoupper", run: strtoupper },
    FastBuiltin { name: "ucfirst", run: ucfirst },
    FastBuiltin { name: "lcfirst", run: lcfirst },
    FastBuiltin { name: "trim", run: trim },
    FastBuiltin { name: "ltrim", run: ltrim },
    FastBuiltin { name: "rtrim", run: rtrim },
    FastBuiltin { name: "chop", run: rtrim },
    FastBuiltin { name: "str_contains", run: str_contains },
    FastBuiltin { name: "str_starts_with", run: str_starts_with },
    FastBuiltin { name: "str_ends_with", run: str_ends_with },
    FastBuiltin { name: "strpos", run: strpos },
    FastBuiltin { name: "stripos", run: stripos },
    FastBuiltin { name: "strrpos", run: strrpos },
    FastBuiltin { name: "strripos", run: strripos },
    FastBuiltin { name: "substr", run: substr },
    FastBuiltin { name: "explode", run: explode },
    FastBuiltin { name: "strcmp", run: strcmp },
    FastBuiltin { name: "strcasecmp", run: strcasecmp },
    FastBuiltin { name: "strncmp", run: strncmp },
];

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "ucwords", run: ucwords, by_ref: &[] },
    Builtin { name: "substr_count", run: substr_count, by_ref: &[] },
    Builtin { name: "str_pad", run: str_pad, by_ref: &[] },
    Builtin { name: "str_split", run: str_split, by_ref: &[] },
    Builtin { name: "implode", run: implode, by_ref: &[] },
    Builtin { name: "join", run: implode, by_ref: &[] },
    Builtin { name: "strrev", run: strrev, by_ref: &[] },
    Builtin { name: "strncasecmp", run: strncasecmp, by_ref: &[] },
    Builtin { name: "str_replace", run: str_replace, by_ref: &[3] },
    Builtin { name: "str_ireplace", run: str_ireplace, by_ref: &[3] },
    Builtin { name: "strtr", run: strtr, by_ref: &[] },
    Builtin { name: "strstr", run: strstr, by_ref: &[] },
    Builtin { name: "strchr", run: strstr, by_ref: &[] },
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
    Ok(Value::str(PhpStr::new(bytes)))
}

/// Optional argument `i`: None when left out or passed as null.
fn opt(args: &[Value], i: usize) -> Option<Value> {
    args.get(i).map(Value::deref).filter(|v| !v.is_null())
}

/// Argument 0, a string, as it is (the caller's handle, shared): what a
/// function that changes nothing returns (RETURN_STR_COPY).
fn same(args: &[Value]) -> Value {
    match lent::str(args, 0) {
        Some(_) => lent::at(args, 0).clone(),
        None => Value::str(PhpStr::empty()),
    }
}

fn chr(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let c = lent::int(args, 0);
    if !(0..=255).contains(&c) {
        ctx.deprecated(
            "chr(): Providing a value not in-between 0 and 255 is deprecated, this is because a byte value must be in the [0, 255] interval. The value used will be constrained using % 256",
        );
    }
    Ok(Value::str(PhpStr::byte(c.rem_euclid(256) as u8)))
}

fn ord(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let s = lent::bytes(args, 0);
    // (8.5) a string that isn't exactly one byte
    match s.len() {
        1 => {}
        0 => ctx.deprecated("ord(): Providing an empty string is deprecated"),
        _ => ctx.deprecated("ord(): Providing a string that is not one byte long is deprecated. Use ord($str[0]) instead"),
    }
    Ok(Value::int(i64::from(s.first().copied().unwrap_or(0))))
}

/// Argument 0 with each byte mapped by `f`: the argument itself when
/// nothing changes (as zend_string_tolower hands back the same string),
/// else built once.
fn map_bytes(args: &[Value], changes: impl Fn(&u8) -> bool, f: impl Fn(&u8) -> u8) -> Result<Value, Fault> {
    let bytes = lent::bytes(args, 0);
    let Some(first) = bytes.iter().position(changes) else { return Ok(same(args)) };
    let mut out = PhpStr::scratch(bytes.len());
    out.extend_from_slice(&bytes[..first]);
    out.extend(bytes[first..].iter().map(f));
    Ok(Value::str(PhpStr::from_scratch(out)))
}

fn strtolower(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    map_bytes(args, u8::is_ascii_uppercase, u8::to_ascii_lowercase)
}

fn strtoupper(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    map_bytes(args, u8::is_ascii_lowercase, u8::to_ascii_uppercase)
}

fn ucfirst(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    match lent::bytes(args, 0).first() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return Ok(same(args)),
    }
    let mut s = lent::bytes(args, 0).to_vec();
    s[0].make_ascii_uppercase();
    Ok(Value::str(PhpStr::new(s)))
}

fn lcfirst(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    match lent::bytes(args, 0).first() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return Ok(same(args)),
    }
    let mut s = lent::bytes(args, 0).to_vec();
    s[0].make_ascii_lowercase();
    Ok(Value::str(PhpStr::new(s)))
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

/// The default mask, " \n\r\t\v\0", worked out at compile time.
const WHITESPACE: [bool; 256] = {
    let mut mask = [false; 256];
    let list = b" \n\r\t\x0b\0";
    let mut i = 0;
    while i < list.len() {
        mask[list[i] as usize] = true;
        i += 1;
    }
    mask
};

fn trim_with(args: &[Value], left: bool, right: bool) -> Result<Value, Fault> {
    let s = lent::bytes(args, 0);
    let mask = if args.len() > 1 { char_mask(lent::bytes(args, 1)) } else { WHITESPACE };
    let mut bytes = s;
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
        return Ok(same(args));
    }
    Ok(Value::str(PhpStr::from(bytes)))
}

fn trim(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    trim_with(args, true, true)
}

fn ltrim(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    trim_with(args, true, false)
}

fn rtrim(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
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

fn str_contains(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(find(lent::bytes(args, 0), lent::bytes(args, 1)).is_some()))
}

fn str_starts_with(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::bytes(args, 0).starts_with(lent::bytes(args, 1))))
}

fn str_ends_with(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::bytes(args, 0).ends_with(lent::bytes(args, 1))))
}

/// The haystack offset an `$offset` argument names, or the ValueError PHP
/// raises when it is outside the haystack.
fn offset(ctx: &dyn Ctx, func: &str, args: &[Value], len: usize) -> Result<usize, Throw> {
    let off = if args.len() > 2 { lent::int(args, 2) } else { 0 };
    let at = if off < 0 { len as i64 + off } else { off };
    if at < 0 || at > len as i64 {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)")));
    }
    Ok(at as usize)
}

/// strpos / stripos: the haystack and needle read in place (folded
/// copies only for stripos).
fn position(ctx: &dyn Ctx, func: &str, args: &[Value], fold: bool) -> Result<Value, Throw> {
    let (h, n) = (lent::bytes(args, 0), lent::bytes(args, 1));
    let at = offset(ctx, func, args, h.len())?;
    let found = if fold { find(&h[at..].to_ascii_lowercase(), &n.to_ascii_lowercase()) } else { find(&h[at..], n) };
    Ok(found.map_or(Value::FALSE, |i| Value::int((at + i) as i64)))
}

fn strpos(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    position(ctx, "strpos", args, false).or_fault(ctx)
}

fn stripos(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    position(ctx, "stripos", args, true).or_fault(ctx)
}

/// strrpos: a negative offset ends the search that far from the end
/// (the needle may start there); a positive one starts it.
fn last_position(ctx: &dyn Ctx, func: &str, args: &[Value], fold: bool) -> Result<Value, Throw> {
    let (h, n) = (lent::bytes(args, 0), lent::bytes(args, 1));
    let folded;
    let (h, n): (&[u8], &[u8]) = if fold {
        folded = (h.to_ascii_lowercase(), n.to_ascii_lowercase());
        (&folded.0, &folded.1)
    } else {
        (h, n)
    };
    let off = if args.len() > 2 { lent::int(args, 2) } else { 0 };
    let len = h.len() as i64;
    if off > len || off < -len {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)")));
    }
    let found = if off >= 0 {
        rfind(&h[off as usize..], n).map(|i| i + off as usize)
    } else {
        // The needle may start at most at len + off.
        let end = ((len + off) as usize + n.len()).min(h.len());
        rfind(&h[..end], n)
    };
    Ok(found.map_or(Value::FALSE, |i| Value::int(i as i64)))
}

fn strrpos(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    last_position(ctx, "strrpos", args, false).or_fault(ctx)
}

fn strripos(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    last_position(ctx, "strripos", args, true).or_fault(ctx)
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

fn substr(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let s = lent::bytes(args, 0);
    let (a, b) = sub_range(s.len(), lent::int(args, 1), lent::given(args, 2).map(|_| lent::int(args, 2)));
    // The whole string: itself (RETURN_STR_COPY); else one copy of the part.
    if a == 0 && b == s.len() {
        return Ok(same(args));
    }
    Ok(Value::str(PhpStr::from(&s[a..b])))
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
    Ok(Value::int(count))
}

/// str_pad($string, $length, $pad_string = " ", $pad_type = STR_PAD_RIGHT)
fn str_pad(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let length = arg(args, 1).to_int();
    let pad = if args.len() > 2 { str_arg(args, 2) } else { PhpStr::from(" ") };
    let kind = if args.len() > 3 { arg(args, 3).to_int() } else { 1 };
    if length < 0 || length as usize <= s.len() {
        return Ok(Value::str(s));
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
    let parts = s.as_bytes().chunks(n as usize).map(|c| Value::str(PhpStr::from(c))).collect();
    Ok(Value::array(PhpArray::from_list(parts)))
}

/// implode(string $separator, array $array) or implode(array $array).
fn implode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (sep, list) = match (arg(args, 0).into_array(), opt(args, 1)) {
        (Ok(a), None) => (PhpStr::empty(), a),
        (Ok(_), Some(_)) => {
            return Err(ctx.throw("TypeError", "implode(): Argument #1 ($separator) must be of type string, array given".into()));
        }
        (Err(_), None) => {
            return Err(ctx.throw("TypeError", "implode(): Argument #2 ($array) must be of type ?array, null given".into()));
        }
        (Err(sep), Some(second)) => match second.into_array() {
            Ok(a) => (sep.to_php_string().unwrap_or_else(PhpStr::empty), a),
            Err(other) => {
                return Err(ctx.throw("TypeError", format!("implode(): Argument #2 ($array) must be of type ?array, {} given", other.type_name())));
            }
        },
    };
    // No pieces: the empty string; one string: itself (php_implode), no copy.
    match list.len() {
        0 => return Ok(Value::str(PhpStr::empty())),
        1 => {
            if let Some(s) = list.iter().next().and_then(|(_, v)| v.deref().into_str().ok()) {
                return Ok(Value::str(s));
            }
        }
        _ => {}
    }
    // Built once, in the thread's buffer, sized for the pieces' strings
    // and the separators between them.
    let len = list.iter().map(|(_, v)| p8_runtime::ops::string_len_hint(v)).sum::<usize>() + sep.len() * (list.len() - 1);
    let mut out = PhpStr::scratch(len);
    let mut diags = Vec::new();
    for (i, (_, v)) in list.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(sep.as_bytes());
        }
        // An object: its __toString().
        if let Ok(o) = v.deref().into_object() {
            if o.class.method(b"__tostring").is_none() {
                return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(o.class.name.as_bytes()))));
            }
            let method = Value::array(PhpArray::from_list(vec![Value::object(o), Value::str(PhpStr::from("__toString"))]));
            let s = ctx.call(&method, &mut [])?;
            out.extend_from_slice(s.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes());
            continue;
        }
        p8_runtime::ops::push_string(&mut out, v, &mut diags);
    }
    if !diags.is_empty() {
        ctx.warn("Array to string conversion");
    }
    Ok(Value::str(PhpStr::from_scratch(out)))
}

/// explode($separator, $string, $limit = PHP_INT_MAX)
fn explode(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let (sep, s) = (lent::bytes(args, 0), lent::bytes(args, 1));
    // A limit of 0 means 1.
    let limit = if args.len() > 2 { lent::int(args, 2).max(i64::MIN + 1) } else { i64::MAX };
    let limit = if limit == 0 { 1 } else { limit };
    if sep.is_empty() {
        // (PHP 8.6's wording, string.c PHP_FUNCTION(explode).)
        return Err(ctx.fault_with("ValueError", "explode(): Argument #1 ($separator) must not be empty, use str_split() to split a string into characters".into()));
    }
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
    let values = parts.into_iter().map(|p| Value::str(PhpStr::from(p))).collect();
    Ok(Value::array(PhpArray::from_list(values)))
}

fn strrev(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut s = str_arg(args, 0).as_bytes().to_vec();
    s.reverse();
    string(s)
}

/// -1, 0 or 1, as PHP 8.2 normalizes string comparisons.
fn sign(o: std::cmp::Ordering) -> Result<Value, Throw> {
    Ok(Value::int(o as i64))
}

fn strcmp(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::int(lent::bytes(args, 0).cmp(lent::bytes(args, 1)) as i64))
}

fn strcasecmp(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let (a, b) = (lent::bytes(args, 0), lent::bytes(args, 1));
    let o = a.iter().map(u8::to_ascii_lowercase).cmp(b.iter().map(u8::to_ascii_lowercase));
    Ok(Value::int(o as i64))
}

fn prefix(ctx: &dyn Ctx, func: &str, args: &[Value]) -> Result<usize, Throw> {
    let n = arg(args, 2).to_int();
    if n < 0 {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #3 ($length) must be greater than or equal to 0")));
    }
    Ok(n as usize)
}

fn strncmp(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let n = prefix(ctx, "strncmp", args).or_fault(ctx)?;
    let (a, b) = (lent::bytes(args, 0), lent::bytes(args, 1));
    Ok(Value::int(a[..n.min(a.len())].cmp(&b[..n.min(b.len())]) as i64))
}

fn strncasecmp(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = prefix(ctx, "strncasecmp", args)?;
    let (a, b) = (str_arg(args, 0).as_bytes().to_ascii_lowercase(), str_arg(args, 1).as_bytes().to_ascii_lowercase());
    sign(a[..n.min(a.len())].cmp(&b[..n.min(b.len())]))
}

/// Replace every `search` in `subject` (case-insensitively if `fold`),
/// counting replacements; None when there is nothing to replace (the
/// subject stays as it is, uncopied).
fn replace_one(subject: &[u8], search: &[u8], replace: &[u8], fold: bool, count: &mut i64) -> Option<Vec<u8>> {
    if search.is_empty() {
        return None;
    }
    let lowered;
    let (hay, needle): (&[u8], std::borrow::Cow<[u8]>) = if fold {
        lowered = subject.to_ascii_lowercase();
        (&lowered, std::borrow::Cow::Owned(search.to_ascii_lowercase()))
    } else {
        (subject, std::borrow::Cow::Borrowed(search))
    };
    let first = find(hay, &needle)?;
    let mut out = PhpStr::scratch(subject.len());
    let mut i = 0;
    let mut at = Some(first);
    while let Some(found) = at {
        out.extend_from_slice(&subject[i..i + found]);
        out.extend_from_slice(replace);
        i += found + needle.len();
        *count += 1;
        at = find(&hay[i..], &needle);
    }
    out.extend_from_slice(&subject[i..]);
    Some(out)
}

/// A value's bytes as a string: a string's own, else converted into `buf`.
fn text_of<'a>(v: &'a Value, buf: &'a mut Vec<u8>, diags: &mut Vec<p8_runtime::ops::Diag>) -> &'a [u8] {
    match v.as_str() {
        Some(s) => s.as_bytes(),
        None => {
            p8_runtime::ops::push_string(buf, v, diags);
            buf
        }
    }
}

/// str_replace on one subject. Strings are read where they are; a
/// subject nothing was replaced in is returned as it is (PHP hands back
/// the same string too).
fn replace_in(subject: &Value, search: &Value, replace: &Value, fold: bool, count: &mut i64, diags: &mut Vec<p8_runtime::ops::Diag>) -> Value {
    let mut subject_buf = Vec::new();
    let subject_bytes = text_of(subject, &mut subject_buf, diags);
    // The text once something was replaced.
    let mut text: Option<Vec<u8>> = None;
    match search.as_array() {
        Some(searches) => {
            // An array of replacements pairs up with the searches (a
            // missing one is ""); a single one goes with each.
            let mut replaces = replace.as_array().map(|r| r.values());
            for s in searches.values() {
                let (mut needle_buf, mut with_buf) = (Vec::new(), Vec::new());
                let needle = text_of(s, &mut needle_buf, diags);
                let with: &[u8] = match &mut replaces {
                    Some(it) => match it.next() {
                        Some(r) => text_of(r, &mut with_buf, diags),
                        None => b"",
                    },
                    None => text_of(replace, &mut with_buf, diags),
                };
                if let Some(out) = replace_one(text.as_deref().unwrap_or(subject_bytes), needle, with, fold, count) {
                    text = Some(out);
                }
            }
        }
        None => {
            let (mut needle_buf, mut with_buf) = (Vec::new(), Vec::new());
            let needle = text_of(search, &mut needle_buf, diags);
            let with = text_of(replace, &mut with_buf, diags);
            text = replace_one(subject_bytes, needle, with, fold, count);
        }
    }
    match text {
        Some(t) => Value::str(PhpStr::from_scratch(t)),
        None if subject.is_str() => subject.clone(),
        None => Value::str(PhpStr::new(subject_buf)),
    }
}

fn replace(ctx: &mut dyn Ctx, args: &mut [Value], fold: bool) -> Result<Value, Throw> {
    let (search, with, subject) = (arg(args, 0), arg(args, 1), arg(args, 2));
    if search.is_str() && with.is_array() {
        let name = if fold { "str_ireplace" } else { "str_replace" };
        return Err(ctx.throw("TypeError", format!("{name}(): Argument #2 ($replace) must be of type string when argument #1 ($search) is a string")));
    }
    let mut count = 0;
    let mut diags = Vec::new();
    let result = match subject.as_array() {
        // Every element is a subject, made a string as any is (a nested
        // array is "Array", warned): php_str_replace_common.
        Some(items) => {
            let mut out = PhpArray::new();
            for (k, v) in items.iter() {
                out.insert(k, replace_in(&v.deref(), &search, &with, fold, &mut count, &mut diags));
            }
            Value::array(out)
        }
        None => replace_in(&subject, &search, &with, fold, &mut count, &mut diags),
    };
    // One report per conversion, in order, as zval_get_string makes them.
    for d in diags {
        match d {
            p8_runtime::ops::Diag::Warning(m) => ctx.warn(&m),
            p8_runtime::ops::Diag::Deprecated(m) => ctx.deprecated(&m),
        }
    }
    if let Some(cell) = args.get(3).and_then(Value::as_reference) {
        *cell.borrow_mut() = Value::int(count);
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
        let Ok(pairs) = arg(args, 1).into_array() else {
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
        let mut out = PhpStr::scratch(bytes.len());
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
        return Ok(Value::str(PhpStr::from_scratch(out)));
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
        None => Ok(Value::FALSE),
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
        None => Ok(Value::FALSE),
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
    let scrubbed;
    // Valid UTF-8 (most text) is scrubbed as it is: no copy.
    let bytes = if utf8_charset(args, 2) && std::str::from_utf8(s.as_bytes()).is_err() {
        let Some(clean) = utf8_scrub(s.as_bytes(), flags) else { return string(Vec::new()) };
        scrubbed = clean;
        &scrubbed[..]
    } else {
        s.as_bytes()
    };
    // Nothing to escape: the string itself.
    let special = |c: &u8| matches!(c, b'&' | b'<' | b'>') || (*c == b'"' && flags & 2 != 0) || (*c == b'\'' && flags & 3 == 3);
    if std::ptr::eq(bytes, s.as_bytes()) && !bytes.iter().any(special) {
        return Ok(Value::str(s));
    }
    let mut out = PhpStr::scratch(bytes.len() + 16);
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
    Ok(Value::str(PhpStr::from_scratch(out)))
}

/// Whether the charset argument at `i` is UTF-8 (left out, null or empty:
/// default_charset, UTF-8).
pub(crate) fn utf8_charset(args: &[Value], i: usize) -> bool {
    match args.get(i).map(Value::deref) {
        Some(v) if !v.is_null() => {
            let c = v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_ascii_lowercase();
            c.is_empty() || c == b"utf-8" || c == b"utf8"
        }
        _ => true,
    }
}

/// The text with its invalid UTF-8 sequences (as html.c's get_next_char
/// cuts them) dropped with ENT_IGNORE, made U+FFFD with ENT_SUBSTITUTE;
/// None (an empty result) with neither.
pub(crate) fn utf8_scrub(s: &[u8], flags: i64) -> Option<Vec<u8>> {
    const ENT_IGNORE: i64 = 4;
    const ENT_SUBSTITUTE: i64 = 8;
    let lead = |c: u8| c < 0x80 || (0xc2..=0xf4).contains(&c);
    let trail = |c: u8| (0x80..=0xbf).contains(&c);
    let mut out = Vec::with_capacity(s.len());
    let mut pos = 0;
    while pos < s.len() {
        let c = s[pos];
        let avail = s.len() - pos;
        let at = |i: usize| s.get(pos + i).copied().unwrap_or(0);
        // Ok(length) of a valid character, Err(bytes) of an invalid sequence.
        let step: Result<usize, usize> = if c < 0x80 {
            Ok(1)
        } else if c < 0xc2 {
            Err(1)
        } else if c < 0xe0 {
            if avail < 2 {
                Err(1)
            } else if !trail(at(1)) {
                Err(if lead(at(1)) { 1 } else { 2 })
            } else {
                Ok(2)
            }
        } else if c < 0xf0 {
            if avail < 3 || !trail(at(1)) || !trail(at(2)) {
                Err(if avail < 2 || lead(at(1)) { 1 } else if avail < 3 || lead(at(2)) { 2 } else { 3 })
            } else {
                let cp = (u32::from(c & 0x0f) << 12) | (u32::from(at(1) & 0x3f) << 6) | u32::from(at(2) & 0x3f);
                if cp < 0x800 || (0xd800..=0xdfff).contains(&cp) { Err(3) } else { Ok(3) }
            }
        } else if c < 0xf5 {
            if avail < 4 || !trail(at(1)) || !trail(at(2)) || !trail(at(3)) {
                Err(if avail < 2 || lead(at(1)) {
                    1
                } else if avail < 3 || lead(at(2)) {
                    2
                } else if avail < 4 || lead(at(3)) {
                    3
                } else {
                    4
                })
            } else {
                let cp = (u32::from(c & 0x07) << 18) | (u32::from(at(1) & 0x3f) << 12) | (u32::from(at(2) & 0x3f) << 6) | u32::from(at(3) & 0x3f);
                if !(0x10000..=0x10ffff).contains(&cp) { Err(4) } else { Ok(4) }
            }
        } else {
            Err(1)
        };
        match step {
            Ok(n) => {
                out.extend_from_slice(&s[pos..pos + n]);
                pos += n;
            }
            Err(n) => {
                if flags & ENT_IGNORE != 0 {
                } else if flags & ENT_SUBSTITUTE != 0 {
                    out.extend_from_slice("\u{fffd}".as_bytes());
                } else {
                    return None;
                }
                pos += n.min(avail);
            }
        }
    }
    Some(out)
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
        return Ok(Value::FALSE);
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.chunks(2) {
        match ((pair[0] as char).to_digit(16), (pair[1] as char).to_digit(16)) {
            (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
            _ => {
                ctx.warn("hex2bin(): Input string must be hexadecimal string");
                return Ok(Value::FALSE);
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
    Ok(Value::int(out.len() as i64))
}

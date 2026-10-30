//! grapheme_* (grapheme/grapheme_string.c, grapheme_util.c): extended
//! grapheme clusters from icu_segmenter, with PHP's ASCII shortcuts kept
//! (they decide some results). Searching matches whole clusters by
//! collation equality, as ICU's string search with a grapheme break
//! iterator does: tertiary strength, secondary for the case-insensitive
//! functions.

use crate::{R, U_ILLEGAL_ARGUMENT_ERROR, arg, b, s};
use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::{CollatorBorrowed, CollatorPreferences};
use icu_segmenter::GraphemeClusterSegmenter;
use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    b!("grapheme_strlen", grapheme_strlen),
    b!("grapheme_strpos", grapheme_strpos),
    b!("grapheme_stripos", grapheme_stripos),
    b!("grapheme_strrpos", grapheme_strrpos),
    b!("grapheme_strripos", grapheme_strripos),
    b!("grapheme_substr", grapheme_substr),
    b!("grapheme_strstr", grapheme_strstr),
    b!("grapheme_stristr", grapheme_stristr),
    Builtin { name: "grapheme_extract", run: grapheme_extract, by_ref: &[4] },
    b!("grapheme_str_split", grapheme_str_split),
];

/// grapheme_ascii_check, as PHP writes it (a "\r\n" pair counts as not ASCII).
fn ascii(b: &[u8]) -> bool {
    for i in 0..b.len() {
        if b[i] > 0x7f || (b.get(i + 1) == Some(&b'\n') && b[i] == b'\r') {
            return false;
        }
    }
    true
}

/// The cluster boundaries (byte offsets, 0 and the end included).
fn boundaries(t: &str) -> Vec<usize> {
    GraphemeClusterSegmenter::new().segment_str(t).collect()
}

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    crate::bytes_arg(args, i)
}

fn utf8<'a>(ctx: &mut dyn Ctx, b: &'a [u8], msg: &str) -> Result<Option<&'a str>, Throw> {
    match std::str::from_utf8(b) {
        Ok(t) => Ok(Some(t)),
        Err(_) => {
            crate::set_global_code(ctx, 10);
            crate::set_message(ctx, None, msg)?;
            Ok(None)
        }
    }
}

fn grapheme_strlen(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let b = bytes(args, 0);
    if ascii(&b) {
        return Ok(Value::Int(b.len() as i64));
    }
    let Some(t) = utf8(ctx, &b, "Error converting input string to UTF-16")? else { return Ok(Value::Null) };
    Ok(Value::Int(boundaries(t).len() as i64 - 1))
}

fn outside(offset: i64, len: usize) -> bool {
    offset > len as i64 || (offset < 0 && -offset > len as i64)
}

fn offset_error(ctx: &mut dyn Ctx) -> Throw {
    let name = ctx.builtin_name();
    ctx.throw("ValueError", format!("{name}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"))
}

fn memfind(h: &[u8], n: &[u8], from: usize) -> Option<usize> {
    if from > h.len() {
        return None;
    }
    if n.is_empty() {
        return Some(from);
    }
    h[from..].windows(n.len()).position(|w| w == n).map(|p| p + from)
}

fn collator(ignore_case: bool) -> Option<CollatorBorrowed<'static>> {
    let mut o = CollatorOptions::default();
    o.strength = Some(if ignore_case { Strength::Secondary } else { Strength::Tertiary });
    CollatorBorrowed::try_new(CollatorPreferences::default(), o).ok()
}

/// grapheme_get_haystack_offset: the boundary `offset` clusters in (from the end when negative).
fn haystack_offset(bounds: &[usize], offset: i64) -> Option<usize> {
    let n = bounds.len() as i64 - 1;
    if offset >= 0 {
        if offset > n { None } else { Some(bounds[offset as usize]) }
    } else if -offset > n {
        None
    } else {
        Some(bounds[(n + offset) as usize])
    }
}

/// grapheme_strpos_utf16: Ok(Some((cluster index, byte position))) of the match.
fn search(ctx: &mut dyn Ctx, h: &str, n: &str, offset: i64, ignore_case: bool, last: bool) -> Result<Option<(usize, usize)>, Throw> {
    if h.is_empty() && !n.is_empty() {
        return Ok(None);
    }
    let hb = boundaries(h);
    let count_to = |pos: usize| hb.iter().take_while(|&&b| b < pos).count();
    if n.is_empty() {
        let Some(off) = haystack_offset(&hb, offset) else { return Err(offset_error(ctx)) };
        let idx = if last && offset >= 0 { hb.len() - 1 } else { count_to(off) };
        return Ok(Some((idx, off)));
    }
    let offset_pos = if offset != 0 {
        match haystack_offset(&hb, offset) {
            Some(p) => p,
            None => return Err(offset_error(ctx)),
        }
    } else {
        0
    };
    let k = boundaries(n).len() - 1;
    let Some(c) = collator(ignore_case) else { return Ok(None) };
    let last_b = hb.len() - 1;
    let matches = (0..last_b).filter_map(|i| {
        let lo = (i + k.saturating_sub(2)).max(i + 1);
        let hi = (i + k + 2).min(last_b);
        (lo..=hi).find(|&j| c.compare(&h[hb[i]..hb[j]], n) == std::cmp::Ordering::Equal).map(|_| (i, hb[i]))
    });
    let found = if last {
        let mut matches = matches;
        if offset >= 0 { matches.next_back().filter(|&(_, p)| p >= offset_pos) } else { matches.take_while(|&(_, p)| p <= offset_pos).last() }
    } else {
        matches.into_iter().find(|&(_, p)| p >= offset_pos)
    };
    Ok(found)
}

fn strpos_common(ctx: &mut dyn Ctx, args: &[Value], ignore_case: bool, last: bool) -> R {
    let h = bytes(args, 0);
    let n = bytes(args, 1);
    let offset = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    if outside(offset, h.len()) {
        return Err(offset_error(ctx));
    }
    let h_ascii = ascii(&h);
    let n_ascii = ascii(&n);
    if h_ascii {
        let (hh, nn) = if ignore_case { (h.to_ascii_lowercase(), n.to_ascii_lowercase()) } else { (h.clone(), n.clone()) };
        if last {
            if let Some(p) = strrpos_ascii(&hh, &nn, offset) {
                return Ok(Value::Int(p));
            }
            if n_ascii {
                return Ok(Value::Bool(false));
            }
        } else if offset >= 0 {
            if let Some(p) = memfind(&hh, &nn, offset as usize) {
                return Ok(Value::Int(p as i64));
            }
            if n_ascii {
                return Ok(Value::Bool(false));
            }
        }
    }
    let Some(ht) = utf8(ctx, &h, "Error converting input string to UTF-16")? else { return Ok(Value::Bool(false)) };
    let Some(nt) = utf8(ctx, &n, "Error converting needle string to UTF-16")? else { return Ok(Value::Bool(false)) };
    Ok(match search(ctx, ht, nt, offset, ignore_case, last)? {
        Some((i, _)) => Value::Int(i as i64),
        None => Value::Bool(false),
    })
}

/// grapheme_strrpos_ascii.
fn strrpos_ascii(h: &[u8], n: &[u8], offset: i64) -> Option<i64> {
    let hl = h.len() as i64;
    let nl = n.len() as i64;
    let (p, e) = if offset >= 0 {
        (offset, hl - nl)
    } else if nl > -offset {
        (0, hl - nl)
    } else {
        (0, hl + offset)
    };
    let mut e = e;
    while e >= p {
        if e + nl <= hl && h[e as usize..(e + nl) as usize] == *n {
            return Some(e - p + if offset > 0 { offset } else { 0 });
        }
        e -= 1;
    }
    None
}

fn grapheme_strpos(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strpos_common(ctx, args, false, false)
}

fn grapheme_stripos(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strpos_common(ctx, args, true, false)
}

fn grapheme_strrpos(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strpos_common(ctx, args, false, true)
}

fn grapheme_strripos(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strpos_common(ctx, args, true, true)
}

fn bstr(b: &[u8]) -> Value {
    Value::Str(PhpStr::new(b.to_vec()))
}

fn grapheme_substr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let b = bytes(args, 0);
    let start = arg(args, 1).to_int();
    let no_length = args.len() < 3 || matches!(arg(args, 2), Value::Null);
    let length = if no_length { b.len() as i64 } else { arg(args, 2).to_int() };
    let name = ctx.builtin_name();
    if !(i32::MIN as i64..=i32::MAX as i64).contains(&start) {
        return Err(ctx.throw("ValueError", format!("{name}(): Argument #2 ($offset) is too large")));
    }
    if !(i32::MIN as i64..=i32::MAX as i64).contains(&length) {
        return Err(ctx.throw("ValueError", format!("{name}(): Argument #3 ($length) is too large")));
    }
    if ascii(&b) {
        let len = b.len() as i64;
        let mut f = start;
        if f < 0 {
            f = (len + f).max(0);
        } else if f > len {
            f = len;
        }
        let mut l = length;
        if l < 0 {
            l = ((len - f) + l).max(0);
        } else if l > len - f {
            l = len - f;
        }
        return Ok(bstr(&b[f as usize..(f + l) as usize]));
    }
    let Some(t) = utf8(ctx, &b, "Error converting input string to UTF-16")? else { return Ok(Value::Bool(false)) };
    let bounds = boundaries(t);
    let n = bounds.len() as i64 - 1;
    // the start boundary
    let start_pos;
    let mut st = start;
    if st < 0 {
        let mut idx = n;
        while st != 0 && idx > 0 {
            idx -= 1;
            st += 1;
        }
        if st != 0 {
            start_pos = 0;
        } else {
            start_pos = bounds[idx as usize];
        }
    } else {
        let mut idx = 0;
        while st != 0 && idx < n {
            idx += 1;
            st -= 1;
        }
        if st != 0 {
            return Ok(s(""));
        }
        start_pos = bounds[idx as usize];
    }
    if length >= b.len() as i64 {
        return Ok(bstr(&b[start_pos..]));
    }
    if length == 0 {
        return Ok(s(""));
    }
    let end_pos = if length < 0 {
        let mut idx = n;
        let mut l = length;
        while l != 0 && idx > 0 {
            idx -= 1;
            l += 1;
        }
        if l != 0 {
            return Ok(s(""));
        }
        bounds[idx as usize]
    } else {
        // `length` clusters on from the iterator's position (the start)
        let start_idx = bounds.iter().position(|&x| x == start_pos).unwrap_or(0) as i64;
        let idx = start_idx + length;
        if idx > n { b.len() } else { bounds[idx as usize] }
    };
    if start_pos > end_pos {
        return Ok(s(""));
    }
    Ok(bstr(&b[start_pos..end_pos]))
}

fn strstr_common(ctx: &mut dyn Ctx, args: &[Value], ignore_case: bool) -> R {
    let h = bytes(args, 0);
    let n = bytes(args, 1);
    let before = args.len() > 2 && arg(args, 2).to_bool();
    if !ignore_case && ascii(&h) && ascii(&n) {
        return Ok(match memfind(&h, &n, 0) {
            Some(p) => bstr(if before { &h[..p] } else { &h[p..] }),
            None => Value::Bool(false),
        });
    }
    let Some(ht) = utf8(ctx, &h, "Error converting input string to UTF-16")? else { return Ok(Value::Bool(false)) };
    let Some(nt) = utf8(ctx, &n, "Error converting needle string to UTF-16")? else { return Ok(Value::Bool(false)) };
    Ok(match search(ctx, ht, nt, 0, ignore_case, false)? {
        Some((_, p)) => bstr(if before { &h[..p] } else { &h[p..] }),
        None => Value::Bool(false),
    })
}

fn grapheme_strstr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strstr_common(ctx, args, false)
}

fn grapheme_stristr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strstr_common(ctx, args, true)
}

fn set_ref(args: &mut [Value], i: usize, v: Value) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = v;
    }
}

fn grapheme_extract(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let b = bytes(args, 0);
    let size = arg(args, 1).to_int();
    let ty = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    let mut start = if args.len() > 3 { arg(args, 3).to_int() } else { 0 };
    if start < 0 {
        start += b.len() as i64;
    }
    set_ref(args, 4, Value::Int(start));
    if !(0..=2).contains(&ty) {
        return Err(ctx.throw("ValueError", "grapheme_extract(): Argument #3 ($type) must be one of GRAPHEME_EXTR_COUNT, GRAPHEME_EXTR_MAXBYTES, or GRAPHEME_EXTR_MAXCHARS".into()));
    }
    if start > i32::MAX as i64 || start < 0 || start as usize >= b.len() {
        crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "grapheme_extract: start not contained in string")?;
        return Ok(Value::Bool(false));
    }
    if size < 0 {
        return Err(ctx.throw("ValueError", "grapheme_extract(): Argument #2 ($size) must be greater than or equal to 0".into()));
    }
    if size > i32::MAX as i64 {
        return Err(ctx.throw("ValueError", "grapheme_extract(): Argument #2 ($size) is too large".into()));
    }
    if size == 0 {
        return Ok(s(""));
    }
    let mut p = start as usize;
    while p < b.len() && (b[p] & 0xC0) == 0x80 {
        p += 1;
        if p >= b.len() {
            crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "grapheme_extract: invalid input string")?;
            return Ok(Value::Bool(false));
        }
    }
    let rest = &b[p..];
    let probe = &rest[..rest.len().min(size as usize + 1)];
    if ascii(probe) {
        let n = (size as usize).min(rest.len());
        set_ref(args, 4, Value::Int((start as usize + n) as i64));
        return Ok(bstr(&rest[..n]));
    }
    // ICU reads the text leniently (UText): invalid bytes are characters of their own
    let text = String::from_utf8_lossy(rest).into_owned();
    if text.len() != rest.len() {
        crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "Error opening UTF-8 text")?;
        return Ok(Value::Bool(false));
    }
    let bounds = boundaries(&text);
    let ret = match ty {
        0 => bounds.get((size as usize).min(bounds.len() - 1)).copied().unwrap_or(0),
        1 => bounds.iter().copied().take_while(|&x| x <= size as usize).last().unwrap_or(0),
        _ => {
            let mut ret = 0;
            let mut count = 0;
            for w in bounds.windows(2) {
                count += text[w[0]..w[1]].chars().count() as i64;
                if count > size {
                    break;
                }
                ret = w[1];
            }
            ret
        }
    };
    set_ref(args, 4, Value::Int((p + ret) as i64));
    Ok(bstr(&rest[..ret]))
}

fn grapheme_str_split(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let b = bytes(args, 0);
    let len = if args.len() > 1 { arg(args, 1).to_int() } else { 1 };
    if len <= 0 || len > (u32::MAX / 4) as i64 {
        return Err(ctx.throw("ValueError", format!("grapheme_str_split(): Argument #2 ($length) must be greater than 0 and less than or equal to {}", u32::MAX / 4)));
    }
    if b.is_empty() {
        return Ok(Value::Array(Rc::new(PhpArray::new())));
    }
    let Some(t) = utf8(ctx, &b, "Error opening UTF-8 text")? else { return Ok(Value::Bool(false)) };
    let bounds = boundaries(t);
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < bounds.len() {
        let j = (i + len as usize).min(bounds.len() - 1);
        out.push(bstr(&b[bounds[i]..bounds[j]]));
        i = j;
    }
    Ok(Value::Array(Rc::new(PhpArray::from_list(out))))
}

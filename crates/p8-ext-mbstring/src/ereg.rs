//! The mb_ereg family (mbregex) over p8's PCRE engine instead of
//! Oniguruma: patterns in the Ruby syntax mbregex uses by default, which
//! for what PHP code writes is PCRE's, with Oniguruma's options mapped
//! (`m` is PCRE's dotall; without `s`, `^` and `$` match at line
//! boundaries, as Ruby's always do).
//!
//! Reference: $PHP_SRC_PATH/ext/mbstring/php_mbregex.c

use crate::funcs::{R, arg_error, int_arg, opt_str, str_arg};
use p8_pcre::{Captures, MatchOptions, Options, Regex};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "mb_ereg", run: mb_ereg, by_ref: &[2] },
    Builtin { name: "mb_eregi", run: mb_eregi, by_ref: &[2] },
    Builtin { name: "mb_ereg_match", run: mb_ereg_match, by_ref: &[] },
    Builtin { name: "mb_ereg_replace", run: mb_ereg_replace, by_ref: &[] },
    Builtin { name: "mb_eregi_replace", run: mb_eregi_replace, by_ref: &[] },
    Builtin { name: "mb_ereg_replace_callback", run: mb_ereg_replace_callback, by_ref: &[] },
    Builtin { name: "mb_split", run: mb_split, by_ref: &[] },
];

/// Oniguruma's option bits as mb_regex_set_options keeps them.
const CASELESS: i64 = 1;
const EXTENDED: i64 = 2;
const MULTILINE: i64 = 4;
const SINGLELINE: i64 = 8;

/// The options in force: an options argument's, else mb_regex_set_options'.
fn option_bits(ctx: &mut dyn Ctx, given: Option<Vec<u8>>) -> i64 {
    match given {
        Some(o) => o.iter().fold(0, |bits, c| match c {
            b'i' => bits | CASELESS,
            b'x' => bits | EXTENDED,
            b'm' => bits | MULTILINE,
            b's' => bits | SINGLELINE,
            b'p' => bits | MULTILINE | SINGLELINE,
            _ => bits,
        }),
        None => match ctx.state("mb_regex_bits") {
            Value::Int(b) => *b,
            _ => MULTILINE | SINGLELINE,
        },
    }
}

fn is_utf8(ctx: &mut dyn Ctx) -> bool {
    match ctx.state("mb_regex_enc") {
        Value::Str(s) => s.as_bytes().eq_ignore_ascii_case(b"UTF-8"),
        _ => true,
    }
}

/// The pattern compiled, or a warning and None (PHP returns false).
fn compile(ctx: &mut dyn Ctx, pattern: &[u8], bits: i64) -> Option<Regex> {
    let utf = is_utf8(ctx);
    let options = Options {
        caseless: bits & CASELESS != 0,
        extended: bits & EXTENDED != 0,
        dotall: bits & MULTILINE != 0,
        multiline: bits & SINGLELINE == 0,
        utf,
        ucp: utf,
        ..Options::default()
    };
    // As Oniguruma does: once a pattern names groups, plain ones don't capture.
    let compiled = Regex::new(pattern, &options).and_then(|re| match re.names().is_empty() {
        true => Ok(re),
        false => Regex::new(pattern, &Options { no_auto_capture: true, ..options.clone() }),
    });
    match compiled {
        Ok(re) => Some(re),
        Err(e) => {
            let f = ctx.builtin_name();
            ctx.warn(&format!("{f}(): mbregex compile err: {}", e.message()));
            None
        }
    }
}

fn search(re: &Regex, s: &[u8], at: usize) -> Option<Captures> {
    re.exec(s, at, &MatchOptions::default()).ok().flatten()
}

/// The length of the character at `at` (a byte outside UTF-8 mode).
fn char_len(re: &Regex, s: &[u8], at: usize) -> usize {
    if !re.is_utf() {
        return 1;
    }
    match s[at] {
        0xF0..=0xF7 => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
    .min(s.len() - at)
}

fn bytes(s: &[u8]) -> Value {
    Value::Str(PhpStr::from(s))
}

/// The groups as mb_ereg gives them: by number (false for one that did
/// not take part or matched empty), then by name.
fn groups(re: &Regex, s: &[u8], caps: &Captures, empty_as: fn(&[u8]) -> Value) -> PhpArray {
    let group = |i: usize| match caps.get(i) {
        Some((b, e)) if b < e => bytes(&s[b..e]),
        Some((b, _)) => empty_as(&s[b..b]),
        None => empty_as(&[]),
    };
    let mut out = PhpArray::new();
    for i in 0..=re.groups() {
        out.insert(Key::Int(i as i64), group(i));
    }
    for (name, i) in re.names() {
        let v = match caps.get(*i) {
            Some((b, e)) if b < e => bytes(&s[b..e]),
            _ => Value::Bool(false),
        };
        out.insert(Key::from_string(PhpStr::from(name.as_slice())), v);
    }
    out
}

fn set_ref(args: &[Value], i: usize, v: Value) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = v;
    }
}

fn ereg(ctx: &mut dyn Ctx, args: &mut [Value], caseless: bool) -> R {
    let pattern = str_arg(args, 0);
    let s = str_arg(args, 1);
    set_ref(args, 2, Value::Array(Rc::new(PhpArray::new())));
    if pattern.is_empty() {
        return Err(arg_error(ctx, 1, "must not be empty"));
    }
    let bits = option_bits(ctx, None) | if caseless { CASELESS } else { 0 };
    let Some(re) = compile(ctx, &pattern, bits) else { return Ok(Value::Bool(false)) };
    if re.is_utf() && p8_pcre::check_utf8(&s).is_err() {
        return Ok(Value::Bool(false));
    }
    let Some(caps) = search(&re, &s, 0) else { return Ok(Value::Bool(false)) };
    set_ref(args, 2, Value::Array(Rc::new(groups(&re, &s, &caps, |_| Value::Bool(false)))));
    Ok(Value::Bool(true))
}

fn mb_ereg(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    ereg(ctx, args, false)
}

fn mb_eregi(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    ereg(ctx, args, true)
}

/// mb_ereg_match: whether the pattern matches at the start of the string.
fn mb_ereg_match(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let pattern = str_arg(args, 0);
    let s = str_arg(args, 1);
    let given = opt_str(args, 2);
    let bits = option_bits(ctx, given);
    let Some(re) = compile(ctx, &pattern, bits) else { return Ok(Value::Bool(false)) };
    if re.is_utf() && p8_pcre::check_utf8(&s).is_err() {
        return Ok(Value::Bool(false));
    }
    let anchored = MatchOptions { anchored: true, ..MatchOptions::default() };
    Ok(Value::Bool(matches!(re.exec(&s, 0, &anchored), Ok(Some(_)))))
}

/// The replacement with `\0`..`\9` and `\k<name>` / `\k'name'` filled in
/// (a reference to a group that did not take part adds nothing).
fn substitute(re: &Regex, s: &[u8], caps: &Captures, replacement: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < replacement.len() {
        let c = replacement[i];
        let mut group = None;
        let mut skip = 0;
        if c == b'\\' && i + 1 < replacement.len() {
            let n = replacement[i + 1];
            if n.is_ascii_digit() {
                group = Some(usize::from(n - b'0'));
                skip = 2;
            } else if n == b'k' && i + 2 < replacement.len() && matches!(replacement[i + 2], b'<' | b'\'') {
                let close = if replacement[i + 2] == b'<' { b'>' } else { b'\'' };
                if let Some(len) = replacement[i + 3..].iter().position(|&b| b == close) {
                    let name = &replacement[i + 3..i + 3 + len];
                    let number = if name.iter().all(u8::is_ascii_digit) && !name.is_empty() {
                        std::str::from_utf8(name).ok().and_then(|n| n.parse().ok())
                    } else {
                        re.names().iter().rev().find(|(n, _)| n == name).map(|(_, g)| *g)
                    };
                    if let Some(g) = number {
                        group = Some(g);
                        skip = len + 4;
                    }
                }
            }
        }
        match group {
            Some(g) => {
                if let Some((b, e)) = caps.get(g).filter(|_| g <= re.groups()) {
                    out.extend_from_slice(&s[b..e]);
                }
                i += skip;
            }
            None => {
                out.push(c);
                i += 1;
            }
        }
    }
}

enum With<'a> {
    Text(&'a [u8]),
    Callback(&'a Value),
}

fn replace(ctx: &mut dyn Ctx, pattern: &[u8], with: With, s: &[u8], bits: i64) -> R {
    let Some(re) = compile(ctx, pattern, bits) else { return Ok(Value::Bool(false)) };
    if re.is_utf() && p8_pcre::check_utf8(s).is_err() {
        return Ok(Value::Null);
    }
    let mut out = Vec::with_capacity(s.len());
    let mut pos = 0;
    while pos <= s.len() {
        let Some(caps) = search(&re, s, pos) else {
            out.extend_from_slice(&s[pos..]);
            break;
        };
        let (beg, end) = caps.get(0).expect("group 0 of a match");
        out.extend_from_slice(&s[pos..beg]);
        match &with {
            With::Text(r) => substitute(&re, s, &caps, r, &mut out),
            With::Callback(f) => {
                let arr = groups(&re, s, &caps, |e| bytes(e));
                let v = ctx.call(f, &mut [Value::Array(Rc::new(arr))])?;
                out.extend_from_slice(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes());
            }
        }
        if pos < end {
            pos = end;
        } else {
            // An empty match: the next character is kept as it is.
            if pos < s.len() {
                let n = char_len(&re, s, pos);
                out.extend_from_slice(&s[pos..pos + n]);
                pos += n;
            } else {
                break;
            }
        }
    }
    Ok(bytes(&out))
}

fn mb_ereg_replace(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (pattern, replacement, s) = (str_arg(args, 0), str_arg(args, 1), str_arg(args, 2));
    let bits = option_bits(ctx, opt_str(args, 3));
    replace(ctx, &pattern, With::Text(&replacement), &s, bits)
}

fn mb_eregi_replace(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (pattern, replacement, s) = (str_arg(args, 0), str_arg(args, 1), str_arg(args, 2));
    let bits = option_bits(ctx, opt_str(args, 3)) | CASELESS;
    replace(ctx, &pattern, With::Text(&replacement), &s, bits)
}

fn mb_ereg_replace_callback(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let pattern = str_arg(args, 0);
    let callback = args.get(1).map(Value::deref).unwrap_or(Value::Null);
    let s = str_arg(args, 2);
    let bits = option_bits(ctx, opt_str(args, 3));
    replace(ctx, &pattern, With::Callback(&callback), &s, bits)
}

/// mb_split: the pieces between matches; `limit` > 0 makes at most that
/// many, the last holding the rest.
fn mb_split(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let pattern = str_arg(args, 0);
    let s = str_arg(args, 1);
    let mut count = int_arg(args, 2, -1);
    let bits = option_bits(ctx, None);
    let Some(re) = compile(ctx, &pattern, bits) else { return Ok(Value::Bool(false)) };
    if count > 0 {
        count -= 1;
    }
    let mut out = PhpArray::new();
    let (mut pos, mut chunk) = (0, 0);
    while count != 0 && pos < s.len() {
        let Some(caps) = search(&re, &s, pos) else { break };
        let (beg, end) = caps.get(0).expect("group 0 of a match");
        if pos < end {
            let _ = out.push(bytes(&s[chunk..beg.max(chunk)]));
            count -= 1;
            chunk = end;
            pos = end;
        } else {
            pos += char_len(&re, &s, pos);
        }
    }
    let _ = out.push(bytes(&s[chunk.min(s.len())..]));
    Ok(Value::Array(Rc::new(out)))
}

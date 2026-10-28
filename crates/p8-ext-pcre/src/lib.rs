#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's pcre extension: preg_match(), preg_replace() and the rest, over
//! p8-pcre (PCRE2's semantics in safe Rust).
//!
//! Reference: $PHP_SRC_PATH/ext/pcre/php_pcre.c

use p8_pcre::{Captures, MatchError, MatchOptions, Options, Regex};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "pcre", register };

const PATTERN_ORDER: i64 = 1;
const SET_ORDER: i64 = 2;
const OFFSET_CAPTURE: i64 = 1 << 8;
const UNMATCHED_AS_NULL: i64 = 1 << 9;
const SPLIT_NO_EMPTY: i64 = 1;
const SPLIT_DELIM_CAPTURE: i64 = 2;
const SPLIT_OFFSET_CAPTURE: i64 = 4;
const GREP_INVERT: i64 = 1;

const NO_ERROR: i64 = 0;
const INTERNAL_ERROR: i64 = 1;
const BACKTRACK_LIMIT_ERROR: i64 = 2;
const RECURSION_LIMIT_ERROR: i64 = 3;
const BAD_UTF8_ERROR: i64 = 4;
const BAD_UTF8_OFFSET_ERROR: i64 = 5;
const JIT_STACKLIMIT_ERROR: i64 = 6;

/// Backtracking points that fit PHP's JIT stack (measured against PHP 8.4:
/// `(?:a|b)*` gives up after 8190 iterations).
const JIT_STACK: u64 = 16_384;

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    for (name, value) in [
        ("PREG_PATTERN_ORDER", PATTERN_ORDER),
        ("PREG_SET_ORDER", SET_ORDER),
        ("PREG_OFFSET_CAPTURE", OFFSET_CAPTURE),
        ("PREG_UNMATCHED_AS_NULL", UNMATCHED_AS_NULL),
        ("PREG_SPLIT_NO_EMPTY", SPLIT_NO_EMPTY),
        ("PREG_SPLIT_DELIM_CAPTURE", SPLIT_DELIM_CAPTURE),
        ("PREG_SPLIT_OFFSET_CAPTURE", SPLIT_OFFSET_CAPTURE),
        ("PREG_GREP_INVERT", GREP_INVERT),
        ("PREG_NO_ERROR", NO_ERROR),
        ("PREG_INTERNAL_ERROR", INTERNAL_ERROR),
        ("PREG_BACKTRACK_LIMIT_ERROR", BACKTRACK_LIMIT_ERROR),
        ("PREG_RECURSION_LIMIT_ERROR", RECURSION_LIMIT_ERROR),
        ("PREG_BAD_UTF8_ERROR", BAD_UTF8_ERROR),
        ("PREG_BAD_UTF8_OFFSET_ERROR", BAD_UTF8_OFFSET_ERROR),
        ("PREG_JIT_STACKLIMIT_ERROR", JIT_STACKLIMIT_ERROR),
        ("PCRE_VERSION_MAJOR", 10),
        ("PCRE_VERSION_MINOR", 44),
    ] {
        registry.add_constant(name.as_bytes(), Constant::Int(value));
    }
    registry.add_constant(b"PCRE_VERSION", Constant::Str(b"10.44 2024-06-07"));
    // No JIT: it would emit machine code, which needs `unsafe`.
    registry.add_constant(b"PCRE_JIT_SUPPORT", Constant::Bool(false));
    registry.add_ini("pcre.backtrack_limit", Some(b"1000000"));
    registry.add_ini("pcre.recursion_limit", Some(b"100000"));
    registry.add_ini("pcre.jit", Some(b"1"));
    registry.add_module("pcre");
    // These call PHP code: callbacks, and __toString() on subjects.
    registry.mark_reenters(&[
        "preg_match",
        "preg_match_all",
        "preg_replace",
        "preg_filter",
        "preg_replace_callback",
        "preg_replace_callback_array",
        "preg_split",
        "preg_grep",
    ]);
}

static BUILTINS: &[Builtin] = &[
    Builtin { name: "preg_match", run: preg_match, by_ref: &[2] },
    Builtin { name: "preg_match_all", run: preg_match_all, by_ref: &[2] },
    Builtin { name: "preg_replace", run: preg_replace, by_ref: &[4] },
    Builtin { name: "preg_filter", run: preg_filter, by_ref: &[4] },
    Builtin { name: "preg_replace_callback", run: preg_replace_callback, by_ref: &[4] },
    Builtin { name: "preg_replace_callback_array", run: preg_replace_callback_array, by_ref: &[3] },
    Builtin { name: "preg_split", run: preg_split, by_ref: &[] },
    Builtin { name: "preg_quote", run: preg_quote, by_ref: &[] },
    Builtin { name: "preg_grep", run: preg_grep, by_ref: &[] },
    Builtin { name: "preg_last_error", run: preg_last_error, by_ref: &[] },
    Builtin { name: "preg_last_error_msg", run: preg_last_error_msg, by_ref: &[] },
];

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

fn str_arg(args: &[Value], i: usize) -> PhpStr {
    arg(args, i).to_php_string().unwrap_or_else(PhpStr::empty)
}

fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(Value::deref) {
        None | Some(Value::Null) => default,
        Some(v) => v.to_int(),
    }
}

/// A value as a string the way zval_get_string() makes one: __toString()
/// for an object, "Array" (with a warning) for an array.
fn to_str(ctx: &mut dyn Ctx, v: &Value) -> Result<PhpStr, Throw> {
    match v.deref() {
        Value::Object(o) => {
            if o.class.method(b"__tostring").is_none() {
                return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(o.class.name.as_bytes()))));
            }
            let method = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(o), Value::Str(PhpStr::from("__toString"))])));
            Ok(ctx.call(&method, &mut [])?.to_php_string().unwrap_or_else(PhpStr::empty))
        }
        Value::Array(_) => {
            ctx.warn("Array to string conversion");
            Ok(PhpStr::from("Array"))
        }
        other => Ok(other.to_php_string().unwrap_or_else(PhpStr::empty)),
    }
}

/// A warning as php_error_docref() gives it: "preg_match(): …".
fn warn(ctx: &mut dyn Ctx, message: &str) {
    let name = ctx.builtin_name();
    ctx.warn(&format!("{name}(): {message}"));
}

fn set_error(ctx: &mut dyn Ctx, code: i64) {
    *ctx.state("preg_last_error") = Value::Int(code);
}

fn error_code(e: MatchError) -> i64 {
    match e {
        MatchError::MatchLimit => BACKTRACK_LIMIT_ERROR,
        MatchError::DepthLimit => RECURSION_LIMIT_ERROR,
    }
}

/// A compiled pattern with what PHP keeps beside it.
struct Compiled {
    re: Regex,
    utf: bool,
    /// Each group's name, by group number.
    names: Vec<Option<PhpStr>>,
}

thread_local! {
    /// PHP's per-request pattern cache, by the pattern string.
    static CACHE: RefCell<HashMap<Vec<u8>, Rc<Compiled>>> = RefCell::new(HashMap::new());
}

/// The compiled pattern for a delimited regex, warning as PHP does when it
/// is malformed (pcre_get_compiled_regex_cache_ex).
fn compiled(ctx: &mut dyn Ctx, regex: &[u8]) -> Option<Rc<Compiled>> {
    if let Some(c) = CACHE.with(|c| c.borrow().get(regex).cloned()) {
        return Some(c);
    }
    let r = compile(ctx, regex);
    match r {
        Some(c) => {
            let c = Rc::new(c);
            CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.len() >= 4096 {
                    cache.clear();
                }
                cache.insert(regex.to_vec(), c.clone());
            });
            Some(c)
        }
        None => {
            set_error(ctx, INTERNAL_ERROR);
            None
        }
    }
}

fn compile(ctx: &mut dyn Ctx, regex: &[u8]) -> Option<Compiled> {
    let mut p = 0;
    while p < regex.len() && matches!(regex[p], b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c') {
        p += 1;
    }
    if p >= regex.len() {
        warn(ctx, "Empty regular expression");
        return None;
    }
    let delimiter = regex[p];
    p += 1;
    if delimiter.is_ascii_alphanumeric() || delimiter == b'\\' || delimiter == 0 {
        warn(ctx, "Delimiter must not be alphanumeric, backslash, or NUL byte");
        return None;
    }
    let end_delimiter = match delimiter {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        d => d,
    };
    let start = p;
    let mut pp = p;
    if end_delimiter == delimiter {
        while pp < regex.len() {
            if regex[pp] == b'\\' && pp + 1 < regex.len() {
                pp += 1;
            } else if regex[pp] == delimiter {
                break;
            }
            pp += 1;
        }
    } else {
        let mut depth = 1;
        while pp < regex.len() {
            if regex[pp] == b'\\' && pp + 1 < regex.len() {
                pp += 1;
            } else if regex[pp] == end_delimiter {
                depth -= 1;
                if depth <= 0 {
                    break;
                }
            } else if regex[pp] == delimiter {
                depth += 1;
            }
            pp += 1;
        }
    }
    if pp >= regex.len() {
        let d = end_delimiter as char;
        if end_delimiter == delimiter {
            warn(ctx, &format!("No ending delimiter '{d}' found"));
        } else {
            warn(ctx, &format!("No ending matching delimiter '{d}' found"));
        }
        return None;
    }
    let pattern = &regex[start..pp];
    let mut o = Options::default();
    for &m in &regex[pp + 1..] {
        match m {
            b'i' => o.caseless = true,
            b'm' => o.multiline = true,
            b'n' => o.no_auto_capture = true,
            b's' => o.dotall = true,
            b'x' => o.extended = true,
            b'A' => o.anchored = true,
            b'D' => o.dollar_endonly = true,
            b'r' => o.caseless_restrict = true,
            b'S' | b'X' | b' ' | b'\n' | b'\r' => {}
            b'U' => o.ungreedy = true,
            b'u' => {
                o.utf = true;
                o.ucp = true;
            }
            b'J' => o.dupnames = true,
            0 => {
                warn(ctx, "NUL byte is not a valid modifier");
                return None;
            }
            m => {
                warn(ctx, &format!("Unknown modifier '{}'", m as char));
                return None;
            }
        }
    }
    if o.utf && contains_backslash_c(pattern) {
        // PCRE2_NEVER_BACKSLASH_C under /u.
        let at = backslash_c_offset(pattern);
        warn(ctx, &format!("Compilation failed: using \\C is incompatible with the 'u' modifier at offset {at}"));
        return None;
    }
    let re = match Regex::new(pattern, &o) {
        Ok(re) => re,
        Err(e) => {
            warn(ctx, &format!("Compilation failed: {e}"));
            return None;
        }
    };
    let mut names = vec![None; re.groups() + 1];
    for (name, group) in re.names() {
        names[*group] = Some(PhpStr::new(name.clone()));
    }
    let utf = re.is_utf();
    Some(Compiled { re, utf, names })
}

fn contains_backslash_c(pattern: &[u8]) -> bool {
    backslash_c_offset(pattern) != usize::MAX
}

fn backslash_c_offset(pattern: &[u8]) -> usize {
    let mut i = 0;
    while i + 1 < pattern.len() {
        if pattern[i] == b'\\' {
            if pattern[i + 1] == b'C' {
                return i + 2;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    usize::MAX
}

#[derive(Clone, Copy, Default)]
struct Flags {
    not_empty_atstart: bool,
    anchored: bool,
}

thread_local! {
    /// The last subject found to be valid UTF-8 (PHP flags the string).
    static VALID_UTF8: RefCell<Option<PhpStr>> = const { RefCell::new(None) };
}

/// pcre2_match's UTF check: the subject from the start offset, less the
/// pattern's longest lookbehind, must be valid UTF-8, and the offset must
/// start a character.
fn check_utf(c: &Compiled, subject: &PhpStr, start: usize) -> Result<(), i64> {
    let s = subject.as_bytes();
    let known = VALID_UTF8.with(|v| v.borrow().as_ref().is_some_and(|k| k.as_bytes().as_ptr() == s.as_ptr() && k.len() == s.len()));
    if !known {
        if p8_pcre::check_utf8(s).is_ok() {
            // Holding the string keeps its buffer, so the pointer can't be
            // reused by another string while it's remembered.
            VALID_UTF8.with(|v| *v.borrow_mut() = Some(subject.clone()));
        } else {
            let mut from = start.min(s.len());
            for _ in 0..c.re.max_lookbehind() {
                if from == 0 {
                    break;
                }
                from -= 1;
                while from > 0 && s[from] & 0xc0 == 0x80 {
                    from -= 1;
                }
            }
            if p8_pcre::check_utf8(&s[from..]).is_err() {
                return Err(BAD_UTF8_ERROR);
            }
        }
    }
    if start < s.len() && s[start] & 0xc0 == 0x80 {
        return Err(BAD_UTF8_OFFSET_ERROR);
    }
    Ok(())
}

/// One pcre2_match() call: Err holds the preg error code.
fn exec(ctx: &dyn Ctx, c: &Compiled, subject: &PhpStr, start: usize, flags: Flags, utf_check: bool) -> Result<Option<Captures>, i64> {
    let s = subject.as_bytes();
    if c.utf && utf_check {
        check_utf(c, subject, start)?;
    }
    let ini = |name: &[u8], default: u64| -> u64 {
        ctx.ini_get(name).map(|v| v.to_int()).filter(|&n| n > 0).map_or(default, |n| n as u64)
    };
    let jit_run = !flags.not_empty_atstart && ctx.ini_get(b"pcre.jit").is_none_or(|v| v.to_int() != 0);
    let opts = MatchOptions {
        anchored: flags.anchored,
        not_empty: false,
        not_empty_atstart: flags.not_empty_atstart,
        match_limit: ini(b"pcre.backtrack_limit", 1_000_000),
        // PHP's JIT has no depth limit, only its 192 KB stack (about 16k
        // backtracking points); its retries after an empty match run in
        // the interpreter, which has the depth limit.
        depth_limit: if jit_run { JIT_STACK } else { ini(b"pcre.recursion_limit", 100_000) },
    };
    c.re.exec(s, start, &opts).map_err(|e| match e {
        MatchError::DepthLimit if jit_run => JIT_STACKLIMIT_ERROR,
        e => error_code(e),
    })
}

/// The length of the character at `at` (calculate_unit_length).
fn unit_len(c: &Compiled, s: &[u8], at: usize) -> usize {
    let mut n = 1;
    if c.utf {
        while at + n < s.len() && s[at + n] & 0xc0 == 0x80 {
            n += 1;
        }
    }
    n
}

fn group_value(s: &[u8], m: &Captures, i: usize, unmatched_as_null: bool) -> Value {
    match m.get(i) {
        Some((a, b)) => Value::Str(PhpStr::new(s[a..b].to_vec())),
        None if unmatched_as_null => Value::Null,
        None => Value::Str(PhpStr::empty()),
    }
}

fn offset_pair(s: &[u8], m: Option<(usize, usize)>, unmatched_as_null: bool) -> Value {
    let pair = match m {
        Some((a, b)) => vec![Value::Str(PhpStr::new(s[a..b].to_vec())), Value::Int(a as i64)],
        None => vec![if unmatched_as_null { Value::Null } else { Value::Str(PhpStr::empty()) }, Value::Int(-1)],
    };
    Value::Array(Rc::new(PhpArray::from_list(pair)))
}

/// Inserts a named group's value: a set value replaces one already there
/// (duplicate names), an unset one only fills a gap (add_named).
fn add_named(arr: &mut PhpArray, name: &PhpStr, v: Value, unmatched: bool) {
    let key = Key::from_string(name.clone());
    if unmatched && arr.get(&key).is_some() {
        return;
    }
    arr.insert(key, v);
}

/// One match's groups as an array (populate_subpat_array).
fn subpat_array(c: &Compiled, s: &[u8], m: &Captures, flags: i64) -> PhpArray {
    let offset_capture = flags & OFFSET_CAPTURE != 0;
    let unmatched_as_null = flags & UNMATCHED_AS_NULL != 0;
    let num = c.re.groups() + 1;
    let count = m.len();
    let mut arr = PhpArray::new();
    let end = if unmatched_as_null { num } else { count };
    for i in 0..end {
        let g = if i < count { m.get(i) } else { None };
        let v = if offset_capture { offset_pair(s, g, unmatched_as_null) } else if i < count { group_value(s, m, i, unmatched_as_null) } else { Value::Null };
        if let Some(name) = &c.names[i] {
            add_named(&mut arr, name, v.clone(), g.is_none());
        }
        let _ = arr.push(v);
    }
    if let Some(mark) = &m.mark {
        arr.insert(Key::Str(PhpStr::from("MARK")), Value::Str(PhpStr::new(mark.clone())));
    }
    arr
}

fn preg_match(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    match_impl(ctx, args, false)
}

fn preg_match_all(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    match_impl(ctx, args, true)
}

fn match_impl(ctx: &mut dyn Ctx, args: &mut [Value], global: bool) -> Result<Value, Throw> {
    let name = ctx.builtin_name();
    let regex = str_arg(args, 0);
    let subject = str_arg(args, 1);
    let s = subject.as_bytes();
    let flags = int_arg(args, 3, 0);
    let offset = int_arg(args, 4, 0);
    let Some(c) = compiled(ctx, regex.as_bytes()) else { return Ok(Value::Bool(false)) };
    if offset == i64::MIN {
        return Err(ctx.throw("ValueError", format!("{name}(): Argument #5 ($offset) must be greater than {}", i64::MIN)));
    }
    let subpats = match args.get(2) {
        Some(Value::Ref(cell)) => {
            *cell.borrow_mut() = Value::Array(Rc::new(PhpArray::new()));
            Some(cell.clone())
        }
        _ => None,
    };
    let mut order = if global { PATTERN_ORDER } else { 0 };
    if flags & 0xff != 0 {
        order = flags & 0xff;
        if (global && !(PATTERN_ORDER..=SET_ORDER).contains(&order)) || (!global && order != 0) {
            return Err(ctx.throw("ValueError", format!("{name}(): Argument #4 ($flags) must be a PREG_* constant")));
        }
    }
    let offset_capture = flags & OFFSET_CAPTURE != 0;
    let unmatched_as_null = flags & UNMATCHED_AS_NULL != 0;
    let mut start = if offset < 0 {
        if offset.unsigned_abs() as usize <= s.len() { s.len() - offset.unsigned_abs() as usize } else { 0 }
    } else {
        offset as usize
    };
    if start > s.len() {
        set_error(ctx, INTERNAL_ERROR);
        return Ok(Value::Bool(false));
    }
    set_error(ctx, NO_ERROR);
    let num = c.re.groups() + 1;
    let mut sets: Vec<PhpArray> = if global && order == PATTERN_ORDER { (0..num).map(|_| PhpArray::new()).collect() } else { Vec::new() };
    let mut marks: Option<PhpArray> = None;
    let mut list = PhpArray::new();
    let mut single: Option<PhpArray> = None;
    let mut matched = 0i64;
    let mut error = NO_ERROR;
    let mut result = exec(ctx, &c, &subject, start, Flags::default(), true);
    loop {
        let m = match result {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(code) => {
                error = code;
                break;
            }
        };
        matched += 1;
        let (m0, m1) = m.get(0).unwrap();
        if subpats.is_some() {
            if m1 < m0 {
                warn(ctx, "Get subpatterns list failed");
                return Ok(Value::Bool(false));
            }
            if global {
                if order == PATTERN_ORDER {
                    let count = m.len();
                    for (i, set) in sets.iter_mut().enumerate() {
                        let v = if i < count {
                            if offset_capture { offset_pair(s, m.get(i), unmatched_as_null) } else { group_value(s, &m, i, unmatched_as_null) }
                        } else if offset_capture {
                            offset_pair(s, None, unmatched_as_null)
                        } else if unmatched_as_null {
                            Value::Null
                        } else {
                            Value::Str(PhpStr::empty())
                        };
                        let _ = set.push(v);
                    }
                    if let Some(mark) = &m.mark {
                        marks.get_or_insert_with(PhpArray::new).insert(Key::Int(matched - 1), Value::Str(PhpStr::new(mark.clone())));
                    }
                } else {
                    let _ = list.push(Value::Array(Rc::new(subpat_array(&c, s, &m, flags))));
                }
            } else {
                single = Some(subpat_array(&c, s, &m, flags));
                break;
            }
        }
        start = m1;
        // An empty match: try again here for a non-empty one, then move on
        // a character (as Perl's /g does).
        if m1 == m0 {
            match exec(ctx, &c, &subject, start, Flags { not_empty_atstart: true, anchored: true }, false) {
                Ok(Some(m)) => {
                    if global {
                        result = Ok(Some(m));
                        continue;
                    }
                    break;
                }
                Ok(None) => {
                    if start < s.len() {
                        start += unit_len(&c, s, start);
                    } else {
                        break;
                    }
                }
                Err(code) => {
                    error = code;
                    break;
                }
            }
        }
        if !global {
            break;
        }
        result = exec(ctx, &c, &subject, start, Flags::default(), false);
    }
    if let Some(cell) = subpats {
        let out = if global {
            if order == PATTERN_ORDER {
                let mut out = PhpArray::new();
                for (i, set) in sets.into_iter().enumerate() {
                    let v = Value::Array(Rc::new(set));
                    if let Some(name) = &c.names[i] {
                        out.insert(Key::from_string(name.clone()), v.clone());
                    }
                    let _ = out.push(v);
                }
                if let Some(marks) = marks {
                    out.insert(Key::Str(PhpStr::from("MARK")), Value::Array(Rc::new(marks)));
                }
                out
            } else {
                list
            }
        } else {
            single.unwrap_or_default()
        };
        *cell.borrow_mut() = Value::Array(Rc::new(out));
    }
    set_error(ctx, error);
    Ok(if error == NO_ERROR { Value::Int(matched) } else { Value::Bool(false) })
}

/// A `\n`, `$n` or `${n}` reference at `at` in a replacement: (group, end).
fn backref(r: &[u8], at: usize) -> Option<(usize, usize)> {
    if r.get(at + 1).copied().unwrap_or(0) == 0 {
        return None;
    }
    let mut i = at;
    let brace = r[i] == b'$' && r[i + 1] == b'{';
    if brace {
        i += 1;
    }
    i += 1;
    let d = *r.get(i).filter(|b| b.is_ascii_digit())?;
    let mut n = (d - b'0') as usize;
    i += 1;
    if let Some(d) = r.get(i).filter(|b| b.is_ascii_digit()) {
        n = n * 10 + (d - b'0') as usize;
        i += 1;
    }
    if brace {
        if r.get(i) != Some(&b'}') {
            return None;
        }
        i += 1;
    }
    Some((n, i))
}

/// A replacement string with its references filled in from a match.
fn expand(out: &mut Vec<u8>, r: &[u8], s: &[u8], m: &Captures) {
    let count = m.len();
    let mut i = 0;
    let mut last = 0u8;
    while i < r.len() {
        let b = r[i];
        if b == b'\\' || b == b'$' {
            if last == b'\\' {
                // An escaped `\` or `$`: it replaces the backslash.
                *out.last_mut().unwrap() = b;
                i += 1;
                last = 0;
                continue;
            }
            if let Some((n, end)) = backref(r, i) {
                if n < count
                    && let Some((a, z)) = m.get(n)
                {
                    out.extend_from_slice(&s[a..z]);
                }
                i = end;
                continue;
            }
        }
        out.push(b);
        i += 1;
        last = b;
    }
}

enum Replacement<'a> {
    Text(&'a [u8]),
    Callback(&'a Value, i64),
}

/// Replaces matches in one subject (php_pcre_replace_impl and its _func
/// twin): None on an error.
fn replace_one(ctx: &mut dyn Ctx, c: &Compiled, subject: &PhpStr, with: &Replacement, mut limit: usize, count: &mut usize) -> Result<Option<Vec<u8>>, Throw> {
    set_error(ctx, NO_ERROR);
    let s = subject.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut start = 0usize;
    let mut last_end = 0usize;
    let mut result = exec(ctx, c, subject, start, Flags::default(), true);
    loop {
        let mut piece = last_end;
        match result {
            Ok(Some(m)) if limit > 0 => {
                let (m0, m1) = m.get(0).unwrap();
                if m1 < m0 {
                    set_error(ctx, INTERNAL_ERROR);
                    return Ok(None);
                }
                *count += 1;
                out.extend_from_slice(&s[piece..m0]);
                match with {
                    Replacement::Text(r) => expand(&mut out, r, s, &m),
                    Replacement::Callback(f, flags) => {
                        let arr = Value::Array(Rc::new(subpat_array(c, s, &m, *flags)));
                        let v = ctx.call(f, &mut [arr])?;
                        let v = to_str(ctx, &v)?;
                        out.extend_from_slice(v.as_bytes());
                    }
                }
                limit -= 1;
                start = m1;
                last_end = m1;
                if m1 == m0 {
                    piece = start;
                    let again = exec(ctx, c, subject, start, Flags { not_empty_atstart: true, anchored: true }, false);
                    match again {
                        Ok(Some(m)) if limit > 0 => {
                            result = Ok(Some(m));
                            continue;
                        }
                        Ok(_) => {
                            if start < s.len() {
                                start += unit_len(c, s, start);
                            } else {
                                out.extend_from_slice(&s[piece..]);
                                return Ok(Some(out));
                            }
                        }
                        Err(code) => {
                            set_error(ctx, code);
                            return Ok(None);
                        }
                    }
                }
            }
            Ok(_) => {
                out.extend_from_slice(&s[piece..]);
                return Ok(Some(out));
            }
            Err(code) => {
                set_error(ctx, code);
                return Ok(None);
            }
        }
        result = exec(ctx, c, subject, start, Flags::default(), false);
    }
}

/// One subject through one pattern: None when the pattern is bad or the
/// match failed.
fn replace_with(ctx: &mut dyn Ctx, regex: &PhpStr, s: &PhpStr, with: &Replacement, limit: usize, count: &mut usize) -> Result<Option<PhpStr>, Throw> {
    let Some(c) = compiled(ctx, regex.as_bytes()) else { return Ok(None) };
    Ok(replace_one(ctx, &c, s, with, limit, count)?.map(PhpStr::new))
}

fn limit_arg(args: &[Value], i: usize) -> usize {
    let l = int_arg(args, i, -1);
    if l < 0 { usize::MAX } else { l as usize }
}

fn set_count(args: &[Value], i: usize, count: usize) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = Value::Int(count as i64);
    }
}

/// Each subject (a string, or an array's values with their keys): Some
/// result per subject, None when it failed or (preg_filter) didn't change.
fn each_subject(
    ctx: &mut dyn Ctx,
    subject: &Value,
    mut f: impl FnMut(&mut dyn Ctx, &PhpStr) -> Result<Option<PhpStr>, Throw>,
) -> Result<Value, Throw> {
    match subject {
        Value::Array(a) => {
            let mut out = PhpArray::new();
            for (k, v) in a.iter() {
                let s = to_str(ctx, v)?;
                if let Some(r) = f(ctx, &s)? {
                    out.insert(k, Value::Str(r));
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        other => {
            let s = to_str(ctx, other)?;
            Ok(f(ctx, &s)?.map_or(Value::Null, Value::Str))
        }
    }
}

fn preg_replace(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    replace_common(ctx, args, false)
}

fn preg_filter(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    replace_common(ctx, args, true)
}

fn replace_common(ctx: &mut dyn Ctx, args: &mut [Value], filter: bool) -> Result<Value, Throw> {
    let name = ctx.builtin_name();
    let (regex, replacement, subject) = (arg(args, 0), arg(args, 1), arg(args, 2));
    if matches!(replacement, Value::Array(_)) && !matches!(regex, Value::Array(_)) {
        return Err(ctx.throw("TypeError", format!("{name}(): Argument #1 ($pattern) must be of type array when argument #2 ($replacement) is an array, string given")));
    }
    let limit = limit_arg(args, 3);
    let mut count = 0usize;
    // The patterns, each with its replacement.
    let mut pairs: Vec<(PhpStr, PhpStr)> = Vec::new();
    match &regex {
        Value::Array(patterns) => {
            let mut reps = match &replacement {
                Value::Array(r) => Some(r.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>().into_iter()),
                _ => None,
            };
            let fixed = match &replacement {
                Value::Array(_) => None,
                other => Some(to_str(ctx, other)?),
            };
            for (_, p) in patterns.iter() {
                let p = to_str(ctx, p)?;
                let r = match (&mut reps, &fixed) {
                    (Some(it), _) => match it.next() {
                        Some(v) => to_str(ctx, &v)?,
                        None => PhpStr::empty(),
                    },
                    (None, Some(f)) => f.clone(),
                    (None, None) => PhpStr::empty(),
                };
                pairs.push((p, r));
            }
        }
        other => pairs.push((to_str(ctx, other)?, to_str(ctx, &replacement)?)),
    }
    let result = each_subject(ctx, &subject, |ctx, s| {
        let before = count;
        let mut cur = s.clone();
        for (p, r) in &pairs {
            match replace_with(ctx, p, &cur, &Replacement::Text(r.as_bytes()), limit, &mut count)? {
                Some(next) => cur = next,
                None => return Ok(None),
            }
        }
        Ok(if filter && count == before { None } else { Some(cur) })
    })?;
    set_count(args, 4, count);
    Ok(result)
}

fn preg_replace_callback(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (regex, callback, subject) = (arg(args, 0), arg(args, 1), arg(args, 2));
    let limit = limit_arg(args, 3);
    let flags = int_arg(args, 5, 0);
    let mut count = 0usize;
    let patterns: Vec<PhpStr> = match &regex {
        Value::Array(a) => {
            let mut v = Vec::new();
            for (_, p) in a.iter() {
                v.push(to_str(ctx, p)?);
            }
            v
        }
        other => vec![to_str(ctx, other)?],
    };
    let result = each_subject(ctx, &subject, |ctx, s| {
        let mut cur = s.clone();
        for p in &patterns {
            match replace_with(ctx, p, &cur, &Replacement::Callback(&callback, flags), limit, &mut count)? {
                Some(next) => cur = next,
                None => return Ok(None),
            }
        }
        Ok(Some(cur))
    })?;
    set_count(args, 4, count);
    Ok(result)
}

fn preg_replace_callback_array(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Array(map) = arg(args, 0) else { return Ok(Value::Null) };
    let mut subject = arg(args, 1);
    let limit = limit_arg(args, 2);
    let flags = int_arg(args, 4, 0);
    let mut count = 0usize;
    for (k, callback) in map.iter() {
        if !ctx.is_callable(callback) {
            return Err(ctx.throw("TypeError", "preg_replace_callback_array(): Argument #1 ($pattern) must contain only valid callbacks".into()));
        }
        let Key::Str(p) = k else {
            return Err(ctx.throw("TypeError", "preg_replace_callback_array(): Argument #1 ($pattern) must contain only string patterns as keys".into()));
        };
        let callback = callback.clone();
        subject = each_subject(ctx, &subject, |ctx, s| replace_with(ctx, &p, s, &Replacement::Callback(&callback, flags), limit, &mut count))?;
        if matches!(subject, Value::Null) {
            set_count(args, 3, count);
            return Ok(Value::Null);
        }
    }
    set_count(args, 3, count);
    Ok(subject)
}

fn preg_split(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let regex = str_arg(args, 0);
    let subject = str_arg(args, 1);
    let s = subject.as_bytes();
    let mut limit = int_arg(args, 2, -1);
    let flags = int_arg(args, 3, 0);
    let Some(c) = compiled(ctx, regex.as_bytes()) else { return Ok(Value::Bool(false)) };
    let no_empty = flags & SPLIT_NO_EMPTY != 0;
    let delim_capture = flags & SPLIT_DELIM_CAPTURE != 0;
    let offset_capture = flags & SPLIT_OFFSET_CAPTURE != 0;
    let mut out = PhpArray::new();
    let piece = |out: &mut PhpArray, a: usize, b: usize| {
        let v = if offset_capture { offset_pair(s, Some((a, b)), false) } else { Value::Str(PhpStr::new(s[a..b].to_vec())) };
        let _ = out.push(v);
    };
    set_error(ctx, NO_ERROR);
    let mut last = 0usize;
    if limit == 0 {
        limit = -1;
    }
    if limit == -1 || limit > 1 {
        let mut start = 0usize;
        let mut result = exec(ctx, &c, &subject, start, Flags::default(), true);
        let mut error = NO_ERROR;
        loop {
            let m = match result {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(code) => {
                    error = code;
                    break;
                }
            };
            let (m0, m1) = m.get(0).unwrap();
            if m1 < m0 {
                error = INTERNAL_ERROR;
                break;
            }
            if !no_empty || m0 != last {
                piece(&mut out, last, m0);
                if limit != -1 {
                    limit -= 1;
                }
            }
            if delim_capture {
                for i in 1..m.len() {
                    match m.get(i) {
                        Some((a, b)) if !no_empty || a != b => piece(&mut out, a, b),
                        None if !no_empty => {
                            let _ = out.push(if offset_capture { offset_pair(s, None, false) } else { Value::Str(PhpStr::empty()) });
                        }
                        _ => {}
                    }
                }
            }
            start = m1;
            last = m1;
            if m1 == m0 {
                if limit != -1 && limit <= 1 {
                    break;
                }
                match exec(ctx, &c, &subject, start, Flags { not_empty_atstart: true, anchored: true }, false) {
                    Ok(Some(m)) => {
                        result = Ok(Some(m));
                        continue;
                    }
                    Ok(None) => {
                        if start < s.len() {
                            start += unit_len(&c, s, start);
                        } else {
                            break;
                        }
                    }
                    Err(code) => {
                        error = code;
                        break;
                    }
                }
            }
            if limit != -1 && limit <= 1 {
                break;
            }
            result = exec(ctx, &c, &subject, start, Flags::default(), false);
        }
        if error != NO_ERROR {
            set_error(ctx, error);
            return Ok(Value::Bool(false));
        }
    }
    if !no_empty || last < s.len() {
        piece(&mut out, last, s.len());
    }
    Ok(Value::Array(Rc::new(out)))
}

fn preg_quote(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let delim = match arg(args, 1) {
        Value::Null => None,
        v => v.to_php_string().and_then(|d| d.as_bytes().first().copied()),
    };
    let mut out = Vec::with_capacity(s.len());
    for &c in s.as_bytes() {
        match c {
            b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')' | b'{' | b'}' | b'=' | b'!' | b'>' | b'<' | b'|' | b':' | b'-' | b'#' => {
                out.push(b'\\');
                out.push(c);
            }
            0 => out.extend_from_slice(b"\\000"),
            _ => {
                if Some(c) == delim {
                    out.push(b'\\');
                }
                out.push(c);
            }
        }
    }
    Ok(Value::Str(PhpStr::new(out)))
}

fn preg_grep(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let regex = str_arg(args, 0);
    let Value::Array(input) = arg(args, 1) else { return Ok(Value::Bool(false)) };
    let invert = int_arg(args, 2, 0) & GREP_INVERT != 0;
    let Some(c) = compiled(ctx, regex.as_bytes()) else { return Ok(Value::Bool(false)) };
    set_error(ctx, NO_ERROR);
    let mut out = PhpArray::new();
    for (k, v) in input.iter() {
        let s = to_str(ctx, v)?;
        match exec(ctx, &c, &s, 0, Flags::default(), true) {
            Ok(m) => {
                if m.is_some() != invert {
                    out.insert(k, v.clone());
                }
            }
            Err(code) => {
                set_error(ctx, code);
                break;
            }
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

fn last_error(ctx: &mut dyn Ctx) -> i64 {
    match ctx.state("preg_last_error") {
        Value::Int(i) => *i,
        _ => NO_ERROR,
    }
}

fn preg_last_error(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(last_error(ctx)))
}

fn preg_last_error_msg(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let msg = match last_error(ctx) {
        NO_ERROR => "No error",
        INTERNAL_ERROR => "Internal error",
        BAD_UTF8_ERROR => "Malformed UTF-8 characters, possibly incorrectly encoded",
        BAD_UTF8_OFFSET_ERROR => "The offset did not correspond to the beginning of a valid UTF-8 code point",
        BACKTRACK_LIMIT_ERROR => "Backtrack limit exhausted",
        RECURSION_LIMIT_ERROR => "Recursion limit exhausted",
        JIT_STACKLIMIT_ERROR => "JIT stack limit exhausted",
        _ => "Unknown error",
    };
    Ok(Value::Str(PhpStr::from(msg)))
}

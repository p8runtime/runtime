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
use p8_runtime::fast::HashMap;
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
    for name in ["pcre.backtrack_limit", "pcre.recursion_limit", "pcre.jit"] {
        registry.add_ini_check(name, ini_changed);
    }
    registry.add_request_start(request_start);
    registry.add_module("pcre");
    // These call PHP code: callbacks, and __toString() on subjects (an
    // array's values; a string parameter's object is converted before the
    // call, so preg_match(), preg_match_all() and preg_split() never do).
    registry.mark_reenters(&[
        "preg_replace",
        "preg_filter",
        "preg_replace_callback",
        "preg_replace_callback_array",
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
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

fn str_arg(args: &[Value], i: usize) -> PhpStr {
    arg(args, i).to_php_string().unwrap_or_else(PhpStr::empty)
}

fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(Value::deref) {
        Some(v) if !v.is_null() => v.to_int(),
        _ => default,
    }
}

/// A value as a string the way zval_get_string() makes one: __toString()
/// for an object, "Array" (with a warning) for an array.
fn to_str(ctx: &mut dyn Ctx, v: &Value) -> Result<PhpStr, Throw> {
    let v = v.deref();
    if let Some(o) = v.as_object() {
        if o.class.method(b"__tostring").is_none() {
            return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(o.class.name.as_bytes()))));
        }
        let method = Value::array(PhpArray::from_list(vec![Value::object(Rc::clone(o)), Value::str(PhpStr::from("__toString"))]));
        Ok(ctx.call(&method, &mut [])?.to_php_string().unwrap_or_else(PhpStr::empty))
    } else if v.is_array() {
        ctx.warn("Array to string conversion");
        Ok(PhpStr::from("Array"))
    } else {
        Ok(v.to_php_string().unwrap_or_else(PhpStr::empty))
    }
}

thread_local! {
    /// The name warnings carry instead of the running builtin's, while
    /// another extension compiles a pattern (see [`Pattern::compile`]).
    static LABEL: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// A warning as php_error_docref() gives it: "preg_match(): …".
fn warn(ctx: &mut dyn Ctx, message: &str) {
    let name = LABEL.with(|l| l.borrow().clone()).unwrap_or_else(|| ctx.builtin_name().to_string());
    ctx.warn(&format!("{name}(): {message}"));
}

/// A compiled pattern for other extensions (zip's addPattern matches file
/// names with one).
pub struct Pattern(Rc<Compiled>);

impl Pattern {
    /// Compile a delimited regex, as preg_match() would; a malformed one
    /// warns as "`label`(): ..." and gives None.
    pub fn compile(ctx: &mut dyn Ctx, label: &str, regex: &[u8]) -> Option<Pattern> {
        LABEL.with(|l| *l.borrow_mut() = Some(label.to_string()));
        let c = compiled(ctx, regex);
        LABEL.with(|l| *l.borrow_mut() = None);
        c.map(Pattern)
    }

    /// Whether the pattern matches somewhere in `subject`.
    pub fn is_match(&self, ctx: &dyn Ctx, subject: &[u8]) -> bool {
        matches!(exec(Limits::get(ctx), &self.0, &PhpStr::new(subject.to_vec()), 0, Flags::default(), true, &mut Captures::default()), Ok(Some(_)))
    }
}

thread_local! {
    /// The pcre.* settings, read once (ini lookups are hash lookups and a
    /// number parse, per match): None when they must be read again.
    static LIMITS: std::cell::Cell<Option<Limits>> = const { std::cell::Cell::new(None) };
    /// preg_last_error()'s code (a request's: reset when one starts).
    static LAST_ERROR: std::cell::Cell<i64> = const { std::cell::Cell::new(NO_ERROR) };
}

/// The settings a match runs under, and the context they were read in.
#[derive(Clone, Copy)]
struct Limits {
    ctx: usize,
    backtrack: u64,
    recursion: u64,
    jit: bool,
}

impl Limits {
    fn get(ctx: &dyn Ctx) -> Limits {
        let at = ctx as *const dyn Ctx as *const () as usize;
        if let Some(l) = LIMITS.get()
            && l.ctx == at
        {
            return l;
        }
        let ini = |name: &[u8], default: u64| -> u64 {
            ctx.ini_get(name).map(|v| v.to_int()).filter(|&n| n > 0).map_or(default, |n| n as u64)
        };
        let l = Limits {
            ctx: at,
            backtrack: ini(b"pcre.backtrack_limit", 1_000_000),
            recursion: ini(b"pcre.recursion_limit", 100_000),
            jit: ctx.ini_get(b"pcre.jit").is_none_or(|v| v.to_int() != 0),
        };
        LIMITS.set(Some(l));
        l
    }
}

/// ini_set() of a pcre.* setting: read them again at the next match.
fn ini_changed(_: &mut dyn Ctx, _: &[u8], _: &PhpStr) -> bool {
    LIMITS.set(None);
    true
}

fn request_start(_: &mut dyn Ctx) -> Result<(), Throw> {
    LIMITS.set(None);
    LAST_ERROR.set(NO_ERROR);
    Ok(())
}

fn set_error(_: &mut dyn Ctx, code: i64) {
    LAST_ERROR.set(code);
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
    /// Each group's name, by group number, as the key it is in a match array.
    names: Vec<Option<Key>>,
    /// Whether any group has a name.
    named: bool,
    /// Capture room kept between calls (see [`Spare`]).
    spare: std::cell::Cell<Captures>,
}

thread_local! {
    /// PHP's per-request pattern cache, by the pattern string.
    static CACHE: RefCell<HashMap<Vec<u8>, Rc<Compiled>>> = RefCell::new(HashMap::default());
    /// The pattern string last looked up, and what it compiled to: a
    /// literal pattern is the same string at each call, found again by
    /// its address without hashing it. (Holding the string keeps its
    /// buffer, so the address can't be another string's meanwhile.)
    static LAST: RefCell<Option<(PhpStr, Rc<Compiled>)>> = const { RefCell::new(None) };
}

/// [`compiled`] for a pattern string.
fn compiled_str(ctx: &mut dyn Ctx, regex: &PhpStr) -> Option<Rc<Compiled>> {
    let hit = LAST.with(|l| match &*l.borrow() {
        Some((s, c)) if PhpStr::ptr_eq(s, regex) => Some(Rc::clone(c)),
        _ => None,
    });
    if hit.is_some() {
        return hit;
    }
    let c = compiled(ctx, regex.as_bytes())?;
    LAST.with(|l| *l.borrow_mut() = Some((regex.clone(), Rc::clone(&c))));
    Some(c)
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
        names[*group] = Some(Key::from_string(PhpStr::new(name.clone())));
    }
    let utf = re.is_utf();
    let named = names.iter().any(Option::is_some);
    Some(Compiled { re, utf, names, named, spare: Default::default() })
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

/// A call's spare capture room (see [`exec`]): the pattern's, handed back
/// to it when the call is done; with the settings its matches run under.
struct Spare<'a>(Captures, &'a Compiled, Limits);

impl Spare<'_> {
    fn of<'a>(ctx: &dyn Ctx, c: &'a Compiled) -> Spare<'a> {
        Spare(c.spare.take(), c, Limits::get(ctx))
    }
}

impl Drop for Spare<'_> {
    fn drop(&mut self) {
        self.1.spare.set(std::mem::take(&mut self.0));
    }
}

/// One pcre2_match() call: Err holds the preg error code. The match is
/// made in `spare`'s room (a loop hands each match back as the next one's
/// spare, so it allocates nothing per match).
fn exec(limits: Limits, c: &Compiled, subject: &PhpStr, start: usize, flags: Flags, utf_check: bool, spare: &mut Captures) -> Result<Option<Captures>, i64> {
    let s = subject.as_bytes();
    if c.utf && utf_check {
        check_utf(c, subject, start)?;
    }
    let jit_run = !flags.not_empty_atstart && limits.jit;
    let opts = MatchOptions {
        anchored: flags.anchored,
        not_empty: false,
        not_empty_atstart: flags.not_empty_atstart,
        match_limit: limits.backtrack,
        // PHP's JIT has no depth limit, only its 192 KB stack (about 16k
        // backtracking points); its retries after an empty match run in
        // the interpreter, which has the depth limit.
        depth_limit: if jit_run { JIT_STACK } else { limits.recursion },
    };
    match c.re.exec_into(s, start, &opts, spare) {
        Ok(true) => Ok(Some(std::mem::take(spare))),
        Ok(false) => Ok(None),
        Err(MatchError::DepthLimit) if jit_run => Err(JIT_STACKLIMIT_ERROR),
        Err(e) => Err(error_code(e)),
    }
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

fn group_value(subject: &PhpStr, m: &Captures, i: usize, unmatched_as_null: bool) -> Value {
    let s = subject.as_bytes();
    match m.get(i) {
        // The whole subject: the same string, no copy.
        Some((0, b)) if b == s.len() => Value::str(subject.clone()),
        // (From a slice: a short one in the string's own allocation.)
        Some((a, b)) => Value::str(PhpStr::from(&s[a..b])),
        None if unmatched_as_null => Value::NULL,
        None => Value::str(PhpStr::empty()),
    }
}

fn offset_pair(s: &[u8], m: Option<(usize, usize)>, unmatched_as_null: bool) -> Value {
    let pair = match m {
        Some((a, b)) => vec![Value::str(PhpStr::from(&s[a..b])), Value::int(a as i64)],
        None => vec![if unmatched_as_null { Value::NULL } else { Value::str(PhpStr::empty()) }, Value::int(-1)],
    };
    Value::array(PhpArray::from_list(pair))
}

/// Inserts a named group's value: a set value replaces one already there
/// (duplicate names), an unset one only fills a gap (add_named).
fn add_named(arr: &mut PhpArray, key: &Key, v: Value, unmatched: bool) {
    if unmatched && arr.get(key).is_some() {
        return;
    }
    arr.insert(key.clone(), v);
}

/// One match's groups as an array (populate_subpat_array).
fn subpat_array(c: &Compiled, subject: &PhpStr, m: &Captures, flags: i64) -> PhpArray {
    let s = subject.as_bytes();
    let offset_capture = flags & OFFSET_CAPTURE != 0;
    let unmatched_as_null = flags & UNMATCHED_AS_NULL != 0;
    let num = c.re.groups() + 1;
    let count = m.len();
    let end = if unmatched_as_null { num } else { count };
    // A list when no group has a name: its room made once.
    let mut arr = if c.named { PhpArray::new() } else { PhpArray::list_with_capacity(end) };
    for i in 0..end {
        let g = if i < count { m.get(i) } else { None };
        let v = if offset_capture { offset_pair(s, g, unmatched_as_null) } else if i < count { group_value(subject, m, i, unmatched_as_null) } else { Value::NULL };
        if let Some(name) = &c.names[i] {
            add_named(&mut arr, name, v.clone(), g.is_none());
        }
        let _ = arr.push(v);
    }
    if let Some(mark) = &m.mark {
        arr.insert(Key::Str(PhpStr::from("MARK")), Value::str(PhpStr::new(mark.clone())));
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
    let Some(c) = compiled_str(ctx, &regex) else { return Ok(Value::FALSE) };
    if offset == i64::MIN {
        return Err(ctx.throw("ValueError", format!("{name}(): Argument #5 ($offset) must be greater than {}", i64::MIN)));
    }
    let subpats = args.get(2).and_then(Value::cloned_reference);
    // The matches argument is an empty array when nothing else goes there
    // (set where the call ends: nothing reads it before).
    let emptied = || {
        if let Some(cell) = &subpats {
            *cell.borrow_mut() = Value::array(PhpArray::new());
        }
    };
    let mut order = if global { PATTERN_ORDER } else { 0 };
    if flags & 0xff != 0 {
        order = flags & 0xff;
        if (global && !(PATTERN_ORDER..=SET_ORDER).contains(&order)) || (!global && order != 0) {
            emptied();
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
        emptied();
        set_error(ctx, INTERNAL_ERROR);
        return Ok(Value::FALSE);
    }
    // (preg_last_error() is set where the call ends: nothing between reads it.)
    let num = c.re.groups() + 1;
    let mut sets: Vec<PhpArray> = if global && order == PATTERN_ORDER { (0..num).map(|_| PhpArray::new()).collect() } else { Vec::new() };
    let mut marks: Option<PhpArray> = None;
    let mut list = PhpArray::new();
    let mut single: Option<PhpArray> = None;
    let mut matched = 0i64;
    let mut error = NO_ERROR;
    let mut spare = Spare::of(ctx, &c);
    let mut result = exec(spare.2, &c, &subject, start, Flags::default(), true, &mut spare.0);
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
                emptied();
                set_error(ctx, NO_ERROR);
                warn(ctx, "Get subpatterns list failed");
                return Ok(Value::FALSE);
            }
            if global {
                if order == PATTERN_ORDER {
                    let count = m.len();
                    for (i, set) in sets.iter_mut().enumerate() {
                        let v = if i < count {
                            if offset_capture { offset_pair(s, m.get(i), unmatched_as_null) } else { group_value(&subject, &m, i, unmatched_as_null) }
                        } else if offset_capture {
                            offset_pair(s, None, unmatched_as_null)
                        } else if unmatched_as_null {
                            Value::NULL
                        } else {
                            Value::str(PhpStr::empty())
                        };
                        let _ = set.push(v);
                    }
                    if let Some(mark) = &m.mark {
                        marks.get_or_insert_with(PhpArray::new).insert(Key::Int(matched - 1), Value::str(PhpStr::new(mark.clone())));
                    }
                } else {
                    let _ = list.push(Value::array(subpat_array(&c, &subject, &m, flags)));
                }
            } else {
                single = Some(subpat_array(&c, &subject, &m, flags));
                spare.0 = m;
                break;
            }
        }
        start = m1;
        spare.0 = m;
        // An empty match: try again here for a non-empty one, then move on
        // a character (as Perl's /g does).
        if m1 == m0 {
            match exec(spare.2, &c, &subject, start, Flags { not_empty_atstart: true, anchored: true }, false, &mut spare.0) {
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
        result = exec(spare.2, &c, &subject, start, Flags::default(), false, &mut spare.0);
    }
    if let Some(cell) = subpats {
        let out = if global {
            if order == PATTERN_ORDER {
                let mut out = PhpArray::new();
                for (i, set) in sets.into_iter().enumerate() {
                    let v = Value::array(set);
                    if let Some(name) = &c.names[i] {
                        out.insert(name.clone(), v.clone());
                    }
                    let _ = out.push(v);
                }
                if let Some(marks) = marks {
                    out.insert(Key::Str(PhpStr::from("MARK")), Value::array(marks));
                }
                out
            } else {
                list
            }
        } else {
            single.unwrap_or_default()
        };
        *cell.borrow_mut() = Value::array(out);
    }
    set_error(ctx, error);
    Ok(if error == NO_ERROR { Value::int(matched) } else { Value::FALSE })
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
    let mut spare = Spare::of(ctx, c);
    let mut result = exec(spare.2, c, subject, start, Flags::default(), true, &mut spare.0);
    match result {
        // About the subject's length: the room made once.
        Ok(Some(_)) if limit > 0 => out.reserve(s.len()),
        // Nothing to replace: no copy (the caller hands back the subject).
        Ok(_) => return Ok(Some(Vec::new())),
        Err(_) => {}
    }
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
                        let arr = Value::array(subpat_array(c, subject, &m, *flags));
                        let v = ctx.call(f, &mut [arr])?;
                        let v = to_str(ctx, &v)?;
                        out.extend_from_slice(v.as_bytes());
                    }
                }
                limit -= 1;
                start = m1;
                last_end = m1;
                spare.0 = m;
                if m1 == m0 {
                    piece = start;
                    let again = exec(spare.2, c,subject, start, Flags { not_empty_atstart: true, anchored: true }, false, &mut spare.0);
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
        result = exec(spare.2, c,subject, start, Flags::default(), false, &mut spare.0);
    }
}

/// One subject through one pattern: None when the pattern is bad or the
/// match failed.
fn replace_with(ctx: &mut dyn Ctx, regex: &PhpStr, s: &PhpStr, with: &Replacement, limit: usize, count: &mut usize) -> Result<Option<PhpStr>, Throw> {
    let Some(c) = compiled_str(ctx, regex) else { return Ok(None) };
    let before = *count;
    let out = replace_one(ctx, &c, s, with, limit, count)?;
    // Nothing replaced: the subject itself (php_pcre_replace_impl hands
    // back the same string), not a copy.
    Ok(out.map(|bytes| if *count == before { s.clone() } else { PhpStr::new(bytes) }))
}

fn limit_arg(args: &[Value], i: usize) -> usize {
    let l = int_arg(args, i, -1);
    if l < 0 { usize::MAX } else { l as usize }
}

fn set_count(args: &[Value], i: usize, count: usize) {
    if let Some(cell) = args.get(i).and_then(Value::as_reference) {
        *cell.borrow_mut() = Value::int(count as i64);
    }
}

/// Each subject (a string, or an array's values with their keys): Some
/// result per subject, None when it failed or (preg_filter) didn't change.
fn each_subject(
    ctx: &mut dyn Ctx,
    subject: &Value,
    mut f: impl FnMut(&mut dyn Ctx, &PhpStr) -> Result<Option<PhpStr>, Throw>,
) -> Result<Value, Throw> {
    match subject.as_array() {
        Some(a) => {
            let mut out = PhpArray::new();
            for (k, v) in a.iter() {
                let s = to_str(ctx, v)?;
                if let Some(r) = f(ctx, &s)? {
                    out.insert(k, Value::str(r));
                }
            }
            Ok(Value::array(out))
        }
        None => {
            let s = to_str(ctx, subject)?;
            Ok(f(ctx, &s)?.map_or(Value::NULL, Value::str))
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
    if replacement.is_array() && !regex.is_array() {
        return Err(ctx.throw("TypeError", format!("{name}(): Argument #1 ($pattern) must be of type array when argument #2 ($replacement) is an array, string given")));
    }
    let limit = limit_arg(args, 3);
    let mut count = 0usize;
    // The patterns, each with its replacement.
    let mut pairs: Vec<(PhpStr, PhpStr)> = Vec::new();
    match regex.as_array() {
        Some(patterns) => {
            let mut reps = replacement.as_array().map(|r| r.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>().into_iter());
            let fixed = if replacement.is_array() { None } else { Some(to_str(ctx, &replacement)?) };
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
        None => pairs.push((to_str(ctx, &regex)?, to_str(ctx, &replacement)?)),
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
    let patterns: Vec<PhpStr> = match regex.as_array() {
        Some(a) => {
            let mut v = Vec::new();
            for (_, p) in a.iter() {
                v.push(to_str(ctx, p)?);
            }
            v
        }
        None => vec![to_str(ctx, &regex)?],
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
    let Ok(map) = arg(args, 0).into_array() else { return Ok(Value::NULL) };
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
        if subject.is_null() {
            set_count(args, 3, count);
            return Ok(Value::NULL);
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
    let Some(c) = compiled_str(ctx, &regex) else { return Ok(Value::FALSE) };
    let no_empty = flags & SPLIT_NO_EMPTY != 0;
    let delim_capture = flags & SPLIT_DELIM_CAPTURE != 0;
    let offset_capture = flags & SPLIT_OFFSET_CAPTURE != 0;
    let mut out = PhpArray::new();
    let piece = |out: &mut PhpArray, a: usize, b: usize| {
        let v = if offset_capture { offset_pair(s, Some((a, b)), false) } else { Value::str(PhpStr::from(&s[a..b])) };
        let _ = out.push(v);
    };
    set_error(ctx, NO_ERROR);
    let mut last = 0usize;
    if limit == 0 {
        limit = -1;
    }
    if limit == -1 || limit > 1 {
        let mut start = 0usize;
        let mut spare = Spare::of(ctx, &c);
        let mut result = exec(spare.2, &c, &subject, start, Flags::default(), true, &mut spare.0);
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
                            let _ = out.push(if offset_capture { offset_pair(s, None, false) } else { Value::str(PhpStr::empty()) });
                        }
                        _ => {}
                    }
                }
            }
            start = m1;
            last = m1;
            spare.0 = m;
            if m1 == m0 {
                if limit != -1 && limit <= 1 {
                    break;
                }
                match exec(spare.2, &c, &subject, start, Flags { not_empty_atstart: true, anchored: true }, false, &mut spare.0) {
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
            result = exec(spare.2, &c, &subject, start, Flags::default(), false, &mut spare.0);
        }
        if error != NO_ERROR {
            set_error(ctx, error);
            return Ok(Value::FALSE);
        }
    }
    if !no_empty || last < s.len() {
        piece(&mut out, last, s.len());
    }
    Ok(Value::array(out))
}

fn preg_quote(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let delim = match arg(args, 1) {
        v if v.is_null() => None,
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
    Ok(Value::str(PhpStr::new(out)))
}

fn preg_grep(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let regex = str_arg(args, 0);
    let Ok(input) = arg(args, 1).into_array() else { return Ok(Value::FALSE) };
    let invert = int_arg(args, 2, 0) & GREP_INVERT != 0;
    let Some(c) = compiled_str(ctx, &regex) else { return Ok(Value::FALSE) };
    set_error(ctx, NO_ERROR);
    let mut out = PhpArray::new();
    let mut spare = Spare::of(ctx, &c);
    for (k, v) in input.iter() {
        let s = to_str(ctx, v)?;
        match exec(spare.2, &c, &s, 0, Flags::default(), true, &mut spare.0) {
            Ok(m) => {
                if m.is_some() != invert {
                    out.insert(k, v.clone());
                }
                if let Some(m) = m {
                    spare.0 = m;
                }
            }
            Err(code) => {
                set_error(ctx, code);
                break;
            }
        }
    }
    Ok(Value::array(out))
}

fn last_error(_: &mut dyn Ctx) -> i64 {
    LAST_ERROR.get()
}

fn preg_last_error(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::int(last_error(ctx)))
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
    Ok(Value::str(PhpStr::from(msg)))
}

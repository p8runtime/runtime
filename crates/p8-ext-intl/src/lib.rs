#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's intl extension over ICU4X: the pure-Rust ICU crates and their
//! compiled CLDR data (no ICU4C, no C). What is here follows php-src 8.4's
//! ext/intl, class by class:
//!
//! - `locale.rs`: Locale and locale_* (ICU's locale IDs are read as uloc.cpp
//!   does, in `uloc.rs`; display names come from ICU4X's experimental
//!   display-name data).
//! - `numfmt.rs`: NumberFormatter: ICU's DecimalFormat patterns, attributes
//!   and symbols, with CLDR's per-locale symbols, patterns and currencies.
//! - `normalizer.rs`: Normalizer (NFC, NFD, NFKC, NFKD).
//! - `datefmt.rs`: IntlDateFormatter: CLDR's date/time styles and ICU's
//!   pattern letters, over the time zone database the date functions use.
//! - `collator.rs`: Collator over ICU4X's collator.
//! - `intlchar.rs`: IntlChar over ICU4X's character properties.
//! - `msgfmt.rs`: MessageFormatter: ICU's MessageFormat syntax (arguments,
//!   number/date/time styles, plural, selectordinal, select).
//! - `grapheme.rs`: grapheme_* over ICU4X's grapheme segmenter.
//! - `idn.rs`: idn_to_ascii / idn_to_utf8 (UTS 46).
//!
//! What a class cannot do correctly yet fails as ICU fails (an error code and
//! false), never with made-up data.
//!
//! Reference: $PHP_SRC_PATH/ext/intl

mod collator;
mod consts;
mod currency_tables;
mod decimal;
mod datefmt;
mod grapheme;
mod idn;
mod intlchar;
mod locale;
mod msgfmt;
mod normalizer;
mod numfmt;
mod timezone;
mod uloc;
mod uloc_tables;

use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::ops::Diag;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};

pub const EXTENSION: Extension = Extension { name: "intl", register };

pub(crate) type R = Result<Value, Throw>;

fn register(registry: &mut Registry) {
    registry.add_ini("intl.default_locale", Some(b""));
    registry.add_ini("intl.error_level", Some(b"0"));
    registry.add_ini("intl.use_exceptions", Some(b"0"));
    for (name, value) in consts::GLOBAL {
        registry.add_constant(name.as_bytes(), *value);
    }
    registry.add_functions(BUILTINS);
    registry.add_functions(locale::BUILTINS);
    registry.add_functions(numfmt::BUILTINS);
    registry.add_functions(normalizer::BUILTINS);
    registry.add_functions(datefmt::BUILTINS);
    registry.add_functions(collator::BUILTINS);
    registry.add_functions(msgfmt::BUILTINS);
    registry.add_functions(grapheme::BUILTINS);
    registry.add_functions(idn::BUILTINS);
    registry.add_functions(timezone::BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_classes(locale::CLASSES);
    registry.add_classes(numfmt::CLASSES);
    registry.add_classes(normalizer::CLASSES);
    registry.add_classes(datefmt::CLASSES);
    registry.add_classes(collator::CLASSES);
    registry.add_classes(intlchar::CLASSES);
    registry.add_classes(msgfmt::CLASSES);
    registry.add_classes(timezone::CLASSES);
    numfmt::register(registry);
    datefmt::register(registry);
    collator::register(registry);
    msgfmt::register(registry);
    timezone::register(registry);
    registry.mark_reenters(&["collator_sort", "collator_asort", "collator_sort_with_sort_keys", "datefmt_format", "datefmt_format_object", "msgfmt_format", "msgfmt_format_message"]);
    registry.add_module("intl");
}

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "IntlException",
    parent: Some("Exception"),
    interfaces: &[],
    flags: 0,
    properties: &[],
    methods: &[],
    constants: &[],
    debug_info: None,
}];

macro_rules! b {
    ($name:literal, $f:path) => {
        p8_runtime::ext::Builtin { name: $name, run: $f, by_ref: &[] }
    };
}
pub(crate) use b;

/// A class's methods over the procedural functions: `$this` becomes the
/// first argument, the stub's checks run first, and the running method's
/// name is kept for messages.
macro_rules! methods {
    ($class:literal; statics: [$(($sname:literal, $sfunc:path)),*]; $($name:literal => $func:ident;)*) => {
        &[$(($sname, Some(p8_runtime::object::Native::Static({
            fn s(ctx: &mut dyn p8_runtime::ext::Ctx, args: &mut [p8_runtime::value::Value]) -> crate::R {
                let prev = crate::CALLED.with(|c| c.replace(Some(concat!($class, "::", $sname))));
                let r = $sfunc(ctx, args);
                crate::CALLED.with(|c| c.set(prev));
                r
            }
            s
        }))),)* $(($name, Some(p8_runtime::object::Native::Method({
            fn m(ctx: &mut dyn p8_runtime::ext::Ctx, this: &std::rc::Rc<p8_runtime::object::Object>, args: &mut [p8_runtime::value::Value]) -> crate::R {
                crate::prep(ctx, concat!($class, "::", $name), args)?;
                let mut all = Vec::with_capacity(args.len() + 1);
                all.push(p8_runtime::value::Value::object(std::rc::Rc::clone(this)));
                all.extend(args.iter().cloned());
                let prev = crate::CALLED.with(|c| c.replace(Some(concat!($class, "::", $name))));
                let r = $func(ctx, &mut all);
                crate::CALLED.with(|c| c.set(prev));
                for (slot, v) in args.iter_mut().zip(all.into_iter().skip(1)) {
                    *slot = v;
                }
                r
            }
            m
        }))),)*]
    };
}
pub(crate) use methods;

/// Static methods over procedural functions (the stub's checks first).
macro_rules! statics {
    ($($name:literal => $func:ident, $stub:literal;)*) => {
        &[$(($name, Some(p8_runtime::object::Native::Static({
            fn m(ctx: &mut dyn p8_runtime::ext::Ctx, args: &mut [p8_runtime::value::Value]) -> crate::R {
                crate::prep(ctx, $stub, args)?;
                let prev = crate::CALLED.with(|c| c.replace(Some($stub)));
                let r = $func(ctx, args);
                crate::CALLED.with(|c| c.set(prev));
                r
            }
            m
        }))),)*]
    };
}
pub(crate) use statics;

static BUILTINS: &[Builtin] = &[
    b!("intl_get_error_code", intl_get_error_code),
    b!("intl_get_error_message", intl_get_error_message),
    b!("intl_is_failure", intl_is_failure),
    b!("intl_error_name", intl_error_name),
];

// ---- errors (intl_error.c) ----

pub(crate) const U_ZERO_ERROR: i64 = 0;
pub(crate) const U_ILLEGAL_ARGUMENT_ERROR: i64 = 1;
pub(crate) const U_PARSE_ERROR: i64 = 9;
pub(crate) const U_UNSUPPORTED_ERROR: i64 = 16;
pub(crate) const U_PATTERN_SYNTAX_ERROR: i64 = 65799;
pub(crate) const U_ARGUMENT_TYPE_MISMATCH: i64 = 65804;

/// An intl object's (or the global) last error: intl_error.
#[derive(Default, Debug, Clone)]
pub(crate) struct IntlError {
    pub code: Cell<i64>,
    pub msg: RefCell<Option<String>>,
}

impl IntlError {
    pub fn reset(&self) {
        self.code.set(U_ZERO_ERROR);
        *self.msg.borrow_mut() = None;
    }

    pub fn set(&self, code: i64, msg: Option<&str>) {
        self.code.set(code);
        if let Some(m) = msg {
            *self.msg.borrow_mut() = Some(m.to_string());
        }
    }

    /// intl_error_get_message: "message: U_NAME", or just the name.
    pub fn message(&self) -> String {
        let name = error_name(self.code.get());
        match &*self.msg.borrow() {
            Some(m) => format!("{m}: {name}"),
            None => name.to_string(),
        }
    }
}

const KEY: &str = "intl";

fn global(ctx: &mut dyn Ctx) -> &mut IntlError {
    let slot = ctx.ext_data(KEY);
    if slot.as_ref().is_none_or(|b| !b.is::<IntlError>()) {
        *slot = Some(Box::new(IntlError::default()));
    }
    slot.as_mut().and_then(|b| b.downcast_mut::<IntlError>()).expect("intl error state")
}

fn ini_int(ctx: &dyn Ctx, name: &[u8]) -> i64 {
    ctx.ini_get(name).map(|v| v.to_int()).unwrap_or(0)
}

/// intl_error_reset(NULL).
pub(crate) fn reset_global(ctx: &mut dyn Ctx) {
    global(ctx).reset();
}


/// intl_error_set_code(NULL, code): the global code only.
pub(crate) fn set_global_code(ctx: &mut dyn Ctx, code: i64) {
    global(ctx).code.set(code);
}

/// intl_errors_set(obj, code, msg): the object's error and the global one;
/// the global message is reported at intl.error_level and thrown as an
/// IntlException with intl.use_exceptions.
pub(crate) fn set_error(ctx: &mut dyn Ctx, obj: Option<&IntlError>, code: i64, msg: &str) -> Result<(), Throw> {
    if let Some(e) = obj {
        e.set(code, Some(msg));
    }
    report(ctx, msg);
    let g = global(ctx);
    g.code.set(code);
    *g.msg.borrow_mut() = Some(msg.to_string());
    if ini_int(ctx, b"intl.use_exceptions") != 0 {
        return Err(ctx.throw("IntlException", msg.to_string()));
    }
    Ok(())
}

/// The message at intl.error_level, as php_error_docref words it ("f(): msg").
fn report(ctx: &mut dyn Ctx, msg: &str) {
    let level = ini_int(ctx, b"intl.error_level");
    if level != 0 {
        let name = called(ctx.builtin_name());
        let text = if name.is_empty() { msg.to_string() } else { format!("{name}(): {msg}") };
        ctx.report(level, &text);
    }
}

/// intl_errors_set_custom_msg(obj, msg): the message on the object and the
/// global error (reported and thrown as the settings say); codes unchanged.
pub(crate) fn set_message(ctx: &mut dyn Ctx, obj: Option<&IntlError>, msg: &str) -> Result<(), Throw> {
    if let Some(e) = obj {
        *e.msg.borrow_mut() = Some(msg.to_string());
    }
    report(ctx, msg);
    *global(ctx).msg.borrow_mut() = Some(msg.to_string());
    if ini_int(ctx, b"intl.use_exceptions") != 0 {
        return Err(ctx.throw("IntlException", msg.to_string()));
    }
    Ok(())
}


/// u_errorName.
pub(crate) fn error_name(code: i64) -> &'static str {
    if code == U_PATTERN_SYNTAX_ERROR {
        return "U_PATTERN_SYNTAX_ERROR";
    }
    if code == U_ARGUMENT_TYPE_MISMATCH {
        return "U_ARGUMENT_TYPE_MISMATCH";
    }
    for (name, value) in consts::GLOBAL {
        if let Constant::Int(v) = value
            && *v == code
            && name.starts_with("U_")
            && !name.ends_with("_START")
            && !name.ends_with("_LIMIT")
        {
            return name;
        }
    }
    match code {
        65536..=65556 => PARSE_ERRORS[(code - 65536) as usize],
        65792..=65812 => FMT_ERRORS[(code - 65792) as usize],
        _ => "[BOGUS UErrorCode]",
    }
}

static PARSE_ERRORS: &[&str] = &[
    "U_BAD_VARIABLE_DEFINITION",
    "U_MALFORMED_RULE",
    "U_MALFORMED_SET",
    "U_MALFORMED_SYMBOL_REFERENCE",
    "U_MALFORMED_UNICODE_ESCAPE",
    "U_MALFORMED_VARIABLE_DEFINITION",
    "U_MALFORMED_VARIABLE_REFERENCE",
    "U_MISMATCHED_SEGMENT_DELIMITERS",
    "U_MISPLACED_ANCHOR_START",
    "U_MISPLACED_CURSOR_OFFSET",
    "U_MISPLACED_QUANTIFIER",
    "U_MISSING_OPERATOR",
    "U_MISSING_SEGMENT_CLOSE",
    "U_MULTIPLE_ANTE_CONTEXTS",
    "U_MULTIPLE_CURSORS",
    "U_MULTIPLE_POST_CONTEXTS",
    "U_TRAILING_BACKSLASH",
    "U_UNDEFINED_SEGMENT_REFERENCE",
    "U_UNDEFINED_VARIABLE",
    "U_UNQUOTED_SPECIAL",
    "U_UNTERMINATED_QUOTE",
];

static FMT_ERRORS: &[&str] = &[
    "U_UNEXPECTED_TOKEN",
    "U_MULTIPLE_DECIMAL_SEPARATORS",
    "U_MULTIPLE_EXPONENTIAL_SYMBOLS",
    "U_MALFORMED_EXPONENTIAL_PATTERN",
    "U_MULTIPLE_PERCENT_SYMBOLS",
    "U_MULTIPLE_PERMILL_SYMBOLS",
    "U_MULTIPLE_PAD_SPECIFIERS",
    "U_PATTERN_SYNTAX_ERROR",
    "U_ILLEGAL_PAD_POSITION",
    "U_UNMATCHED_BRACES",
    "U_UNSUPPORTED_PROPERTY",
    "U_UNSUPPORTED_ATTRIBUTE",
    "U_ARGUMENT_TYPE_MISMATCH",
    "U_DUPLICATE_KEYWORD",
    "U_UNDEFINED_KEYWORD",
    "U_DEFAULT_KEYWORD_MISSING",
    "U_DECIMAL_NUMBER_SYNTAX_ERROR",
    "U_FORMAT_INEXACT_ERROR",
    "U_NUMBER_ARG_OUTOFBOUNDS_ERROR",
    "U_NUMBER_SKELETON_SYNTAX_ERROR",
    "U_MF_UNRESOLVED_VARIABLE_ERROR",
];

/// intl_get_error_message() as a string (for an exception's message).
pub(crate) fn global_message(ctx: &mut dyn Ctx) -> String {
    global(ctx).message()
}

fn intl_get_error_code(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::int(global(ctx).code.get()))
}

fn intl_get_error_message(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::str(global(ctx).message()))
}

fn intl_is_failure(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::bool(arg(args, 0).to_int() > U_ZERO_ERROR))
}

fn intl_error_name(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::str(error_name(arg(args, 0).to_int())))
}

// ---- shared helpers ----

thread_local! {
    /// The static method running (its stub name), for messages that name
    /// the function: a method and its procedural twin share the code.
    pub(crate) static CALLED: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// The name of the running function for messages: the static method's when
/// one is running, else `function`.
pub(crate) fn called(function: &'static str) -> &'static str {
    CALLED.with(Cell::get).unwrap_or(function)
}

pub(crate) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

/// Argument `i` as a string (after the stub's checks), or None when absent or null.
pub(crate) fn opt_str(args: &[Value], i: usize) -> Option<String> {
    let v = arg(args, i);
    if v.is_null_or_undef() {
        return None;
    }
    Some(lossy(v.to_php_string().as_ref().map(|s| s.as_bytes()).unwrap_or_default()))
}

pub(crate) fn str_arg(args: &[Value], i: usize) -> String {
    opt_str(args, i).unwrap_or_default()
}

pub(crate) fn bytes_arg(args: &[Value], i: usize) -> Vec<u8> {
    arg(args, i).to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()
}

pub(crate) fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

pub(crate) fn s(text: impl Into<String>) -> Value {
    Value::str(PhpStr::new(text.into().into_bytes()))
}

/// PHP's argument checks for the method `stub` ("NumberFormatter::format").
pub(crate) fn prep(ctx: &mut dyn Ctx, stub: &str, args: &mut [Value]) -> Result<(), Throw> {
    let Some(sig) = p8_runtime::stubs::lookup(stub.as_bytes()) else { return Ok(()) };
    let mut diags = Vec::new();
    let checked = p8_runtime::args::check(sig, args, &mut diags);
    for d in diags {
        match d {
            Diag::Warning(m) => ctx.warn(&m),
            Diag::Deprecated(m) => ctx.deprecated(&m),
        }
    }
    checked.map_err(|(class, message)| ctx.throw(class, message))
}

/// The default locale: intl.default_locale, else ICU's (from the
/// environment as uprv_getDefaultLocaleID reads it).
pub(crate) fn default_locale(ctx: &mut dyn Ctx) -> String {
    if let Some(v) = ctx.ini_get(b"intl.default_locale")
        && let Some(s) = v.to_php_string()
        && !s.is_empty()
    {
        return lossy(s.as_bytes());
    }
    icu_default_locale()
}

pub(crate) fn icu_default_locale() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var)
            && !v.is_empty()
        {
            if v == "C" || v == "POSIX" || v.starts_with("C.") {
                return "en_US_POSIX".into();
            }
            let name = uloc::get_name(&v);
            return if name.is_empty() { "en_US_POSIX".into() } else { name };
        }
    }
    "en_US_POSIX".into()
}

/// The ICU4X locale an ICU locale ID stands for (its language, script,
/// region and the -u- keywords ICU4X reads); `und` when it has none.
pub(crate) fn icu4x_locale(id: &str) -> icu_locale_core::Locale {
    let p = uloc::parts(id);
    let mut tag = if p.lang.is_empty() || p.lang.contains('-') { "und".to_string() } else { p.lang.clone() };
    if !p.script.is_empty() {
        tag.push('-');
        tag.push_str(&p.script);
    }
    if !p.region.is_empty() {
        tag.push('-');
        tag.push_str(&p.region);
    }
    let mut u = Vec::new();
    for (k, v) in &p.keywords {
        let key = match k.as_str() {
            "numbers" => "nu",
            "calendar" => "ca",
            "collation" => "co",
            "currency" => "cu",
            "hours" => "hc",
            _ => continue,
        };
        let v = match v.to_ascii_lowercase().as_str() {
            "gregorian" => "gregory".to_string(),
            "phonebook" => "phonebk".to_string(),
            "traditional" => "trad".to_string(),
            other => other.to_string(),
        };
        u.push(format!("{key}-{v}"));
    }
    if !u.is_empty() {
        tag.push_str("-u-");
        tag.push_str(&u.join("-"));
    }
    icu_locale_core::Locale::try_from_str(&tag)
        .or_else(|_| icu_locale_core::Locale::try_from_str(if p.lang.is_empty() { "und" } else { &p.lang }))
        .unwrap_or(icu_locale_core::Locale::UNKNOWN)
}



/// A data request's metadata that does not log a miss (one is expected).
pub(crate) fn silent() -> icu_provider::DataRequestMetadata {
    let mut m = icu_provider::DataRequestMetadata::default();
    m.silent = true;
    m
}

/// ICU's "valid locale" for a request: the longest prefix of the ID that ICU
/// has data for (ures_getLocaleByType ULOC_VALID_LOCALE), "root" for none;
/// an alias (zh_CN) resolves through the likely script (zh_Hans_CN).
pub(crate) fn valid_locale(id: &str) -> String {
    let mut name = uloc::get_name(id);
    if let Some(at) = name.find('@') {
        name.truncate(at);
    }
    let p = uloc::subtags(&name);
    if !uloc_tables::AVAILABLE.contains(&name.as_str()) && p.script.is_empty() && !p.region.is_empty() && p.variant.is_empty()
        && let Ok(mut l) = icu_locale_core::LanguageIdentifier::try_from_str(&format!("{}-{}", p.lang, p.region))
    {
        icu_locale::LocaleExpander::new_common().maximize(&mut l);
        if let Some(script) = l.script {
            let with_script = format!("{}_{}_{}", p.lang, script, p.region);
            if uloc_tables::AVAILABLE.contains(&with_script.as_str()) {
                return with_script;
            }
        }
    }
    loop {
        if uloc_tables::AVAILABLE.contains(&name.as_str()) {
            return name;
        }
        match name.rfind('_') {
            Some(i) => {
                name.truncate(i);
                while name.ends_with('_') {
                    name.pop();
                }
            }
            None => return "root".into(),
        }
    }
}

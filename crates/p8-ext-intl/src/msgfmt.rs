//! MessageFormatter and msgfmt_* (msgformat/*.c, msgformat_helpers.cpp):
//! ICU's MessageFormat syntax (apostrophe mode DOUBLE_OPTIONAL) with
//! numbered or named arguments; the number, date and time types with their
//! styles or patterns; plural and selectordinal (CLDR's plural rules via
//! icu_plurals, `=n` cases, `offset:`, `#`), select and choice; PHP's
//! conversion of the arguments by the type the pattern gives them.
//!
//! Not available (U_UNSUPPORTED_ERROR): the spellout, ordinal and duration
//! types (rule-based number formats), number skeletons (`::`), and parse()
//! of anything but plain arguments and numbers.

use crate::datefmt::{self, Zone};
use crate::decimal::Dec;
use crate::numfmt::{self, Formatter as NumFmt, Num};
use crate::{IntlError, R, U_ILLEGAL_ARGUMENT_ERROR, U_PATTERN_SYNTAX_ERROR, U_UNSUPPORTED_ERROR, U_ZERO_ERROR, arg, b, default_locale, lossy, s, str_arg};
use icu_plurals::{PluralCategory, PluralRules};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassHooks, ClassSpec, Ctx, Registry, Throw};
use p8_runtime::object::Object;
use p8_runtime::value::{Kind as ValueKind, Value};
use std::cell::RefCell;
use std::rc::Rc;

// ---- the pattern ----

#[derive(Clone, Debug)]
enum Node {
    Text(String),
    /// `#` in a plural sub-message.
    Pound,
    Arg(String, Kind),
}

#[derive(Clone, Debug)]
enum Kind {
    None,
    Simple(String, Option<String>),
    Plural { ordinal: bool, offset: f64, cases: Vec<(String, Vec<Node>)> },
    Select(Vec<(String, Vec<Node>)>),
    Choice(Vec<(f64, bool, Vec<Node>)>),
}

struct P<'a> {
    c: Vec<char>,
    i: usize,
    _s: &'a str,
}

/// Where the pattern failed; `BAD_TYPE` (an unknown argument type) fails as ICU's U_ILLEGAL_ARGUMENT_ERROR.
#[derive(Debug)]
struct SyntaxError(usize);

const BAD_TYPE: usize = usize::MAX;

impl P<'_> {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }

    /// A message up to `}` (nested) or the end; `in_plural` makes `#` special.
    fn message(&mut self, nested: bool, in_plural: bool, in_choice: bool) -> Result<Vec<Node>, SyntaxError> {
        let mut out = Vec::new();
        let mut text = String::new();
        loop {
            let Some(c) = self.peek() else {
                if nested {
                    return Err(SyntaxError(self.i));
                }
                break;
            };
            match c {
                '\'' => {
                    let next = self.c.get(self.i + 1).copied();
                    if next == Some('\'') {
                        text.push('\'');
                        self.i += 2;
                    } else if matches!(next, Some('{') | Some('}')) || (in_plural && next == Some('#')) || (in_choice && next == Some('|')) {
                        self.i += 1;
                        // quoted until the next single apostrophe
                        while let Some(q) = self.peek() {
                            if q == '\'' {
                                if self.c.get(self.i + 1) == Some(&'\'') {
                                    text.push('\'');
                                    self.i += 2;
                                    continue;
                                }
                                self.i += 1;
                                break;
                            }
                            text.push(q);
                            self.i += 1;
                        }
                    } else {
                        text.push('\'');
                        self.i += 1;
                    }
                }
                '{' => {
                    if !text.is_empty() {
                        out.push(Node::Text(std::mem::take(&mut text)));
                    }
                    self.i += 1;
                    out.push(self.argument()?);
                }
                '}' => {
                    if nested {
                        break;
                    }
                    return Err(SyntaxError(self.i));
                }
                '#' if in_plural => {
                    if !text.is_empty() {
                        out.push(Node::Text(std::mem::take(&mut text)));
                    }
                    out.push(Node::Pound);
                    self.i += 1;
                }
                '|' if in_choice => break,
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        if !text.is_empty() {
            out.push(Node::Text(text));
        }
        Ok(out)
    }

    fn word(&mut self) -> String {
        let mut w = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, '{' | '}' | ',' | '#' | '|' | ':') {
                break;
            }
            w.push(c);
            self.i += 1;
        }
        w
    }

    fn argument(&mut self) -> Result<Node, SyntaxError> {
        self.ws();
        let name = self.word();
        if name.is_empty() || (name.starts_with(|c: char| c.is_ascii_digit()) && !name.chars().all(|c| c.is_ascii_digit())) || (name.len() > 1 && name.starts_with('0') && name.chars().all(|c| c.is_ascii_digit())) {
            return Err(SyntaxError(self.i));
        }
        self.ws();
        match self.peek() {
            Some('}') => {
                self.i += 1;
                return Ok(Node::Arg(name, Kind::None));
            }
            Some(',') => self.i += 1,
            _ => return Err(SyntaxError(self.i)),
        }
        self.ws();
        let ty = self.word().to_ascii_lowercase();
        self.ws();
        let kind = match ty.as_str() {
            "plural" | "selectordinal" | "select" => {
                if self.peek() != Some(',') {
                    return Err(SyntaxError(self.i));
                }
                self.i += 1;
                let style_start = self.i;
                let fail_at_style = |e: SyntaxError, end: usize| if e.0 >= end { SyntaxError(style_start) } else { e };
                let end = self.c.len();
                let mut cases = Vec::new();
                let mut offset = 0.0;
                loop {
                    self.ws();
                    match self.peek() {
                        Some('}') => {
                            self.i += 1;
                            break;
                        }
                        None => return Err(SyntaxError(style_start)),
                        _ => {}
                    }
                    let start = self.i;
                    let mut key = self.word();
                    if ty != "select" && key == "offset" && self.peek() == Some(':') {
                        self.i += 1;
                        self.ws();
                        let v = self.word();
                        offset = v.parse().map_err(|_| SyntaxError(self.i))?;
                        continue;
                    }
                    if key.is_empty() {
                        return Err(SyntaxError(start));
                    }
                    if key.starts_with('=') {
                        key = format!("={}", key[1..].parse::<f64>().map_err(|_| SyntaxError(start))?);
                    }
                    self.ws();
                    if self.peek() != Some('{') {
                        return Err(SyntaxError(self.i));
                    }
                    self.i += 1;
                    let msg = self.message(true, ty != "select", false).map_err(|e| fail_at_style(e, end))?;
                    if self.peek() != Some('}') {
                        return Err(fail_at_style(SyntaxError(self.i), end));
                    }
                    self.i += 1;
                    cases.push((key, msg));
                }
                if !cases.iter().any(|(k, _)| k == "other") {
                    return Err(SyntaxError(self.i));
                }
                if ty == "select" { Kind::Select(cases) } else { Kind::Plural { ordinal: ty == "selectordinal", offset, cases } }
            }
            "choice" => {
                if self.peek() != Some(',') {
                    return Err(SyntaxError(self.i));
                }
                self.i += 1;
                let mut cases = Vec::new();
                loop {
                    self.ws();
                    let mut num = String::new();
                    while let Some(c) = self.peek() {
                        if c == '#' || c == '<' || c == '≤' {
                            break;
                        }
                        num.push(c);
                        self.i += 1;
                    }
                    let num = num.trim();
                    let limit = match num {
                        "∞" => f64::INFINITY,
                        "-∞" => f64::NEG_INFINITY,
                        n => n.parse::<f64>().map_err(|_| SyntaxError(self.i))?,
                    };
                    let strict = self.peek() == Some('<');
                    self.i += 1;
                    let msg = self.message(true, false, true)?;
                    cases.push((limit, strict, msg));
                    match self.peek() {
                        Some('|') => self.i += 1,
                        Some('}') => {
                            self.i += 1;
                            break;
                        }
                        _ => return Err(SyntaxError(self.i)),
                    }
                }
                Kind::Choice(cases)
            }
            "number" | "date" | "time" | "spellout" | "ordinal" | "duration" => {
                let style = match self.peek() {
                    Some(',') => {
                        self.i += 1;
                        // the style runs to the matching '}', quotes kept
                        let mut depth = 0;
                        let mut st = String::new();
                        let mut quoted = false;
                        loop {
                            let Some(c) = self.peek() else { return Err(SyntaxError(self.i)) };
                            if c == '\'' {
                                quoted = !quoted;
                            } else if !quoted && c == '{' {
                                depth += 1;
                            } else if !quoted && c == '}' {
                                if depth == 0 {
                                    break;
                                }
                                depth -= 1;
                            }
                            st.push(c);
                            self.i += 1;
                        }
                        Some(st)
                    }
                    _ => None,
                };
                self.ws();
                if self.peek() != Some('}') {
                    return Err(SyntaxError(self.i));
                }
                self.i += 1;
                Kind::Simple(ty, style)
            }
            _ if !ty.is_empty() && ty.chars().all(|c| c.is_alphanumeric() || c == '_') => return Err(SyntaxError(BAD_TYPE)),
            _ => return Err(SyntaxError(self.i)),
        };
        Ok(Node::Arg(name, kind))
    }
}

fn parse_message(pattern: &str) -> Result<Vec<Node>, SyntaxError> {
    let mut p = P { c: pattern.chars().collect(), i: 0, _s: pattern };
    p.message(false, false, false)
}

/// ICU's UParseError as PHP prints it: the 16 characters around the offset.
fn parse_error_text(pattern: &str, offset: usize) -> String {
    let c: Vec<char> = pattern.chars().collect();
    let pre: String = c[offset.saturating_sub(15).min(c.len())..offset.min(c.len())].iter().collect();
    let post: String = c[offset.min(c.len())..(offset + 15).min(c.len())].iter().collect();
    format!("parse error at offset {offset}, after \"{pre}\", before or at \"{post}\"")
}

/// The type PHP converts each argument to (umsg_parse_format): by name.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ArgType {
    Str,
    Double,
    Int64,
    Date,
    /// Untyped in a numbered pattern: by the value's PHP type.
    Any,
    /// Two types declared for one argument.
    Conflict,
}

fn collect_types(nodes: &[Node], named: bool, out: &mut Vec<(String, ArgType)>) {
    for n in nodes {
        if let Node::Arg(name, kind) = n {
            let t = match kind {
                Kind::None => {
                    if named {
                        ArgType::Str
                    } else {
                        ArgType::Any
                    }
                }
                Kind::Simple(ty, style) => match ty.as_str() {
                    "number" if style.as_deref().map(str::trim) == Some("integer") => ArgType::Int64,
                    "date" | "time" => ArgType::Date,
                    _ => ArgType::Double,
                },
                Kind::Plural { .. } | Kind::Choice(_) => ArgType::Double,
                Kind::Select(_) => ArgType::Str,
            };
            match out.iter().find(|(k, _)| k == name) {
                None => out.push((name.clone(), t)),
                // a name given two types (Any stands for none)
                Some((_, have)) if *have != t && *have != ArgType::Any && t != ArgType::Any => out.push((name.clone(), ArgType::Conflict)),
                Some(_) => {}
            }
            match kind {
                Kind::Plural { cases, .. } | Kind::Select(cases) => {
                    for (_, m) in cases {
                        collect_types(m, named, out);
                    }
                }
                Kind::Choice(cases) => {
                    for (_, _, m) in cases {
                        collect_types(m, named, out);
                    }
                }
                _ => {}
            }
        }
    }
}

fn uses_named(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Arg(name, kind) => {
            !name.chars().all(|c| c.is_ascii_digit())
                || match kind {
                    Kind::Plural { cases, .. } | Kind::Select(cases) => cases.iter().any(|(_, m)| uses_named(m)),
                    Kind::Choice(cases) => cases.iter().any(|(_, _, m)| uses_named(m)),
                    _ => false,
                }
        }
        _ => false,
    })
}

/// An argument after PHP's conversion.
#[derive(Clone, Debug)]
enum Arg {
    Str(String),
    Int(i64),
    Double(f64),
    /// Milliseconds.
    Date(f64),
}

impl Arg {
    fn number(&self) -> Option<Num> {
        match self {
            Arg::Int(i) => Some(Num::Dec(Dec::from_i64(*i))),
            Arg::Double(d) => Some(Num::from_f64(*d)),
            Arg::Date(ms) => Some(Num::from_f64(*ms)),
            Arg::Str(_) => None,
        }
    }
}

struct Ctx2<'a> {
    locale: &'a str,
    zone: Zone,
    args: &'a [(String, Arg)],
}

fn find<'a>(args: &'a [(String, Arg)], name: &str) -> Option<&'a Arg> {
    args.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn decimal_formatter(locale: &str) -> Option<NumFmt> {
    NumFmt::new(locale, numfmt::STYLE_DECIMAL, None).ok()
}

/// Format the message's nodes. Err: an ICU error code.
fn format_nodes(nodes: &[Node], cx: &Ctx2, pound: Option<&Num>, out: &mut String) -> Result<(), i64> {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Pound => match pound {
                Some(num) => out.push_str(&decimal_formatter(cx.locale).ok_or(U_UNSUPPORTED_ERROR)?.format(num.clone())),
                None => out.push('#'),
            },
            Node::Arg(name, kind) => {
                let Some(a) = find(cx.args, name) else {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                    continue;
                };
                format_arg(a, kind, cx, out)?;
            }
        }
    }
    Ok(())
}

fn format_arg(a: &Arg, kind: &Kind, cx: &Ctx2, out: &mut String) -> Result<(), i64> {
    match kind {
        Kind::None => match a {
            Arg::Str(s) => out.push_str(s),
            Arg::Date(ms) => out.push_str(&format_date(cx, *ms, datefmt::SHORT, datefmt::SHORT, None)?),
            other => out.push_str(&decimal_formatter(cx.locale).ok_or(U_UNSUPPORTED_ERROR)?.format(other.number().unwrap_or(Num::Nan))),
        },
        Kind::Simple(ty, style) => match ty.as_str() {
            "number" => {
                let num = match a {
                    Arg::Str(s) => Num::from_f64(s.trim().parse().unwrap_or(0.0)),
                    other => other.number().unwrap_or(Num::Nan),
                };
                let f = match style.as_deref().map(|s| if s.trim().starts_with("::") || matches!(s.trim(), "integer" | "percent" | "currency") { s.trim() } else { s }) {
                    None => NumFmt::new(cx.locale, numfmt::STYLE_DECIMAL, None),
                    Some("integer") => NumFmt::new(cx.locale, numfmt::STYLE_DECIMAL, None).map(|mut f| {
                        f.props.max_frac = 0;
                        f.props.min_frac = 0;
                        f.props.parse_int_only = true;
                        f
                    }),
                    Some("percent") => NumFmt::new(cx.locale, numfmt::STYLE_PERCENT, None),
                    Some("currency") => NumFmt::new(cx.locale, numfmt::STYLE_CURRENCY, None),
                    Some(st) if st.starts_with("::") => return Err(U_UNSUPPORTED_ERROR),
                    Some(pattern) => NumFmt::new(cx.locale, numfmt::STYLE_PATTERN_DECIMAL, Some(pattern)),
                }
                .map_err(|(c, _)| c)?;
                out.push_str(&f.format(num));
            }
            "date" | "time" => {
                let ms = match a {
                    Arg::Date(ms) => *ms,
                    Arg::Int(i) => *i as f64,
                    Arg::Double(d) => *d,
                    Arg::Str(_) => return Err(U_ILLEGAL_ARGUMENT_ERROR),
                };
                let style = match style.as_deref().map(|s| if matches!(s.trim(), "short" | "medium" | "long" | "full") { s.trim() } else { s }) {
                    None | Some("medium") => Ok(datefmt::MEDIUM),
                    Some("short") => Ok(datefmt::SHORT),
                    Some("long") => Ok(datefmt::LONG),
                    Some("full") => Ok(datefmt::FULL),
                    Some(p) => Err(p.to_string()),
                };
                let text = match style {
                    Ok(st) if ty == "date" => format_date(cx, ms, st, datefmt::NONE, None)?,
                    Ok(st) => format_date(cx, ms, datefmt::NONE, st, None)?,
                    Err(p) => format_date(cx, ms, datefmt::PATTERN, datefmt::PATTERN, Some(&p))?,
                };
                out.push_str(&text);
            }
            _ => return Err(U_UNSUPPORTED_ERROR),
        },
        Kind::Plural { ordinal, offset, cases } => {
            let Some(num) = a.number().or_else(|| if let Arg::Str(s) = a { Some(Num::from_f64(s.trim().parse().unwrap_or(0.0))) } else { None }) else { return Err(U_ILLEGAL_ARGUMENT_ERROR) };
            let value = num.to_f64();
            // explicit values first, on the number itself
            let explicit = cases.iter().find(|(k, _)| k.strip_prefix('=').and_then(|v| v.parse::<f64>().ok()) == Some(value));
            let shown = Num::from_f64(value - offset);
            let msg = match explicit {
                Some((_, m)) => m,
                None => {
                    let keyword = plural_keyword(cx.locale, *ordinal, &shown);
                    cases.iter().find(|(k, _)| *k == keyword).or_else(|| cases.iter().find(|(k, _)| k == "other")).map(|(_, m)| m).ok_or(U_ILLEGAL_ARGUMENT_ERROR)?
                }
            };
            format_nodes(msg, cx, Some(&shown), out)?;
        }
        Kind::Select(cases) => {
            let key = match a {
                Arg::Str(s) => s.clone(),
                _ => return Err(U_ILLEGAL_ARGUMENT_ERROR),
            };
            let msg = cases.iter().find(|(k, _)| *k == key).or_else(|| cases.iter().find(|(k, _)| k == "other")).map(|(_, m)| m).ok_or(U_ILLEGAL_ARGUMENT_ERROR)?;
            format_nodes(msg, cx, None, out)?;
        }
        Kind::Choice(cases) => {
            let v = a.number().map(|n| n.to_f64()).unwrap_or(0.0);
            let mut chosen = cases.first().map(|c| &c.2);
            for (limit, strict, m) in cases {
                if (*strict && v > *limit) || (!*strict && v >= *limit) {
                    chosen = Some(m);
                } else {
                    break;
                }
            }
            if let Some(m) = chosen {
                format_nodes(m, cx, None, out)?;
            }
        }
    }
    Ok(())
}

/// The plural keyword CLDR's rules give the number as the default format shows it.
fn plural_keyword(locale: &str, ordinal: bool, n: &Num) -> &'static str {
    let loc = crate::icu4x_locale(locale);
    let rules = if ordinal { PluralRules::try_new_ordinal((&loc).into()) } else { PluralRules::try_new_cardinal((&loc).into()) };
    let Ok(rules) = rules else { return "other" };
    let text = match n {
        Num::Dec(d) => {
            let mut d = d.clone();
            d.round_at(-3, crate::decimal::ROUND_HALFEVEN);
            d.neg = false;
            d.to_plain()
        }
        _ => return "other",
    };
    let Ok(dec) = fixed_decimal::Decimal::try_from_str(&text) else { return "other" };
    let op = icu_plurals::PluralOperands::from(&dec);
    match rules.category_for(op) {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
}

fn format_date(cx: &Ctx2, ms: f64, date: i64, time: i64, pattern: Option<&str>) -> Result<String, i64> {
    let f = datefmt::Formatter::new(cx.locale, date, time, cx.zone.clone(), datefmt::GREGORIAN, pattern)?;
    f.format_ms(ms as i64).ok_or(U_ILLEGAL_ARGUMENT_ERROR)
}

// ---- the PHP class ----

#[derive(Clone)]
pub struct Fmt {
    locale: String,
    pattern: String,
    nodes: Vec<Node>,
}

#[derive(Default)]
pub struct State {
    fmt: RefCell<Option<Fmt>>,
    err: IntlError,
}

fn state(o: &Object) -> &State {
    o.native::<State>().expect("MessageFormatter state")
}

pub fn register(registry: &mut Registry) {
    registry.add_data("MessageFormatter", || Rc::new(State::default()));
    registry.add_hooks(
        "MessageFormatter",
        ClassHooks {
            clone_data: Some(|o: &Object| {
                let s = state(o);
                Rc::new(State { fmt: RefCell::new(s.fmt.borrow().clone()), err: s.err.clone() })
            }),
            ..Default::default()
        },
    );
}

pub static BUILTINS: &[Builtin] = &[
    b!("msgfmt_create", f_create),
    b!("msgfmt_format", f_format),
    b!("msgfmt_format_message", f_format_message),
    b!("msgfmt_parse", f_parse),
    b!("msgfmt_parse_message", f_parse_message),
    b!("msgfmt_set_pattern", f_set_pattern),
    b!("msgfmt_get_pattern", f_get_pattern),
    b!("msgfmt_get_locale", f_get_locale),
    b!("msgfmt_get_error_code", f_get_error_code),
    b!("msgfmt_get_error_message", f_get_error_message),
];

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "MessageFormatter",
    parent: None,
    interfaces: &[],
    flags: p8_runtime::object::CLASS_NOT_SERIALIZABLE,
    properties: &[],
    methods: crate::methods! {
        "MessageFormatter";
        statics: [("create", create_static), ("formatMessage", format_message_static), ("parseMessage", parse_message_static)];
        "__construct" => m_construct;
        "format" => f_format;
        "parse" => f_parse;
        "setPattern" => f_set_pattern;
        "getPattern" => f_get_pattern;
        "getLocale" => f_get_locale;
        "getErrorCode" => f_get_error_code;
        "getErrorMessage" => f_get_error_message;
    },
    constants: crate::consts::MESSAGE_FORMATTER,
    debug_info: None,
}];

fn create_static(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::prep(ctx, "MessageFormatter::create", args)?;
    f_create(ctx, args)
}

fn format_message_static(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::prep(ctx, "MessageFormatter::formatMessage", args)?;
    f_format_message(ctx, args)
}

fn parse_message_static(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::prep(ctx, "MessageFormatter::parseMessage", args)?;
    f_parse_message(ctx, args)
}

/// msgfmt_ctor: the formatter, or Err(()) with the global error set.
fn make(ctx: &mut dyn Ctx, args: &[Value], err: Option<&IntlError>, func: &str) -> Result<Result<Fmt, ()>, Throw> {
    crate::reset_global(ctx);
    let mut locale = str_arg(args, 0);
    let pattern = crate::bytes_arg(args, 1);
    if locale.len() > crate::locale::MAX_LOCALE_LEN {
        crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("Locale string too long, should be no longer than {} characters", crate::locale::MAX_LOCALE_LEN))?;
        return Ok(Err(()));
    }
    let Ok(pattern) = String::from_utf8(pattern) else {
        crate::set_global_code(ctx, 10);
        crate::set_message(ctx, err, &format!("{func}: error converting pattern to UTF-16"))?;
        return Ok(Err(()));
    };
    if locale.is_empty() {
        locale = default_locale(ctx);
    }
    let fail = |ctx: &mut dyn Ctx, code: i64| -> Result<Result<Fmt, ()>, Throw> {
        if let Some(e) = err {
            e.code.set(code);
        }
        crate::set_global_code(ctx, code);
        crate::set_message(ctx, err, &format!("{func}: message formatter creation failed"))?;
        Ok(Err(()))
    };
    if pattern.is_empty() {
        return fail(ctx, U_ILLEGAL_ARGUMENT_ERROR);
    }
    match parse_message(&pattern) {
        Ok(nodes) => Ok(Ok(Fmt { locale, pattern, nodes })),
        Err(SyntaxError(BAD_TYPE)) => fail(ctx, U_ILLEGAL_ARGUMENT_ERROR),
        Err(SyntaxError(at)) => {
            if let Some(e) = err {
                e.code.set(U_PATTERN_SYNTAX_ERROR);
            }
            crate::set_global_code(ctx, U_PATTERN_SYNTAX_ERROR);
            crate::set_message(ctx, err, &format!("pattern syntax error ({})", parse_error_text(&pattern, at)))?;
            Ok(Err(()))
        }
    }
}

fn f_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let class = ctx.class(b"MessageFormatter").expect("MessageFormatter");
    let o = ctx.new_object(&class);
    let st = state(&o);
    match make(ctx, args, Some(&st.err), "msgfmt_create")? {
        Ok(f) => {
            *st.fmt.borrow_mut() = Some(f);
            Ok(Value::object(o))
        }
        Err(()) => Ok(Value::NULL),
    }
}

fn m_construct(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(this) = arg(args, 0).into_object() else { unreachable!() };
    let st = state(&this);
    match make(ctx, &args[1..], Some(&st.err), "msgfmt_create")? {
        Ok(f) => {
            *st.fmt.borrow_mut() = Some(f);
            Ok(Value::NULL)
        }
        Err(()) => {
            let msg = crate::global_message(ctx);
            Err(ctx.throw("IntlException", msg))
        }
    }
}

fn obj(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Rc<Object>, Throw> {
    let Ok(o) = arg(args, 0).into_object() else { unreachable!("checked against the stub") };
    if state(&o).fmt.borrow().is_none() {
        return Err(ctx.throw("Error", "Found unconstructed MessageFormatter".into()));
    }
    state(&o).err.reset();
    Ok(o)
}

/// The arguments by name, or the error (code, message) converting them gave.
type Converted = Result<Vec<(String, Arg)>, (i64, String)>;

/// umsg_format_helper: the arguments converted, or Err(code, message).
fn convert_args(ctx: &mut dyn Ctx, f: &Fmt, a: &PhpArray) -> Result<Converted, Throw> {
    let mut types = Vec::new();
    let named = uses_named(&f.nodes);
    collect_types(&f.nodes, named, &mut types);
    if types.iter().any(|(_, t)| *t == ArgType::Conflict) {
        return Ok(Err((crate::U_ARGUMENT_TYPE_MISMATCH, "Inconsistent types declared for an argument".into())));
    }
    let mut out = Vec::new();
    for (k, v) in a.iter() {
        let v = v.deref();
        let key = match &k {
            Key::Int(i) => {
                if *i < 0 || *i > i32::MAX as i64 {
                    return Ok(Err((U_ILLEGAL_ARGUMENT_ERROR, "Found negative or too large array key".into())));
                }
                i.to_string()
            }
            Key::Str(s) => lossy(s.as_bytes()),
        };
        let ty = types.iter().find(|(n, _)| *n == key).map(|(_, t)| *t).unwrap_or(ArgType::Any);
        let as_string = |ctx: &mut dyn Ctx, v: &Value| -> Result<String, Throw> {
            Ok(if v.is_object() {
                let r = ctx.call(&Value::array(PhpArray::from_list(vec![v.clone(), s("__toString")])), &mut [])?;
                r.to_php_string().map(|s| lossy(s.as_bytes())).unwrap_or_default()
            } else {
                v.to_php_string().map(|s| lossy(s.as_bytes())).unwrap_or_default()
            })
        };
        let conv = match ty {
            ArgType::Str => Arg::Str(as_string(ctx, &v)?),
            ArgType::Double => Arg::Double(v.to_float()),
            ArgType::Int64 => Arg::Int(match v.as_float() {
                Some(f) => f as i64,
                None => v.to_int(),
            }),
            ArgType::Date => match date_ms(ctx, &v)? {
                Some(ms) => Arg::Date(ms),
                None if v.is_object() => {
                    return Ok(Err((U_ILLEGAL_ARGUMENT_ERROR, "msgfmt_format: invalid object type for date/time (only IntlCalendar and DateTimeInterface permitted)".into())));
                }
                None => return Ok(Err((U_ILLEGAL_ARGUMENT_ERROR, format!("The argument for key '{key}' cannot be used as a date or time")))),
            },
            ArgType::Conflict => unreachable!("refused above"),
            ArgType::Any => match v.kind() {
                ValueKind::Object if v.as_object().expect("an object").class.flags & p8_runtime::object::CLASS_RESOURCE != 0 => {
                    return Ok(Err((U_ILLEGAL_ARGUMENT_ERROR, format!("No strategy to convert the value given for the argument with key '{key}' is available"))));
                }
                ValueKind::Float => Arg::Double(v.as_float().expect("a float")),
                ValueKind::Int => Arg::Int(v.as_int().expect("an int")),
                ValueKind::Null => Arg::Int(0),
                ValueKind::Bool => Arg::Int(i64::from(v.as_bool().expect("a bool"))),
                ValueKind::Str | ValueKind::Object => {
                    if let Some(o) = v.as_object()
                        && is_datetime(o)
                    {
                        match date_ms(ctx, &v)? {
                            Some(ms) => Arg::Date(ms),
                            None => Arg::Str(String::new()),
                        }
                    } else {
                        Arg::Str(as_string(ctx, &v)?)
                    }
                }
                _ => return Ok(Err((U_ILLEGAL_ARGUMENT_ERROR, format!("No strategy to convert the value given for the argument with key '{key}' is available")))),
            },
        };
        out.push((key, conv));
    }
    Ok(Ok(out))
}

pub(crate) fn is_datetime(o: &Object) -> bool {
    let name = lossy(o.class.name.as_bytes()).to_ascii_lowercase();
    name == "datetime" || name == "datetimeimmutable" || o.class.parent.as_ref().is_some_and(|p| is_datetime_class(p))
}

fn is_datetime_class(c: &p8_runtime::object::Class) -> bool {
    let name = lossy(c.name.as_bytes()).to_ascii_lowercase();
    name == "datetime" || name == "datetimeimmutable" || c.parent.as_ref().is_some_and(|p| is_datetime_class(p))
}

/// intl_zval_to_millis: seconds (int, float, numeric string) or a DateTimeInterface.
fn date_ms(ctx: &mut dyn Ctx, v: &Value) -> Result<Option<f64>, Throw> {
    Ok(match v.kind() {
        ValueKind::Int => Some(v.as_int().expect("an int") as f64 * 1000.0),
        ValueKind::Float => Some(v.as_float().expect("a float") * 1000.0),
        ValueKind::Str => p8_runtime::numeric::numeric_string(v.as_str().expect("a string").as_bytes()).map(|n| match n {
            p8_runtime::numeric::Number::Int(i) => i as f64 * 1000.0,
            p8_runtime::numeric::Number::Float(f) => f * 1000.0,
        }),
        ValueKind::Object if is_datetime(v.as_object().expect("an object")) => {
            let mut a = [s("U.v")];
            let r = ctx.call(&Value::array(PhpArray::from_list(vec![v.clone(), s("format")])), &mut a)?;
            r.to_php_string().and_then(|s| lossy(s.as_bytes()).parse::<f64>().ok()).map(|f| (f * 1000.0).round())
        }
        _ => None,
    })
}

fn php_zone(ctx: &mut dyn Ctx) -> Zone {
    let name = ctx.call(&s("date_default_timezone_get"), &mut []).ok().and_then(|v| v.to_php_string()).map(|s| lossy(s.as_bytes())).unwrap_or_else(|| "UTC".into());
    datefmt::zone_from_id(&name).unwrap_or(Zone::Iana(chrono_tz::Tz::UTC))
}

fn do_format(ctx: &mut dyn Ctx, f: &Fmt, a: &PhpArray, err: Option<&IntlError>) -> R {
    let args = match convert_args(ctx, f, a)? {
        Ok(a) => a,
        Err((code, msg)) => {
            if let Some(e) = err {
                e.code.set(code);
            }
            crate::set_global_code(ctx, code);
            crate::set_message(ctx, err, &msg)?;
            return Ok(Value::FALSE);
        }
    };
    let cx = Ctx2 { locale: &f.locale, zone: php_zone(ctx), args: &args };
    let mut out = String::new();
    match format_nodes(&f.nodes, &cx, None, &mut out) {
        Ok(()) => Ok(s(out)),
        Err(code) => {
            if let Some(e) = err {
                e.code.set(code);
            }
            crate::set_global_code(ctx, code);
            crate::set_message(ctx, err, "Call to ICU MessageFormat::format() has failed")?;
            Ok(Value::FALSE)
        }
    }
}

fn f_format(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let f = st.fmt.borrow().clone().expect("constructed");
    let Ok(a) = arg(args, 1).into_array() else { return Ok(Value::FALSE) };
    let r = do_format(ctx, &f, &a, Some(&st.err))?;
    if st.err.code.get() == U_ZERO_ERROR {
        crate::set_global_code(ctx, U_ZERO_ERROR);
    }
    Ok(r)
}

fn f_format_message(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let f = match make(ctx, args, None, "msgfmt_format_message")? {
        Ok(f) => f,
        Err(()) => return Ok(Value::FALSE),
    };
    let Ok(a) = arg(args, 2).into_array() else { return Ok(Value::FALSE) };
    do_format(ctx, &f, &a, None)
}

/// parse(): plain arguments and numbers between literal text.
fn do_parse(f: &Fmt, text: &str) -> Result<Vec<Value>, i64> {
    let mut out: Vec<(usize, Value)> = Vec::new();
    let mut i = 0;
    let nodes = &f.nodes;
    for (k, n) in nodes.iter().enumerate() {
        match n {
            Node::Text(t) => {
                let rest = text.get(i..).ok_or(crate::U_PARSE_ERROR)?;
                if !rest.starts_with(t.as_str()) {
                    return Err(crate::U_PARSE_ERROR);
                }
                i += t.len();
            }
            Node::Arg(name, kind) => {
                let idx: usize = name.parse().map_err(|_| U_UNSUPPORTED_ERROR)?;
                let rest = text.get(i..).ok_or(crate::U_PARSE_ERROR)?;
                let until = match nodes.get(k + 1) {
                    Some(Node::Text(t)) => rest.find(t.as_str()).ok_or(crate::U_PARSE_ERROR)?,
                    None => rest.len(),
                    _ => return Err(U_UNSUPPORTED_ERROR),
                };
                let piece = &rest[..until];
                let v = match kind {
                    Kind::None => s(piece),
                    Kind::Simple(ty, style) if ty == "number" => {
                        let nf = match style.as_deref().map(str::trim) {
                            None => NumFmt::new(&f.locale, numfmt::STYLE_DECIMAL, None),
                            Some("integer") => NumFmt::new(&f.locale, numfmt::STYLE_DECIMAL, None).map(|mut x| {
                                x.props.parse_int_only = true;
                                x
                            }),
                            Some("percent") => NumFmt::new(&f.locale, numfmt::STYLE_PERCENT, None),
                            Some("currency") => NumFmt::new(&f.locale, numfmt::STYLE_CURRENCY, None),
                            Some(p) => NumFmt::new(&f.locale, numfmt::STYLE_PATTERN_DECIMAL, Some(p)),
                        }
                        .map_err(|(c, _)| c)?;
                        let (num, _, _) = nf.parse(piece, 0).ok_or(crate::U_PARSE_ERROR)?;
                        let fv = num.to_f64();
                        if fv.fract() == 0.0 && fv.abs() < 9.2e18 { Value::int(fv as i64) } else { Value::float(fv) }
                    }
                    _ => return Err(U_UNSUPPORTED_ERROR),
                };
                i += until;
                out.push((idx, v));
            }
            Node::Pound => return Err(U_UNSUPPORTED_ERROR),
        }
    }
    out.sort_by_key(|(k, _)| *k);
    Ok(out.into_iter().map(|(_, v)| v).collect())
}

fn parse_result(ctx: &mut dyn Ctx, r: Result<Vec<Value>, i64>, err: Option<&IntlError>) -> R {
    match r {
        Ok(vals) => Ok(Value::array(PhpArray::from_list(vals))),
        Err(code) => {
            if let Some(e) = err {
                e.code.set(code);
            }
            crate::set_global_code(ctx, code);
            crate::set_message(ctx, err, "Parsing failed")?;
            Ok(Value::FALSE)
        }
    }
}

fn f_parse(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let f = st.fmt.borrow().clone().expect("constructed");
    let text = str_arg(args, 1);
    parse_result(ctx, do_parse(&f, &text), Some(&st.err))
}

fn f_parse_message(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let f = match make(ctx, args, None, "msgfmt_parse_message")? {
        Ok(f) => f,
        Err(()) => return Ok(Value::FALSE),
    };
    let text = str_arg(args, 2);
    parse_result(ctx, do_parse(&f, &text), None)
}

fn f_set_pattern(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let pattern = str_arg(args, 1);
    match parse_message(&pattern) {
        Ok(nodes) => {
            let mut b = st.fmt.borrow_mut();
            let f = b.as_mut().expect("constructed");
            f.pattern = pattern;
            f.nodes = nodes;
            Ok(Value::TRUE)
        }
        Err(SyntaxError(at)) => {
            st.err.code.set(U_PATTERN_SYNTAX_ERROR);
            crate::set_global_code(ctx, U_PATTERN_SYNTAX_ERROR);
            crate::set_message(ctx, Some(&st.err), &format!("Error setting symbol value at line 0, offset {at}"))?;
            Ok(Value::FALSE)
        }
    }
}

fn f_get_pattern(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(s(state(&o).fmt.borrow().as_ref().expect("constructed").pattern.clone()))
}

fn f_get_locale(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(s(state(&o).fmt.borrow().as_ref().expect("constructed").locale.clone()))
}

fn f_get_error_code(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(o) = arg(args, 0).into_object() else { unreachable!() };
    Ok(Value::int(state(&o).err.code.get()))
}

fn f_get_error_message(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(o) = arg(args, 0).into_object() else { unreachable!() };
    Ok(s(state(&o).err.message()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(locale: &str, pattern: &str, args: Vec<(&str, Arg)>) -> String {
        let nodes = parse_message(pattern).unwrap();
        let args: Vec<(String, Arg)> = args.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let cx = Ctx2 { locale, zone: Zone::Iana(chrono_tz::Tz::UTC), args: &args };
        let mut out = String::new();
        format_nodes(&nodes, &cx, None, &mut out).unwrap();
        out
    }

    #[test]
    fn formats_like_icu() {
        assert_eq!(fmt("en", "Hello {0}!", vec![("0", Arg::Str("World".into()))]), "Hello World!");
        assert_eq!(fmt("en", "{n, plural, =0{none} one{# item} other{# items}}", vec![("n", Arg::Double(1.0))]), "1 item");
        assert_eq!(fmt("en", "{n, plural, =0{none} one{# item} other{# items}}", vec![("n", Arg::Double(1234.0))]), "1,234 items");
        assert_eq!(fmt("en", "{n, plural, =0{none} one{# item} other{# items}}", vec![("n", Arg::Double(0.0))]), "none");
        assert_eq!(fmt("en", "{g, select, female{she} other{they}}", vec![("g", Arg::Str("female".into()))]), "she");
        assert_eq!(fmt("en", "It''s '{'quoted'}' {0}", vec![("0", Arg::Str("x".into()))]), "It's {quoted} x");
        assert_eq!(fmt("en", "{0, number, integer}", vec![("0", Arg::Int(1234))]), "1,234");
        assert_eq!(fmt("en", "missing {1}", vec![]), "missing {1}");
        assert_eq!(fmt("en", "{n, selectordinal, one{#st} two{#nd} few{#rd} other{#th}}", vec![("n", Arg::Double(22.0))]), "22nd");
    }
}

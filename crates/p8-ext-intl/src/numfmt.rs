//! NumberFormatter and numfmt_* (formatter/*.c) as ICU's DecimalFormat
//! behaves: a pattern (`#,##0.00 ¤`) parsed into properties that the
//! attributes then change, symbols from CLDR (ICU4X's decimal, percent and
//! currency data), exact decimal rounding, strict parsing.
//!
//! Not available (creation fails with U_UNSUPPORTED_ERROR, as ICU does for a
//! style it lacks): SPELLOUT, ORDINAL, DURATION and PATTERN_RULEBASED, which
//! need ICU's rule-based number format data (RBNF), not in ICU4X.
//!
//! Symbols CLDR has but ICU4X does not carry (NaN, infinity, the exponent
//! and per-mille signs) are CLDR's root values.

use crate::decimal::{self, Dec};
use crate::{
    IntlError, R, U_ILLEGAL_ARGUMENT_ERROR, U_UNSUPPORTED_ERROR, U_ZERO_ERROR, arg, b, default_locale, lossy, prep, s, set_error, str_arg,
};
use icu_experimental::dimension::provider::currency::essentials::CurrencyEssentialsV1;
use icu_experimental::dimension::provider::currency::fractions::CurrencyFractionsV1;
use icu_experimental::dimension::provider::currency::symbols::{CurrencyDecimalSymbolsV1, CurrencySymbolsV1};
use icu_experimental::dimension::provider::percent::DecimalPercentV1;
use icu_locale_core::preferences::extensions::unicode::keywords::CurrencyType;
use icu_pattern::{DoublePlaceholderKey, PatternItem};
use icu_provider::prelude::*;
use p8_runtime::ext::{Builtin, ClassHooks, ClassSpec, Ctx, Registry, Throw};
use p8_runtime::object::Object;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub const STYLE_PATTERN_DECIMAL: i64 = 0;
pub const STYLE_DECIMAL: i64 = 1;
pub const STYLE_CURRENCY: i64 = 2;
pub const STYLE_PERCENT: i64 = 3;
pub const STYLE_SCIENTIFIC: i64 = 4;
pub const STYLE_CURRENCY_ISO: i64 = 10;
pub const STYLE_CURRENCY_PLURAL: i64 = 11;
pub const STYLE_CURRENCY_ACCOUNTING: i64 = 12;
pub const STYLE_CASH_CURRENCY: i64 = 13;
pub const STYLE_CURRENCY_STANDARD: i64 = 16;

const TYPE_DEFAULT: i64 = 0;
const TYPE_INT32: i64 = 1;
const TYPE_INT64: i64 = 2;
const TYPE_DOUBLE: i64 = 3;
const TYPE_CURRENCY: i64 = 4;

const UNLIMITED: i32 = 2_000_000_000;

// ---- symbols ----

const SYM_DECIMAL: usize = 0;
const SYM_GROUPING: usize = 1;
const SYM_PERCENT: usize = 3;
const SYM_ZERO: usize = 4;
const SYM_MINUS: usize = 6;
const SYM_PLUS: usize = 7;
const SYM_CURRENCY: usize = 8;
const SYM_INTL_CURRENCY: usize = 9;
const SYM_MONETARY: usize = 10;
const SYM_EXPONENTIAL: usize = 11;
const SYM_PERMILL: usize = 12;
const SYM_INFINITY: usize = 14;
const SYM_NAN: usize = 15;
const SYM_MONETARY_GROUPING: usize = 17;
const SYMBOL_COUNT: i64 = 18;

/// DecimalFormatSymbols: UNUM_*_SYMBOL by index, and the digits.
#[derive(Clone, Debug)]
pub struct Symbols {
    pub s: Vec<String>,
    pub digits: [char; 10],
}

impl Symbols {
    /// CLDR's symbols for an ICU locale ID (its `numbers` keyword honored).
    pub fn for_locale(id: &str) -> Symbols {
        let loc = crate::icu4x_locale(id);
        let dl = DataLocale::from(&loc);
        let nu = crate::uloc::parts(id).keywords.into_iter().find(|(k, _)| k == "numbers").map(|(_, v)| v.to_ascii_lowercase());
        let mut s: Vec<String> = [".", ",", ";", "%", "0", "#", "-", "+", "¤", "XXX", ".", "E", "‰", "*", "∞", "NaN", "@", ","].iter().map(|x| x.to_string()).collect();
        let mut numsys = "latn".to_string();
        let load = |attrs: Option<&str>| -> Option<DataResponse<icu_decimal::provider::DecimalSymbolsV1>> {
            let id = match attrs {
                Some(a) => DataIdentifierBorrowed::for_marker_attributes_and_locale(DataMarkerAttributes::try_from_str(a).ok()?, &dl),
                None => DataIdentifierBorrowed::for_locale(&dl),
            };
            DataProvider::<icu_decimal::provider::DecimalSymbolsV1>::load(&icu_decimal::provider::Baked, DataRequest { id, metadata: crate::silent() }).ok()
        };
        let resp = nu.as_deref().and_then(|n| load(Some(n)).filter(|r| r.payload.get().numsys() == n)).or_else(|| load(None));
        if let Some(r) = resp {
            let d = r.payload.get();
            s[SYM_DECIMAL] = d.decimal_separator().to_string();
            s[SYM_GROUPING] = d.grouping_separator().to_string();
            s[SYM_MONETARY] = s[SYM_DECIMAL].clone();
            s[SYM_MONETARY_GROUPING] = s[SYM_GROUPING].clone();
            s[SYM_MINUS] = d.minus_sign_prefix().to_string();
            s[SYM_PLUS] = d.plus_sign_prefix().to_string();
            numsys = d.numsys().to_string();
        }
        let wanted = nu.unwrap_or_else(|| numsys.clone());
        let mut digits = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];
        if let Ok(a) = DataMarkerAttributes::try_from_str(&wanted)
            && let Ok(r) = DataProvider::<icu_decimal::provider::DecimalDigitsV1>::load(
                &icu_decimal::provider::Baked,
                DataRequest { id: DataIdentifierBorrowed::for_marker_attributes(a), metadata: crate::silent() },
            )
        {
            digits = *r.payload.get();
        }
        s[SYM_ZERO] = digits[0].to_string();
        if let Ok(r) = DataProvider::<DecimalPercentV1>::load(&icu_experimental::provider::Baked, DataRequest { id: DataIdentifierBorrowed::for_locale(&dl), metadata: crate::silent() }) {
            let sign: String = r.payload.get().unsigned_pattern.iter().filter_map(|i| if let PatternItem::Literal(l) = i { Some(l.to_string()) } else { None }).collect();
            let sign = sign.trim_matches(|c: char| c.is_whitespace() || c == '\u{a0}' || c == '\u{202f}').to_string();
            if !sign.is_empty() {
                s[SYM_PERCENT] = sign;
            }
        }
        Symbols { s, digits }
    }

    /// The monetary separators CLDR gives `currency` in this locale (de_AT's '.' grouping, fr_CH's '.' decimal).
    fn apply_currency(&mut self, id: &str, currency: &str) {
        let Ok(ct) = CurrencyType::try_from_str(&currency.to_ascii_lowercase()) else { return };
        let dl = DataLocale::from(&crate::icu4x_locale(id));
        let mut buf = Default::default();
        let attrs = CurrencyDecimalSymbolsV1::make_attributes(None, ct, &mut buf);
        let req = DataRequest { id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attrs, &dl), metadata: crate::silent() };
        if let Ok(Some(r)) = DataProvider::<CurrencyDecimalSymbolsV1>::load(&icu_experimental::provider::Baked, req).allow_identifier_not_found() {
            let d = r.payload.get();
            self.s[SYM_MONETARY] = d.decimal_separator().to_string();
            self.s[SYM_MONETARY_GROUPING] = d.grouping_separator().to_string();
        }
    }
}

/// The grouping sizes CLDR gives the locale's decimal pattern.
fn grouping_sizes(id: &str) -> (i32, i32) {
    let dl = DataLocale::from(&crate::icu4x_locale(id));
    match DataProvider::<icu_decimal::provider::DecimalSymbolsV1>::load(&icu_decimal::provider::Baked, DataRequest { id: DataIdentifierBorrowed::for_locale(&dl), metadata: crate::silent() }) {
        Ok(r) => {
            let g = r.payload.get().grouping_sizes;
            (g.primary as i32, if g.secondary == 0 { g.primary as i32 } else { g.secondary as i32 })
        }
        Err(_) => (3, 3),
    }
}

/// A currency's symbol in a locale (CLDR), None when CLDR has none (the ISO code is used).
pub fn currency_symbol(id: &str, code: &str) -> Option<String> {
    let ct = CurrencyType::try_from_str(&code.to_ascii_lowercase()).ok()?;
    let dl = DataLocale::from(&crate::icu4x_locale(id));
    let mut buf = Default::default();
    let attrs = CurrencySymbolsV1::make_attributes(ct, CurrencySymbolsV1::SHORT, &mut buf);
    let req = DataRequest { id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attrs, &dl), metadata: crate::silent() };
    let r = DataProvider::<CurrencySymbolsV1>::load(&icu_experimental::provider::Baked, req).allow_identifier_not_found().ok()??;
    let sym = r.payload.get().as_str().to_string();
    if sym.is_empty() { None } else { Some(sym) }
}

/// A currency's fraction digits (CLDR's supplemental data).
pub fn currency_digits(code: &str) -> i32 {
    let Ok(ct) = CurrencyType::try_from_str(&code.to_ascii_lowercase()) else { return 2 };
    match DataProvider::<CurrencyFractionsV1>::load(&icu_experimental::provider::Baked, Default::default()) {
        Ok(r) => r.payload.get().resolve(ct).digits as i32,
        Err(_) => 2,
    }
}

/// The locale's currency: its `currency` keyword, else its region's.
pub fn locale_currency(id: &str) -> String {
    let p = crate::uloc::parts(id);
    if let Some((_, v)) = p.keywords.iter().find(|(k, _)| k == "currency") {
        return v.to_ascii_uppercase();
    }
    let mut region = p.region.clone();
    if region.is_empty() && !p.lang.is_empty() {
        if let Ok(mut l) = icu_locale_core::LanguageIdentifier::try_from_str(&p.lang) {
            icu_locale::LocaleExpander::new_common().maximize(&mut l);
            region = l.region.map(|r| r.to_string()).unwrap_or_default();
        }
        // ICU uses no currency for a language-only locale
        region.clear();
    }
    crate::currency_tables::REGION_CURRENCY.iter().find(|(r, _)| *r == region).map_or("XXX".to_string(), |(_, c)| c.to_string())
}

// ---- patterns ----

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Lit(String),
    Percent,
    Permille,
    Minus,
    Plus,
    Currency(u8),
}

pub type Affix = Vec<Tok>;

#[derive(Clone, Debug)]
pub struct Props {
    pub pos_prefix: Affix,
    pub pos_suffix: Affix,
    pub neg: Option<(Affix, Affix)>,
    pub min_int: i32,
    pub max_int: i32,
    pub min_frac: i32,
    pub max_frac: i32,
    pub sig_used: bool,
    pub min_sig: i32,
    pub max_sig: i32,
    pub grouping_used: bool,
    pub g1: i32,
    pub g2: i32,
    pub decimal_always: bool,
    pub multiplier: i64,
    pub rounding_mode: i64,
    pub increment: f64,
    pub exp: Option<(i32, bool)>,
    pub width: i32,
    pub pad: String,
    pub pad_pos: i64,
    pub pad_in_pattern: bool,
    pub parse_int_only: bool,
    pub lenient: bool,
    /// Fraction digits set by an attribute (they then win over a currency's).
    pub frac_explicit: bool,
}

impl Default for Props {
    fn default() -> Props {
        Props {
            pos_prefix: Vec::new(),
            pos_suffix: Vec::new(),
            neg: None,
            min_int: 1,
            max_int: UNLIMITED,
            min_frac: 0,
            max_frac: 3,
            sig_used: false,
            min_sig: -1,
            max_sig: -1,
            grouping_used: true,
            g1: 3,
            g2: 0,
            decimal_always: false,
            multiplier: 1,
            rounding_mode: decimal::ROUND_HALFEVEN,
            increment: 0.0,
            exp: None,
            width: -1,
            pad: " ".into(),
            pad_pos: 0,
            pad_in_pattern: false,
            parse_int_only: false,
            lenient: false,
            frac_explicit: false,
        }
    }
}

impl Props {
    pub fn has_currency(&self) -> bool {
        let has = |a: &Affix| a.iter().any(|t| matches!(t, Tok::Currency(_)));
        has(&self.pos_prefix) || has(&self.pos_suffix) || self.neg.as_ref().is_some_and(|(p, s)| has(p) || has(s))
    }
}

/// A pattern's syntax error (U_PATTERN_SYNTAX_ERROR family) at a UTF-16 offset.
#[derive(Debug)]
pub struct PatternError {
    pub code: i64,
    pub offset: usize,
}

struct Reader<'a> {
    c: Vec<char>,
    i: usize,
    _s: &'a str,
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
}

fn syntax(r: &Reader) -> PatternError {
    PatternError { code: crate::U_PATTERN_SYNTAX_ERROR, offset: r.i }
}

fn read_affix(r: &mut Reader) -> Result<Affix, PatternError> {
    let mut out: Affix = Vec::new();
    let mut lit = String::new();
    let flush = |lit: &mut String, out: &mut Affix| {
        if !lit.is_empty() {
            out.push(Tok::Lit(std::mem::take(lit)));
        }
    };
    while let Some(c) = r.peek() {
        match c {
            '#' | '@' | ';' | '*' | '.' | ',' | '0'..='9' => break,
            '\'' => {
                r.i += 1;
                if r.peek() == Some('\'') {
                    lit.push('\'');
                    r.i += 1;
                    continue;
                }
                loop {
                    match r.peek() {
                        None => return Err(syntax(r)),
                        Some('\'') => {
                            r.i += 1;
                            if r.peek() == Some('\'') {
                                lit.push('\'');
                                r.i += 1;
                            } else {
                                break;
                            }
                        }
                        Some(x) => {
                            lit.push(x);
                            r.i += 1;
                        }
                    }
                }
            }
            '%' | '‰' | '-' | '+' | '¤' => {
                flush(&mut lit, &mut out);
                r.i += 1;
                out.push(match c {
                    '%' => Tok::Percent,
                    '‰' => Tok::Permille,
                    '-' => Tok::Minus,
                    '+' => Tok::Plus,
                    _ => {
                        let mut n = 1;
                        while r.peek() == Some('¤') && n < 5 {
                            n += 1;
                            r.i += 1;
                        }
                        Tok::Currency(n)
                    }
                });
            }
            _ => {
                lit.push(c);
                r.i += 1;
            }
        }
    }
    flush(&mut lit, &mut out);
    Ok(out)
}

/// `*x` padding at one of the four positions.
fn read_pad(r: &mut Reader, pos: i64, p: &mut Props) -> Result<(), PatternError> {
    if r.peek() != Some('*') {
        return Ok(());
    }
    if p.pad_in_pattern {
        return Err(PatternError { code: 65798, offset: r.i });
    }
    r.i += 1;
    let Some(c) = r.peek() else { return Err(syntax(r)) };
    r.i += 1;
    p.pad = if c == '\'' {
        let mut s = String::new();
        while let Some(x) = r.peek() {
            r.i += 1;
            if x == '\'' {
                break;
            }
            s.push(x);
        }
        s
    } else {
        c.to_string()
    };
    p.pad_pos = pos;
    p.pad_in_pattern = true;
    Ok(())
}

/// Parse an ICU decimal pattern (applyPattern).
pub fn parse_pattern(pattern: &str) -> Result<Props, PatternError> {
    let mut r = Reader { c: pattern.chars().collect(), i: 0, _s: pattern };
    let mut p = Props { pad: " ".into(), ..Props::default() };
    read_pad(&mut r, 0, &mut p)?;
    p.pos_prefix = read_affix(&mut r)?;
    read_pad(&mut r, 1, &mut p)?;
    // the number
    let mut int_hash = 0;
    let mut int_zero = 0;
    let mut sig_at = 0;
    let mut sig_hash = 0;
    let mut group_sizes: Vec<i32> = Vec::new();
    let mut since_comma: i32 = -1;
    let mut frac_zero = 0;
    let mut frac_hash = 0;
    let mut seen_dot = false;
    let mut inc_digits = String::new();
    let mut inc_nonzero = false;
    let start = r.i;
    while let Some(c) = r.peek() {
        match c {
            '#' if !seen_dot => {
                // ("# cannot follow 0 before decimal point"; after a run of
                // @ a # is an optional significant digit, `@@##`: ICU's
                // parseIntegerFormat.)
                if int_zero > 0 {
                    return Err(syntax(&r));
                }
                if sig_at > 0 {
                    sig_hash += 1;
                } else {
                    int_hash += 1;
                }
                if since_comma >= 0 {
                    since_comma += 1;
                }
            }
            '@' if !seen_dot => {
                if int_zero > 0 || sig_hash > 0 {
                    return Err(syntax(&r));
                }
                sig_at += 1;
                if since_comma >= 0 {
                    since_comma += 1;
                }
            }
            '0'..='9' if !seen_dot => {
                if sig_at > 0 {
                    return Err(syntax(&r));
                }
                int_zero += 1;
                inc_digits.push(c);
                if c != '0' {
                    inc_nonzero = true;
                }
                if since_comma >= 0 {
                    since_comma += 1;
                }
            }
            ',' if !seen_dot => {
                if since_comma >= 0 {
                    group_sizes.push(since_comma);
                }
                since_comma = 0;
            }
            '.' => {
                if seen_dot {
                    return Err(PatternError { code: 65793, offset: r.i });
                }
                seen_dot = true;
                inc_digits.push('.');
            }
            '0'..='9' if seen_dot => {
                if frac_hash > 0 {
                    return Err(syntax(&r));
                }
                frac_zero += 1;
                inc_digits.push(c);
                if c != '0' {
                    inc_nonzero = true;
                }
            }
            '#' if seen_dot => frac_hash += 1,
            ',' | '@' if seen_dot => return Err(syntax(&r)),
            _ => break,
        }
        r.i += 1;
    }
    if r.i == start {
        return Err(syntax(&r));
    }
    if since_comma == 0 {
        return Err(syntax(&r));
    }
    // exponent
    if r.peek() == Some('E') {
        r.i += 1;
        let mut sign = false;
        if r.peek() == Some('+') {
            sign = true;
            r.i += 1;
        }
        let mut n = 0;
        while r.peek() == Some('0') {
            n += 1;
            r.i += 1;
        }
        if n == 0 {
            return Err(PatternError { code: 65795, offset: r.i });
        }
        p.exp = Some((n, sign));
    }
    read_pad(&mut r, 2, &mut p)?;
    p.pos_suffix = read_affix(&mut r)?;
    read_pad(&mut r, 3, &mut p)?;
    if r.peek() == Some(';') {
        r.i += 1;
        let np = read_affix(&mut r)?;
        // the negative subpattern's number is skipped
        while let Some(c) = r.peek() {
            if matches!(c, '#' | '@' | ',' | '.' | '0'..='9' | 'E') {
                r.i += 1;
            } else {
                break;
            }
        }
        let ns = read_affix(&mut r)?;
        p.neg = Some((np, ns));
    }
    if r.peek().is_some() {
        // ICU: an unquoted special character where none may be
        return Err(PatternError { code: 65555, offset: 0 });
    }
    // grouping
    if since_comma > 0 {
        p.g1 = since_comma;
        p.g2 = group_sizes.last().copied().filter(|&g| g != since_comma).unwrap_or(0);
        p.grouping_used = true;
    } else {
        p.g1 = 0;
        p.g2 = 0;
        p.grouping_used = false;
    }
    if sig_at > 0 {
        p.sig_used = true;
        p.min_sig = sig_at;
        p.max_sig = sig_at + sig_hash;
        p.min_int = 1;
        p.min_frac = 0;
        p.max_frac = 0;
    } else {
        p.min_int = int_zero;
        p.min_frac = frac_zero;
        p.max_frac = frac_zero + frac_hash;
        if p.exp.is_some() {
            p.max_int = int_zero + int_hash;
        } else if int_zero == 0 && frac_zero + frac_hash > 0 {
            // "#.##": no required integer digit
            p.min_int = 0;
        } else if int_zero == 0 {
            // "#": ICU still shows (and writes back) one digit
            p.min_int = 1;
        }
    }
    if inc_nonzero {
        p.increment = inc_digits.trim_start_matches('.').parse::<f64>().unwrap_or(0.0);
        if !inc_digits.contains('.') {
            p.increment = inc_digits.parse::<f64>().unwrap_or(0.0);
        }
    }
    // percent and per-mille set the multiplier
    let all: Vec<&Tok> = p.pos_prefix.iter().chain(p.pos_suffix.iter()).collect();
    if all.iter().any(|t| **t == Tok::Percent) {
        p.multiplier = 100;
    } else if all.iter().any(|t| **t == Tok::Permille) {
        p.multiplier = 1000;
    }
    if p.pad_in_pattern {
        // the width the pattern spans, without the pad spec itself
        let digits = int_hash + int_zero + sig_at + sig_hash + frac_zero + frac_hash + group_sizes.len() as i32 + (since_comma > 0) as i32 + seen_dot as i32;
        let affix_len = |a: &Affix| -> i32 {
            a.iter()
                .map(|t| match t {
                    Tok::Lit(s) => s.chars().count() as i32,
                    Tok::Currency(n) => *n as i32,
                    _ => 1,
                })
                .sum()
        };
        p.width = digits + affix_len(&p.pos_prefix) + affix_len(&p.pos_suffix);
    }
    Ok(p)
}

fn affix_pattern(a: &Affix) -> String {
    let mut s = String::new();
    for t in a {
        match t {
            Tok::Lit(l) => {
                let needs_quote = l.chars().any(|c| matches!(c, '#' | '@' | ';' | '*' | '.' | ',' | '0'..='9' | '%' | '‰' | '-' | '+' | '¤' | '\''));
                if needs_quote {
                    s.push('\'');
                    s.push_str(&l.replace('\'', "''"));
                    s.push('\'');
                } else {
                    s.push_str(l);
                }
            }
            Tok::Percent => s.push('%'),
            Tok::Permille => s.push('‰'),
            Tok::Minus => s.push('-'),
            Tok::Plus => s.push('+'),
            Tok::Currency(n) => {
                for _ in 0..*n {
                    s.push('¤');
                }
            }
        }
    }
    s
}

/// toPattern: the pattern the properties stand for.
pub fn to_pattern(p: &Props, currency_digits: Option<i32>) -> String {
    let mut num;
    let (min_frac, max_frac) = match currency_digits {
        Some(d) if !p.frac_explicit => (d, d),
        _ => (p.min_frac, p.max_frac),
    };
    if p.sig_used && p.exp.is_none() {
        let min = p.min_sig.max(1);
        let max = p.max_sig.max(min);
        let body: String = "@".repeat(min as usize) + &"#".repeat((max - min) as usize);
        num = if p.grouping_used && p.g1 > 0 {
            let len = body.chars().count() as i32;
            let mut b = body.clone();
            if len <= p.g1 {
                b = "#".repeat((p.g1 + 1 - len) as usize) + &b;
            }
            insert_grouping(&b, p.g1, p.g2)
        } else {
            body
        };
    } else {
        let min_int = p.min_int.max(0);
        let int_len = if p.exp.is_some() {
            p.max_int.max(min_int).min(9)
        } else if p.grouping_used && p.g1 > 0 {
            let g = if p.g2 > 0 { p.g1 + p.g2 + 1 } else { p.g1 + 1 };
            g.max(min_int)
        } else {
            min_int.max(1)
        };
        let mut int: String = (0..int_len).map(|i| if i < int_len - min_int { '#' } else { '0' }).collect();
        if min_int == 0 && !p.grouping_used && p.exp.is_none() {
            int = "#".into();
        }
        num = if p.grouping_used && p.g1 > 0 && p.exp.is_none() { insert_grouping(&int, p.g1, p.g2) } else { int };
        let inc = if p.increment > 0.0 { Some(Dec::from_f64(p.increment)) } else { None };
        if let Some(inc) = inc {
            // the increment's digits replace the trailing zeros
            let low = inc.lowest().min(0);
            let frac_digits = (-low).max(min_frac);
            let int_part = inc.to_plain().split('.').next().unwrap_or("0").to_string();
            if int_part != "0" {
                let n = num.len();
                let t = int_part.len().min(n);
                num.replace_range(n - t.., &int_part[int_part.len() - t..]);
            }
            if frac_digits > 0 {
                num.push('.');
                for k in 1..=frac_digits {
                    num.push((b'0' + inc.digit_at(-k)) as char);
                }
                for _ in frac_digits..max_frac {
                    num.push('#');
                }
            }
        } else if (max_frac > 0 && max_frac < UNLIMITED) || p.decimal_always {
            num.push('.');
            num.push_str(&"0".repeat(min_frac as usize));
            num.push_str(&"#".repeat((max_frac - min_frac).max(0) as usize));
        }
        if let Some((n, sign)) = p.exp {
            num.push('E');
            if sign {
                num.push('+');
            }
            num.push_str(&"0".repeat(n as usize));
        }
    }
    let mut out = String::new();
    let pad = |pos: i64, out: &mut String| {
        if p.pad_in_pattern && p.pad_pos == pos {
            out.push('*');
            out.push_str(&p.pad);
        }
    };
    pad(0, &mut out);
    out.push_str(&affix_pattern(&p.pos_prefix));
    pad(1, &mut out);
    out.push_str(&num);
    pad(2, &mut out);
    out.push_str(&affix_pattern(&p.pos_suffix));
    pad(3, &mut out);
    if let Some((np, ns)) = &p.neg {
        // ICU omits a negative subpattern that is "-" + the positive one
        let mut implied = vec![Tok::Minus];
        implied.extend(p.pos_prefix.iter().cloned());
        if !(*np == implied && *ns == p.pos_suffix) {
            out.push(';');
            out.push_str(&affix_pattern(np));
            out.push_str(&num);
            out.push_str(&affix_pattern(ns));
        }
    }
    out
}

fn insert_grouping(int: &str, g1: i32, g2: i32) -> String {
    let chars: Vec<char> = int.chars().collect();
    let mut out: Vec<char> = Vec::new();
    let mut count = 0;
    let mut size = g1;
    for (i, c) in chars.iter().rev().enumerate() {
        if count == size && i < chars.len() && size > 0 {
            out.push(',');
            count = 0;
            size = if g2 > 0 { g2 } else { 0 };
        }
        out.push(*c);
        count += 1;
    }
    out.reverse();
    out.into_iter().collect()
}

// ---- the formatter ----

#[derive(Clone, Debug)]
pub struct Formatter {
    pub locale: String,
    pub props: Props,
    pub syms: Symbols,
    pub currency: String,
    pub currency_symbol_set: bool,
}

impl Formatter {
    /// unum_open.
    pub fn new(locale: &str, style: i64, pattern: Option<&str>) -> Result<Formatter, (i64, usize)> {
        let syms = Symbols::for_locale(locale);
        let currency = locale_currency(locale);
        let (g1, g2) = grouping_sizes(locale);
        let grouped = |frac: &str| -> String {
            let int = if g2 != g1 && g2 > 0 { format!("#,{},{}0", "#".repeat(g2 as usize), "#".repeat((g1 - 1).max(0) as usize)) } else { format!("#,{}0", "#".repeat((g1 - 1).max(0) as usize)) };
            if g1 == 0 { format!("0{frac}") } else { format!("{int}{frac}") }
        };
        let pat = match style {
            STYLE_PATTERN_DECIMAL => match pattern {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => "#".to_string(),
            },
            STYLE_DECIMAL => grouped(".###"),
            STYLE_PERCENT => percent_pattern(locale, &grouped("")),
            STYLE_SCIENTIFIC => "#E0".to_string(),
            STYLE_CURRENCY | STYLE_CURRENCY_ISO | STYLE_CURRENCY_PLURAL | STYLE_CASH_CURRENCY | STYLE_CURRENCY_STANDARD | STYLE_CURRENCY_ACCOUNTING => {
                let mut p = currency_pattern(locale, &grouped(".00"), style == STYLE_CURRENCY_ACCOUNTING, &currency);
                if style == STYLE_CURRENCY_ISO {
                    p = p.replace('¤', "¤¤");
                }
                p
            }
            _ => return Err((U_UNSUPPORTED_ERROR, 0)),
        };
        let mut props = parse_pattern(&pat).map_err(|e| (e.code, e.offset))?;
        if style == STYLE_PATTERN_DECIMAL && pattern.is_none_or(str::is_empty) {
            props.max_frac = UNLIMITED;
        }
        if style == STYLE_SCIENTIFIC {
            props.min_int = 1;
            props.max_int = 1;
            props.max_frac = UNLIMITED;
        }
        let mut f = Formatter { locale: locale.to_string(), props, syms, currency: String::new(), currency_symbol_set: false };
        f.set_currency(&currency);
        Ok(f)
    }

    pub fn set_currency(&mut self, code: &str) {
        self.currency = code.to_string();
        self.syms.s[SYM_INTL_CURRENCY] = code.to_string();
        self.syms.s[SYM_CURRENCY] = if code == "XXX" { "¤".to_string() } else { currency_symbol(&self.locale, code).unwrap_or_else(|| code.to_string()) };
        self.syms.apply_currency(&self.locale, code);
        self.currency_symbol_set = false;
    }

    fn currency_digits(&self) -> Option<i32> {
        if self.props.has_currency() {
            Some(if self.currency == "XXX" { 2 } else { currency_digits(&self.currency) })
        } else {
            None
        }
    }

    pub fn pattern(&self) -> String {
        to_pattern(&self.props, self.currency_digits())
    }

    /// Format a finite or special value.
    pub fn format(&self, v: Num) -> String {
        let p = &self.props;
        let (neg, mut dec) = match v {
            Num::Nan => return self.with_affixes(false, self.syms.s[SYM_NAN].clone()),
            Num::Inf(neg) => return self.with_affixes(neg, self.syms.s[SYM_INFINITY].clone()),
            Num::Dec(d) => (d.neg, d),
        };
        if p.multiplier != 1 && p.multiplier != 0 {
            dec.multiply(p.multiplier.unsigned_abs());
            if p.multiplier < 0 {
                dec.neg = !dec.neg;
            }
        }
        let neg = if p.multiplier < 0 { !neg } else { neg };
        let currency = self.props.has_currency();
        let (min_frac, max_frac) = match self.currency_digits() {
            Some(d) if !p.frac_explicit => (d, d),
            _ => (p.min_frac, p.max_frac),
        };
        let body = if let Some((exp_digits, exp_sign)) = p.exp {
            self.format_scientific(&mut dec, exp_digits, exp_sign, min_frac, max_frac)
        } else {
            if p.sig_used {
                let max = if p.max_sig < 1 { 6 } else { p.max_sig };
                if let Some(m) = dec.magnitude() {
                    dec.round_at(m - max + 1, p.rounding_mode);
                }
            } else if p.increment > 0.0 {
                dec.round_increment(&Dec::from_f64(p.increment), p.rounding_mode);
            } else {
                dec.round_at(-max_frac, p.rounding_mode);
            }
            dec.truncate_int(p.max_int);
            let (min_int, min_frac) = if p.sig_used {
                // the significant digits shown, at least min_sig
                let min_sig = p.min_sig.max(1);
                let mag = dec.magnitude().unwrap_or(0);
                let shown_frac = (min_sig - 1 - mag).max(-dec.lowest()).max(0);
                (1, shown_frac)
            } else {
                (p.min_int, min_frac)
            };
            self.digits_body(&dec, min_int, min_frac, currency)
        };
        self.with_affixes(neg, body)
    }

    fn sep(&self, currency: bool) -> (&str, &str) {
        if currency {
            (&self.syms.s[SYM_MONETARY], &self.syms.s[SYM_MONETARY_GROUPING])
        } else {
            (&self.syms.s[SYM_DECIMAL], &self.syms.s[SYM_GROUPING])
        }
    }

    fn digit(&self, d: u8) -> char {
        self.syms.digits[d as usize]
    }

    fn digits_body(&self, dec: &Dec, min_int: i32, min_frac: i32, currency: bool) -> String {
        let p = &self.props;
        let (dsep, gsep) = self.sep(currency);
        let int_len = dec.point.max(0).max(min_int);
        let mut s = String::new();
        let mut since = 0;
        let mut group = p.g1;
        let mut int_chars: Vec<String> = Vec::new();
        for pos in 0..int_len {
            if p.grouping_used && p.g1 > 0 && since == group && pos > 0 {
                int_chars.push(gsep.to_string());
                since = 0;
                if p.g2 > 0 {
                    group = p.g2;
                }
            }
            int_chars.push(self.digit(dec.digit_at(pos)).to_string());
            since += 1;
        }
        int_chars.reverse();
        for c in int_chars {
            s.push_str(&c);
        }
        let frac_len = (-dec.lowest()).max(0).max(min_frac);
        if frac_len > 0 || p.decimal_always {
            s.push_str(dsep);
            for k in 1..=frac_len {
                s.push(self.digit(dec.digit_at(-k)));
            }
        }
        if s.is_empty() {
            // nothing shown ("#" for zero): one zero digit
            s.push(self.digit(0));
        }
        s
    }

    fn format_scientific(&self, dec: &mut Dec, exp_digits: i32, exp_sign: bool, min_frac: i32, max_frac: i32) -> String {
        let p = &self.props;
        let engineering = p.max_int > p.min_int && p.max_int > 1;
        let min_int = if engineering { 1 } else { p.min_int.max(if p.max_int == 0 { 0 } else { 1 }) };
        let (min_sig, max_sig) = if p.sig_used {
            (p.min_sig.max(1), p.max_sig.max(1))
        } else if max_frac >= UNLIMITED {
            (1, i32::MAX)
        } else {
            (p.min_int.max(1) + min_frac, p.min_int.max(1) + max_frac)
        };
        let mut exponent = 0;
        if let Some(m) = dec.magnitude() {
            let compute = |m: i32| -> i32 {
                if engineering {
                    let g = p.max_int;
                    m.div_euclid(g) * g
                } else {
                    m - (min_int.max(1) - 1)
                }
            };
            exponent = compute(m);
            if max_sig != i32::MAX {
                let digits_before = m - exponent + 1;
                let keep = if engineering { max_sig.max(digits_before) } else { max_sig };
                dec.round_at(m - keep + 1, p.rounding_mode);
                if let Some(m2) = dec.magnitude()
                    && m2 != m
                {
                    exponent = compute(m2);
                }
            }
            dec.shift(-exponent);
        }
        let int_digits = if engineering { dec.point.max(1) } else { min_int.max(1) };
        let frac_min = (min_sig - int_digits).max(0);
        let mut body = String::new();
        for pos in (0..int_digits.max(dec.point)).rev() {
            body.push(self.digit(dec.digit_at(pos)));
        }
        let frac_len = (-dec.lowest()).max(0).max(frac_min);
        if frac_len > 0 || p.decimal_always {
            body.push_str(&self.syms.s[SYM_DECIMAL]);
            for k in 1..=frac_len {
                body.push(self.digit(dec.digit_at(-k)));
            }
        }
        body.push_str(&self.syms.s[SYM_EXPONENTIAL]);
        if exponent < 0 {
            body.push_str(&self.syms.s[SYM_MINUS]);
        } else if exp_sign {
            body.push_str(&self.syms.s[SYM_PLUS]);
        }
        let e = exponent.unsigned_abs().to_string();
        for _ in e.len()..exp_digits as usize {
            body.push(self.digit(0));
        }
        for c in e.bytes() {
            body.push(self.digit(c - b'0'));
        }
        body
    }

    fn render_affix(&self, a: &Affix) -> String {
        let mut s = String::new();
        for t in a {
            match t {
                Tok::Lit(l) => s.push_str(l),
                Tok::Percent => s.push_str(&self.syms.s[SYM_PERCENT]),
                Tok::Permille => s.push_str(&self.syms.s[SYM_PERMILL]),
                Tok::Minus => s.push_str(&self.syms.s[SYM_MINUS]),
                Tok::Plus => s.push_str(&self.syms.s[SYM_PLUS]),
                Tok::Currency(1) => s.push_str(&self.syms.s[SYM_CURRENCY]),
                Tok::Currency(_) => s.push_str(&self.syms.s[SYM_INTL_CURRENCY]),
            }
        }
        s
    }

    fn affixes(&self, neg: bool) -> (String, String) {
        let p = &self.props;
        if neg {
            match &p.neg {
                Some((np, ns)) => (self.render_affix(np), self.render_affix(ns)),
                None => (format!("{}{}", self.syms.s[SYM_MINUS], self.render_affix(&p.pos_prefix)), self.render_affix(&p.pos_suffix)),
            }
        } else {
            (self.render_affix(&p.pos_prefix), self.render_affix(&p.pos_suffix))
        }
    }

    fn with_affixes(&self, neg: bool, body: String) -> String {
        let (mut prefix, mut suffix) = self.affixes(neg);
        // currency spacing: a letter-like symbol next to a digit gets a no-break space
        let ends_currency = |a: &Affix, at_end: bool| -> bool {
            if at_end { matches!(a.last(), Some(Tok::Currency(_))) } else { matches!(a.first(), Some(Tok::Currency(_))) }
        };
        let (pa, sa) = match (neg, &self.props.neg) {
            (true, Some((np, ns))) => (np.clone(), ns.clone()),
            _ => (self.props.pos_prefix.clone(), self.props.pos_suffix.clone()),
        };
        let first_digit = body.chars().next().is_some_and(|c| c.is_numeric());
        let last_digit = body.chars().last().is_some_and(|c| c.is_numeric());
        if ends_currency(&pa, true) && first_digit && prefix.chars().last().is_some_and(needs_spacing) {
            prefix.push('\u{a0}');
        }
        if ends_currency(&sa, false) && last_digit && suffix.chars().next().is_some_and(needs_spacing) {
            suffix.insert(0, '\u{a0}');
        }
        let mut out = format!("{prefix}{body}{suffix}");
        let p = &self.props;
        if p.width > 0 {
            let len = out.chars().count() as i32;
            if len < p.width {
                let pad: String = p.pad.repeat((p.width - len) as usize);
                out = match p.pad_pos {
                    0 => format!("{pad}{out}"),
                    1 => format!("{prefix}{pad}{body}{suffix}"),
                    2 => format!("{prefix}{body}{pad}{suffix}"),
                    _ => format!("{out}{pad}"),
                };
            }
        }
        out
    }

    /// Parse `text` from `start` (a byte offset): the value and where parsing stopped.
    pub fn parse(&self, text: &str, start: usize) -> Option<(Num, usize, Option<String>)> {
        let t = text.get(start..)?;
        let p = &self.props;
        let currency = p.has_currency();
        let mut cur_found: Option<String> = None;
        // prefixes: try the negative one first, then the positive (longest match)
        let (pp, ps) = self.affixes(false);
        let (np, ns) = self.affixes(true);
        let mut candidates = vec![(true, np, ns), (false, pp, ps)];
        candidates.sort_by_key(|c| std::cmp::Reverse(c.1.len()));
        for (neg, prefix, suffix) in candidates {
            let mut i = 0;
            if currency {
                if let Some((len, code)) = self.match_currency_affix(t, &prefix, true) {
                    i = len;
                    cur_found = code;
                } else {
                    continue;
                }
            } else if let Some(rest) = t.strip_prefix(prefix.as_str()) {
                i = t.len() - rest.len();
            } else if !prefix.is_empty() {
                continue;
            }
            let Some((num, used)) = self.parse_number(&t[i..]) else { continue };
            i += used;
            // the suffix must follow
            if currency {
                let trimmed = suffix.trim_start_matches('\u{a0}');
                if !trimmed.is_empty() {
                    let rest = t[i..].trim_start_matches(['\u{a0}', ' ']);
                    match self.match_currency_affix(rest, trimmed, false) {
                        Some((len, code)) => {
                            i = t.len() - rest.len() + len;
                            if code.is_some() {
                                cur_found = code;
                            }
                        }
                        None => continue,
                    }
                }
            } else if !suffix.is_empty() {
                match t[i..].strip_prefix(suffix.as_str()) {
                    Some(rest) => i = t.len() - rest.len(),
                    None => continue,
                }
            }
            let mut num = num;
            if neg {
                num = match num {
                    Num::Dec(mut d) => {
                        d.neg = !d.neg;
                        Num::Dec(d)
                    }
                    Num::Inf(n) => Num::Inf(!n),
                    n => n,
                };
            }
            if p.multiplier > 1
                && let Num::Dec(d) = &num
            {
                let v = d.to_f64() / p.multiplier as f64;
                num = Num::Dec(Dec::from_f64(v));
            }
            return Some((num, start + i, cur_found.or_else(|| Some(self.currency.clone()))));
        }
        None
    }

    /// Match an affix holding the currency sign: its literal parts as they
    /// are, the sign as this formatter's symbol or any ISO code.
    fn match_currency_affix(&self, t: &str, rendered: &str, _prefix: bool) -> Option<(usize, Option<String>)> {
        let sym = &self.syms.s[SYM_CURRENCY];
        let rendered = rendered.trim_end_matches('\u{a0}');
        if let Some(at) = rendered.find(sym.as_str()) {
            let (before, after) = (&rendered[..at], &rendered[at + sym.len()..]);
            let rest = t.strip_prefix(before)?;
            // the symbol, or an ISO code
            let (len, code) = if let Some(r2) = rest.strip_prefix(sym.as_str()) {
                (rest.len() - r2.len(), Some(self.currency.clone()))
            } else {
                let code: String = rest.chars().take(3).collect();
                if code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase()) {
                    (3, Some(code))
                } else {
                    return None;
                }
            };
            let rest2 = rest[len..].trim_start_matches('\u{a0}');
            let rest3 = rest2.strip_prefix(after)?;
            return Some((t.len() - rest3.len(), code));
        }
        let rest = t.strip_prefix(rendered)?;
        Some((t.len() - rest.len(), None))
    }

    /// The number at the start of `t`: its value and the bytes it used.
    fn parse_number(&self, t: &str) -> Option<(Num, usize)> {
        let currency = self.props.has_currency();
        let (dsep, gsep) = self.sep(currency);
        if let Some(r) = t.strip_prefix(self.syms.s[SYM_INFINITY].as_str()) {
            return Some((Num::Inf(false), t.len() - r.len()));
        }
        if let Some(r) = t.strip_prefix(self.syms.s[SYM_NAN].as_str()) {
            return Some((Num::Nan, t.len() - r.len()));
        }
        let digit_of = |c: char| -> Option<u8> {
            if let Some(d) = c.to_digit(10) {
                return Some(d as u8);
            }
            self.syms.digits.iter().position(|&x| x == c).map(|d| d as u8)
        };
        let mut i = 0;
        let mut int = String::new();
        let mut frac = String::new();
        let mut groups: Vec<usize> = Vec::new();
        let mut cur_group = 0usize;
        let mut seen_group = false;
        let mut seen_dec = false;
        let mut last_ok = 0;
        let bytes_at = |i: usize| &t[i..];
        loop {
            let rest = bytes_at(i);
            let Some(c) = rest.chars().next() else { break };
            if let Some(d) = digit_of(c) {
                if seen_dec {
                    frac.push((b'0' + d) as char);
                } else {
                    int.push((b'0' + d) as char);
                    cur_group += 1;
                }
                i += c.len_utf8();
                last_ok = i;
                continue;
            }
            if !seen_dec && !self.props.parse_int_only && rest.starts_with(dsep) {
                if seen_group && cur_group != self.props.g1 as usize {
                    return None;
                }
                seen_dec = true;
                i += dsep.len();
                last_ok = i;
                continue;
            }
            let is_group = !seen_dec && self.props.grouping_used && (rest.starts_with(gsep) || c == ' ' || (gsep == "\u{a0}" || gsep == "\u{202f}") && (c == ' ' || c == '\u{a0}' || c == '\u{202f}'));
            if is_group && !int.is_empty() {
                let glen = if rest.starts_with(gsep) { gsep.len() } else { c.len_utf8() };
                let next = rest[glen..].chars().next();
                if next.and_then(digit_of).is_none() {
                    break;
                }
                if seen_group && cur_group != self.props.g1 as usize && cur_group != self.props.g2.max(0) as usize {
                    return None;
                }
                groups.push(cur_group);
                seen_group = true;
                cur_group = 0;
                i += glen;
                continue;
            }
            break;
        }
        if seen_group && !seen_dec && cur_group != self.props.g1 as usize {
            return None;
        }
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        i = last_ok;
        // exponent
        let mut exp = 0i32;
        let rest = &t[i..];
        let e = &self.syms.s[SYM_EXPONENTIAL];
        let rest_exp = if rest.len() >= e.len() && rest.is_char_boundary(e.len()) && rest[..e.len()].eq_ignore_ascii_case(e) { Some(&rest[e.len()..]) } else { None };
        if let Some(r) = rest_exp {
            let (sign, r2) = if let Some(r2) = r.strip_prefix(self.syms.s[SYM_MINUS].as_str()) {
                (-1, r2)
            } else if let Some(r2) = r.strip_prefix('-') {
                (-1, r2)
            } else if let Some(r2) = r.strip_prefix(self.syms.s[SYM_PLUS].as_str()) {
                (1, r2)
            } else {
                (1, r)
            };
            let ds: String = r2.chars().take_while(|&c| digit_of(c).is_some()).map(|c| (b'0' + digit_of(c).unwrap_or(0)) as char).collect();
            if !ds.is_empty() {
                exp = sign * ds.parse::<i32>().unwrap_or(0);
                let used: usize = r2.chars().take(ds.chars().count()).map(char::len_utf8).sum();
                i = t.len() - r2.len() + used;
            }
        }
        let text = format!("{}{}{}e{exp}", if int.is_empty() { "0" } else { &int }, if frac.is_empty() { "" } else { "." }, frac);
        let d = Dec::parse(&text)?;
        Some((Num::Dec(d), i))
    }
}

fn needs_spacing(c: char) -> bool {
    use icu_properties::props::GeneralCategory as G;
    let gc = icu_properties::CodePointMapData::<G>::new().get(c);
    !matches!(gc, G::MathSymbol | G::CurrencySymbol | G::ModifierSymbol | G::OtherSymbol | G::SpaceSeparator | G::LineSeparator | G::ParagraphSeparator)
}

fn percent_pattern(locale: &str, number: &str) -> String {
    let dl = DataLocale::from(&crate::icu4x_locale(locale));
    let Ok(r) = DataProvider::<DecimalPercentV1>::load(&icu_experimental::provider::Baked, DataRequest { id: DataIdentifierBorrowed::for_locale(&dl), metadata: crate::silent() }) else {
        return format!("{number}%");
    };
    let mut out = String::new();
    for item in r.payload.get().unsigned_pattern.iter() {
        match item {
            PatternItem::Placeholder(_) => out.push_str(number),
            PatternItem::Literal(l) => {
                for c in l.chars() {
                    if c == '%' || c.is_whitespace() || c == '\u{a0}' || c == '\u{202f}' {
                        out.push(c);
                    } else if Symbols::for_locale(locale).s[SYM_PERCENT].contains(c) {
                        out.push('%');
                    } else {
                        out.push_str(&quote(&c.to_string()));
                    }
                }
            }
        }
    }
    // the percent sign may be a multi-character symbol: keep one '%'
    while out.contains("%%") {
        out = out.replace("%%", "%");
    }
    out
}

fn quote(s: &str) -> String {
    if s.chars().any(|c| matches!(c, '#' | '@' | ';' | '*' | '.' | ',' | '0'..='9' | '%' | '‰' | '-' | '+' | '¤' | '\'')) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        s.to_string()
    }
}

fn currency_pattern(locale: &str, number: &str, accounting: bool, currency: &str) -> String {
    let dl = DataLocale::from(&crate::icu4x_locale(locale));
    let Ok(r) = DataProvider::<CurrencyEssentialsV1>::load(&icu_experimental::provider::Baked, DataRequest { id: DataIdentifierBorrowed::for_locale(&dl), metadata: crate::silent() }) else {
        return format!("¤{number}");
    };
    let e = r.payload.get();
    let sym = currency_symbol(locale, currency).unwrap_or_else(|| currency.to_string());
    let starts = sym.chars().next().is_some_and(char::is_alphabetic);
    let ends = sym.chars().last().is_some_and(char::is_alphabetic);
    let minus = Symbols::for_locale(locale).s[SYM_MINUS].clone();
    let render = |p: &icu_pattern::DoublePlaceholderPattern| -> String {
        let mut out = String::new();
        for item in p.iter() {
            match item {
                PatternItem::Placeholder(DoublePlaceholderKey::Place0) => out.push_str(number),
                PatternItem::Placeholder(DoublePlaceholderKey::Place1) => out.push('¤'),
                PatternItem::Literal(l) => {
                    let l = if minus.chars().count() > 1 { l.replace(minus.as_str(), "-") } else { l.to_string() };
                    for c in l.chars() {
                        match c {
                            '-' | '(' | ')' | ' ' | '\u{a0}' | '\u{200e}' | '\u{200f}' | '\u{61c}' => out.push(c),
                            _ => out.push_str(&quote(&c.to_string())),
                        }
                    }
                }
            }
        }
        out
    };
    let (pos, neg) = if accounting {
        (e.get_positive_accounting(starts, ends), e.get_negative_accounting(starts, ends))
    } else {
        (e.get_positive(starts, ends), e.get_negative(starts, ends))
    };
    let mut out = render(pos);
    if let Some(n) = neg {
        let n = render(n);
        out.push(';');
        out.push_str(&n);
    }
    out
}

/// A number to format.
#[derive(Clone, Debug)]
pub enum Num {
    Dec(Dec),
    Inf(bool),
    Nan,
}

impl Num {
    pub fn from_f64(v: f64) -> Num {
        if v.is_nan() {
            Num::Nan
        } else if v.is_infinite() {
            Num::Inf(v < 0.0)
        } else {
            Num::Dec(Dec::from_f64(v))
        }
    }

    pub fn to_f64(&self) -> f64 {
        match self {
            Num::Dec(d) => d.to_f64(),
            Num::Inf(n) => {
                if *n {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }
            }
            Num::Nan => f64::NAN,
        }
    }
}

// ---- the PHP class ----

#[derive(Default)]
pub struct State {
    pub fmt: RefCell<Option<Formatter>>,
    pub err: IntlError,
}

fn state(o: &Object) -> &State {
    o.native::<State>().expect("NumberFormatter state")
}

pub fn register(registry: &mut Registry) {
    registry.add_data("NumberFormatter", || Rc::new(State::default()));
    registry.add_hooks(
        "NumberFormatter",
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
    b!("numfmt_create", f_create),
    b!("numfmt_format", f_format),
    Builtin { name: "numfmt_parse", run: f_parse, by_ref: &[3] },
    b!("numfmt_format_currency", f_format_currency),
    Builtin { name: "numfmt_parse_currency", run: f_parse_currency, by_ref: &[2, 3] },
    b!("numfmt_set_attribute", f_set_attribute),
    b!("numfmt_get_attribute", f_get_attribute),
    b!("numfmt_set_text_attribute", f_set_text_attribute),
    b!("numfmt_get_text_attribute", f_get_text_attribute),
    b!("numfmt_set_symbol", f_set_symbol),
    b!("numfmt_get_symbol", f_get_symbol),
    b!("numfmt_set_pattern", f_set_pattern),
    b!("numfmt_get_pattern", f_get_pattern),
    b!("numfmt_get_locale", f_get_locale),
    b!("numfmt_get_error_code", f_get_error_code),
    b!("numfmt_get_error_message", f_get_error_message),
];

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "NumberFormatter",
    parent: None,
    interfaces: &[],
    flags: p8_runtime::object::CLASS_NOT_SERIALIZABLE,
    properties: &[],
    methods: crate::methods! {
        "NumberFormatter";
        statics: [("create", create_static)];
        "__construct" => m_construct;
        "format" => f_format;
        "parse" => f_parse;
        "formatCurrency" => f_format_currency;
        "parseCurrency" => f_parse_currency;
        "setAttribute" => f_set_attribute;
        "getAttribute" => f_get_attribute;
        "setTextAttribute" => f_set_text_attribute;
        "getTextAttribute" => f_get_text_attribute;
        "setSymbol" => f_set_symbol;
        "getSymbol" => f_get_symbol;
        "setPattern" => f_set_pattern;
        "getPattern" => f_get_pattern;
        "getLocale" => f_get_locale;
        "getErrorCode" => f_get_error_code;
        "getErrorMessage" => f_get_error_message;
    },
    constants: crate::consts::NUMBER_FORMATTER,
    debug_info: None,
}];

fn create_static(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    prep(ctx, "NumberFormatter::create", args)?;
    f_create(ctx, args)
}

/// Whether ICU knows the locale's language (uloc_getISO3Language is not empty).
pub fn known_language(locale: &str) -> bool {
    let lang = crate::uloc::subtags(locale).lang;
    if lang.is_empty() {
        return false;
    }
    crate::uloc_tables::LANG3.iter().any(|(three, two)| *two == lang || *three == lang) || crate::uloc_tables::AVAILABLE.iter().any(|a| a.split('_').next() == Some(lang.as_str()))
}

/// numfmt_ctor: Ok(formatter) or the error to report.
fn construct(ctx: &mut dyn Ctx, st: &State, args: &[Value], func: &str) -> Result<Result<(), ()>, Throw> {
    let mut locale = str_arg(args, 0);
    let style = arg(args, 1).to_int();
    let pattern = crate::opt_str(args, 2);
    if locale.len() > crate::locale::MAX_LOCALE_LEN {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("Locale string too long, should be no longer than {} characters", crate::locale::MAX_LOCALE_LEN))?;
        return Ok(Err(()));
    }
    if st.fmt.borrow().is_some() {
        return Err(ctx.throw("Error", "NumberFormatter object is already constructed".into()));
    }
    if locale.is_empty() {
        locale = default_locale(ctx);
    }
    if !known_language(&locale) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($locale) \"{locale}\" is invalid")));
    }
    let canonical = crate::uloc::canonicalize(&locale);
    let locale = if canonical.is_empty() { locale } else { canonical };
    match Formatter::new(&locale, style, pattern.as_deref()) {
        Ok(f) => {
            *st.fmt.borrow_mut() = Some(f);
            st.err.reset();
            crate::set_global_code(ctx, U_ZERO_ERROR);
            Ok(Ok(()))
        }
        Err((code, _)) => {
            st.err.set(code, None);
            crate::set_global_code(ctx, code);
            crate::set_message(ctx, Some(&st.err), "numfmt_create: number formatter creation failed")?;
            Ok(Err(()))
        }
    }
}

fn f_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let class = ctx.class(b"NumberFormatter").expect("NumberFormatter");
    let o = ctx.new_object(&class);
    let st = state(&o);
    match construct(ctx, st, args, "numfmt_create")? {
        Ok(()) => Ok(Value::object(o)),
        Err(()) => Ok(Value::NULL),
    }
}

fn m_construct(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(this) = arg(args, 0).into_object() else { unreachable!() };
    match construct(ctx, state(&this), &args[1..], "NumberFormatter::__construct") {
        Ok(Ok(())) => Ok(Value::NULL),
        Ok(Err(())) => Err(ctx.throw("IntlException", "Constructor failed".into())),
        Err(t) => Err(t),
    }
}

/// The formatter argument (procedural) or $this; Error when it is not constructed.
fn obj(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Rc<Object>, Throw> {
    let Ok(o) = arg(args, 0).into_object() else { unreachable!("checked against the stub") };
    if state(&o).fmt.borrow().is_none() {
        return Err(ctx.throw("Error", "Found unconstructed NumberFormatter".into()));
    }
    state(&o).err.reset();
    Ok(o)
}

/// INTL_METHOD_CHECK_STATUS: the global code follows the object's; a
/// failure gets `msg` and returns false.
fn check(ctx: &mut dyn Ctx, st: &State, msg: &str) -> Result<bool, Throw> {
    let code = st.err.code.get();
    crate::set_global_code(ctx, code);
    if code > U_ZERO_ERROR {
        crate::set_message(ctx, Some(&st.err), msg)?;
        return Ok(false);
    }
    Ok(true)
}

fn fail(ctx: &mut dyn Ctx, st: &State, code: i64, msg: &str) -> R {
    st.err.code.set(code);
    check(ctx, st, msg)?;
    Ok(Value::FALSE)
}

fn with<T>(o: &Object, f: impl FnOnce(&mut Formatter) -> T) -> T {
    let st = state(o);
    let mut b = st.fmt.borrow_mut();
    f(b.as_mut().expect("constructed"))
}

fn f_format(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let number = arg(args, 1);
    let mut ty = if args.len() > 2 { arg(args, 2).to_int() } else { TYPE_DEFAULT };
    if ty == TYPE_DEFAULT {
        ty = if number.is_int() { TYPE_INT64 } else { TYPE_DOUBLE };
    }
    let is_method = crate::CALLED.with(|c| c.get()).is_some();
    let num = match ty {
        TYPE_INT32 => Num::Dec(Dec::from_i64(number.to_int() as i32 as i64)),
        TYPE_INT64 => Num::Dec(Dec::from_i64(match number.as_float() {
            Some(f) => f as i64,
            None => number.to_int(),
        })),
        TYPE_DOUBLE => Num::from_f64(number.to_float()),
        TYPE_CURRENCY => {
            let msg = if is_method {
                "NumberFormatter::format(): Argument #2 ($type) cannot be NumberFormatter::TYPE_CURRENCY constant, use NumberFormatter::formatCurrency() method instead"
            } else {
                "numfmt_format(): Argument #3 ($type) cannot be NumberFormatter::TYPE_CURRENCY constant, use numfmt_format_currency() function instead"
            };
            return Err(ctx.throw("ValueError", msg.into()));
        }
        _ => {
            let msg = if is_method { "NumberFormatter::format(): Argument #2 ($type) must be a NumberFormatter::TYPE_* constant" } else { "numfmt_format(): Argument #3 ($type) must be a NumberFormatter::TYPE_* constant" };
            return Err(ctx.throw("ValueError", msg.into()));
        }
    };
    let out = with(&o, |f| f.format(num));
    check(ctx, st, "Number formatting failed")?;
    Ok(s(out))
}

fn f_format_currency(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let number = arg(args, 1).to_float();
    let code: String = str_arg(args, 2).chars().take(3).collect::<String>().to_ascii_uppercase();
    let out = with(&o, |f| {
        let mut g = f.clone();
        if code.is_empty() {
            g.set_currency("XXX");
        } else if code != f.currency || !f.currency_symbol_set {
            g.set_currency(&code);
        }
        if !g.props.has_currency() {
            // a pattern without the sign: ICU still formats with the currency's digits
            g.props.pos_prefix.insert(0, Tok::Currency(1));
        }
        g.format(Num::from_f64(number))
    });
    check(ctx, st, "Number formatting failed")?;
    Ok(s(out))
}

fn set_position(args: &mut [Value], i: usize, v: Value) {
    if let Some(cell) = args.get(i).and_then(Value::as_reference) {
        *cell.borrow_mut() = v;
    }
}

fn f_parse(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let text = lossy(&crate::bytes_arg(args, 1));
    let ty = if args.len() > 2 { arg(args, 2).to_int() } else { TYPE_DOUBLE };
    let has_pos = args.len() > 3;
    let start = if has_pos { arg(args, 3).to_int().max(0) as usize } else { 0 };
    let is_method = crate::CALLED.with(|c| c.get()).is_some();
    match ty {
        TYPE_INT32 | TYPE_INT64 | TYPE_DOUBLE => {}
        TYPE_CURRENCY => {
            let msg = if is_method {
                "NumberFormatter::parse(): Argument #2 ($type) cannot be NumberFormatter::TYPE_CURRENCY constant, use NumberFormatter::parseCurrency() method instead"
            } else {
                "numfmt_parse(): Argument #3 ($type) cannot be NumberFormatter::TYPE_CURRENCY constant, use numfmt_parse_currency() function instead"
            };
            return Err(ctx.throw("ValueError", msg.into()));
        }
        _ => {
            let msg = if is_method { "NumberFormatter::parse(): Argument #2 ($type) must be a NumberFormatter::TYPE_* constant" } else { "numfmt_parse(): Argument #3 ($type) must be a NumberFormatter::TYPE_* constant" };
            return Err(ctx.throw("ValueError", msg.into()));
        }
    }
    let parsed = with(&o, |f| f.parse(&text, start.min(text.len())));
    let (value, pos) = match parsed {
        Some((n, end, _)) => {
            let v = n.to_f64();
            let value = match ty {
                TYPE_INT32 => Value::int(clamp_i32(v)),
                TYPE_INT64 => match n {
                    Num::Dec(d) => {
                        let mut t = d.clone();
                        t.round_at(0, decimal::ROUND_DOWN);
                        match t.to_i64() {
                            Some(i) => Value::int(i),
                            None => Value::int(if d.neg { i64::MIN } else { i64::MAX }),
                        }
                    }
                    _ => Value::int(0),
                },
                _ => Value::float(v),
            };
            (value, end)
        }
        None => {
            st.err.code.set(crate::U_PARSE_ERROR);
            (Value::FALSE, start)
        }
    };
    if has_pos {
        set_position(args, 3, Value::int(pos as i64));
    }
    if !check(ctx, st, "Number parsing failed")? {
        return Ok(Value::FALSE);
    }
    Ok(value)
}

fn clamp_i32(v: f64) -> i64 {
    if v.is_nan() {
        0
    } else {
        v.trunc().clamp(i32::MIN as f64, i32::MAX as f64) as i64
    }
}

fn f_parse_currency(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let text = lossy(&crate::bytes_arg(args, 1));
    let has_pos = args.len() > 3 && !arg(args, 3).is_null();
    let start = if has_pos { arg(args, 3).to_int().max(0) as usize } else { 0 };
    let parsed = with(&o, |f| {
        let mut g = f.clone();
        if !g.props.has_currency() {
            g.props.pos_prefix.insert(0, Tok::Currency(1));
        }
        g.parse(&text, start.min(text.len()))
    });
    match parsed {
        Some((n, end, cur)) => {
            if has_pos {
                set_position(args, 3, Value::int(end as i64));
            }
            set_position(args, 2, s(cur.unwrap_or_default()));
            check(ctx, st, "Number parsing failed")?;
            Ok(Value::float(n.to_f64()))
        }
        None => {
            if has_pos {
                set_position(args, 3, Value::int(start as i64));
            }
            fail(ctx, st, crate::U_PARSE_ERROR, "Number parsing failed")
        }
    }
}

const A_PARSE_INT_ONLY: i64 = 0;
const A_GROUPING_USED: i64 = 1;
const A_DECIMAL_ALWAYS_SHOWN: i64 = 2;
const A_MAX_INTEGER_DIGITS: i64 = 3;
const A_MIN_INTEGER_DIGITS: i64 = 4;
const A_INTEGER_DIGITS: i64 = 5;
const A_MAX_FRACTION_DIGITS: i64 = 6;
const A_MIN_FRACTION_DIGITS: i64 = 7;
const A_FRACTION_DIGITS: i64 = 8;
const A_MULTIPLIER: i64 = 9;
const A_GROUPING_SIZE: i64 = 10;
const A_ROUNDING_MODE: i64 = 11;
const A_ROUNDING_INCREMENT: i64 = 12;
const A_FORMAT_WIDTH: i64 = 13;
const A_PADDING_POSITION: i64 = 14;
const A_SECONDARY_GROUPING_SIZE: i64 = 15;
const A_SIGNIFICANT_DIGITS_USED: i64 = 16;
const A_MIN_SIGNIFICANT_DIGITS: i64 = 17;
const A_MAX_SIGNIFICANT_DIGITS: i64 = 18;
const A_LENIENT_PARSE: i64 = 19;

fn f_get_attribute(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let a = arg(args, 1).to_int();
    let v: Option<Value> = with(&o, |f| {
        let p = &f.props;
        let cd = f.currency_digits().filter(|_| !p.frac_explicit);
        let int = |v: i32| if v == -1 { None } else { Some(Value::int(v as i64)) };
        match a {
            A_PARSE_INT_ONLY => Some(Value::int(p.parse_int_only as i64)),
            A_GROUPING_USED => Some(Value::int(p.grouping_used as i64)),
            A_DECIMAL_ALWAYS_SHOWN => Some(Value::int(p.decimal_always as i64)),
            A_MAX_INTEGER_DIGITS => int(p.max_int),
            A_MIN_INTEGER_DIGITS | A_INTEGER_DIGITS => int(p.min_int),
            A_MAX_FRACTION_DIGITS => int(cd.unwrap_or(p.max_frac)),
            A_MIN_FRACTION_DIGITS | A_FRACTION_DIGITS => int(cd.unwrap_or(p.min_frac)),
            A_MULTIPLIER => Some(Value::int(p.multiplier)),
            A_GROUPING_SIZE => Some(Value::int(p.g1 as i64)),
            A_ROUNDING_MODE => Some(Value::int(p.rounding_mode)),
            A_ROUNDING_INCREMENT => Some(Value::float(p.increment)),
            A_FORMAT_WIDTH => int(p.width),
            A_PADDING_POSITION => Some(Value::int(p.pad_pos)),
            A_SECONDARY_GROUPING_SIZE => Some(Value::int(p.g2 as i64)),
            A_SIGNIFICANT_DIGITS_USED => Some(Value::int(p.sig_used as i64)),
            A_MIN_SIGNIFICANT_DIGITS => int(p.min_sig),
            A_MAX_SIGNIFICANT_DIGITS => int(p.max_sig),
            A_LENIENT_PARSE => Some(Value::int(p.lenient as i64)),
            _ => None,
        }
    });
    match v {
        Some(v) => {
            check(ctx, st, "Error getting attribute value")?;
            Ok(v)
        }
        None => fail(ctx, st, U_UNSUPPORTED_ERROR, "Error getting attribute value"),
    }
}

fn f_set_attribute(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let a = arg(args, 1).to_int();
    let val = arg(args, 2);
    let n = val.to_int().clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    // ICU keeps digit counts within 999 (kMaxIntFracSig); the maximum integer digits may stay unlimited
    let n = if matches!(a, A_MAX_INTEGER_DIGITS | A_FORMAT_WIDTH | A_MULTIPLIER | A_ROUNDING_MODE | A_PADDING_POSITION | A_GROUPING_SIZE | A_SECONDARY_GROUPING_SIZE) { n } else { n.min(999) };
    let n = if matches!(a, A_GROUPING_SIZE | A_SECONDARY_GROUPING_SIZE | A_FORMAT_WIDTH) { n.min(999) } else { n };
    let ok = with(&o, |f| {
        let cd = f.currency_digits().filter(|_| !f.props.frac_explicit);
        let p = &mut f.props;
        if let Some(d) = cd
            && matches!(a, A_MAX_FRACTION_DIGITS | A_MIN_FRACTION_DIGITS | A_FRACTION_DIGITS)
        {
            p.min_frac = d;
            p.max_frac = d;
        }
        match a {
            A_PARSE_INT_ONLY => p.parse_int_only = n != 0,
            A_GROUPING_USED => p.grouping_used = n != 0,
            A_DECIMAL_ALWAYS_SHOWN => p.decimal_always = n != 0,
            A_MAX_INTEGER_DIGITS => {
                p.max_int = n.max(0);
                if p.min_int > p.max_int {
                    p.min_int = p.max_int;
                }
            }
            A_MIN_INTEGER_DIGITS => {
                p.min_int = n.max(0);
                if p.max_int < p.min_int {
                    p.max_int = p.min_int;
                }
            }
            A_INTEGER_DIGITS => {
                p.min_int = n.max(0);
                p.max_int = n.max(0);
            }
            A_MAX_FRACTION_DIGITS => {
                p.max_frac = n.max(0);
                if p.min_frac > p.max_frac {
                    p.min_frac = p.max_frac;
                }
                p.frac_explicit = true;
            }
            A_MIN_FRACTION_DIGITS => {
                p.min_frac = n.max(0);
                if p.max_frac < p.min_frac {
                    p.max_frac = p.min_frac;
                }
                p.frac_explicit = true;
            }
            A_FRACTION_DIGITS => {
                p.min_frac = n.max(0);
                p.max_frac = n.max(0);
                p.frac_explicit = true;
            }
            A_MULTIPLIER => p.multiplier = n as i64,
            A_GROUPING_SIZE => p.g1 = n,
            A_ROUNDING_MODE => p.rounding_mode = n as i64,
            A_ROUNDING_INCREMENT => p.increment = val.to_float().max(0.0),
            A_FORMAT_WIDTH => p.width = n,
            A_PADDING_POSITION => p.pad_pos = n as i64,
            A_SECONDARY_GROUPING_SIZE => p.g2 = n,
            A_SIGNIFICANT_DIGITS_USED => {
                p.sig_used = n != 0;
                if p.sig_used && p.min_sig < 1 {
                    p.min_sig = 1;
                    p.max_sig = 6;
                }
            }
            A_MIN_SIGNIFICANT_DIGITS => {
                p.min_sig = n.max(1);
                if p.max_sig < p.min_sig {
                    p.max_sig = p.min_sig;
                }
                p.sig_used = true;
            }
            A_MAX_SIGNIFICANT_DIGITS => {
                p.max_sig = n.max(1);
                if p.min_sig > p.max_sig || p.min_sig < 1 {
                    p.min_sig = 1.min(p.max_sig);
                }
                p.sig_used = true;
            }
            A_LENIENT_PARSE => p.lenient = n != 0,
            _ => return false,
        }
        true
    });
    if !ok {
        return fail(ctx, st, U_UNSUPPORTED_ERROR, "Error setting attribute value");
    }
    check(ctx, st, "Error setting attribute value")?;
    Ok(Value::TRUE)
}

const T_POSITIVE_PREFIX: i64 = 0;
const T_POSITIVE_SUFFIX: i64 = 1;
const T_NEGATIVE_PREFIX: i64 = 2;
const T_NEGATIVE_SUFFIX: i64 = 3;
const T_PADDING_CHARACTER: i64 = 4;
const T_CURRENCY_CODE: i64 = 5;
const T_DEFAULT_RULESET: i64 = 6;
const T_PUBLIC_RULESETS: i64 = 7;

fn f_get_text_attribute(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let a = arg(args, 1).to_int();
    let v = with(&o, |f| {
        let (pp, ps) = f.affixes(false);
        let (np, ns) = f.affixes(true);
        match a {
            T_POSITIVE_PREFIX => Some(pp),
            T_POSITIVE_SUFFIX => Some(ps),
            T_NEGATIVE_PREFIX => Some(np),
            T_NEGATIVE_SUFFIX => Some(ns),
            T_PADDING_CHARACTER => Some(f.props.pad.clone()),
            T_CURRENCY_CODE => Some(f.currency.clone()),
            _ => None,
        }
    });
    match v {
        Some(v) => {
            check(ctx, st, "Error getting attribute value")?;
            Ok(s(v))
        }
        None => fail(ctx, st, U_UNSUPPORTED_ERROR, "Error getting attribute value"),
    }
}

fn f_set_text_attribute(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let a = arg(args, 1).to_int();
    let v = str_arg(args, 2);
    let ok = with(&o, |f| {
        let lit = |v: &str| if v.is_empty() { Vec::new() } else { vec![Tok::Lit(v.to_string())] };
        let implied_neg = |f: &Formatter| -> (Affix, Affix) {
            let mut p = vec![Tok::Minus];
            p.extend(f.props.pos_prefix.iter().cloned());
            (p, f.props.pos_suffix.clone())
        };
        match a {
            T_POSITIVE_PREFIX => {
                if f.props.neg.is_none() {
                    f.props.neg = Some(implied_neg(f));
                }
                f.props.pos_prefix = lit(&v);
            }
            T_POSITIVE_SUFFIX => {
                if f.props.neg.is_none() {
                    f.props.neg = Some(implied_neg(f));
                }
                f.props.pos_suffix = lit(&v);
            }
            T_NEGATIVE_PREFIX => {
                let n = f.props.neg.take().unwrap_or_else(|| implied_neg(f));
                f.props.neg = Some((lit(&v), n.1));
            }
            T_NEGATIVE_SUFFIX => {
                let n = f.props.neg.take().unwrap_or_else(|| implied_neg(f));
                f.props.neg = Some((n.0, lit(&v)));
            }
            T_PADDING_CHARACTER => f.props.pad = v.chars().next().map(String::from).unwrap_or_else(|| " ".into()),
            T_CURRENCY_CODE => {
                let code: String = v.chars().take(3).collect::<String>().to_ascii_uppercase();
                f.set_currency(&code);
            }
            T_DEFAULT_RULESET | T_PUBLIC_RULESETS => return false,
            _ => return false,
        }
        true
    });
    if !ok {
        return fail(ctx, st, U_UNSUPPORTED_ERROR, "Error setting text attribute");
    }
    check(ctx, st, "Error setting text attribute")?;
    Ok(Value::TRUE)
}

fn f_get_symbol(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let sym = arg(args, 1).to_int();
    if !(0..SYMBOL_COUNT).contains(&sym) {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "numfmt_get_symbol: invalid symbol value")?;
        return Ok(Value::FALSE);
    }
    let o = obj(ctx, args)?;
    let st = state(&o);
    let v = with(&o, |f| f.syms.s[sym as usize].clone());
    check(ctx, st, "Error getting symbol value")?;
    Ok(s(v))
}

fn f_set_symbol(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let sym = arg(args, 1).to_int();
    if !(0..SYMBOL_COUNT).contains(&sym) {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "numfmt_set_symbol: invalid symbol value")?;
        return Ok(Value::FALSE);
    }
    let o = obj(ctx, args)?;
    let st = state(&o);
    let v = str_arg(args, 2);
    with(&o, |f| {
        let i = sym as usize;
        f.syms.s[i] = v.clone();
        if i == SYM_ZERO
            && let Some(z) = v.chars().next()
            && let Some(base) = z.to_digit(10).map(|d| z as u32 - d)
        {
            for d in 0..10 {
                f.syms.digits[d] = char::from_u32(base + d as u32).unwrap_or('0');
            }
        }
        if i == SYM_CURRENCY {
            f.currency_symbol_set = true;
        }
    });
    check(ctx, st, "Error setting symbol value")?;
    Ok(Value::TRUE)
}

fn f_get_pattern(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let p = with(&o, |f| f.pattern());
    check(ctx, st, "Error getting formatter pattern")?;
    Ok(s(p))
}

fn f_set_pattern(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let pat = str_arg(args, 1);
    match parse_pattern(&pat) {
        Ok(p) => {
            with(&o, |f| f.props = p);
            check(ctx, st, "Error setting pattern value")?;
            Ok(Value::TRUE)
        }
        Err(e) => {
            st.err.code.set(e.code);
            crate::set_global_code(ctx, e.code);
            crate::set_message(ctx, Some(&st.err), &format!("Error setting pattern value at line 0, offset {}", e.offset))?;
            Ok(Value::FALSE)
        }
    }
}

fn f_get_locale(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let ty = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    if ty != 0 && ty != 1 {
        return fail(ctx, st, U_ILLEGAL_ARGUMENT_ERROR, "Error getting locale");
    }
    let loc = with(&o, |f| crate::valid_locale(&f.locale));
    check(ctx, st, "Error getting locale")?;
    Ok(s(loc))
}

fn f_get_error_code(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(o) = arg(args, 0).into_object() else { unreachable!() };
    Ok(Value::int(state(&o).err.code.get()))
}

fn f_get_error_message(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(o) = arg(args, 0).into_object() else { unreachable!() };
    Ok(s(state(&o).err.message()))
}

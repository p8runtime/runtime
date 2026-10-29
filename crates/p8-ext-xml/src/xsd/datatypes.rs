//! XML Schema built-in datatypes: their lexical spaces, whitespace
//! handling and value ordering, as libxml2's xmlschemastypes.c has them.

use super::regex::{is_name_char, is_name_start};
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum B {
    AnyType,
    AnySimpleType,
    String,
    NormalizedString,
    Token,
    Language,
    Name,
    NCName,
    NMToken,
    NMTokens,
    Id,
    IdRef,
    IdRefs,
    Entity,
    Entities,
    Boolean,
    Decimal,
    Integer,
    NonPositiveInteger,
    NegativeInteger,
    Long,
    Int,
    Short,
    Byte,
    NonNegativeInteger,
    UnsignedLong,
    UnsignedInt,
    UnsignedShort,
    UnsignedByte,
    PositiveInteger,
    Float,
    Double,
    Duration,
    DateTime,
    Time,
    Date,
    GYearMonth,
    GYear,
    GMonthDay,
    GDay,
    GMonth,
    HexBinary,
    Base64Binary,
    AnyUri,
    QName,
    Notation,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ws {
    Preserve,
    Replace,
    Collapse,
}

pub(crate) const ALL: &[(B, &str)] = &[
    (B::AnyType, "anyType"),
    (B::AnySimpleType, "anySimpleType"),
    (B::String, "string"),
    (B::NormalizedString, "normalizedString"),
    (B::Token, "token"),
    (B::Language, "language"),
    (B::Name, "Name"),
    (B::NCName, "NCName"),
    (B::NMToken, "NMTOKEN"),
    (B::NMTokens, "NMTOKENS"),
    (B::Id, "ID"),
    (B::IdRef, "IDREF"),
    (B::IdRefs, "IDREFS"),
    (B::Entity, "ENTITY"),
    (B::Entities, "ENTITIES"),
    (B::Boolean, "boolean"),
    (B::Decimal, "decimal"),
    (B::Integer, "integer"),
    (B::NonPositiveInteger, "nonPositiveInteger"),
    (B::NegativeInteger, "negativeInteger"),
    (B::Long, "long"),
    (B::Int, "int"),
    (B::Short, "short"),
    (B::Byte, "byte"),
    (B::NonNegativeInteger, "nonNegativeInteger"),
    (B::UnsignedLong, "unsignedLong"),
    (B::UnsignedInt, "unsignedInt"),
    (B::UnsignedShort, "unsignedShort"),
    (B::UnsignedByte, "unsignedByte"),
    (B::PositiveInteger, "positiveInteger"),
    (B::Float, "float"),
    (B::Double, "double"),
    (B::Duration, "duration"),
    (B::DateTime, "dateTime"),
    (B::Time, "time"),
    (B::Date, "date"),
    (B::GYearMonth, "gYearMonth"),
    (B::GYear, "gYear"),
    (B::GMonthDay, "gMonthDay"),
    (B::GDay, "gDay"),
    (B::GMonth, "gMonth"),
    (B::HexBinary, "hexBinary"),
    (B::Base64Binary, "base64Binary"),
    (B::AnyUri, "anyURI"),
    (B::QName, "QName"),
    (B::Notation, "NOTATION"),
];

impl B {
    pub(crate) fn from_name(name: &str) -> Option<B> {
        ALL.iter().find(|(_, n)| *n == name).map(|(b, _)| *b)
    }

    /// The type it restricts (anyType for anySimpleType's parent).
    pub(crate) fn parent(self) -> Option<B> {
        use B::*;
        Some(match self {
            AnyType => return None,
            AnySimpleType => AnyType,
            String | Boolean | Decimal | Float | Double | Duration | DateTime | Time | Date | GYearMonth | GYear | GMonthDay | GDay | GMonth | HexBinary | Base64Binary | AnyUri | QName | Notation => AnySimpleType,
            NormalizedString => String,
            Token => NormalizedString,
            Language | Name | NMToken => Token,
            NCName => Name,
            Id | IdRef | Entity => NCName,
            NMTokens => AnySimpleType,
            IdRefs | Entities => AnySimpleType,
            Integer => Decimal,
            NonPositiveInteger | Long | NonNegativeInteger => Integer,
            NegativeInteger => NonPositiveInteger,
            Int => Long,
            Short => Int,
            Byte => Short,
            UnsignedLong => NonNegativeInteger,
            UnsignedInt => UnsignedLong,
            UnsignedShort => UnsignedInt,
            UnsignedByte => UnsignedShort,
            PositiveInteger => NonNegativeInteger,
        })
    }

    pub(crate) fn ws(self) -> Ws {
        use B::*;
        match self {
            String | AnyType | AnySimpleType => Ws::Preserve,
            NormalizedString => Ws::Replace,
            _ => Ws::Collapse,
        }
    }

    /// The item type of a built-in list type.
    pub(crate) fn list_item(self) -> Option<B> {
        match self {
            B::NMTokens => Some(B::NMToken),
            B::IdRefs => Some(B::IdRef),
            B::Entities => Some(B::Entity),
            _ => None,
        }
    }

    pub(crate) fn is_numeric(self) -> bool {
        let mut at = Some(self);
        while let Some(b) = at {
            if matches!(b, B::Decimal | B::Float | B::Double) {
                return true;
            }
            at = b.parent();
        }
        false
    }

    /// Whether `self` is `other` or derives from it.
    pub(crate) fn derives_from(self, other: B) -> bool {
        let mut at = Some(self);
        while let Some(b) = at {
            if b == other {
                return true;
            }
            at = b.parent();
        }
        false
    }
}

/// The whitespace facet applied to `v`.
pub(crate) fn normalize(ws: Ws, v: &str) -> String {
    match ws {
        Ws::Preserve => v.to_string(),
        Ws::Replace => v.chars().map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c }).collect(),
        Ws::Collapse => v.split([' ', '\t', '\n', '\r']).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" "),
    }
}

fn is_ncname(v: &str) -> bool {
    let mut chars = v.chars();
    match chars.next() {
        Some(c) if is_name_start(c) && c != ':' => chars.all(|c| is_name_char(c) && c != ':'),
        _ => false,
    }
}

fn is_xml_name(v: &str) -> bool {
    let mut chars = v.chars();
    match chars.next() {
        Some(c) if is_name_start(c) => chars.all(is_name_char),
        _ => false,
    }
}

/// Whether `v` (already whitespace-normalised) is in the lexical space of `b`.
pub(crate) fn lexical_ok(b: B, v: &str) -> bool {
    use B::*;
    match b {
        AnyType | AnySimpleType | String | NormalizedString | Token => true,
        Language => {
            let mut parts = v.split('-');
            let first = parts.next().unwrap_or("");
            (1..=8).contains(&first.len()) && first.chars().all(|c| c.is_ascii_alphabetic()) && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
        }
        Name => is_xml_name(v),
        NCName | Id | IdRef | Entity => is_ncname(v),
        NMToken => !v.is_empty() && v.chars().all(is_name_char),
        NMTokens | IdRefs | Entities => {
            let item = b.list_item().unwrap_or(NMToken);
            !v.is_empty() && v.split(' ').all(|t| lexical_ok(item, t))
        }
        Boolean => matches!(v, "true" | "false" | "1" | "0"),
        Decimal => parse_dec(v, true).is_some(),
        Integer | NonPositiveInteger | NegativeInteger | Long | Int | Short | Byte | NonNegativeInteger | UnsignedLong | UnsignedInt | UnsignedShort | UnsignedByte | PositiveInteger => integer_ok(b, v),
        Float | Double => float_ok(v),
        Duration => duration_ok(v),
        DateTime | Time | Date | GYearMonth | GYear | GMonthDay | GDay | GMonth => datetime_ok(b, v),
        HexBinary => v.len().is_multiple_of(2) && v.chars().all(|c| c.is_ascii_hexdigit()),
        Base64Binary => base64_ok(v),
        AnyUri => uri_ok(v),
        QName => {
            let mut parts = v.splitn(2, ':');
            let a = parts.next().unwrap_or("");
            match parts.next() {
                None => is_ncname(a),
                Some(l) => is_ncname(a) && is_ncname(l),
            }
        }
        Notation => {
            let mut parts = v.splitn(2, ':');
            let a = parts.next().unwrap_or("");
            match parts.next() {
                None => is_ncname(a),
                Some(l) => is_ncname(a) && is_ncname(l),
            }
        }
    }
}

// ---- decimals ----

#[derive(Clone, Debug)]
pub(crate) struct Dec {
    pub neg: bool,
    /// Digits before the point, no leading zeros ("" for 0).
    pub int: String,
    /// Digits after the point, no trailing zeros.
    pub frac: String,
}

pub(crate) fn parse_dec(v: &str, allow_frac: bool) -> Option<Dec> {
    let (neg, rest) = match v.as_bytes().first()? {
        b'-' => (true, &v[1..]),
        b'+' => (false, &v[1..]),
        _ => (false, v),
    };
    let (int, frac) = match rest.split_once('.') {
        Some((i, f)) => {
            if !allow_frac {
                return None;
            }
            (i, f)
        }
        None => (rest, ""),
    };
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    if !int.bytes().all(|c| c.is_ascii_digit()) || !frac.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let int = int.trim_start_matches('0').to_string();
    let frac = frac.trim_end_matches('0').to_string();
    let zero = int.is_empty() && frac.is_empty();
    Some(Dec { neg: neg && !zero, int, frac })
}

impl Dec {
    pub(crate) fn cmp(&self, o: &Dec) -> Ordering {
        match (self.neg, o.neg) {
            (false, true) => return Ordering::Greater,
            (true, false) => return Ordering::Less,
            _ => {}
        }
        let mag = self.mag_cmp(o);
        if self.neg { mag.reverse() } else { mag }
    }

    fn mag_cmp(&self, o: &Dec) -> Ordering {
        self.int.len().cmp(&o.int.len()).then_with(|| self.int.cmp(&o.int)).then_with(|| self.frac.cmp(&o.frac))
    }

    pub(crate) fn total_digits(&self) -> usize {
        let n = self.int.len() + self.frac.len();
        n.max(1)
    }
}

fn integer_ok(b: B, v: &str) -> bool {
    let Some(d) = parse_dec(v, false) else { return false };
    let zero = d.int.is_empty();
    let lim = |s: &str| parse_dec(s, false).expect("constant");
    use B::*;
    match b {
        Integer => true,
        NonPositiveInteger => d.neg || zero,
        NegativeInteger => d.neg,
        NonNegativeInteger => !d.neg,
        PositiveInteger => !d.neg && !zero,
        Long => d.cmp(&lim("-9223372036854775808")) != Ordering::Less && d.cmp(&lim("9223372036854775807")) != Ordering::Greater,
        Int => d.cmp(&lim("-2147483648")) != Ordering::Less && d.cmp(&lim("2147483647")) != Ordering::Greater,
        Short => d.cmp(&lim("-32768")) != Ordering::Less && d.cmp(&lim("32767")) != Ordering::Greater,
        Byte => d.cmp(&lim("-128")) != Ordering::Less && d.cmp(&lim("127")) != Ordering::Greater,
        UnsignedLong => !d.neg && d.cmp(&lim("18446744073709551615")) != Ordering::Greater,
        UnsignedInt => !d.neg && d.cmp(&lim("4294967295")) != Ordering::Greater,
        UnsignedShort => !d.neg && d.cmp(&lim("65535")) != Ordering::Greater,
        UnsignedByte => !d.neg && d.cmp(&lim("255")) != Ordering::Greater,
        _ => false,
    }
}

fn float_ok(v: &str) -> bool {
    if matches!(v, "INF" | "-INF" | "NaN" | "+INF") {
        return true;
    }
    let rest = v.strip_prefix(['+', '-']).unwrap_or(v);
    let (mant, exp) = match rest.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (rest, None),
    };
    let (i, f) = mant.split_once('.').unwrap_or((mant, ""));
    if i.is_empty() && f.is_empty() {
        return false;
    }
    if !i.bytes().all(|c| c.is_ascii_digit()) || !f.bytes().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match exp {
        None => true,
        Some(e) => {
            let e = e.strip_prefix(['+', '-']).unwrap_or(e);
            !e.is_empty() && e.bytes().all(|c| c.is_ascii_digit())
        }
    }
}

fn duration_ok(v: &str) -> bool {
    let v = v.strip_prefix('-').unwrap_or(v);
    let Some(rest) = v.strip_prefix('P') else { return false };
    if rest.is_empty() {
        return false;
    }
    let (date, time) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };
    let mut any = false;
    let mut order = "YMD".chars().peekable();
    let mut num = String::new();
    for c in date.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        if num.is_empty() {
            return false;
        }
        loop {
            match order.next() {
                Some(o) if o == c => break,
                Some(_) => continue,
                None => return false,
            }
        }
        num.clear();
        any = true;
    }
    if !num.is_empty() {
        return false;
    }
    if let Some(t) = time {
        if t.is_empty() {
            return false;
        }
        let mut order = "HMS".chars();
        let mut num = String::new();
        let mut frac_ok = false;
        for c in t.chars() {
            if c.is_ascii_digit() {
                num.push(c);
                continue;
            }
            if c == '.' {
                if num.is_empty() || frac_ok {
                    return false;
                }
                frac_ok = true;
                num.push(c);
                continue;
            }
            if num.is_empty() || num.ends_with('.') {
                return false;
            }
            if frac_ok && c != 'S' {
                return false;
            }
            loop {
                match order.next() {
                    Some(o) if o == c => break,
                    Some(_) => continue,
                    None => return false,
                }
            }
            num.clear();
            frac_ok = false;
            any = true;
        }
        if !num.is_empty() {
            return false;
        }
    }
    any
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn digits(&mut self, n: usize) -> Option<u32> {
        let part = self.s.get(self.i..self.i + n)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.i += n;
        std::str::from_utf8(part).ok()?.parse().ok()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn year(&mut self) -> Option<i64> {
        let neg = self.eat(b'-');
        let start = self.i;
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        let n = self.i - start;
        if n < 4 || (n > 4 && self.s[start] == b'0') {
            return None;
        }
        let y: i64 = std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()?;
        if y == 0 {
            return None;
        }
        Some(if neg { -y } else { y })
    }

    fn tz(&mut self) -> Option<()> {
        if self.i == self.s.len() {
            return Some(());
        }
        if self.eat(b'Z') {
            return if self.i == self.s.len() { Some(()) } else { None };
        }
        if !(self.eat(b'+') || self.eat(b'-')) {
            return None;
        }
        let h = self.digits(2)?;
        if !self.eat(b':') {
            return None;
        }
        let m = self.digits(2)?;
        if (h > 14 || m > 59 || (h == 14 && m != 0)) || self.i != self.s.len() {
            return None;
        }
        Some(())
    }

    fn time(&mut self) -> Option<()> {
        let h = self.digits(2)?;
        if !self.eat(b':') {
            return None;
        }
        let m = self.digits(2)?;
        if !self.eat(b':') {
            return None;
        }
        let s = self.digits(2)?;
        let mut frac = false;
        if self.eat(b'.') {
            let start = self.i;
            while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
            }
            if self.i == start {
                return None;
            }
            frac = self.s[start..self.i].iter().any(|&c| c != b'0');
        }
        if h > 24 || m > 59 || s > 59 || (h == 24 && (m != 0 || s != 0 || frac)) {
            return None;
        }
        Some(())
    }
}

fn datetime_ok(b: B, v: &str) -> bool {
    let mut c = Cursor { s: v.as_bytes(), i: 0 };
    let ok = (|| -> Option<()> {
        match b {
            B::DateTime => {
                let y = c.year()?;
                if !c.eat(b'-') {
                    return None;
                }
                let m = c.digits(2)?;
                if !c.eat(b'-') {
                    return None;
                }
                let d = c.digits(2)?;
                if !(1..=12).contains(&m) || d < 1 || d > days_in(y, m) || !c.eat(b'T') {
                    return None;
                }
                c.time()?;
                c.tz()
            }
            B::Date => {
                let y = c.year()?;
                if !c.eat(b'-') {
                    return None;
                }
                let m = c.digits(2)?;
                if !c.eat(b'-') {
                    return None;
                }
                let d = c.digits(2)?;
                if !(1..=12).contains(&m) || d < 1 || d > days_in(y, m) {
                    return None;
                }
                c.tz()
            }
            B::Time => {
                c.time()?;
                c.tz()
            }
            B::GYearMonth => {
                c.year()?;
                if !c.eat(b'-') {
                    return None;
                }
                let m = c.digits(2)?;
                if !(1..=12).contains(&m) {
                    return None;
                }
                c.tz()
            }
            B::GYear => {
                c.year()?;
                c.tz()
            }
            B::GMonthDay => {
                if !(c.eat(b'-') && c.eat(b'-')) {
                    return None;
                }
                let m = c.digits(2)?;
                if !c.eat(b'-') {
                    return None;
                }
                let d = c.digits(2)?;
                if !(1..=12).contains(&m) || d < 1 || d > days_in(4, m) {
                    return None;
                }
                c.tz()
            }
            B::GDay => {
                if !(c.eat(b'-') && c.eat(b'-') && c.eat(b'-')) {
                    return None;
                }
                let d = c.digits(2)?;
                if !(1..=31).contains(&d) {
                    return None;
                }
                c.tz()
            }
            B::GMonth => {
                if !(c.eat(b'-') && c.eat(b'-')) {
                    return None;
                }
                let m = c.digits(2)?;
                if !(1..=12).contains(&m) {
                    return None;
                }
                c.tz()
            }
            _ => None,
        }
    })();
    ok.is_some()
}

fn base64_ok(v: &str) -> bool {
    let compact: String = v.chars().filter(|c| *c != ' ').collect();
    if !compact.len().is_multiple_of(4) {
        return false;
    }
    let pad = compact.chars().rev().take_while(|c| *c == '=').count();
    if pad > 2 {
        return false;
    }
    compact[..compact.len() - pad].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
}

/// libxml2 runs xmlParseURI over anyURI values (RFC 3986 with a few
/// allowances): a value with a character outside the URI sets is invalid.
fn uri_ok(v: &str) -> bool {
    // libxml2 first replaces the characters a URI may not hold by '_',
    // so what remains to fail is a bad percent escape or a second '#'.
    let b = v.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if !(b.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit)) {
                return false;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    v.matches('#').count() <= 1
}

// ---- comparing values ----

fn to_f64(v: &str) -> f64 {
    match v {
        "INF" | "+INF" => f64::INFINITY,
        "-INF" => f64::NEG_INFINITY,
        "NaN" => f64::NAN,
        _ => v.parse().unwrap_or(f64::NAN),
    }
}

/// Seconds since a fixed origin for a date/time value, UTC-adjusted;
/// None when it is not an ordered kind.
fn time_key(b: B, v: &str) -> Option<f64> {
    let (neg, body) = match v.strip_prefix('-') {
        Some(r) if matches!(b, B::DateTime | B::Date | B::GYear | B::GYearMonth) => (true, r),
        _ => (false, v),
    };
    let (core, tz) = split_tz(body);
    let num = |s: &str| s.parse::<f64>().unwrap_or(0.0);
    let days_before = |y: f64| 365.0 * y + (y / 4.0).floor() - (y / 100.0).floor() + (y / 400.0).floor();
    let month_days = |y: i64, m: u32| (1..m).map(|mm| days_in(y, mm) as f64).sum::<f64>();
    let secs = match b {
        B::DateTime | B::Date => {
            let (d, t) = core.split_once('T').unwrap_or((core, ""));
            let mut p = d.split('-');
            let y = num(p.next()?);
            let yi = y as i64;
            let m = num(p.next()?) as u32;
            let day = num(p.next()?);
            let yy = if neg { -y } else { y };
            let days = days_before(yy - 1.0) + month_days(yi, m) + day;
            let mut s = days * 86400.0;
            if !t.is_empty() {
                let mut tp = t.split(':');
                s += num(tp.next()?) * 3600.0 + num(tp.next()?) * 60.0 + num(tp.next()?);
            }
            s
        }
        B::Time => {
            let mut tp = core.split(':');
            num(tp.next()?) * 3600.0 + num(tp.next()?) * 60.0 + num(tp.next()?)
        }
        B::GYear => num(core) * 366.0 * 86400.0,
        B::GYearMonth => {
            let mut p = core.split('-');
            num(p.next()?) * 366.0 * 86400.0 + num(p.next()?) * 31.0 * 86400.0
        }
        B::GMonthDay => {
            let mut p = core.trim_start_matches('-').split('-');
            num(p.next()?) * 31.0 * 86400.0 + num(p.next()?) * 86400.0
        }
        B::GMonth => num(core.trim_start_matches('-')) * 31.0 * 86400.0,
        B::GDay => num(core.trim_start_matches('-')) * 86400.0,
        _ => return None,
    };
    Some(secs - tz)
}

fn split_tz(v: &str) -> (&str, f64) {
    if let Some(core) = v.strip_suffix('Z') {
        return (core, 0.0);
    }
    if v.len() > 6 {
        let (core, tail) = v.split_at(v.len() - 6);
        let tb = tail.as_bytes();
        if (tb[0] == b'+' || tb[0] == b'-') && tb[3] == b':' {
            let h: f64 = tail[1..3].parse().unwrap_or(0.0);
            let m: f64 = tail[4..6].parse().unwrap_or(0.0);
            let off = h * 3600.0 + m * 60.0;
            return (core, if tb[0] == b'-' { -off } else { off });
        }
    }
    (v, 0.0)
}

fn duration_key(v: &str) -> f64 {
    let neg = v.starts_with('-');
    let body = v.trim_start_matches('-').trim_start_matches('P');
    let (date, time) = body.split_once('T').unwrap_or((body, ""));
    let mut total = 0.0;
    let mut num = String::new();
    for c in date.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            let n: f64 = num.parse().unwrap_or(0.0);
            total += n * match c {
                'Y' => 365.0 * 86400.0,
                'M' => 30.0 * 86400.0,
                _ => 86400.0,
            };
            num.clear();
        }
    }
    for c in time.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            let n: f64 = num.parse().unwrap_or(0.0);
            total += n * match c {
                'H' => 3600.0,
                'M' => 60.0,
                _ => 1.0,
            };
            num.clear();
        }
    }
    if neg { -total } else { total }
}

/// Orders two lexical values of type `b`, if the type has an order.
pub(crate) fn compare(b: B, x: &str, y: &str) -> Option<Ordering> {
    if b.derives_from(B::Decimal) {
        return Some(parse_dec(x, true)?.cmp(&parse_dec(y, true)?));
    }
    match b {
        B::Float | B::Double => to_f64(x).partial_cmp(&to_f64(y)),
        B::Duration => duration_key(x).partial_cmp(&duration_key(y)),
        B::DateTime | B::Date | B::Time | B::GYear | B::GYearMonth | B::GMonthDay | B::GDay | B::GMonth => time_key(b, x)?.partial_cmp(&time_key(b, y)?),
        _ => None,
    }
}

/// Whether two values of type `b` are the same value.
pub(crate) fn equal(b: B, x: &str, y: &str) -> bool {
    if b == B::Boolean {
        let t = |s: &str| matches!(s, "true" | "1");
        return t(x) == t(y);
    }
    if b.is_numeric() || matches!(b, B::Duration | B::DateTime | B::Date | B::Time | B::GYear | B::GYearMonth | B::GMonthDay | B::GDay | B::GMonth) {
        return compare(b, x, y) == Some(Ordering::Equal) || (x == y);
    }
    if b == B::HexBinary {
        return x.eq_ignore_ascii_case(y);
    }
    x == y
}

/// The length facet's unit count for `v` of type `b`.
pub(crate) fn length(b: B, v: &str) -> usize {
    match b {
        B::HexBinary => v.len() / 2,
        B::Base64Binary => {
            let compact: String = v.chars().filter(|c| *c != ' ').collect();
            let pad = compact.chars().rev().take_while(|c| *c == '=').count();
            (compact.len() / 4) * 3 - pad.min(2)
        }
        B::NMTokens | B::IdRefs | B::Entities => v.split(' ').filter(|s| !s.is_empty()).count(),
        _ => v.chars().count(),
    }
}

/// Digits for the totalDigits and fractionDigits facets.
pub(crate) fn digits(v: &str) -> Option<(usize, usize)> {
    let d = parse_dec(v, true)?;
    Some((d.total_digits(), d.frac.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical() {
        assert!(lexical_ok(B::Integer, "-12"));
        assert!(!lexical_ok(B::Integer, "1.5"));
        assert!(lexical_ok(B::Byte, "127"));
        assert!(!lexical_ok(B::Byte, "128"));
        assert!(lexical_ok(B::Boolean, "1"));
        assert!(!lexical_ok(B::Boolean, "yes"));
        assert!(lexical_ok(B::DateTime, "2020-02-29T12:00:00Z"));
        assert!(!lexical_ok(B::Date, "2021-02-29"));
        assert!(lexical_ok(B::Duration, "P1Y2M3DT4H5M6S"));
        assert!(!lexical_ok(B::Duration, "P"));
        assert!(lexical_ok(B::AnyUri, "a b"));
        assert!(!lexical_ok(B::AnyUri, "a%zz"));
        assert!(lexical_ok(B::Decimal, "-.5"));
        assert!(lexical_ok(B::Double, "1e5"));
    }
}

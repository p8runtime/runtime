//! IntlDateFormatter and datefmt_* (dateformat/*.c, *.cpp): ICU's
//! SimpleDateFormat patterns over the proleptic Gregorian calendar.
//!
//! The date/time styles are CLDR's (the pattern ICU4X's datetime formatter
//! selects for the matching field set and length); the pattern letters are
//! ICU's. Numeric fields are computed here; text fields (eras, months,
//! weekdays, day periods, time zone names) are CLDR's, formatted by ICU4X one
//! field at a time. Time zones are the IANA database the date functions use
//! (chrono-tz).
//!
//! Not available: calendars other than the Gregorian (a locale whose
//! traditional calendar is not Gregorian fails with U_UNSUPPORTED_ERROR when
//! TRADITIONAL is asked for), IntlCalendar arguments, the quarter names
//! (CLDR's root "Q1" is used), and the RELATIVE_* styles' words beyond
//! today/yesterday/tomorrow (ICU4X's relative-time data).

use crate::{IntlError, R, U_ILLEGAL_ARGUMENT_ERROR, U_UNSUPPORTED_ERROR, U_ZERO_ERROR, arg, b, default_locale, lossy, prep, s, str_arg};
use chrono::{Datelike, Offset, TimeZone as _, Timelike};
use chrono_tz::OffsetComponents;
use icu_calendar::Gregorian;
use icu_datetime::fieldsets::enums::CompositeFieldSet;
use icu_datetime::input::ZonedDateTime;
use icu_datetime::pattern::{DateTimePattern, FixedCalendarDateTimeNames};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassHooks, ClassSpec, Ctx, Registry, Throw};
use p8_runtime::object::Object;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const FULL: i64 = 0;
pub const LONG: i64 = 1;
pub const MEDIUM: i64 = 2;
pub const SHORT: i64 = 3;
pub const NONE: i64 = -1;
pub const PATTERN: i64 = -2;
pub const RELATIVE: i64 = 128;
pub const GREGORIAN: i64 = 1;
pub const TRADITIONAL: i64 = 0;

fn style_ok(s: i64) -> bool {
    matches!(s, FULL | LONG | MEDIUM | SHORT | NONE | PATTERN) || (RELATIVE..=RELATIVE + 3).contains(&s)
}

// ---- time zones ----

/// A formatter's time zone: an IANA zone, or a fixed offset (ICU's custom "GMT+02:00").
#[derive(Clone, Debug)]
pub enum Zone {
    Iana(chrono_tz::Tz),
    Fixed(i32),
}

impl Zone {
    pub fn id(&self) -> String {
        match self {
            Zone::Iana(tz) => tz.name().to_string(),
            Zone::Fixed(off) => gmt_id(*off),
        }
    }

    /// The offset and the daylight-saving part of it at a UTC instant (ms).
    pub fn offsets_at(&self, ms: i64) -> (i32, i32) {
        match self {
            Zone::Fixed(off) => (*off, 0),
            Zone::Iana(tz) => match tz.timestamp_millis_opt(ms).single() {
                Some(dt) => {
                    let o = dt.offset();
                    (o.fix().local_minus_utc(), o.dst_offset().num_seconds() as i32)
                }
                None => (0, 0),
            },
        }
    }
}

/// The current time.
pub fn now_utc() -> chrono::DateTime<chrono::Utc> {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

fn gmt_id(off: i32) -> String {
    if off == 0 {
        return "GMT".into();
    }
    let a = off.unsigned_abs();
    let (h, m, s) = (a / 3600, a / 60 % 60, a % 60);
    let sign = if off < 0 { '-' } else { '+' };
    if s != 0 { format!("GMT{sign}{h:02}:{m:02}:{s:02}") } else { format!("GMT{sign}{h:02}:{m:02}") }
}

/// TimeZone::createTimeZone: an IANA ID (any case), or a custom "GMT+h[:mm]" one.
pub fn zone_from_id(id: &str) -> Option<Zone> {
    if id.eq_ignore_ascii_case("UTC") || id.eq_ignore_ascii_case("GMT") || id.eq_ignore_ascii_case("Etc/UTC") {
        return Some(Zone::Iana(if id.eq_ignore_ascii_case("GMT") { chrono_tz::Tz::GMT } else { chrono_tz::Tz::UTC }));
    }
    if let Some(rest) = id.strip_prefix("GMT").or_else(|| id.strip_prefix("gmt")).filter(|r| r.starts_with(['+', '-']))
        && let Some(off) = parse_offset(rest)
    {
        return Some(Zone::Fixed(off));
    }
    chrono_tz::Tz::from_str_insensitive(id).ok().map(Zone::Iana)
}

/// "+2", "+02", "+0230", "+02:30", "-5:00" as seconds.
fn parse_offset(s: &str) -> Option<i32> {
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    let (h, m) = if let Some((h, m)) = rest.split_once(':') {
        (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?)
    } else if rest.len() <= 2 {
        (rest.parse::<i32>().ok()?, 0)
    } else if rest.len() == 4 {
        (rest[..2].parse::<i32>().ok()?, rest[2..].parse::<i32>().ok()?)
    } else {
        return None;
    };
    if !(0..=23).contains(&h) || !(0..60).contains(&m) || !rest.bytes().all(|c| c.is_ascii_digit() || c == b':') {
        return None;
    }
    Some(sign * (h * 3600 + m * 60))
}

/// The PHP default time zone (date_default_timezone_get()).
fn php_default_zone(ctx: &mut dyn Ctx) -> Zone {
    let name = ctx.call(&s("date_default_timezone_get"), &mut []).ok().and_then(|v| v.to_php_string()).map(|s| lossy(s.as_bytes())).unwrap_or_else(|| "UTC".into());
    zone_from_id(&name).unwrap_or(Zone::Iana(chrono_tz::Tz::UTC))
}

/// timezone_process_timezone_argument: null (the default), an ID, a
/// DateTimeZone or an IntlTimeZone.
fn zone_arg(ctx: &mut dyn Ctx, v: &Value, func: &str) -> Result<Result<Zone, String>, Throw> {
    match v {
        Value::Null | Value::Undef => Ok(Ok(php_default_zone(ctx))),
        Value::Object(o) => {
            let class = lossy(o.class.name.as_bytes()).to_ascii_lowercase();
            if class == "intltimezone" {
                return Ok(Ok(crate::timezone::zone_of(o)));
            }
            let mut none: [Value; 0] = [];
            let name = ctx.call(&Value::Array(Rc::new(PhpArray::from_list(vec![v.clone(), s("getName")]))), &mut none)?;
            let name = name.to_php_string().map(|s| lossy(s.as_bytes())).unwrap_or_default();
            let id = if name.starts_with(['+', '-']) { format!("GMT{name}") } else { name };
            Ok(zone_from_id(&id).ok_or_else(|| format!("{func}: No such time zone: '{id}'")))
        }
        other => {
            let id = other.to_php_string().map(|s| lossy(s.as_bytes())).unwrap_or_default();
            Ok(zone_from_id(&id).ok_or_else(|| format!("{func}: No such time zone: '{id}'")))
        }
    }
}

// ---- the calendar fields of an instant ----

#[derive(Clone, Copy, Debug)]
pub struct Fields {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
    /// 0 = Sunday.
    pub weekday: u32,
    pub yday: u32,
    pub offset: i32,
    pub dst: i32,
}

pub fn fields_at(ms: i64, zone: &Zone) -> Option<Fields> {
    let (offset, dst) = zone.offsets_at(ms);
    let local = chrono::DateTime::from_timestamp_millis(ms.checked_add(offset as i64 * 1000)?)?.naive_utc();
    Some(Fields {
        year: local.year() as i64,
        month: local.month(),
        day: local.day(),
        hour: local.hour(),
        minute: local.minute(),
        second: local.second(),
        millis: local.and_utc().timestamp_subsec_millis(),
        weekday: local.weekday().num_days_from_sunday(),
        yday: local.ordinal(),
        offset,
        dst,
    })
}

/// The locale's first day of the week (0 = Sunday) and the minimum days of a first week.
fn week_rules(locale: &str) -> (u32, u32) {
    let loc = crate::icu4x_locale(locale);
    let first = icu_calendar::week::WeekInformation::try_new((&loc).into()).map(|w| w.first_weekday as u32 % 7).unwrap_or(1);
    let region = crate::uloc::parts(locale).region;
    // CLDR's minDays: 4 in most of Europe (ISO 8601 weeks), 1 elsewhere
    let four = [
        "AD", "AN", "AT", "AX", "BE", "BG", "CH", "CZ", "DE", "DK", "EE", "ES", "FI", "FJ", "FO", "FR", "GB", "GF", "GG", "GI", "GP", "GR", "HU", "IE", "IM", "IS", "IT", "JE", "LI", "LT", "LU", "MC", "MQ", "NL", "NO", "PL", "PT", "RE", "RU", "SE", "SJ", "SK", "SM", "VA",
    ];
    (first, if four.contains(&region.as_str()) { 4 } else { 1 })
}

fn days_in_year(y: i64) -> u32 {
    if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366 } else { 365 }
}

/// (week of year, week year) as ICU's Calendar computes them.
fn week_of_year(f: &Fields, first: u32, min_days: u32) -> (u32, i64) {
    let dow_rel = |wd: u32| (wd + 7 - first) % 7; // 0 = first day of week
    let jan1_dow = (f.weekday + 7 * 60 - (f.yday - 1) % 7) % 7;
    let jan1_rel = dow_rel(jan1_dow);
    // days of the first (partial) week in this year
    let first_week_days = 7 - jan1_rel;
    let week1_start = if first_week_days >= min_days { 1i64 - jan1_rel as i64 } else { 1 + first_week_days as i64 };
    let yday = f.yday as i64;
    if yday < week1_start {
        // belongs to the last week of the previous year
        let prev_len = days_in_year(f.year - 1) as i64;
        let prev = Fields { year: f.year - 1, yday: (yday + prev_len) as u32, ..*f };
        return week_of_year(&prev, first, min_days);
    }
    let week = ((yday - week1_start) / 7 + 1) as u32;
    // the last days may be week 1 of the next year
    let len = days_in_year(f.year) as i64;
    let next_jan1_rel = (jan1_rel as i64 + len) % 7;
    let next_first_week_days = 7 - next_jan1_rel;
    if next_first_week_days >= min_days as i64 && yday > len - next_jan1_rel && next_jan1_rel != 0 {
        return (1, f.year + 1);
    }
    (week, f.year)
}

fn week_of_month(f: &Fields, first: u32, min_days: u32) -> u32 {
    let dow_rel = |wd: u32| (wd + 7 - first) % 7;
    let first_dow = (f.weekday + 7 * 10 - (f.day - 1) % 7) % 7;
    let rel = dow_rel(first_dow);
    let first_week_days = 7 - rel;
    let start: i64 = if first_week_days >= min_days { 1 - rel as i64 } else { 1 + first_week_days as i64 };
    if (f.day as i64) < start { 0 } else { ((f.day as i64 - start) / 7 + 1) as u32 }
}

// ---- patterns ----

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Lit(String),
    Field(char, usize),
}

/// ICU's pattern syntax: letters repeat into fields, quotes make literals.
pub fn parse_pattern(p: &str) -> Vec<Item> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let c: Vec<char> = p.chars().collect();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '\'' {
            if c.get(i + 1) == Some(&'\'') {
                lit.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < c.len() {
                if c[i] == '\'' {
                    if c.get(i + 1) == Some(&'\'') {
                        lit.push('\'');
                        i += 2;
                        continue;
                    }
                    break;
                }
                lit.push(c[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if ch.is_ascii_alphabetic() {
            if !lit.is_empty() {
                out.push(Item::Lit(std::mem::take(&mut lit)));
            }
            let mut n = 1;
            while c.get(i + n) == Some(&ch) {
                n += 1;
            }
            out.push(Item::Field(ch, n));
            i += n;
            continue;
        }
        lit.push(ch);
        i += 1;
    }
    if !lit.is_empty() {
        out.push(Item::Lit(lit));
    }
    out
}

// ---- the formatter ----

type Names = FixedCalendarDateTimeNames<Gregorian, CompositeFieldSet>;

pub struct Formatter {
    pub locale: String,
    pub date_type: i64,
    pub time_type: i64,
    pub calendar: i64,
    pub pattern: String,
    pub items: Vec<Item>,
    pub zone: Zone,
    pub lenient: bool,
    digits: [char; 10],
    /// ICU4X names for one-field patterns, by pattern.
    names: RefCell<HashMap<String, Option<Names>>>,
}

impl Clone for Formatter {
    fn clone(&self) -> Formatter {
        Formatter {
            locale: self.locale.clone(),
            date_type: self.date_type,
            time_type: self.time_type,
            calendar: self.calendar,
            pattern: self.pattern.clone(),
            items: self.items.clone(),
            zone: self.zone.clone(),
            lenient: self.lenient,
            digits: self.digits,
            names: RefCell::new(HashMap::new()),
        }
    }
}

/// The CLDR pattern for a date style and a time style (ICU4X's selection).
pub fn style_pattern(locale: &str, date: i64, time: i64) -> Option<String> {
    use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder, ZoneStyle};
    use icu_datetime::options::{Length, TimePrecision};
    let base = |s: i64| if s >= RELATIVE { s - RELATIVE } else { s };
    let (date, time) = (base(date), base(time));
    let mut b = FieldSetBuilder::new();
    let len_of = |s: i64| match s {
        FULL | LONG => Length::Long,
        MEDIUM => Length::Medium,
        _ => Length::Short,
    };
    if date != NONE {
        b.length = Some(len_of(date));
        b.date_fields = Some(if date == FULL { DateFields::YMDE } else { DateFields::YMD });
    } else {
        b.length = Some(len_of(time));
    }
    if time != NONE {
        b.time_precision = Some(if time == SHORT { TimePrecision::Minute } else { TimePrecision::Second });
        match time {
            FULL => b.zone_style = Some(ZoneStyle::SpecificLong),
            LONG => b.zone_style = Some(ZoneStyle::SpecificShort),
            _ => {}
        }
    }
    let fs = b.build_composite().ok()?;
    let loc = crate::icu4x_locale(locale);
    let f = icu_datetime::DateTimeFormatter::<CompositeFieldSet>::try_new((&loc).into(), fs).ok()?;
    let sample = ZonedDateTime::try_strict_from_str("2023-11-14T17:13:20-05:00[America/New_York]", Gregorian, icu_time::zone::IanaParser::new()).ok()?;
    let sample = ZonedDateTime { date: sample.date.to_any(), time: sample.time, zone: sample.zone };
    Some(requote(&f.format(&sample).pattern().to_string()))
}

/// ICU4X writes a pattern's literal letters unquoted ("y г."); ICU's
/// patterns (CLDR's) quote the Latin, Greek and Cyrillic ones ("y 'г'.").
fn requote(p: &str) -> String {
    let mut out = String::new();
    for item in parse_pattern(p) {
        match item {
            Item::Field(c, n) => out.push_str(&c.to_string().repeat(n)),
            Item::Lit(l) => {
                let mut run = String::new();
                let quoted = |c: char| c == '\'' || (c.is_alphabetic() && (c as u32) < 0x0530);
                let flush = |run: &mut String, out: &mut String| {
                    if !run.is_empty() {
                        out.push('\'');
                        out.push_str(&run.replace('\'', "''"));
                        out.push('\'');
                        run.clear();
                    }
                };
                for c in l.chars() {
                    if quoted(c) {
                        run.push(c);
                    } else {
                        flush(&mut run, &mut out);
                        out.push(c);
                    }
                }
                flush(&mut run, &mut out);
            }
        }
    }
    out
}

impl Formatter {
    pub fn new(locale: &str, date_type: i64, time_type: i64, zone: Zone, calendar: i64, pattern: Option<&str>) -> Result<Formatter, i64> {
        let pat = match pattern {
            Some(p) if !p.is_empty() => p.to_string(),
            _ if date_type == PATTERN => String::new(),
            _ if date_type == NONE && time_type == NONE => "yyyyMMdd hh:mm a".to_string(),
            _ => style_pattern(locale, date_type, time_type).ok_or(U_UNSUPPORTED_ERROR)?,
        };
        let digits = crate::numfmt::Symbols::for_locale(locale).digits;
        Ok(Formatter {
            locale: locale.to_string(),
            date_type,
            time_type,
            calendar,
            items: parse_pattern(&pat),
            pattern: pat,
            zone,
            lenient: true,
            digits,
            names: RefCell::new(HashMap::new()),
        })
    }

    pub fn set_pattern(&mut self, p: &str) {
        self.pattern = p.to_string();
        self.items = parse_pattern(p);
    }

    fn num(&self, v: i64, width: usize) -> String {
        let s = v.unsigned_abs().to_string();
        let mut out = String::new();
        if v < 0 {
            out.push('-');
        }
        for _ in s.len()..width {
            out.push(self.digits[0]);
        }
        for c in s.bytes() {
            out.push(self.digits[(c - b'0') as usize]);
        }
        out
    }

    /// A text field from CLDR via ICU4X (one-field pattern), or None.
    fn text(&self, pattern: &str, zdt: &ZonedDateTime<Gregorian, icu_time::TimeZoneInfo<icu_time::zone::models::AtTime>>) -> Option<String> {
        let mut cache = self.names.borrow_mut();
        let entry = cache.entry(pattern.to_string()).or_insert_with(|| {
            let loc = crate::icu4x_locale(&self.locale);
            let mut names = Names::try_new((&loc).into()).ok()?;
            let p = DateTimePattern::try_from_pattern_str(pattern).ok()?;
            names.include_for_pattern(&p).ok()?;
            Some(names)
        });
        let names = entry.as_ref()?;
        let p = DateTimePattern::try_from_pattern_str(pattern).ok()?;
        let formatted = names.with_pattern_unchecked(&p).format(zdt);
        let out = writeable::TryWriteable::try_write_to_string(&formatted).ok()?.into_owned();
        Some(out)
    }

    fn zdt(&self, f: &Fields) -> Option<ZonedDateTime<Gregorian, icu_time::TimeZoneInfo<icu_time::zone::models::AtTime>>> {
        let off = f.offset;
        let sign = if off < 0 { '-' } else { '+' };
        let a = off.unsigned_abs();
        let id = match &self.zone {
            Zone::Iana(tz) => tz.name().to_string(),
            Zone::Fixed(_) => "Etc/Unknown".to_string(),
        };
        let year = if (0..=9999).contains(&f.year) { format!("{:04}", f.year) } else { format!("{}{:06}", if f.year < 0 { '-' } else { '+' }, f.year.unsigned_abs()) };
        let s = format!("{year}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}[{id}]", f.month, f.day, f.hour, f.minute, f.second, a / 3600, a / 60 % 60);
        ZonedDateTime::try_strict_from_str(&s, Gregorian, icu_time::zone::IanaParser::new()).ok()
    }

    /// Format the instant `ms` (UTC milliseconds).
    pub fn format_ms(&self, ms: i64) -> Option<String> {
        let f = fields_at(ms, &self.zone)?;
        let zdt = self.zdt(&f);
        let (first, min_days) = week_rules(&self.locale);
        let mut out = String::new();
        for item in &self.items {
            match item {
                Item::Lit(l) => out.push_str(l),
                Item::Field(c, n) => out.push_str(&self.field(*c, *n, &f, ms, zdt.as_ref(), first, min_days)),
            }
        }
        Some(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn field(&self, c: char, n: usize, f: &Fields, ms: i64, zdt: Option<&ZonedDateTime<Gregorian, icu_time::TimeZoneInfo<icu_time::zone::models::AtTime>>>, first: u32, min_days: u32) -> String {
        let text = |p: &str| zdt.and_then(|z| self.text(p, z));
        let era_year = if f.year > 0 { f.year } else { 1 - f.year };
        let rep = |ch: char, k: usize| ch.to_string().repeat(k);
        match c {
            'G' => text(&rep('G', n.min(5))).unwrap_or_else(|| if f.year > 0 { "AD".into() } else { "BC".into() }),
            'y' => {
                if n == 2 {
                    self.num(era_year % 100, 2)
                } else {
                    self.num(era_year, n)
                }
            }
            'Y' => {
                let (_, wy) = week_of_year(f, first, min_days);
                if n == 2 { self.num(wy.rem_euclid(100), 2) } else { self.num(wy, n) }
            }
            'u' => self.num(f.year, n),
            'r' => self.num(f.year, n),
            'U' => self.num(f.year, n),
            'Q' | 'q' => {
                let q = (f.month - 1) / 3 + 1;
                match n {
                    1 | 2 => self.num(q as i64, n),
                    _ => format!("Q{q}"),
                }
            }
            'M' | 'L' => match n {
                1 | 2 => self.num(f.month as i64, n),
                _ => text(&rep(c, n.min(5))).unwrap_or_else(|| self.num(f.month as i64, 2)),
            },
            'l' => String::new(),
            'w' => self.num(week_of_year(f, first, min_days).0 as i64, n),
            'W' => self.num(week_of_month(f, first, min_days) as i64, n),
            'd' => self.num(f.day as i64, n),
            'D' => self.num(f.yday as i64, n),
            'F' => self.num(((f.day - 1) / 7 + 1) as i64, n),
            'g' => self.num(ms.div_euclid(86_400_000) + 2_440_588, n),
            'E' => text(&rep('E', n.clamp(3, 6))).unwrap_or_default(),
            'e' | 'c' => {
                if n <= 2 {
                    let local = (f.weekday + 7 - first) % 7 + 1;
                    self.num(local as i64, n)
                } else {
                    let p = if c == 'c' { rep('c', n.min(6)) } else { rep('E', n.min(6)) };
                    text(&p).or_else(|| text(&rep('E', n.min(6)))).unwrap_or_default()
                }
            }
            'a' => text(&rep('a', n.min(5))).unwrap_or_else(|| if f.hour < 12 { "AM".into() } else { "PM".into() }),
            'b' | 'B' => text(&rep(c, n.min(5))).unwrap_or_else(|| if f.hour < 12 { "AM".into() } else { "PM".into() }),
            'h' => self.num(if f.hour.is_multiple_of(12) { 12 } else { (f.hour % 12) as i64 }, n),
            'H' => self.num(f.hour as i64, n),
            'k' => self.num(if f.hour == 0 { 24 } else { f.hour as i64 }, n),
            'K' => self.num((f.hour % 12) as i64, n),
            'm' => self.num(f.minute as i64, n),
            's' => self.num(f.second as i64, n),
            'S' => {
                let ms3 = format!("{:03}", f.millis);
                let mut d: String = ms3.chars().take(n).collect();
                while d.len() < n {
                    d.push('0');
                }
                d.bytes().map(|b| self.digits[(b - b'0') as usize]).collect()
            }
            'A' => self.num(((f.hour * 3600 + f.minute * 60 + f.second) * 1000 + f.millis) as i64, n),
            'Z' => match n {
                1..=3 => iso_offset(f.offset, false, true, false),
                4 => self.localized_gmt(f.offset, true, zdt),
                _ => iso_offset(f.offset, true, true, true),
            },
            'O' => self.localized_gmt(f.offset, n >= 4, zdt),
            'X' | 'x' => {
                let z = c == 'X';
                match n {
                    1 => iso_hours(f.offset, z),
                    2 => iso_offset(f.offset, false, false, z),
                    3 => iso_offset(f.offset, true, false, z),
                    4 => iso_offset(f.offset, false, true, z),
                    _ => iso_offset(f.offset, true, true, z),
                }
            }
            'z' => {
                let p = if n >= 4 { "zzzz" } else { "z" };
                text(p).unwrap_or_else(|| self.localized_gmt(f.offset, n >= 4, zdt))
            }
            'v' => {
                let p = if n >= 4 { "vvvv" } else { "v" };
                text(p).unwrap_or_else(|| self.localized_gmt(f.offset, n >= 4, zdt))
            }
            'V' => match n {
                1 => text("V").unwrap_or_else(|| "unk".into()),
                2 => self.zone.id(),
                3 => text("VVV").unwrap_or_else(|| "Unknown City".into()),
                _ => text("VVVV").unwrap_or_else(|| self.localized_gmt(f.offset, true, zdt)),
            },
            _ => String::new(),
        }
    }

    fn localized_gmt(&self, off: i32, long: bool, zdt: Option<&ZonedDateTime<Gregorian, icu_time::TimeZoneInfo<icu_time::zone::models::AtTime>>>) -> String {
        if let Some(z) = zdt
            && let Some(t) = self.text(if long && off != 0 { "OOOO" } else { "O" }, z)
            && matches!(self.zone, Zone::Iana(_))
        {
            // the zero offset is the bare GMT format ("GMT", "UTC")
            return if off == 0 { t.trim_end_matches("+0").trim_end_matches("+00:00").to_string() } else { t };
        }
        if off == 0 {
            return "GMT".into();
        }
        let a = off.unsigned_abs();
        let sign = if off < 0 { '-' } else { '+' };
        let (h, m) = (a / 3600, a / 60 % 60);
        if long {
            format!("GMT{sign}{h:02}:{m:02}")
        } else if m == 0 {
            format!("GMT{sign}{h}")
        } else {
            format!("GMT{sign}{h}:{m:02}")
        }
    }

    /// Parse `text` from byte `start` with the pattern: the instant (ms) and where parsing stopped.
    pub fn parse(&self, text: &str, start: usize) -> Result<(i64, usize, Parsed), usize> {
        let mut p = Parsed::default();
        let mut i = start;
        let items = &self.items;
        for (k, item) in items.iter().enumerate() {
            match item {
                Item::Lit(l) => {
                    let rest = &text[i..];
                    if let Some(r) = rest.strip_prefix(l.as_str()) {
                        i += rest.len() - r.len();
                    } else if self.lenient {
                        // whitespace and punctuation are matched loosely
                        let want: String = l.chars().filter(|c| !c.is_whitespace() && *c != '\u{202f}' && *c != '\u{a0}').collect();
                        let r = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{202f}' || c == '\u{a0}');
                        let r = r.strip_prefix(want.as_str()).ok_or(i)?;
                        let r = r.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{202f}' || c == '\u{a0}');
                        i += rest.len() - r.len();
                    } else {
                        return Err(i);
                    }
                }
                Item::Field(c, n) => {
                    let next_is_numeric = matches!(items.get(k + 1), Some(Item::Field(nc, _)) if "yMdHhKkmsSDuw".contains(*nc));
                    i = self.parse_field(text, i, *c, *n, &mut p, next_is_numeric)?;
                }
            }
        }
        let ms = p.to_ms(&self.zone).ok_or(i)?;
        Ok((ms, i, p))
    }

    fn parse_field(&self, text: &str, i: usize, c: char, n: usize, p: &mut Parsed, abut: bool) -> Result<usize, usize> {
        let rest = &text[i..];
        let digit = |ch: char| ch.to_digit(10).or_else(|| self.digits.iter().position(|&d| d == ch).map(|d| d as u32));
        let num = |max: usize| -> Option<(i64, usize)> {
            let mut v: i64 = 0;
            let mut used = 0;
            let mut count = 0;
            let (neg, body) = match rest.strip_prefix('-') {
                Some(b) if c == 'u' || c == 'y' => (true, b),
                _ => (false, rest),
            };
            for ch in body.chars() {
                if count >= max {
                    break;
                }
                let Some(d) = digit(ch) else { break };
                v = v.checked_mul(10)?.checked_add(d as i64)?;
                used += ch.len_utf8();
                count += 1;
            }
            if count == 0 {
                return None;
            }
            Some((if neg { -v } else { v }, used + neg as usize))
        };
        let max = if abut { n.max(1) } else { usize::MAX };
        match c {
            'y' | 'u' | 'Y' => {
                let (v, used) = num(max).ok_or(i)?;
                let digits = text[i..i + used].trim_start_matches('-').chars().count();
                p.year = Some(if n == 2 && digits == 2 && c != 'u' {
                    // two-digit years: within 80 years before and 20 after now
                    let now = crate::datefmt::now_utc().year() as i64;
                    let base = now - 80;
                    let mut y = base - base.rem_euclid(100) + v;
                    if y < base {
                        y += 100;
                    }
                    y
                } else {
                    v
                });
                Ok(i + used)
            }
            'M' | 'L' if n <= 2 => {
                let (v, used) = num(max).ok_or(i)?;
                p.month = Some(v as u32);
                Ok(i + used)
            }
            'M' | 'L' => {
                for m in 1..=12u32 {
                    let f = Fields { year: 2023, month: m, day: 1, hour: 12, minute: 0, second: 0, millis: 0, weekday: 0, yday: 1, offset: 0, dst: 0 };
                    let Some(z) = self.zdt(&f) else { continue };
                    for len in [n.min(5), 4, 3] {
                        if let Some(name) = self.text(&c.to_string().repeat(len), &z)
                            && !name.is_empty()
                            && rest.to_lowercase().starts_with(&name.to_lowercase())
                        {
                            p.month = Some(m);
                            return Ok(i + name.len());
                        }
                    }
                }
                Err(i)
            }
            'E' | 'c' | 'e' if n >= 3 || c == 'E' => {
                for d in 0..7u32 {
                    let f = Fields { year: 2023, month: 1, day: 1 + d, hour: 12, minute: 0, second: 0, millis: 0, weekday: d, yday: 1 + d, offset: 0, dst: 0 };
                    let Some(z) = self.zdt(&f) else { continue };
                    for len in [n.clamp(3, 6), 4, 3] {
                        if let Some(name) = self.text(&"E".repeat(len), &z)
                            && !name.is_empty()
                            && rest.to_lowercase().starts_with(&name.to_lowercase())
                        {
                            return Ok(i + name.len());
                        }
                    }
                }
                Err(i)
            }
            'a' | 'b' | 'B' => {
                for (h, pm) in [(9, false), (15, true)] {
                    let f = Fields { year: 2023, month: 1, day: 1, hour: h, minute: 0, second: 0, millis: 0, weekday: 0, yday: 1, offset: 0, dst: 0 };
                    let Some(z) = self.zdt(&f) else { continue };
                    if let Some(name) = self.text("a", &z)
                        && rest.to_lowercase().starts_with(&name.to_lowercase())
                    {
                        p.pm = Some(pm);
                        return Ok(i + name.len());
                    }
                }
                Err(i)
            }
            'G' => {
                for (name, ad) in [("AD", true), ("BC", false), ("Anno Domini", true), ("Before Christ", false)] {
                    if rest.starts_with(name) {
                        p.bc = !ad;
                        return Ok(i + name.len());
                    }
                }
                Err(i)
            }
            'd' => {
                let (v, used) = num(max.min(2).max(if abut { n } else { 2 })).ok_or(i)?;
                p.day = Some(v as u32);
                Ok(i + used)
            }
            'D' => {
                let (v, used) = num(max).ok_or(i)?;
                p.yday = Some(v as u32);
                Ok(i + used)
            }
            'H' | 'k' => {
                let (v, used) = num(max.min(2).max(if abut { n } else { 2 })).ok_or(i)?;
                p.hour = Some(if c == 'k' && v == 24 { 0 } else { v as u32 });
                Ok(i + used)
            }
            'h' | 'K' => {
                let (v, used) = num(max.min(2).max(if abut { n } else { 2 })).ok_or(i)?;
                p.hour12 = Some(if c == 'h' && v == 12 { 0 } else { v as u32 });
                Ok(i + used)
            }
            'm' => {
                let (v, used) = num(max.min(2).max(if abut { n } else { 2 })).ok_or(i)?;
                p.minute = Some(v as u32);
                Ok(i + used)
            }
            's' => {
                let (v, used) = num(max.min(2).max(if abut { n } else { 2 })).ok_or(i)?;
                p.second = Some(v as u32);
                Ok(i + used)
            }
            'S' => {
                let (v, used) = num(max).ok_or(i)?;
                let digits = text[i..i + used].chars().count() as u32;
                let ms = if digits >= 3 { v / 10i64.pow(digits - 3) } else { v * 10i64.pow(3 - digits) };
                p.millis = Some(ms as u32);
                Ok(i + used)
            }
            'Z' | 'X' | 'x' | 'O' | 'z' | 'v' => {
                let r = rest.trim_start_matches("GMT").trim_start_matches("UTC");
                let skipped = rest.len() - r.len();
                if r.starts_with('Z') && c != 'x' {
                    p.offset = Some(0);
                    return Ok(i + skipped + 1);
                }
                let off_len = r.char_indices().take_while(|(k, ch)| (*k == 0 && (*ch == '+' || *ch == '-')) || (*k > 0 && (ch.is_ascii_digit() || *ch == ':'))).count();
                if off_len > 1
                    && let Some(off) = parse_offset(&r[..off_len])
                {
                    p.offset = Some(off);
                    return Ok(i + skipped + off_len);
                }
                if skipped > 0 {
                    p.offset = Some(0);
                    return Ok(i + skipped);
                }
                // a zone name: the formatter's own zone's names
                Err(i)
            }
            _ => Err(i),
        }
    }
}

fn iso_hours(off: i32, z: bool) -> String {
    if off == 0 && z {
        return "Z".into();
    }
    let a = off.unsigned_abs();
    let sign = if off < 0 { '-' } else { '+' };
    if a.is_multiple_of(3600) { format!("{sign}{:02}", a / 3600) } else { format!("{sign}{:02}{:02}", a / 3600, a / 60 % 60) }
}

fn iso_offset(off: i32, colon: bool, _seconds_optional: bool, z: bool) -> String {
    if off == 0 && z {
        return "Z".into();
    }
    let a = off.unsigned_abs();
    let sign = if off < 0 { '-' } else { '+' };
    if colon { format!("{sign}{:02}:{:02}", a / 3600, a / 60 % 60) } else { format!("{sign}{:02}{:02}", a / 3600, a / 60 % 60) }
}

#[derive(Default, Debug, Clone)]
pub struct Parsed {
    pub year: Option<i64>,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub yday: Option<u32>,
    pub hour: Option<u32>,
    pub hour12: Option<u32>,
    pub pm: Option<bool>,
    pub minute: Option<u32>,
    pub second: Option<u32>,
    pub millis: Option<u32>,
    pub offset: Option<i32>,
    pub bc: bool,
}

impl Parsed {
    fn hour(&self) -> u32 {
        match (self.hour, self.hour12, self.pm) {
            (Some(h), _, _) => h,
            (None, Some(h), Some(true)) => h + 12,
            (None, Some(h), _) => h,
            (None, None, Some(true)) => 12,
            _ => 0,
        }
    }

    /// The instant: missing fields are ICU's defaults (1970-01-01 00:00:00).
    pub fn to_ms(&self, zone: &Zone) -> Option<i64> {
        let mut year = self.year.unwrap_or(1970);
        if self.bc {
            year = 1 - year;
        }
        let date = match (self.yday, self.month) {
            (Some(yd), None) => chrono::NaiveDate::from_yo_opt(year as i32, yd)?,
            _ => {
                // lenient: overflowing days and months roll over
                let month0 = self.month.unwrap_or(1) as i64 - 1;
                let y = year + month0.div_euclid(12);
                let m = month0.rem_euclid(12) as u32 + 1;
                let base = chrono::NaiveDate::from_ymd_opt(y as i32, m, 1)?;
                base.checked_add_signed(chrono::Duration::days(self.day.unwrap_or(1) as i64 - 1))?
            }
        };
        let secs = self.hour() as i64 * 3600 + self.minute.unwrap_or(0) as i64 * 60 + self.second.unwrap_or(0) as i64;
        let local_ms = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis() + secs * 1000 + self.millis.unwrap_or(0) as i64;
        let offset = match self.offset {
            Some(o) => o,
            None => {
                // the zone's offset at that local time
                let guess = zone.offsets_at(local_ms).0;
                zone.offsets_at(local_ms - guess as i64 * 1000).0
            }
        };
        Some(local_ms - offset as i64 * 1000)
    }
}

// ---- the PHP class ----

#[derive(Default)]
pub struct State {
    pub fmt: RefCell<Option<Formatter>>,
    pub err: IntlError,
}

fn state(o: &Object) -> &State {
    o.native::<State>().expect("IntlDateFormatter state")
}

pub fn register(registry: &mut Registry) {
    registry.add_data("IntlDateFormatter", || Rc::new(State::default()));
    registry.add_hooks(
        "IntlDateFormatter",
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
    b!("datefmt_create", f_create),
    b!("datefmt_get_datetype", f_get_datetype),
    b!("datefmt_get_timetype", f_get_timetype),
    b!("datefmt_get_calendar", f_get_calendar),
    b!("datefmt_set_calendar", f_set_calendar),
    b!("datefmt_get_timezone_id", f_get_timezone_id),
    b!("datefmt_get_timezone", f_get_timezone),
    b!("datefmt_set_timezone", f_set_timezone),
    b!("datefmt_set_pattern", f_set_pattern),
    b!("datefmt_get_pattern", f_get_pattern),
    b!("datefmt_get_locale", f_get_locale),
    b!("datefmt_set_lenient", f_set_lenient),
    b!("datefmt_is_lenient", f_is_lenient),
    b!("datefmt_format", f_format),
    b!("datefmt_format_object", f_format_object),
    Builtin { name: "datefmt_parse", run: f_parse, by_ref: &[2] },
    Builtin { name: "datefmt_localtime", run: f_localtime, by_ref: &[2] },
    b!("datefmt_get_error_code", f_get_error_code),
    b!("datefmt_get_error_message", f_get_error_message),
];

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "IntlDateFormatter",
    parent: None,
    interfaces: &[],
    flags: p8_runtime::object::CLASS_NOT_SERIALIZABLE,
    properties: &[],
    methods: crate::methods! {
        "IntlDateFormatter";
        statics: [("create", create_static), ("formatObject", format_object_static)];
        "__construct" => m_construct;
        "getDateType" => f_get_datetype;
        "getTimeType" => f_get_timetype;
        "getCalendar" => f_get_calendar;
        "setCalendar" => f_set_calendar;
        "getTimeZoneId" => f_get_timezone_id;
        "getTimeZone" => f_get_timezone;
        "setTimeZone" => f_set_timezone;
        "setPattern" => f_set_pattern;
        "getPattern" => f_get_pattern;
        "getLocale" => f_get_locale;
        "setLenient" => f_set_lenient;
        "isLenient" => f_is_lenient;
        "format" => f_format;
        "parse" => f_parse;
        "localtime" => f_localtime;
        "getErrorCode" => f_get_error_code;
        "getErrorMessage" => f_get_error_message;
    },
    constants: crate::consts::INTL_DATE_FORMATTER,
    debug_info: None,
}];

fn create_static(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    prep(ctx, "IntlDateFormatter::create", args)?;
    f_create(ctx, args)
}

fn format_object_static(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    prep(ctx, "IntlDateFormatter::formatObject", args)?;
    f_format_object(ctx, args)
}

/// datefmt_ctor: Ok(Ok) constructed; Ok(Err) failed (the global error says why).
fn construct(ctx: &mut dyn Ctx, st: &State, args: &[Value]) -> Result<Result<(), ()>, Throw> {
    crate::reset_global(ctx);
    let locale_arg = crate::opt_str(args, 0).unwrap_or_default();
    let date_type = if args.len() > 1 { arg(args, 1).to_int() } else { FULL };
    let time_type = if args.len() > 2 { arg(args, 2).to_int() } else { FULL };
    let tz = arg(args, 3);
    let cal = arg(args, 4);
    let pattern = crate::opt_str(args, 5);
    let fail = |ctx: &mut dyn Ctx, code: i64, msg: &str| -> Result<Result<(), ()>, Throw> {
        crate::set_error(ctx, None, code, msg)?;
        Ok(Err(()))
    };
    if st.fmt.borrow().is_some() {
        st.err.set(U_ILLEGAL_ARGUMENT_ERROR, Some("datefmt_create: cannot call constructor twice"));
        return fail(ctx, U_ILLEGAL_ARGUMENT_ERROR, "datefmt_create: cannot call constructor twice");
    }
    if !style_ok(date_type) {
        return fail(ctx, U_ILLEGAL_ARGUMENT_ERROR, "datefmt_create: invalid date format style");
    }
    if !style_ok(time_type) {
        return fail(ctx, U_ILLEGAL_ARGUMENT_ERROR, "datefmt_create: invalid time format style");
    }
    if date_type == PATTERN && time_type != PATTERN {
        return fail(ctx, U_ILLEGAL_ARGUMENT_ERROR, "datefmt_create: time format must be IntlDateFormatter::PATTERN if date format is IntlDateFormatter::PATTERN");
    }
    if locale_arg.len() > crate::locale::MAX_LOCALE_LEN {
        return fail(ctx, U_ILLEGAL_ARGUMENT_ERROR, &format!("Locale string too long, should be no longer than {} characters", crate::locale::MAX_LOCALE_LEN));
    }
    let locale = if locale_arg.is_empty() { default_locale(ctx) } else { locale_arg.clone() };
    if (locale_arg.len() == 1 && locale_arg != "C") || (locale_arg.len() > 1 && !crate::numfmt::known_language(&locale)) {
        let func = crate::called("datefmt_create");
        let func = if func == "IntlDateFormatter::create" || func == "datefmt_create" { func } else { "IntlDateFormatter::__construct" };
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($locale) \"{locale_arg}\" is invalid")));
    }
    let canonical = crate::uloc::canonicalize(&locale);
    let locale = if canonical.is_empty() { locale } else { canonical };
    let calendar = match cal {
        Value::Null | Value::Undef => GREGORIAN,
        Value::Int(c) if c == GREGORIAN || c == TRADITIONAL => c,
        Value::Int(_) => {
            return fail(
                ctx,
                U_ILLEGAL_ARGUMENT_ERROR,
                "datefmt_create: Invalid value for calendar type; it must be one of IntlDateFormatter::TRADITIONAL (locale's default calendar) or IntlDateFormatter::GREGORIAN. Alternatively, it can be an IntlCalendar object",
            );
        }
        _ => return fail(ctx, U_UNSUPPORTED_ERROR, "datefmt_create: IntlCalendar arguments are not supported"),
    };
    if calendar == TRADITIONAL && !gregorian_locale(&locale) {
        return fail(ctx, U_UNSUPPORTED_ERROR, "datefmt_create: date formatter creation failed");
    }
    let zone = match zone_arg(ctx, &tz, "datefmt_create")? {
        Ok(z) => z,
        Err(msg) => return fail(ctx, U_ILLEGAL_ARGUMENT_ERROR, &msg),
    };
    match Formatter::new(&locale, date_type, time_type, zone, calendar, pattern.as_deref()) {
        Ok(f) => {
            *st.fmt.borrow_mut() = Some(f);
            Ok(Ok(()))
        }
        Err(code) => fail(ctx, code, "datefmt_create: date formatter creation failed"),
    }
}

/// Whether the locale's traditional calendar is the Gregorian one.
fn gregorian_locale(locale: &str) -> bool {
    let p = crate::uloc::parts(locale);
    if let Some((_, c)) = p.keywords.iter().find(|(k, _)| k == "calendar") {
        return c == "gregorian";
    }
    !matches!(p.region.as_str(), "TH" | "SA" | "AF" | "IR") && !(p.lang == "th" && p.region.is_empty()) && !(p.lang == "fa" && p.region.is_empty())
}

fn f_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let class = ctx.class(b"IntlDateFormatter").expect("IntlDateFormatter");
    let o = ctx.new_object(&class);
    match construct(ctx, state(&o), args)? {
        Ok(()) => Ok(Value::Object(o)),
        Err(()) => Ok(Value::Null),
    }
}

fn m_construct(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(this) = arg(args, 0) else { unreachable!() };
    match construct(ctx, state(&this), &args[1..])? {
        Ok(()) => Ok(Value::Null),
        Err(()) => {
            let msg = crate::global_message(ctx);
            Err(ctx.throw("IntlException", msg))
        }
    }
}

fn obj(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Rc<Object>, Throw> {
    let Value::Object(o) = arg(args, 0) else { unreachable!("checked against the stub") };
    if state(&o).fmt.borrow().is_none() {
        return Err(ctx.throw("Error", "Found unconstructed IntlDateFormatter".into()));
    }
    state(&o).err.reset();
    Ok(o)
}

fn check(ctx: &mut dyn Ctx, st: &State, msg: &str) -> Result<bool, Throw> {
    let code = st.err.code.get();
    crate::set_global_code(ctx, code);
    if code > U_ZERO_ERROR {
        crate::set_message(ctx, Some(&st.err), msg)?;
        return Ok(false);
    }
    Ok(true)
}

fn with<T>(o: &Object, f: impl FnOnce(&mut Formatter) -> T) -> T {
    let st = state(o);
    let mut b = st.fmt.borrow_mut();
    f(b.as_mut().expect("constructed"))
}

fn f_get_datetype(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(Value::Int(with(&o, |f| f.date_type)))
}

fn f_get_timetype(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(Value::Int(with(&o, |f| f.time_type)))
}

fn f_get_calendar(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(Value::Int(with(&o, |f| f.calendar)))
}

fn f_set_calendar(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    match arg(args, 1) {
        Value::Null => {
            with(&o, |f| f.calendar = GREGORIAN);
        }
        Value::Int(c) if c == GREGORIAN || (c == TRADITIONAL && with(&o, |f| gregorian_locale(&f.locale))) => {
            with(&o, |f| f.calendar = c);
        }
        Value::Int(c) if c == TRADITIONAL => {
            st.err.code.set(U_UNSUPPORTED_ERROR);
            check(ctx, st, "datefmt_set_calendar: error creating ICU Calendar object")?;
            return Ok(Value::Bool(false));
        }
        Value::Int(_) => {
            st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
            check(
                ctx,
                st,
                "datefmt_set_calendar: Invalid value for calendar type; it must be one of IntlDateFormatter::TRADITIONAL (locale's default calendar) or IntlDateFormatter::GREGORIAN. Alternatively, it can be an IntlCalendar object",
            )?;
            return Ok(Value::Bool(false));
        }
        _ => {
            st.err.code.set(U_UNSUPPORTED_ERROR);
            check(ctx, st, "datefmt_set_calendar: IntlCalendar arguments are not supported")?;
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn f_get_timezone_id(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(s(with(&o, |f| f.zone.id())))
}

fn f_get_timezone(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let zone = with(&o, |f| f.zone.clone());
    Ok(crate::timezone::new_object(ctx, zone))
}

fn f_set_timezone(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    match zone_arg(ctx, &arg(args, 1), "datefmt_set_timezone")? {
        Ok(z) => {
            with(&o, |f| f.zone = z);
            Ok(Value::Bool(true))
        }
        Err(msg) => {
            st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
            check(ctx, st, &msg)?;
            Ok(Value::Bool(false))
        }
    }
}

fn f_get_pattern(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(s(with(&o, |f| f.pattern.clone())))
}

fn f_set_pattern(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let p = str_arg(args, 1);
    with(&o, |f| f.set_pattern(&p));
    Ok(Value::Bool(true))
}

fn f_get_locale(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let ty = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    if ty != 0 && ty != 1 {
        st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
        check(ctx, st, "Error getting locale")?;
        return Ok(Value::Bool(false));
    }
    let loc = with(&o, |f| crate::valid_locale(&f.locale));
    // ICU's actual locale for date formats is the language data's
    let loc = if ty == 0 { crate::uloc::subtags(&loc).lang } else { loc };
    Ok(s(loc))
}

fn f_set_lenient(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let l = arg(args, 1).to_bool();
    with(&o, |f| f.lenient = l);
    Ok(Value::Null)
}

fn f_is_lenient(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(Value::Bool(with(&o, |f| f.lenient)))
}

/// The instant a format() argument stands for, in ms; Err(message) when it is not one.
fn instant(ctx: &mut dyn Ctx, v: &Value, func: &str) -> Result<Result<i64, String>, Throw> {
    match v {
        Value::Int(i) => Ok(Ok(i.saturating_mul(1000))),
        Value::Float(f) => Ok(Ok((f * 1000.0) as i64)),
        Value::Str(st) => {
            let t = lossy(st.as_bytes());
            match t.trim().parse::<f64>() {
                Ok(f) if p8_runtime::numeric::numeric_string(st.as_bytes()).is_some() => Ok(Ok((f * 1000.0) as i64)),
                _ => Ok(Err(format!("{func}: string '{t}' is not numeric, which would be required for it to be a valid date"))),
            }
        }
        Value::Array(a) => Ok(array_instant(a).ok_or_else(|| format!("{func}: date format failed"))),
        Value::Object(_) => {
            let mut fmt = [s("U.v")];
            let r = ctx.call(&Value::Array(Rc::new(PhpArray::from_list(vec![v.clone(), s("format")]))), &mut fmt)?;
            let t = r.to_php_string().map(|s| lossy(s.as_bytes())).unwrap_or_default();
            Ok(t.parse::<f64>().map(|f| (f * 1000.0).round() as i64).map_err(|_| format!("{func}: error getting time from object")))
        }
        _ => Ok(Err(format!("{func}: takes either an array or an integer timestamp value or a DateTime object"))),
    }
}

/// A localtime()-style array as an instant (its fields read in UTC, as ICU's calendar in PHP does).
fn array_instant(a: &PhpArray) -> Option<i64> {
    let get = |k: &str| a.get(&Key::from_string(PhpStr::from(k.as_bytes()))).map(|v| v.deref().to_int()).unwrap_or(0);
    let year = get("tm_year") + 1900;
    let date = chrono::NaiveDate::from_ymd_opt(year as i32, 1, 1)?;
    let date = date.checked_add_months(chrono::Months::new(get("tm_mon").clamp(0, 1200) as u32))?;
    let date = date.checked_add_signed(chrono::Duration::days(get("tm_mday") - 1))?;
    let secs = get("tm_hour") * 3600 + get("tm_min") * 60 + get("tm_sec");
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis() + secs * 1000)
}

fn f_format(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let v = arg(args, 1);
    let ms = match &v {
        Value::Array(a) => {
            // the array's fields are local to the formatter's zone
            match array_instant(a) {
                Some(local) => {
                    let zone = with(&o, |f| f.zone.clone());
                    let off = zone.offsets_at(local - zone.offsets_at(local).0 as i64 * 1000).0;
                    local - off as i64 * 1000
                }
                None => {
                    st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
                    check(ctx, st, "datefmt_format: date format failed")?;
                    return Ok(Value::Bool(false));
                }
            }
        }
        _ => match instant(ctx, &v, "datefmt_format")? {
            Ok(ms) => ms,
            Err(msg) => {
                st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
                check(ctx, st, &msg)?;
                return Ok(Value::Bool(false));
            }
        },
    };
    let out = with(&o, |f| {
        if f.date_type >= RELATIVE
            && let Some(r) = relative_day(f, ms)
        {
            return Some(r);
        }
        f.format_ms(ms)
    });
    match out {
        Some(t) => Ok(s(t)),
        None => {
            st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
            check(ctx, st, "datefmt_format: date format failed")?;
            Ok(Value::Bool(false))
        }
    }
}

/// RELATIVE_* styles: "today", "yesterday", "tomorrow" (CLDR via ICU4X) for those days, else the plain style.
fn relative_day(f: &Formatter, ms: i64) -> Option<String> {
    let now = crate::datefmt::now_utc().timestamp_millis();
    let a = fields_at(ms, &f.zone)?;
    let b = fields_at(now, &f.zone)?;
    let da = chrono::NaiveDate::from_ymd_opt(a.year as i32, a.month, a.day)?;
    let db = chrono::NaiveDate::from_ymd_opt(b.year as i32, b.month, b.day)?;
    let diff = (da - db).num_days();
    if !(-1..=1).contains(&diff) {
        return None;
    }
    let loc = crate::icu4x_locale(&f.locale);
    let mut opts = icu_experimental::relativetime::options::RelativeTimeFormatterOptions::default();
    opts.numeric = Some(icu_experimental::relativetime::options::Numeric::Auto);
    let rf = icu_experimental::relativetime::RelativeTimeFormatter::try_new_long_day((&loc).into(), opts).ok()?;
    let word = rf.format(fixed_decimal::Decimal::from(diff)).to_string();
    let mut word = word;
    // ICU capitalizes the word for standalone use in some locales; keep CLDR's form
    if f.time_type == NONE {
        return Some(word);
    }
    let time = style_pattern(&f.locale, NONE, f.time_type)?;
    let mut tf = f.clone();
    tf.set_pattern(&time);
    let t = tf.format_ms(ms)?;
    word.push_str(", ");
    word.push_str(&t);
    Some(word)
}

fn f_format_object(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::reset_global(ctx);
    let object = arg(args, 0);
    let format = arg(args, 1);
    let locale = crate::opt_str(args, 2).filter(|l| !l.is_empty()).unwrap_or_else(|| default_locale(ctx));
    let fail = |ctx: &mut dyn Ctx, msg: &str| -> R {
        crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("datefmt_format_object: {msg}"))?;
        Ok(Value::Bool(false))
    };
    match &object {
        Value::Object(o) if crate::msgfmt::is_datetime(o) => {}
        _ => return fail(ctx, "the passed object must be an instance of either IntlCalendar or DateTimeInterface"),
    }
    let ok = |s: i64| matches!(s, FULL | LONG | MEDIUM | SHORT | NONE) || (RELATIVE..=RELATIVE + 3).contains(&s);
    let (date_type, time_type, pattern) = match &format {
        Value::Null | Value::Undef => (MEDIUM, MEDIUM, None),
        Value::Int(i) => {
            if !ok(*i) {
                return fail(ctx, "the date/time format type is invalid");
            }
            (*i, *i, None)
        }
        Value::Array(a) => {
            let vals: Vec<Value> = a.values().map(Value::deref).collect();
            if vals.len() != 2 {
                return fail(ctx, "bad format; if array, it must have two elements");
            }
            if !ok(vals[0].to_int()) {
                return fail(ctx, "bad format; the date format (first element of the array) is not valid");
            }
            if !ok(vals[1].to_int()) {
                return fail(ctx, "bad format; the time format (second element of the array) is not valid");
            }
            (vals[0].to_int(), vals[1].to_int(), None)
        }
        v => {
            let p = v.to_php_string().map(|s| lossy(s.as_bytes())).unwrap_or_default();
            if p.is_empty() {
                return fail(ctx, "the format is empty");
            }
            (PATTERN, PATTERN, Some(p))
        }
    };
    // the object's own time zone
    let zone = {
        let tz = ctx.call(&Value::Array(Rc::new(PhpArray::from_list(vec![object.clone(), s("getTimezone")]))), &mut [])?;
        match tz {
            Value::Object(_) => match zone_arg(ctx, &tz, "datefmt_format_object")? {
                Ok(z) => z,
                Err(msg) => {
                    crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
                    return Ok(Value::Bool(false));
                }
            },
            _ => php_default_zone(ctx),
        }
    };
    let ms = match instant(ctx, &object, "datefmt_format_object")? {
        Ok(ms) => ms,
        Err(msg) => {
            crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
            return Ok(Value::Bool(false));
        }
    };
    let canonical = crate::uloc::canonicalize(&locale);
    let f = match Formatter::new(if canonical.is_empty() { &locale } else { &canonical }, date_type, time_type, zone, GREGORIAN, pattern.as_deref()) {
        Ok(f) => f,
        Err(code) => {
            crate::set_error(ctx, None, code, "datefmt_format_object: date formatter creation failed")?;
            return Ok(Value::Bool(false));
        }
    };
    match f.format_ms(ms) {
        Some(t) => Ok(s(t)),
        None => {
            crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "datefmt_format_object: error formatting date")?;
            Ok(Value::Bool(false))
        }
    }
}

fn set_ref(args: &mut [Value], i: usize, v: Value) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = v;
    }
}

fn f_parse(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let text = str_arg(args, 1);
    let has_pos = args.len() > 2;
    let start = if has_pos { (arg(args, 2).to_int().max(0) as usize).min(text.len()) } else { 0 };
    if has_pos && !text.is_char_boundary(start) {
        st.err.code.set(crate::U_PARSE_ERROR);
        check(ctx, st, "Date parsing failed")?;
        return Ok(Value::Bool(false));
    }
    let r = with(&o, |f| f.parse(&text, start));
    match r {
        Ok((ms, end, _)) if !has_pos && end != text.len() && !with(&o, |f| f.lenient) => {
            let _ = (ms, end);
            st.err.code.set(crate::U_PARSE_ERROR);
            check(ctx, st, "Date parsing failed")?;
            Ok(Value::Bool(false))
        }
        Ok((ms, end, _)) => {
            if has_pos {
                set_ref(args, 2, Value::Int(end as i64));
            }
            let secs = ms.div_euclid(1000);
            if ms % 1000 != 0 { Ok(Value::Float(ms as f64 / 1000.0)) } else { Ok(Value::Int(secs)) }
        }
        Err(at) => {
            if has_pos {
                set_ref(args, 2, Value::Int(at as i64));
            }
            st.err.code.set(crate::U_PARSE_ERROR);
            check(ctx, st, "Date parsing failed")?;
            Ok(Value::Bool(false))
        }
    }
}

fn f_localtime(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let text = str_arg(args, 1);
    let has_pos = args.len() > 2;
    let start = if has_pos { (arg(args, 2).to_int().max(0) as usize).min(text.len()) } else { 0 };
    let r = with(&o, |f| f.parse(&text, start).map(|(ms, end, _)| (ms, end, fields_at(ms, &f.zone))));
    match r {
        Ok((_, end, Some(fl))) => {
            if has_pos {
                set_ref(args, 2, Value::Int(end as i64));
            }
            let mut a = PhpArray::new();
            let mut put = |k: &str, v: i64| a.insert(Key::from_string(PhpStr::from(k.as_bytes())), Value::Int(v));
            put("tm_sec", fl.second as i64);
            put("tm_min", fl.minute as i64);
            put("tm_hour", fl.hour as i64);
            put("tm_year", fl.year - 1900);
            put("tm_mday", fl.day as i64);
            put("tm_wday", fl.weekday as i64);
            put("tm_yday", fl.yday as i64);
            put("tm_mon", fl.month as i64 - 1);
            put("tm_isdst", (fl.dst != 0) as i64);
            Ok(Value::Array(Rc::new(a)))
        }
        _ => {
            st.err.code.set(crate::U_PARSE_ERROR);
            check(ctx, st, "Date parsing failed")?;
            Ok(Value::Bool(false))
        }
    }
}

fn f_get_error_code(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(o) = arg(args, 0) else { unreachable!() };
    Ok(Value::Int(state(&o).err.code.get()))
}

fn f_get_error_message(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(o) = arg(args, 0) else { unreachable!() };
    Ok(s(state(&o).err.message()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weeks_follow_icu() {
        // 2023-11-14, a Tuesday: week 46 in en_US (Sunday, 1 day) and ISO weeks
        let f = Fields { year: 2023, month: 11, day: 14, hour: 0, minute: 0, second: 0, millis: 0, weekday: 2, yday: 318, offset: 0, dst: 0 };
        assert_eq!(week_of_year(&f, 0, 1), (46, 2023));
        assert_eq!(week_of_year(&f, 1, 4), (46, 2023));
        // 2021-01-01 (Friday) is in ISO week 53 of 2020
        let f = Fields { year: 2021, month: 1, day: 1, hour: 0, minute: 0, second: 0, millis: 0, weekday: 5, yday: 1, offset: 0, dst: 0 };
        assert_eq!(week_of_year(&f, 1, 4), (53, 2020));
        assert_eq!(parse_pattern("yyyy-MM-dd 'at' HH''mm"), vec![Item::Field('y', 4), Item::Lit("-".into()), Item::Field('M', 2), Item::Lit("-".into()), Item::Field('d', 2), Item::Lit(" at ".into()), Item::Field('H', 2), Item::Lit("'".into()), Item::Field('m', 2)]);
    }
}

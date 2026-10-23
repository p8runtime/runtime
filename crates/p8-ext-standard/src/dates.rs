//! Dates and times, the functions (the DateTime classes come later):
//! date, gmdate, idate, mktime, gmmktime, checkdate, getdate, localtime,
//! strtotime, date_default_timezone_get/set. Time zones are the IANA
//! database's (chrono-tz); the default is date.timezone, else UTC.
//!
//! Reference: $PHP_SRC_PATH/ext/date/php_date.c,
//! $PHP_SRC_PATH/ext/date/lib/parse_date.re (strtotime's formats)

use crate::{arg, str_arg};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike};
use chrono_tz::Tz;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "date", run: date, by_ref: &[] },
    Builtin { name: "gmdate", run: gmdate, by_ref: &[] },
    Builtin { name: "idate", run: idate, by_ref: &[] },
    Builtin { name: "mktime", run: mktime, by_ref: &[] },
    Builtin { name: "gmmktime", run: gmmktime, by_ref: &[] },
    Builtin { name: "checkdate", run: checkdate, by_ref: &[] },
    Builtin { name: "getdate", run: getdate, by_ref: &[] },
    Builtin { name: "localtime", run: localtime, by_ref: &[] },
    Builtin { name: "strtotime", run: strtotime, by_ref: &[] },
    Builtin { name: "date_default_timezone_get", run: date_default_timezone_get, by_ref: &[] },
    Builtin { name: "date_default_timezone_set", run: date_default_timezone_set, by_ref: &[] },
];

/// A zone: an IANA one, or a fixed offset (seconds east of UTC).
#[derive(Clone, Copy)]
pub(crate) enum Zone {
    Named(Tz),
    Fixed(i32),
}

/// A moment as a zone sees it.
pub(crate) struct Local {
    pub dt: NaiveDateTime,
    pub offset: i32,
    pub abbr: String,
    pub dst: bool,
}

impl Zone {
    pub(crate) fn at(self, ts: i64) -> Local {
        let utc = chrono::DateTime::from_timestamp(ts, 0).unwrap_or_default().naive_utc();
        match self {
            Zone::Named(tz) => {
                let o = tz.offset_from_utc_datetime(&utc);
                let offset = o.fix().local_minus_utc();
                let abbr = chrono_tz::OffsetName::abbreviation(&o).map(str::to_string).unwrap_or_else(|| fixed_name(offset, true));
                let dst = chrono_tz::OffsetComponents::dst_offset(&o).num_seconds() != 0;
                Local { dt: utc + chrono::Duration::seconds(i64::from(offset)), offset, abbr, dst }
            }
            Zone::Fixed(offset) => Local { dt: utc + chrono::Duration::seconds(i64::from(offset)), offset, abbr: fixed_name(offset, true), dst: false },
        }
    }

    /// The timestamp of a wall-clock time here (in a gap: moved on by the
    /// gap; when it repeats: the first).
    pub(crate) fn timestamp(self, dt: NaiveDateTime) -> i64 {
        match self {
            Zone::Fixed(offset) => dt.and_utc().timestamp() - i64::from(offset),
            Zone::Named(tz) => match tz.from_local_datetime(&dt) {
                chrono::LocalResult::Single(t) => t.timestamp(),
                chrono::LocalResult::Ambiguous(a, _) => a.timestamp(),
                chrono::LocalResult::None => {
                    let before = tz.offset_from_utc_datetime(&(dt - chrono::Duration::hours(3))).fix().local_minus_utc();
                    dt.and_utc().timestamp() - i64::from(before)
                }
            },
        }
    }
}

/// "+02:00" (colon) or "+0200".
fn fixed_name(offset: i32, colon: bool) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let a = offset.abs();
    if colon { format!("{sign}{:02}:{:02}", a / 3600, a / 60 % 60) } else { format!("{sign}{:02}{:02}", a / 3600, a / 60 % 60) }
}

/// The zone date() and friends use: date_default_timezone_set's, else
/// date.timezone, else UTC.
pub(crate) fn default_zone(ctx: &mut dyn Ctx) -> (Zone, String) {
    let name = match ctx.state("date_tz") {
        Value::Str(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
        _ => match ctx.ini_get(b"date.timezone") {
            Some(Value::Str(s)) if !s.is_empty() => String::from_utf8_lossy(s.as_bytes()).into_owned(),
            _ => "UTC".into(),
        },
    };
    match name.parse::<Tz>() {
        Ok(tz) => (Zone::Named(tz), name),
        Err(_) => (Zone::Named(Tz::UTC), "UTC".into()),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The optional timestamp argument `i` (null or missing: now).
fn timestamp_arg(args: &[Value], i: usize) -> i64 {
    match args.get(i).map(Value::deref) {
        None | Some(Value::Null) => now(),
        Some(v) => v.to_int(),
    }
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).and_then(|d| d.pred_opt()).map_or(30, |d| d.day())
}

/// A 12-hour clock's hour (12 for 0 and 12).
fn hour12(h: u32) -> u32 {
    match h % 12 {
        0 => 12,
        x => x,
    }
}

/// date()'s format, for a moment in a zone.
pub(crate) fn format(fmt: &[u8], ts: i64, l: &Local, zone_name: &str) -> Vec<u8> {
    let d = l.dt;
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        let s: String = match c {
            b'd' => format!("{:02}", d.day()),
            b'D' => DAYS[d.weekday().num_days_from_sunday() as usize][..3].into(),
            b'j' => d.day().to_string(),
            b'l' => DAYS[d.weekday().num_days_from_sunday() as usize].into(),
            b'N' => d.weekday().number_from_monday().to_string(),
            b'S' => match d.day() {
                1 | 21 | 31 => "st",
                2 | 22 => "nd",
                3 | 23 => "rd",
                _ => "th",
            }
            .into(),
            b'w' => d.weekday().num_days_from_sunday().to_string(),
            b'z' => d.ordinal0().to_string(),
            b'W' => format!("{:02}", d.iso_week().week()),
            b'F' => MONTHS[d.month0() as usize].into(),
            b'm' => format!("{:02}", d.month()),
            b'M' => MONTHS[d.month0() as usize][..3].into(),
            b'n' => d.month().to_string(),
            b't' => days_in_month(d.year(), d.month()).to_string(),
            b'L' => u8::from(NaiveDate::from_ymd_opt(d.year(), 2, 29).is_some()).to_string(),
            b'o' => d.iso_week().year().to_string(),
            b'X' | b'x' => {
                let y = d.year();
                if c == b'X' || !(0..10000).contains(&y) {
                    format!("{}{:04}", if y < 0 { "-" } else { "+" }, y.abs())
                } else {
                    format!("{y:04}")
                }
            }
            b'Y' => {
                let y = d.year();
                if y < 0 { format!("-{:04}", -y) } else { format!("{y:04}") }
            }
            b'y' => format!("{:02}", d.year().rem_euclid(100)),
            b'a' => if d.hour() < 12 { "am" } else { "pm" }.into(),
            b'A' => if d.hour() < 12 { "AM" } else { "PM" }.into(),
            b'B' => {
                let beat = ((ts + 3600).rem_euclid(86400) * 10 / 864) % 1000;
                format!("{beat:03}")
            }
            b'g' => hour12(d.hour()).to_string(),
            b'G' => d.hour().to_string(),
            b'h' => format!("{:02}", hour12(d.hour())),
            b'H' => format!("{:02}", d.hour()),
            b'i' => format!("{:02}", d.minute()),
            b's' => format!("{:02}", d.second()),
            b'u' => "000000".into(),
            b'v' => "000".into(),
            b'e' => zone_name.to_string(),
            b'I' => u8::from(l.dst).to_string(),
            b'O' => fixed_name(l.offset, false),
            b'P' => fixed_name(l.offset, true),
            b'p' => if l.offset == 0 { "Z".into() } else { fixed_name(l.offset, true) },
            b'T' => l.abbr.clone(),
            b'Z' => l.offset.to_string(),
            b'c' => format!("{}T{:02}:{:02}:{:02}{}", d.format("%Y-%m-%d"), d.hour(), d.minute(), d.second(), fixed_name(l.offset, true)),
            b'r' => format!(
                "{}, {:02} {} {:04} {:02}:{:02}:{:02} {}",
                &DAYS[d.weekday().num_days_from_sunday() as usize][..3],
                d.day(),
                &MONTHS[d.month0() as usize][..3],
                d.year(),
                d.hour(),
                d.minute(),
                d.second(),
                fixed_name(l.offset, false)
            ),
            b'U' => ts.to_string(),
            b'\\' => {
                if let Some(&next) = fmt.get(i) {
                    out.push(next);
                    i += 1;
                }
                continue;
            }
            other => {
                out.push(other);
                continue;
            }
        };
        out.extend_from_slice(s.as_bytes());
    }
    out
}

/// date(string $format, ?int $timestamp = null)
fn date(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let ts = timestamp_arg(args, 1);
    let (zone, name) = default_zone(ctx);
    let l = zone.at(ts);
    Ok(Value::Str(PhpStr::new(format(str_arg(args, 0).as_bytes(), ts, &l, &name))))
}

fn gmdate(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let ts = timestamp_arg(args, 1);
    let mut l = Zone::Named(Tz::UTC).at(ts);
    l.abbr = "GMT".into();
    Ok(Value::Str(PhpStr::new(format(str_arg(args, 0).as_bytes(), ts, &l, "UTC"))))
}

/// idate(string $format, ?int $timestamp = null): one part, as an int.
fn idate(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let f = str_arg(args, 0);
    if f.len() != 1 {
        return Err(ctx.throw("ValueError", "idate(): Argument #1 ($format) must be one character".into()));
    }
    let ts = timestamp_arg(args, 1);
    let (zone, _) = default_zone(ctx);
    let l = zone.at(ts);
    let d = l.dt;
    let v = match f.as_bytes()[0] {
        b'B' => ((ts + 3600).rem_euclid(86400) * 10 / 864) % 1000,
        b'd' => i64::from(d.day()),
        b'h' => i64::from(hour12(d.hour())),
        b'H' => i64::from(d.hour()),
        b'i' => i64::from(d.minute()),
        b'I' => i64::from(l.dst),
        b'L' => i64::from(NaiveDate::from_ymd_opt(d.year(), 2, 29).is_some()),
        b'm' => i64::from(d.month()),
        b'N' => i64::from(d.weekday().number_from_monday()),
        b'o' => i64::from(d.iso_week().year()),
        b's' => i64::from(d.second()),
        b't' => i64::from(days_in_month(d.year(), d.month())),
        b'U' => ts,
        b'w' => i64::from(d.weekday().num_days_from_sunday()),
        b'W' => i64::from(d.iso_week().week()),
        b'y' => i64::from(d.year() % 100),
        b'Y' => i64::from(d.year()),
        b'z' => i64::from(d.ordinal0()),
        b'Z' => i64::from(l.offset),
        _ => return Err(ctx.throw("ValueError", "idate(): Argument #1 ($format) must be a valid date format character".into())),
    };
    Ok(Value::Int(v))
}

/// A wall-clock time from parts that may overflow (month 13, day 0,
/// hour 25), carried as mktime carries them.
pub(crate) fn civil(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<NaiveDateTime> {
    let months = y * 12 + (mo - 1);
    let (y, m) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    let first = NaiveDate::from_ymd_opt(i32::try_from(y).ok()?, m as u32, 1)?;
    let date = first.checked_add_signed(chrono::Duration::days(d - 1))?;
    let secs = h * 3600 + mi * 60 + s;
    date.and_hms_opt(0, 0, 0)?.checked_add_signed(chrono::Duration::seconds(secs))
}

/// mktime(int $hour, ?int $minute = null, ?int $second = null, ?int $month = null, ?int $day = null, ?int $year = null)
fn make_time(ctx: &mut dyn Ctx, args: &[Value], zone: Zone) -> Value {
    let l = zone.at(now());
    let d = l.dt;
    let part = |i: usize, default: i64| match args.get(i).map(Value::deref) {
        None | Some(Value::Null) => default,
        Some(v) => v.to_int(),
    };
    let mut year = part(5, i64::from(d.year()));
    // Two-digit years: 0-69 are 2000-2069, 70-100 are 1970-2000.
    if args.len() > 5 && (0..70).contains(&year) {
        year += 2000;
    } else if args.len() > 5 && (70..=100).contains(&year) {
        year += 1900;
    }
    let dt = civil(year, part(3, i64::from(d.month())), part(4, i64::from(d.day())), part(0, i64::from(d.hour())), part(1, i64::from(d.minute())), part(2, i64::from(d.second())));
    let _ = ctx;
    dt.map_or(Value::Bool(false), |dt| Value::Int(zone.timestamp(dt)))
}

fn mktime(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (zone, _) = default_zone(ctx);
    Ok(make_time(ctx, args, zone))
}

fn gmmktime(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(make_time(ctx, args, Zone::Fixed(0)))
}

fn checkdate(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (m, d, y) = (arg(args, 0).to_int(), arg(args, 1).to_int(), arg(args, 2).to_int());
    let ok = (1..=32767).contains(&y) && (1..=12).contains(&m) && d >= 1 && d <= i64::from(days_in_month(y as i32, m as u32));
    Ok(Value::Bool(ok))
}

fn getdate(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let ts = timestamp_arg(args, 0);
    let (zone, _) = default_zone(ctx);
    let d = zone.at(ts).dt;
    let mut out = PhpArray::new();
    let mut put = |k: &str, v: Value| out.insert(Key::from_string(PhpStr::from(k)), v);
    put("seconds", Value::Int(i64::from(d.second())));
    put("minutes", Value::Int(i64::from(d.minute())));
    put("hours", Value::Int(i64::from(d.hour())));
    put("mday", Value::Int(i64::from(d.day())));
    put("wday", Value::Int(i64::from(d.weekday().num_days_from_sunday())));
    put("mon", Value::Int(i64::from(d.month())));
    put("year", Value::Int(i64::from(d.year())));
    put("yday", Value::Int(i64::from(d.ordinal0())));
    put("weekday", Value::str(DAYS[d.weekday().num_days_from_sunday() as usize]));
    put("month", Value::str(MONTHS[d.month0() as usize]));
    out.insert(Key::Int(0), Value::Int(ts));
    Ok(Value::Array(Rc::new(out)))
}

/// localtime(?int $timestamp = null, bool $associative = false)
fn localtime(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let ts = timestamp_arg(args, 0);
    let assoc = args.len() > 1 && arg(args, 1).to_bool();
    let (zone, _) = default_zone(ctx);
    let l = zone.at(ts);
    let d = l.dt;
    let parts = [
        ("tm_sec", i64::from(d.second())),
        ("tm_min", i64::from(d.minute())),
        ("tm_hour", i64::from(d.hour())),
        ("tm_mday", i64::from(d.day())),
        ("tm_mon", i64::from(d.month0())),
        ("tm_year", i64::from(d.year()) - 1900),
        ("tm_wday", i64::from(d.weekday().num_days_from_sunday())),
        ("tm_yday", i64::from(d.ordinal0())),
        ("tm_isdst", i64::from(l.dst)),
    ];
    let mut out = PhpArray::new();
    for (i, (k, v)) in parts.iter().enumerate() {
        let key = if assoc { Key::from_string(PhpStr::from(*k)) } else { Key::Int(i as i64) };
        out.insert(key, Value::Int(*v));
    }
    Ok(Value::Array(Rc::new(out)))
}

fn date_default_timezone_get(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Str(PhpStr::from(default_zone(ctx).1)))
}

fn date_default_timezone_set(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    let text = String::from_utf8_lossy(name.as_bytes()).into_owned();
    // Zone names are matched without regard to case, as PHP does.
    let found = text.parse::<Tz>().ok().or_else(|| chrono_tz::TZ_VARIANTS.iter().find(|z| z.name().eq_ignore_ascii_case(&text)).copied());
    match found {
        Some(tz) => {
            *ctx.state("date_tz") = Value::Str(PhpStr::from(tz.name()));
            Ok(Value::Bool(true))
        }
        None => {
            ctx.report(8, &format!("date_default_timezone_set(): Timezone ID '{text}' is invalid"));
            Ok(Value::Bool(false))
        }
    }
}

/// What strtotime's text says: absolute parts it sets, relative moves.
#[derive(Default)]
struct Parsed {
    date: Option<(i64, i64, i64)>,
    time: Option<(i64, i64, i64)>,
    rel: [i64; 6],
    weekday: Option<(u32, i64)>,
    first_last_day: Option<bool>,
    zone: Option<Zone>,
    stamp: Option<i64>,
    reset_time: bool,
}

fn month_number(word: &str) -> Option<i64> {
    let w = word.to_ascii_lowercase();
    let w = w.trim_end_matches('.');
    MONTHS.iter().position(|m| {
        let m = m.to_ascii_lowercase();
        m == w || (w.len() >= 3 && m.starts_with(w) && (w.len() == 3 || w == "sept"))
    })
    .map(|i| i as i64 + 1)
}

fn day_number(word: &str) -> Option<u32> {
    let w = word.to_ascii_lowercase();
    DAYS.iter().position(|d| {
        let d = d.to_ascii_lowercase();
        d == w || (w.len() >= 3 && w.len() <= 4 && d.starts_with(&w)) || w == format!("{}s", d)
    })
    .map(|i| i as u32)
}

/// A relative unit's index in `rel` and its multiplier.
fn unit(word: &str) -> Option<(usize, i64)> {
    let w = word.to_ascii_lowercase();
    let w = w.trim_end_matches('s');
    Some(match w {
        "year" => (0, 1),
        "month" => (1, 1),
        "fortnight" | "forthnight" => (2, 14),
        "week" => (2, 7),
        "day" => (2, 1),
        "hour" => (3, 1),
        "minute" | "min" => (4, 1),
        "second" | "sec" => (5, 1),
        "weekday" => (2, 1),
        _ => return None,
    })
}

/// strtotime's tokens: words, numbers (with their sign), punctuation.
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() || c == b',' {
            i += 1;
            continue;
        }
        let start = i;
        if c.is_ascii_alphabetic() {
            while i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == b'.' || b[i] == b'_' || b[i] == b'/') {
                if b[i] == b'/' && !b[start..i].iter().all(|c| c.is_ascii_alphabetic()) {
                    break;
                }
                i += 1;
            }
        } else if c.is_ascii_digit() || ((c == b'+' || c == b'-' || c == b'@') && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], b':' | b'-' | b'/' | b'.' | b'T' | b't') && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
                i += 1;
            }
        } else {
            i += 1;
        }
        out.push(s[start..i].to_string());
    }
    out
}

fn parse_time_text(t: &str) -> Option<(i64, i64, i64)> {
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let h: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let s: i64 = parts.get(2).map_or(Some(0.0), |x| x.parse::<f64>().ok())? as i64;
    (h < 25 && m < 60 && s < 61).then_some((h, m, s))
}

fn parse(text: &str) -> Option<Parsed> {
    let mut p = Parsed::default();
    let toks = tokens(text.trim());
    let mut i = 0;
    let lower = |t: &str| t.to_ascii_lowercase();
    while i < toks.len() {
        let t = toks[i].clone();
        let l = lower(&t);
        let next = toks.get(i + 1).map(|x| lower(x));
        i += 1;
        // @timestamp
        if let Some(n) = t.strip_prefix('@') {
            p.stamp = Some(n.parse().ok()?);
            continue;
        }
        match l.as_str() {
            "now" => continue,
            "today" | "midnight" => {
                p.time = Some((0, 0, 0));
                continue;
            }
            "noon" => {
                p.time = Some((12, 0, 0));
                continue;
            }
            "tomorrow" => {
                p.rel[2] += 1;
                p.time = Some((0, 0, 0));
                continue;
            }
            "yesterday" => {
                p.rel[2] -= 1;
                p.time = Some((0, 0, 0));
                continue;
            }
            "utc" | "gmt" | "z" | "zulu" => {
                p.zone = Some(Zone::Fixed(0));
                continue;
            }
            "est" | "edt" | "cst" | "cdt" | "mst" | "mdt" | "pst" | "pdt" | "cet" | "cest" | "bst" => {
                let h = match l.as_str() {
                    "est" => -5,
                    "edt" | "cdt" => if l == "edt" { -4 } else { -5 },
                    "cst" => -6,
                    "mst" => -7,
                    "mdt" => -6,
                    "pst" => -8,
                    "pdt" => -7,
                    "cet" | "bst" => 1,
                    _ => 2,
                };
                p.zone = Some(Zone::Fixed(h * 3600));
                continue;
            }
            "first" | "last" if next.as_deref() == Some("day") && toks.get(i + 1).is_some_and(|x| lower(x) == "of") => {
                p.first_last_day = Some(l == "first");
                i += 2;
                continue;
            }
            "next" | "last" | "previous" | "this" => {
                let n = match l.as_str() {
                    "next" => 1,
                    "this" => 0,
                    _ => -1,
                };
                let w = next.clone()?;
                i += 1;
                if let Some(d) = day_number(&w) {
                    p.weekday = Some((d, n));
                    p.reset_time = true;
                } else if let Some((idx, mul)) = unit(&w) {
                    p.rel[idx] += n * mul;
                } else {
                    return None;
                }
                continue;
            }
            "ago" => {
                for r in p.rel.iter_mut() {
                    *r = -*r;
                }
                continue;
            }
            "am" | "pm" | "a.m." | "p.m." => {
                let (h, m, s) = p.time?;
                let h = if l.starts_with('p') { if h == 12 { 12 } else { h + 12 } } else if h == 12 { 0 } else { h };
                p.time = Some((h, m, s));
                continue;
            }
            "of" | "at" | "on" | "t" => continue,
            _ => {}
        }
        if let Some(d) = day_number(&l) {
            p.weekday = Some((d, 0));
            p.reset_time = true;
            continue;
        }
        if let Some(m) = month_number(&l) {
            // Month name forms: "Jan 5 2024", "Jan 2024", "5 Jan 2024".
            let mut day = 1;
            let mut year = None;
            if let Some(n) = next.as_deref().and_then(|x| x.trim_end_matches(['s', 't', 'n', 'd', 'r', 'h']).parse::<i64>().ok()) {
                if n > 31 {
                    year = Some(n);
                } else {
                    day = n;
                }
                i += 1;
                if year.is_none()
                    && let Some(y) = toks.get(i).and_then(|x| x.parse::<i64>().ok()).filter(|y| *y > 31 || toks[i].len() == 4)
                {
                    year = Some(y);
                    i += 1;
                }
            }
            let y = year.map(|y| if y < 100 { if y < 70 { y + 2000 } else { y + 1900 } } else { y });
            p.date = Some((y.unwrap_or(i64::MIN), m, day));
            continue;
        }
        // A number: a relative amount ("+1 day", "3 weeks"), a date or a time.
        if let Some(n) = t.parse::<i64>().ok().filter(|_| !t.contains([':', '/', '.']) && !t[1..].contains('-'))
            && let Some(w) = next.as_deref()
        {
            if let Some((idx, mul)) = unit(w) {
                p.rel[idx] += n * mul;
                i += 1;
                continue;
            }
            if let Some(m) = month_number(w) {
                i += 1;
                let y = toks.get(i).and_then(|x| x.parse::<i64>().ok()).filter(|_| toks[i].len() >= 2);
                if y.is_some() {
                    i += 1;
                }
                p.date = Some((y.unwrap_or(i64::MIN), m, n));
                continue;
            }
            if matches!(w, "am" | "pm" | "a.m." | "p.m.") {
                p.time = Some((n, 0, 0));
                continue;
            }
        }
        // A zone offset after a time: +02:00, -0500.
        if (t.starts_with('+') || t.starts_with('-')) && (p.time.is_some() || t.contains(':')) && t.len() >= 3 {
            let digits: String = t[1..].chars().filter(char::is_ascii_digit).collect();
            let (h, m) = match digits.len() {
                1 | 2 => (digits.parse::<i32>().ok()?, 0),
                4 => (digits[..2].parse::<i32>().ok()?, digits[2..].parse::<i32>().ok()?),
                _ => return None,
            };
            let off = (h * 3600 + m * 60) * if t.starts_with('-') { -1 } else { 1 };
            p.zone = Some(Zone::Fixed(off));
            continue;
        }
        if t.contains(':') && !t.contains('-') && !t.contains('T') && !t.contains('t') {
            let (clock, zone) = split_zone(&t);
            p.time = Some(parse_time_text(clock)?);
            if let Some(z) = zone {
                p.zone = Some(z);
            }
            continue;
        }
        // Dates: 2024-01-15[T10:00:00], 2024/01/15, 01/15/2024, 15-01-2024, 15.01.2024.
        let (date_part, time_part) = match t.find(['T', 't']).filter(|&at| at > 4) {
            Some(at) => (&t[..at], Some(&t[at + 1..])),
            None => (t.as_str(), None),
        };
        let nums: Vec<&str> = date_part.trim_start_matches('+').split(['-', '/', '.']).collect();
        if nums.len() == 3 && nums.iter().all(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit())) {
            let v: Vec<i64> = nums.iter().filter_map(|x| x.parse().ok()).collect();
            let (y, m, d) = if nums[0].len() == 4 {
                (v[0], v[1], v[2])
            } else if date_part.contains('/') {
                (v[2], v[0], v[1])
            } else {
                (v[2], v[1], v[0])
            };
            let y = if nums[if nums[0].len() == 4 { 0 } else { 2 }].len() <= 2 { if y < 70 { y + 2000 } else { y + 1900 } } else { y };
            if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
                return None;
            }
            p.date = Some((y, m, d));
            if let Some(tp) = time_part {
                let (clock, zone) = split_zone(tp);
                p.time = Some(parse_time_text(clock)?);
                if let Some(z) = zone {
                    p.zone = Some(z);
                }
            } else {
                p.time.get_or_insert((0, 0, 0));
            }
            continue;
        }
        if nums.len() == 2 && date_part.contains('/') && nums.iter().all(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit())) {
            let (m, d): (i64, i64) = (nums[0].parse().ok()?, nums[1].parse().ok()?);
            p.date = Some((i64::MIN, m, d));
            p.time.get_or_insert((0, 0, 0));
            continue;
        }
        // A zone offset: +0200, -05:00.
        if (t.starts_with('+') || t.starts_with('-')) && t.len() >= 3 {
            let digits: String = t[1..].chars().filter(char::is_ascii_digit).collect();
            let (h, m) = match digits.len() {
                1 | 2 => (digits.parse::<i32>().ok()?, 0),
                4 => (digits[..2].parse::<i32>().ok()?, digits[2..].parse::<i32>().ok()?),
                _ => return None,
            };
            let off = (h * 3600 + m * 60) * if t.starts_with('-') { -1 } else { 1 };
            p.zone = Some(Zone::Fixed(off));
            continue;
        }
        // A zone name (Europe/Paris).
        if let Ok(tz) = t.parse::<Tz>() {
            p.zone = Some(Zone::Named(tz));
            continue;
        }
        // A bare year after a date.
        if let Ok(n) = t.parse::<i64>()
            && t.len() == 4
            && let Some((y, m, d)) = p.date
            && y == i64::MIN
        {
            p.date = Some((n, m, d));
            continue;
        }
        return None;
    }
    Some(p)
}

/// "10:30:00+02:00" → the clock and the zone after it.
fn split_zone(t: &str) -> (&str, Option<Zone>) {
    if let Some(stripped) = t.strip_suffix(['Z', 'z']) {
        return (stripped, Some(Zone::Fixed(0)));
    }
    match t.rfind(['+', '-']) {
        Some(at) if at > 0 => {
            let z = &t[at..];
            let digits: String = z[1..].chars().filter(char::is_ascii_digit).collect();
            let off = match digits.len() {
                2 => digits.parse::<i32>().ok().map(|h| h * 3600),
                4 => Some(digits[..2].parse::<i32>().unwrap_or(0) * 3600 + digits[2..].parse::<i32>().unwrap_or(0) * 60),
                _ => None,
            };
            match off {
                Some(o) => (&t[..at], Some(Zone::Fixed(if z.starts_with('-') { -o } else { o }))),
                None => (t, None),
            }
        }
        _ => (t, None),
    }
}

/// strtotime's result for `text`, relative to `base` in `zone`.
pub(crate) fn resolve(text: &str, base: i64, zone: Zone) -> Option<i64> {
    let p = parse(text)?;
    if let Some(ts) = p.stamp {
        return Some(ts + p.rel[2] * 86400 + p.rel[3] * 3600 + p.rel[4] * 60 + p.rel[5]);
    }
    let zone = p.zone.unwrap_or(zone);
    let now = zone.at(base).dt;
    let (mut y, mut m, mut d) = (i64::from(now.year()), i64::from(now.month()), i64::from(now.day()));
    let (mut h, mut mi, mut s) = (i64::from(now.hour()), i64::from(now.minute()), i64::from(now.second()));
    if let Some((yy, mm, dd)) = p.date {
        if yy != i64::MIN {
            y = yy;
        }
        m = mm;
        d = dd;
        if p.time.is_none() {
            (h, mi, s) = (0, 0, 0);
        }
    }
    if let Some((hh, mm, ss)) = p.time {
        (h, mi, s) = (hh, mm, ss);
    } else if p.reset_time {
        (h, mi, s) = (0, 0, 0);
    }
    y += p.rel[0];
    m += p.rel[1];
    if let Some(first) = p.first_last_day {
        let probe = civil(y, m, 1, 0, 0, 0)?;
        d = if first { 1 } else { i64::from(days_in_month(probe.year(), probe.month())) };
        y = i64::from(probe.year());
        m = i64::from(probe.month());
    }
    let mut dt = civil(y, m, d + p.rel[2], h + p.rel[3], mi + p.rel[4], s + p.rel[5])?;
    if let Some((wd, n)) = p.weekday {
        let current = dt.weekday().num_days_from_sunday() as i64;
        let mut diff = (i64::from(wd) - current).rem_euclid(7);
        match n {
            1 if diff == 0 => diff = 7,
            -1 => diff = if diff == 0 { -7 } else { diff - 7 },
            _ => {}
        }
        dt += chrono::Duration::days(diff);
    }
    Some(zone.timestamp(dt))
}

/// strtotime(string $datetime, ?int $baseTimestamp = null)
fn strtotime(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let text = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let base = timestamp_arg(args, 1);
    let (zone, _) = default_zone(ctx);
    if text.trim().is_empty() {
        return Ok(Value::Bool(false));
    }
    Ok(resolve(&text, base, zone).map_or(Value::Bool(false), Value::Int))
}

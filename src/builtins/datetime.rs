use crate::core::value::{ArrayKey, Handle, Val};
use crate::vm::engine::VM;
use chrono::{
    DateTime as ChronoDateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Offset,
    TimeZone, Timelike, Utc, Weekday,
};
use chrono_tz::Tz;
use indexmap::IndexMap;
use regex::Regex;
use std::rc::Rc;
use std::str::FromStr;

use lazy_static::lazy_static;

lazy_static! {
    static ref RELATIVE_TIME_RE: Regex = Regex::new(
        r"^([+-]?\d+)\s*(year|month|week|day|hour|minute|second|fortnight)s?(\s+ago)?$"
    ).unwrap();
    
    static ref WEEKDAY_REFERENCE_RE: Regex = Regex::new(
        r"^(next|last|this|previous)\s+(monday|tuesday|wednesday|thursday|friday|saturday|sunday|mon|tue|wed|thu|fri|sat|sun)$"
    ).unwrap();
    
    static ref SPECIAL_PHRASE_RE: Regex = Regex::new(
        r"^(first|last)\s+day\s+of\s+(next|this|last)\s+month$"
    ).unwrap();
}

// ============================================================================
// Internal Data Structures
// ============================================================================

#[derive(Debug)]
pub struct DateTimeZoneData {
    pub tz: Tz,
}

#[derive(Debug)]
pub struct DateTimeData {
    pub dt: ChronoDateTime<Tz>,
}

#[derive(Debug)]
pub struct DateIntervalData {
    pub y: i64,
    pub m: i64,
    pub d: i64,
    pub h: i64,
    pub i: i64,
    pub s: i64,
    pub f: f64,
    pub invert: i64,
    pub days: Option<i64>,
}

use std::cell::RefCell;

#[derive(Debug)]
pub struct DatePeriodData {
    pub start: ChronoDateTime<Tz>,
    pub end: Option<ChronoDateTime<Tz>>,
    pub interval: Rc<DateIntervalData>,
    pub recurrences: Option<i64>,
    pub include_start_date: bool,

    // Iteration state
    pub current_date: RefCell<Option<ChronoDateTime<Tz>>>,
    pub current_index: RefCell<i64>,
}

// ============================================================================
// Date/Time Constants
// ============================================================================

pub const DATE_ATOM: &str = "Y-m-d\\TH:i:sP";
pub const DATE_COOKIE: &str = "l, d-M-Y H:i:s T";
pub const DATE_ISO8601: &str = "Y-m-d\\TH:i:sO";
pub const DATE_ISO8601_EXPANDED: &str = "X-m-d\\TH:i:sP";
pub const DATE_RFC822: &str = "D, d M y H:i:s O";
pub const DATE_RFC850: &str = "l, d-M-y H:i:s T";
pub const DATE_RFC1036: &str = "D, d M y H:i:s O";
pub const DATE_RFC1123: &str = "D, d M Y H:i:s O";
pub const DATE_RFC7231: &str = "D, d M Y H:i:s \\G\\M\\T";
pub const DATE_RFC2822: &str = "D, d M Y H:i:s O";
pub const DATE_RFC3339: &str = "Y-m-d\\TH:i:sP";
pub const DATE_RFC3339_EXTENDED: &str = "Y-m-d\\TH:i:s.vP";
pub const DATE_RSS: &str = "D, d M Y H:i:s O";
pub const DATE_W3C: &str = "Y-m-d\\TH:i:sP";

// Deprecated constants for date_sunrise/date_sunset
pub const SUNFUNCS_RET_TIMESTAMP: i64 = 0;
pub const SUNFUNCS_RET_STRING: i64 = 1;
pub const SUNFUNCS_RET_DOUBLE: i64 = 2;

// ============================================================================
// Helper Functions
// ============================================================================

fn get_string_arg(vm: &VM, handle: Handle) -> Result<Vec<u8>, String> {
    let val = vm.arena.get(handle);
    match &val.value {
        Val::String(s) => Ok(s.to_vec()),
        _ => Err("Expected string argument".into()),
    }
}

fn get_int_arg(vm: &VM, handle: Handle) -> Result<i64, String> {
    let val = vm.arena.get(handle);
    match &val.value {
        Val::Int(i) => Ok(*i),
        _ => Err("Expected integer argument".into()),
    }
}

fn get_float_arg(vm: &VM, handle: Handle) -> Result<f64, String> {
    let val = vm.arena.get(handle);
    match &val.value {
        Val::Float(f) => Ok(*f),
        Val::Int(i) => Ok(*i as f64),
        _ => Err("Expected float argument".into()),
    }
}

fn parse_timezone(tz_str: &str) -> Result<Tz, String> {
    Tz::from_str(tz_str).map_err(|_| format!("Unknown or invalid timezone: {}", tz_str))
}

fn make_array_key(key: &str) -> ArrayKey {
    ArrayKey::Str(Rc::new(key.as_bytes().to_vec()))
}

fn get_internal_data<T: 'static>(vm: &VM, handle: Handle) -> Result<Rc<T>, String> {
    let val = vm.arena.get(handle);
    if let Val::Object(payload_handle) = &val.value {
        let payload = vm.arena.get(*payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                if let Ok(data) = internal.clone().downcast::<T>() {
                    return Ok(data);
                }
            }
        }
    }
    Err(format!(
        "Object does not have the expected internal data: {}",
        std::any::type_name::<T>()
    ))
}

fn format_php_date(dt: &ChronoDateTime<Tz>, format: &str) -> String {
    let mut result = String::new();
    let mut chars = format.chars().peekable();
    let mut escape_next = false;

    while let Some(ch) = chars.next() {
        if escape_next {
            result.push(ch);
            escape_next = false;
            continue;
        }

        if ch == '\\' {
            escape_next = true;
            continue;
        }

        match ch {
            // Day
            'd' => result.push_str(&format!("{:02}", dt.day())),
            'D' => {
                let day = match dt.weekday() {
                    Weekday::Mon => "Mon",
                    Weekday::Tue => "Tue",
                    Weekday::Wed => "Wed",
                    Weekday::Thu => "Thu",
                    Weekday::Fri => "Fri",
                    Weekday::Sat => "Sat",
                    Weekday::Sun => "Sun",
                };
                result.push_str(day);
            }
            'j' => result.push_str(&dt.day().to_string()),
            'l' => {
                let day = match dt.weekday() {
                    Weekday::Mon => "Monday",
                    Weekday::Tue => "Tuesday",
                    Weekday::Wed => "Wednesday",
                    Weekday::Thu => "Thursday",
                    Weekday::Fri => "Friday",
                    Weekday::Sat => "Saturday",
                    Weekday::Sun => "Sunday",
                };
                result.push_str(day);
            }
            'N' => result.push_str(&dt.weekday().num_days_from_monday().to_string()),
            'S' => {
                let day = dt.day();
                let suffix = match day {
                    1 | 21 | 31 => "st",
                    2 | 22 => "nd",
                    3 | 23 => "rd",
                    _ => "th",
                };
                result.push_str(suffix);
            }
            'w' => result.push_str(&dt.weekday().number_from_sunday().to_string()),
            'z' => result.push_str(&dt.ordinal0().to_string()),

            // Week
            'W' => result.push_str(&format!("{:02}", dt.iso_week().week())),

            // Month
            'F' => {
                let month = match dt.month() {
                    1 => "January",
                    2 => "February",
                    3 => "March",
                    4 => "April",
                    5 => "May",
                    6 => "June",
                    7 => "July",
                    8 => "August",
                    9 => "September",
                    10 => "October",
                    11 => "November",
                    12 => "December",
                    _ => "",
                };
                result.push_str(month);
            }
            'm' => result.push_str(&format!("{:02}", dt.month())),
            'M' => {
                let month = match dt.month() {
                    1 => "Jan",
                    2 => "Feb",
                    3 => "Mar",
                    4 => "Apr",
                    5 => "May",
                    6 => "Jun",
                    7 => "Jul",
                    8 => "Aug",
                    9 => "Sep",
                    10 => "Oct",
                    11 => "Nov",
                    12 => "Dec",
                    _ => "",
                };
                result.push_str(month);
            }
            'n' => result.push_str(&dt.month().to_string()),
            't' => {
                let days_in_month = NaiveDate::from_ymd_opt(dt.year(), dt.month() + 1, 1)
                    .unwrap_or(NaiveDate::from_ymd_opt(dt.year() + 1, 1, 1).unwrap())
                    .signed_duration_since(
                        NaiveDate::from_ymd_opt(dt.year(), dt.month(), 1).unwrap(),
                    )
                    .num_days();
                result.push_str(&days_in_month.to_string());
            }

            // Year
            'L' => {
                let is_leap = NaiveDate::from_ymd_opt(dt.year(), 2, 29).is_some();
                result.push(if is_leap { '1' } else { '0' });
            }
            'o' => result.push_str(&dt.iso_week().year().to_string()),
            'X' => result.push_str(&format!("{:+05}", dt.year())),
            'x' => result.push_str(&format!("{:+05}", dt.iso_week().year())),
            'Y' => result.push_str(&dt.year().to_string()),
            'y' => result.push_str(&format!("{:02}", dt.year() % 100)),

            // Time
            'a' => result.push_str(if dt.hour() < 12 { "am" } else { "pm" }),
            'A' => result.push_str(if dt.hour() < 12 { "AM" } else { "PM" }),
            'B' => {
                // Swatch Internet time
                let seconds = (dt.hour() * 3600 + dt.minute() * 60 + dt.second()) as f64;
                let beats = ((seconds + 3600.0) / 86.4).floor() as i32 % 1000;
                result.push_str(&format!("{:03}", beats));
            }
            'g' => {
                let hour = dt.hour();
                result.push_str(
                    &(if hour == 0 || hour == 12 {
                        12
                    } else {
                        hour % 12
                    })
                    .to_string(),
                );
            }
            'G' => result.push_str(&dt.hour().to_string()),
            'h' => {
                let hour = dt.hour();
                result.push_str(&format!(
                    "{:02}",
                    if hour == 0 || hour == 12 {
                        12
                    } else {
                        hour % 12
                    }
                ));
            }
            'H' => result.push_str(&format!("{:02}", dt.hour())),
            'i' => result.push_str(&format!("{:02}", dt.minute())),
            's' => result.push_str(&format!("{:02}", dt.second())),
            'u' => result.push_str(&format!("{:06}", dt.timestamp_subsec_micros())),
            'v' => result.push_str(&format!("{:03}", dt.timestamp_subsec_millis())),

            // Timezone
            'e' => result.push_str(&dt.timezone().name()),
            'I' => result.push('0'), // Daylight saving time (simplified)
            'O' => {
                let offset = dt.offset().fix().local_minus_utc();
                let sign = if offset >= 0 { '+' } else { '-' };
                let offset = offset.abs();
                let hours = offset / 3600;
                let minutes = (offset % 3600) / 60;
                result.push_str(&format!("{}{:02}{:02}", sign, hours, minutes));
            }
            'P' => {
                let offset = dt.offset().fix().local_minus_utc();
                let sign = if offset >= 0 { '+' } else { '-' };
                let offset = offset.abs();
                let hours = offset / 3600;
                let minutes = (offset % 3600) / 60;
                result.push_str(&format!("{}{:02}:{:02}", sign, hours, minutes));
            }
            'p' => {
                let offset = dt.offset().fix().local_minus_utc();
                if offset == 0 {
                    result.push('Z');
                } else {
                    let sign = if offset >= 0 { '+' } else { '-' };
                    let offset = offset.abs();
                    let hours = offset / 3600;
                    let minutes = (offset % 3600) / 60;
                    if minutes == 0 {
                        result.push_str(&format!("{}{:02}", sign, hours));
                    } else {
                        result.push_str(&format!("{}{}:{:02}", sign, hours, minutes));
                    }
                }
            }
            'T' => result.push_str(&dt.timezone().name()),
            'Z' => result.push_str(&dt.offset().fix().local_minus_utc().to_string()),

            // Full Date/Time
            'c' => result.push_str(&format_php_date(dt, DATE_ISO8601)),
            'r' => result.push_str(&format_php_date(dt, DATE_RFC2822)),
            'U' => result.push_str(&dt.timestamp().to_string()),

            _ => result.push(ch),
        }
    }

    result
}

// ============================================================================
// DateTimeZone Class
// ============================================================================

/// DateTimeZone::__construct(string $timezone)
pub fn php_datetimezone_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTimeZone::__construct() called outside object context")?;

    if args.is_empty() {
        return Err("DateTimeZone::__construct() expects exactly 1 parameter, 0 given".into());
    }

    let tz_str = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();
    let tz = parse_timezone(&tz_str)?;

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        let payload = vm.arena.get_mut(*payload_handle);
        if let Val::ObjPayload(ref mut obj_data) = payload.value {
            obj_data.internal = Some(Rc::new(DateTimeZoneData { tz }));
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}

/// DateTimeZone::getName(): string
pub fn php_datetimezone_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTimeZone::getName() called outside object context")?;
    let data = get_internal_data::<DateTimeZoneData>(vm, this_handle)?;

    Ok(vm
        .arena
        .alloc(Val::String(data.tz.name().as_bytes().to_vec().into())))
}

/// DateTimeZone::getOffset(DateTimeInterface $datetime): int
pub fn php_datetimezone_get_offset(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTimeZone::getOffset() called outside object context")?;
    let data = get_internal_data::<DateTimeZoneData>(vm, this_handle)?;

    if args.is_empty() {
        return Err("DateTimeZone::getOffset() expects exactly 1 parameter, 0 given".into());
    }

    let dt_data = get_internal_data::<DateTimeData>(vm, args[0])?;
    let offset = data
        .tz
        .offset_from_utc_datetime(&dt_data.dt.naive_utc())
        .fix()
        .local_minus_utc();

    Ok(vm.arena.alloc(Val::Int(offset as i64)))
}

/// DateTimeZone::getLocation(): array|false
pub fn php_datetimezone_get_location(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // Simplified - return false for now as chrono-tz doesn't easily provide this
    Ok(vm.arena.alloc(Val::Bool(false)))
}

/// DateTimeZone::listIdentifiers(int $timezoneGroup = DateTimeZone::ALL, ?string $countryCode = null): array
pub fn php_datetimezone_list_identifiers(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let mut map = IndexMap::new();
    for (i, tz) in chrono_tz::TZ_VARIANTS.iter().enumerate() {
        map.insert(
            ArrayKey::Int(i as i64),
            vm.arena
                .alloc(Val::String(tz.name().as_bytes().to_vec().into())),
        );
    }

    Ok(vm
        .arena
        .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
            map,
            next_free: chrono_tz::TZ_VARIANTS.len() as i64,
        }))))
}

// ============================================================================
// DateTime Class
// ============================================================================

/// DateTime::__construct(string $datetime = "now", ?DateTimeZone $timezone = null)
pub fn php_datetime_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTime::__construct() called outside object context")?;

    let datetime_str = if !args.is_empty() {
        String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string()
    } else {
        "now".to_string()
    };

    let tz = if args.len() > 1 {
        let tz_data = get_internal_data::<DateTimeZoneData>(vm, args[1])?;
        tz_data.tz
    } else {
        Tz::UTC
    };

    let dt = if datetime_str == "now" {
        Utc::now().with_timezone(&tz)
    } else if let Ok(dt) = ChronoDateTime::parse_from_rfc3339(&datetime_str) {
        dt.with_timezone(&tz)
    } else if let Ok(ndt) = NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S") {
        tz.from_local_datetime(&ndt).unwrap()
    } else if let Ok(nd) = NaiveDate::parse_from_str(&datetime_str, "%Y-%m-%d") {
        tz.from_local_datetime(&nd.and_hms_opt(0, 0, 0).unwrap())
            .unwrap()
    } else {
        return Err(format!("Failed to parse datetime string: {}", datetime_str));
    };

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        let payload = vm.arena.get_mut(*payload_handle);
        if let Val::ObjPayload(ref mut obj_data) = payload.value {
            obj_data.internal = Some(Rc::new(DateTimeData { dt }));
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}

/// DateTime::format(string $format): string
pub fn php_datetime_format(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTime::format() called outside object context")?;
    let data = get_internal_data::<DateTimeData>(vm, this_handle)?;

    if args.is_empty() {
        return Err("DateTime::format() expects exactly 1 parameter, 0 given".into());
    }

    let format = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();
    let formatted = format_php_date(&data.dt, &format);

    Ok(vm.arena.alloc(Val::String(formatted.into_bytes().into())))
}

fn add_interval(
    dt: &ChronoDateTime<Tz>,
    interval: &DateIntervalData,
    subtract: bool,
) -> ChronoDateTime<Tz> {
    let mut new_dt = dt.clone();
    let invert = if subtract {
        interval.invert == 0
    } else {
        interval.invert == 1
    };
    let sign = if invert { -1 } else { 1 };

    // Add years
    if interval.y != 0 {
        let new_year = new_dt.year() + (interval.y * sign) as i32;
        new_dt = new_dt.with_year(new_year).unwrap_or(new_dt);
    }

    // Add months
    if interval.m != 0 {
        let total_months = new_dt.month0() as i64 + (interval.m * sign);
        let year_adj = if total_months >= 0 {
            (total_months / 12) as i32
        } else {
            ((total_months - 11) / 12) as i32
        };
        let new_month = ((total_months % 12 + 12) % 12) as u32;
        new_dt = new_dt.with_year(new_dt.year() + year_adj).unwrap_or(new_dt);
        new_dt = new_dt.with_month0(new_month).unwrap_or(new_dt);
    }

    // Add days, hours, minutes, seconds
    let duration = chrono::Duration::days(interval.d)
        + chrono::Duration::hours(interval.h)
        + chrono::Duration::minutes(interval.i)
        + chrono::Duration::seconds(interval.s);

    if sign == -1 {
        new_dt = new_dt - duration;
    } else {
        new_dt = new_dt + duration;
    }

    new_dt
}

pub fn php_datetime_add(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    if args.is_empty() {
        return Err("DateTime::add() expects exactly 1 parameter, 0 given".into());
    }
    let interval_handle = args[0];

    let dt_data = get_internal_data::<DateTimeData>(vm, this_handle)?;
    let interval_data = get_internal_data::<DateIntervalData>(vm, interval_handle)?;

    let new_dt = add_interval(&dt_data.dt, &interval_data, false);

    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(h) => *h,
        _ => return Err("Invalid 'this'".into()),
    };

    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        obj_data.internal = Some(Rc::new(DateTimeData { dt: new_dt }));
    }

    Ok(this_handle)
}

pub fn php_datetime_sub(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    if args.is_empty() {
        return Err("DateTime::sub() expects exactly 1 parameter, 0 given".into());
    }
    let interval_handle = args[0];

    let dt_data = get_internal_data::<DateTimeData>(vm, this_handle)?;
    let interval_data = get_internal_data::<DateIntervalData>(vm, interval_handle)?;

    let new_dt = add_interval(&dt_data.dt, &interval_data, true);

    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(h) => *h,
        _ => return Err("Invalid 'this'".into()),
    };

    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        obj_data.internal = Some(Rc::new(DateTimeData { dt: new_dt }));
    }

    Ok(this_handle)
}

pub fn php_datetime_diff(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    if args.is_empty() {
        return Err("DateTime::diff() expects exactly 1 parameter, 0 given".into());
    }
    let other_handle = args[0];

    let dt1 = get_internal_data::<DateTimeData>(vm, this_handle)?;
    let dt2 = get_internal_data::<DateTimeData>(vm, other_handle)?;

    let diff = dt2.dt.clone() - dt1.dt.clone();
    let total_seconds = diff.num_seconds();
    let abs_seconds = total_seconds.abs();

    let days = abs_seconds / 86400;
    let hours = (abs_seconds % 86400) / 3600;
    let minutes = (abs_seconds % 3600) / 60;
    let seconds = abs_seconds % 60;

    let data = DateIntervalData {
        y: 0,
        m: 0,
        d: days,
        h: hours,
        i: minutes,
        s: seconds,
        f: 0.0,
        invert: if total_seconds < 0 { 1 } else { 0 },
        days: Some(days),
    };

    let interval_sym = vm.context.interner.intern(b"DateInterval");
    let dummy_spec = vm.arena.alloc(Val::String(b"PT0S".to_vec().into()));
    let interval_handle = vm.instantiate_class(interval_sym, &[dummy_spec])?;

    let payload_handle = match &vm.arena.get(interval_handle).value {
        Val::Object(h) => *h,
        _ => return Err("Failed to create DateInterval".into()),
    };

    // Allocate values first to avoid double borrow
    let y_val = vm.arena.alloc(Val::Int(0));
    let m_val = vm.arena.alloc(Val::Int(0));
    let d_val = vm.arena.alloc(Val::Int(days));
    let h_val = vm.arena.alloc(Val::Int(hours));
    let i_val = vm.arena.alloc(Val::Int(minutes));
    let s_val = vm.arena.alloc(Val::Int(seconds));
    let f_val = vm.arena.alloc(Val::Float(0.0));
    let invert_val = vm
        .arena
        .alloc(Val::Int(if total_seconds < 0 { 1 } else { 0 }));
    let days_val = vm.arena.alloc(Val::Int(days));

    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        obj_data.internal = Some(Rc::new(data));

        // Set properties
        let y_sym = vm.context.interner.intern(b"y");
        let m_sym = vm.context.interner.intern(b"m");
        let d_sym = vm.context.interner.intern(b"d");
        let h_sym = vm.context.interner.intern(b"h");
        let i_sym = vm.context.interner.intern(b"i");
        let s_sym = vm.context.interner.intern(b"s");
        let f_sym = vm.context.interner.intern(b"f");
        let invert_sym = vm.context.interner.intern(b"invert");
        let days_sym = vm.context.interner.intern(b"days");

        obj_data.properties.insert(y_sym, y_val);
        obj_data.properties.insert(m_sym, m_val);
        obj_data.properties.insert(d_sym, d_val);
        obj_data.properties.insert(h_sym, h_val);
        obj_data.properties.insert(i_sym, i_val);
        obj_data.properties.insert(s_sym, s_val);
        obj_data.properties.insert(f_sym, f_val);
        obj_data.properties.insert(invert_sym, invert_val);
        obj_data.properties.insert(days_sym, days_val);
    }

    Ok(interval_handle)
}

fn convert_php_to_chrono_format(php_format: &str) -> String {
    let mut chrono_format = String::new();
    let mut chars = php_format.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            'Y' => chrono_format.push_str("%Y"),
            'y' => chrono_format.push_str("%y"),
            'm' => chrono_format.push_str("%m"),
            'd' => chrono_format.push_str("%d"),
            'H' => chrono_format.push_str("%H"),
            'i' => chrono_format.push_str("%M"),
            's' => chrono_format.push_str("%S"),
            'v' => chrono_format.push_str("%3f"),
            'u' => chrono_format.push_str("%6f"),
            _ => chrono_format.push(ch),
        }
    }
    chrono_format
}

pub fn php_datetime_create_from_format(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("DateTime::createFromFormat() expects at least 2 parameters".into());
    }

    let format = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();
    let datetime_str = String::from_utf8_lossy(&get_string_arg(vm, args[1])?).to_string();

    let chrono_format = convert_php_to_chrono_format(&format);

    if let Ok(naive) = NaiveDateTime::parse_from_str(&datetime_str, &chrono_format) {
        let tz: Tz = vm.context.config.timezone.parse().unwrap_or(Tz::UTC);
        let dt = tz.from_utc_datetime(&naive);

        let datetime_sym = vm.context.interner.intern(b"DateTime");
        let obj_handle = vm.instantiate_class(datetime_sym, &[])?;

        let payload_handle = match &vm.arena.get(obj_handle).value {
            Val::Object(h) => *h,
            _ => return Err("Failed to create DateTime".into()),
        };

        if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
            obj_data.internal = Some(Rc::new(DateTimeData { dt }));
        }

        Ok(obj_handle)
    } else {
        Ok(vm.arena.alloc(Val::Bool(false)))
    }
}

/// DateTime::getTimestamp(): int
pub fn php_datetime_get_timestamp(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTime::getTimestamp() called outside object context")?;
    let data = get_internal_data::<DateTimeData>(vm, this_handle)?;

    Ok(vm.arena.alloc(Val::Int(data.dt.timestamp())))
}

/// DateTime::setTimestamp(int $timestamp): DateTime
pub fn php_datetime_set_timestamp(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTime::setTimestamp() called outside object context")?;
    let data = get_internal_data::<DateTimeData>(vm, this_handle)?;

    if args.is_empty() {
        return Err("DateTime::setTimestamp() expects exactly 1 parameter, 0 given".into());
    }

    let timestamp = get_int_arg(vm, args[0])?;
    let new_dt = data.dt.timezone().timestamp_opt(timestamp, 0).unwrap();

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        let payload = vm.arena.get_mut(*payload_handle);
        if let Val::ObjPayload(ref mut obj_data) = payload.value {
            obj_data.internal = Some(Rc::new(DateTimeData { dt: new_dt }));
        }
    }

    Ok(this_handle)
}

/// DateTime::getTimezone(): DateTimeZone|false
pub fn php_datetime_get_timezone(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTime::getTimezone() called outside object context")?;
    let data = get_internal_data::<DateTimeData>(vm, this_handle)?;

    // Create a new DateTimeZone object with the timezone name
    let dtz_sym = vm.context.interner.intern(b"DateTimeZone");
    let tz_name = data.dt.timezone().name().as_bytes().to_vec();
    let tz_handle = vm.arena.alloc(Val::String(Rc::new(tz_name)));
    let dtz_handle = vm.instantiate_class(dtz_sym, &[tz_handle])?;

    Ok(dtz_handle)
}

/// DateTime::setTimezone(DateTimeZone $timezone): DateTime
pub fn php_datetime_set_timezone(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateTime::setTimezone() called outside object context")?;
    let data = get_internal_data::<DateTimeData>(vm, this_handle)?;

    if args.is_empty() {
        return Err("DateTime::setTimezone() expects exactly 1 parameter, 0 given".into());
    }

    let tz_data = get_internal_data::<DateTimeZoneData>(vm, args[0])?;
    let new_dt = data.dt.with_timezone(&tz_data.tz);

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        let payload = vm.arena.get_mut(*payload_handle);
        if let Val::ObjPayload(ref mut obj_data) = payload.value {
            obj_data.internal = Some(Rc::new(DateTimeData { dt: new_dt }));
        }
    }

    Ok(this_handle)
}

// ============================================================================
// DateInterval Class
// ============================================================================

/// DateInterval::__construct(string $duration)
pub fn php_dateinterval_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DateInterval::__construct() called outside object context")?;

    if args.is_empty() {
        return Err("DateInterval::__construct() expects exactly 1 parameter, 0 given".into());
    }

    let duration_str = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();

    // ISO 8601 duration parser: P[n]Y[n]M[n]DT[n]H[n]M[n]S
    let re =
        Regex::new(r"^P(?:(\d+)Y)?(?:(\d+)M)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$")
            .unwrap();

    if let Some(caps) = re.captures(&duration_str) {
        let y = caps.get(1).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let m = caps.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let d = caps.get(3).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let h = caps.get(4).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let i = caps.get(5).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let s = caps.get(6).map_or(0, |m| m.as_str().parse().unwrap_or(0));

        let data = DateIntervalData {
            y,
            m,
            d,
            h,
            i,
            s,
            f: 0.0,
            invert: 0,
            days: None,
        };

        let payload_handle = match &vm.arena.get(this_handle).value {
            Val::Object(h) => *h,
            _ => return Err("Invalid 'this'".into()),
        };

        // Pre-allocate all values to avoid double borrow of vm.arena
        let y_val = vm.arena.alloc(Val::Int(y));
        let m_val = vm.arena.alloc(Val::Int(m));
        let d_val = vm.arena.alloc(Val::Int(d));
        let h_val = vm.arena.alloc(Val::Int(h));
        let i_val = vm.arena.alloc(Val::Int(i));
        let s_val = vm.arena.alloc(Val::Int(s));
        let f_val = vm.arena.alloc(Val::Float(0.0));
        let invert_val = vm.arena.alloc(Val::Int(0));
        let days_val = vm.arena.alloc(Val::Bool(false));

        if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
            obj_data.internal = Some(Rc::new(data));

            // Also set public properties for PHP access
            let y_sym = vm.context.interner.intern(b"y");
            let m_sym = vm.context.interner.intern(b"m");
            let d_sym = vm.context.interner.intern(b"d");
            let h_sym = vm.context.interner.intern(b"h");
            let i_sym = vm.context.interner.intern(b"i");
            let s_sym = vm.context.interner.intern(b"s");
            let f_sym = vm.context.interner.intern(b"f");
            let invert_sym = vm.context.interner.intern(b"invert");
            let days_sym = vm.context.interner.intern(b"days");

            obj_data.properties.insert(y_sym, y_val);
            obj_data.properties.insert(m_sym, m_val);
            obj_data.properties.insert(d_sym, d_val);
            obj_data.properties.insert(h_sym, h_val);
            obj_data.properties.insert(i_sym, i_val);
            obj_data.properties.insert(s_sym, s_val);
            obj_data.properties.insert(f_sym, f_val);
            obj_data.properties.insert(invert_sym, invert_val);
            obj_data.properties.insert(days_sym, days_val);
        }

        Ok(vm.arena.alloc(Val::Null))
    } else {
        Err(format!("Invalid duration string: {}", duration_str))
    }
}

pub fn php_datetime_modify(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    if args.is_empty() {
        return Err("DateTime::modify() expects exactly 1 parameter, 0 given".into());
    }
    let modify_handle = args[0];
    let modify_str = String::from_utf8_lossy(&get_string_arg(vm, modify_handle)?).to_string();

    let dt_data = get_internal_data::<DateTimeData>(vm, this_handle)?;

    // Simple implementation for now: just parse the new string relative to current time
    // In a real implementation, we'd use a relative date parser.
    let new_dt = if modify_str == "now" {
        Utc::now().with_timezone(&dt_data.dt.timezone())
    } else if let Ok(ndt) = NaiveDateTime::parse_from_str(&modify_str, "%Y-%m-%d %H:%M:%S") {
        dt_data.dt.timezone().from_local_datetime(&ndt).unwrap()
    } else {
        return Err(format!("Failed to parse modify string: {}", modify_str));
    };

    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(h) => *h,
        _ => return Err("Invalid 'this'".into()),
    };

    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        obj_data.internal = Some(Rc::new(DateTimeData { dt: new_dt }));
    }

    Ok(this_handle)
}

/// DateInterval::format(string $format): string
pub fn php_dateinterval_format(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let (interval_handle, format_handle) = if args.len() >= 2 {
        // Procedural: date_interval_format($interval, $format)
        (args[0], args[1])
    } else if args.len() == 1 {
        // Method: $interval->format($format)
        let this_handle = vm
            .frames
            .last()
            .and_then(|f| f.this)
            .ok_or("DateInterval::format() called outside object context")?;
        (this_handle, args[0])
    } else {
        return Err("date_interval_format() expects at least 1 parameter".into());
    };

    let format_str = String::from_utf8_lossy(&get_string_arg(vm, format_handle)?).to_string();
    let data = get_internal_data::<DateIntervalData>(vm, interval_handle)?;

    let mut result = String::new();
    let mut chars = format_str.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            if let Some(next) = chars.next() {
                match next {
                    'Y' => result.push_str(&format!("{:02}", data.y)),
                    'y' => result.push_str(&data.y.to_string()),
                    'M' => result.push_str(&format!("{:02}", data.m)),
                    'm' => result.push_str(&data.m.to_string()),
                    'D' => result.push_str(&format!("{:02}", data.d)),
                    'd' => result.push_str(&data.d.to_string()),
                    'H' => result.push_str(&format!("{:02}", data.h)),
                    'h' => result.push_str(&data.h.to_string()),
                    'I' => result.push_str(&format!("{:02}", data.i)),
                    'i' => result.push_str(&data.i.to_string()),
                    'S' => result.push_str(&format!("{:02}", data.s)),
                    's' => result.push_str(&data.s.to_string()),
                    'F' => result.push_str(&format!("{:06}", (data.f * 1000000.0) as i64)),
                    'f' => result.push_str(&((data.f * 1000000.0) as i64).to_string()),
                    'R' => result.push(if data.invert == 1 { '-' } else { '+' }),
                    'r' => {
                        if data.invert == 1 {
                            result.push('-');
                        }
                    }
                    'a' => {
                        if let Some(days) = data.days {
                            result.push_str(&days.to_string());
                        } else {
                            result.push_str("(unknown)");
                        }
                    }
                    '%' => result.push('%'),
                    _ => {
                        result.push('%');
                        result.push(next);
                    }
                }
            } else {
                result.push('%');
            }
        } else {
            result.push(c);
        }
    }

    Ok(vm.arena.alloc(Val::String(result.into_bytes().into())))
}

// ============================================================================
// DatePeriod Class
// ============================================================================

/// DatePeriod::__construct(...)
pub fn php_dateperiod_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("DatePeriod::__construct() called outside object context")?;

    if args.len() < 2 {
        return Err("DatePeriod::__construct() expects at least 2 parameters".into());
    }

    let start_data = get_internal_data::<DateTimeData>(vm, args[0])?;
    let interval_data = get_internal_data::<DateIntervalData>(vm, args[1])?;

    let mut end = None;
    let mut recurrences = None;
    let mut options = 0;

    if args.len() >= 3 {
        let arg2 = vm.arena.get(args[2]);
        match &arg2.value {
            Val::Int(r) => recurrences = Some(*r),
            Val::Object(_) => {
                let end_data = get_internal_data::<DateTimeData>(vm, args[2])?;
                end = Some(end_data.dt.clone());
            }
            _ => {
                return Err(
                    "DatePeriod::__construct(): Argument #3 must be of type DateTimeInterface|int"
                        .into(),
                );
            }
        }
    }

    if args.len() >= 4 {
        options = get_int_arg(vm, args[3])?;
    }

    let include_start_date = (options & 1) == 0;

    let data = DatePeriodData {
        start: start_data.dt.clone(),
        end,
        interval: interval_data,
        recurrences,
        include_start_date,
        current_date: RefCell::new(None),
        current_index: RefCell::new(0),
    };

    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(h) => *h,
        _ => return Err("Invalid 'this'".into()),
    };

    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        obj_data.internal = Some(Rc::new(data));
    }

    Ok(this_handle)
}

pub fn php_dateperiod_get_start_date(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;

    let dt_sym = vm.context.interner.intern(b"DateTimeImmutable");
    let now_str = vm.arena.alloc(Val::String(b"now".to_vec().into()));
    let dt_handle = vm.instantiate_class(dt_sym, &[now_str])?;
    let payload_handle = match &vm.arena.get(dt_handle).value {
        Val::Object(h) => *h,
        _ => return Err("Failed to create DateTimeImmutable".into()),
    };
    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        obj_data.internal = Some(Rc::new(DateTimeData {
            dt: data.start.clone(),
        }));
    }
    Ok(dt_handle)
}

pub fn php_dateperiod_get_end_date(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;

    if let Some(end) = &data.end {
        let dt_sym = vm.context.interner.intern(b"DateTimeImmutable");
        let now_str = vm.arena.alloc(Val::String(b"now".to_vec().into()));
        let dt_handle = vm.instantiate_class(dt_sym, &[now_str])?;
        let payload_handle = match &vm.arena.get(dt_handle).value {
            Val::Object(h) => *h,
            _ => return Err("Failed to create DateTimeImmutable".into()),
        };
        if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
            obj_data.internal = Some(Rc::new(DateTimeData { dt: end.clone() }));
        }
        Ok(dt_handle)
    } else {
        Ok(vm.arena.alloc(Val::Null))
    }
}

pub fn php_dateperiod_get_interval(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;

    let interval_sym = vm.context.interner.intern(b"DateInterval");
    let dummy_spec = vm.arena.alloc(Val::String(b"PT0S".to_vec().into()));
    let interval_handle = vm.instantiate_class(interval_sym, &[dummy_spec])?;
    let payload_handle = match &vm.arena.get(interval_handle).value {
        Val::Object(h) => *h,
        _ => return Err("Failed to create DateInterval".into()),
    };
    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        obj_data.internal = Some(data.interval.clone());
    }
    Ok(interval_handle)
}

pub fn php_dateperiod_get_recurrences(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;

    if let Some(r) = data.recurrences {
        Ok(vm.arena.alloc(Val::Int(r)))
    } else {
        Ok(vm.arena.alloc(Val::Null))
    }
}

pub fn php_dateperiod_rewind(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;

    *data.current_index.borrow_mut() = 0;
    if data.include_start_date {
        *data.current_date.borrow_mut() = Some(data.start.clone());
    } else {
        let next = add_interval(&data.start, &data.interval, false);
        *data.current_date.borrow_mut() = Some(next);
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_dateperiod_valid(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;

    let current = data.current_date.borrow();
    if current.is_none() {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }

    let current_dt = current.as_ref().unwrap();

    if let Some(end_dt) = &data.end {
        if current_dt >= end_dt {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
    }

    if let Some(max_recurrences) = data.recurrences {
        if *data.current_index.borrow() >= max_recurrences {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
    }

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_dateperiod_current(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;

    let current = data.current_date.borrow();
    if let Some(dt) = current.as_ref() {
        let dt_sym = vm.context.interner.intern(b"DateTime");
        let now_str = vm.arena.alloc(Val::String(b"now".to_vec().into()));
        let dt_handle = vm.instantiate_class(dt_sym, &[now_str])?;
        let payload_handle = match &vm.arena.get(dt_handle).value {
            Val::Object(h) => *h,
            _ => return Err("Failed to create DateTime".into()),
        };
        if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
            obj_data.internal = Some(Rc::new(DateTimeData { dt: dt.clone() }));
        }
        Ok(dt_handle)
    } else {
        Ok(vm.arena.alloc(Val::Null))
    }
}

pub fn php_dateperiod_key(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;
    let index = *data.current_index.borrow();
    Ok(vm.arena.alloc(Val::Int(index)))
}

pub fn php_dateperiod_next(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Missing 'this'")?;
    let data = get_internal_data::<DatePeriodData>(vm, this_handle)?;

    let mut current = data.current_date.borrow_mut();
    if let Some(dt) = current.as_ref() {
        let next = add_interval(dt, &data.interval, false);
        *current = Some(next);
        *data.current_index.borrow_mut() += 1;
    }

    Ok(vm.arena.alloc(Val::Null))
}

// ============================================================================
// Date/Time Functions
// ============================================================================

/// checkdate(int $month, int $day, int $year): bool
pub fn php_checkdate(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 3 {
        return Err("checkdate() expects exactly 3 parameters".into());
    }

    let month = get_int_arg(vm, args[0])?;
    let day = get_int_arg(vm, args[1])?;
    let year = get_int_arg(vm, args[2])?;

    let is_valid = month >= 1
        && month <= 12
        && year >= 1
        && year <= 32767
        && NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32).is_some();

    Ok(vm.arena.alloc(Val::Bool(is_valid)))
}

/// date(string $format, ?int $timestamp = null): string
pub fn php_date(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("date() expects 1 or 2 parameters".into());
    }

    let format = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();

    let timestamp = if args.len() == 2 {
        get_int_arg(vm, args[1])?
    } else {
        Utc::now().timestamp()
    };

    let tz = parse_timezone(&vm.context.config.timezone)?;
    let dt = Utc.timestamp_opt(timestamp, 0).unwrap().with_timezone(&tz);

    let formatted = format_php_date(&dt, &format);
    Ok(vm.arena.alloc(Val::String(formatted.into_bytes().into())))
}

/// gmdate(string $format, ?int $timestamp = null): string
pub fn php_gmdate(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("gmdate() expects 1 or 2 parameters".into());
    }

    let format = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();

    let timestamp = if args.len() == 2 {
        get_int_arg(vm, args[1])?
    } else {
        Utc::now().timestamp()
    };

    let dt = Utc
        .timestamp_opt(timestamp, 0)
        .unwrap()
        .with_timezone(&Tz::UTC);

    let formatted = format_php_date(&dt, &format);
    Ok(vm.arena.alloc(Val::String(formatted.into_bytes().into())))
}

/// time(): int
pub fn php_time(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if !args.is_empty() {
        return Err("time() expects exactly 0 parameters".into());
    }

    let timestamp = Utc::now().timestamp();
    Ok(vm.arena.alloc(Val::Int(timestamp)))
}

/// microtime(bool $as_float = false): string|float
pub fn php_microtime(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() > 1 {
        return Err("microtime() expects at most 1 parameter".into());
    }

    let as_float = if args.len() == 1 {
        let val = vm.arena.get(args[0]);
        matches!(&val.value, Val::Bool(true) | Val::Int(1))
    } else {
        false
    };

    let now = Utc::now();
    let secs = now.timestamp();
    let usecs = now.timestamp_subsec_micros();

    if as_float {
        let float_time = secs as f64 + (usecs as f64 / 1_000_000.0);
        Ok(vm.arena.alloc(Val::Float(float_time)))
    } else {
        let result = format!("0.{:06} {}", usecs, secs);
        Ok(vm.arena.alloc(Val::String(result.into_bytes().into())))
    }
}

/// sleep(int $seconds): int
/// Reference: $PHP_SRC_PATH/ext/standard/basic_functions.c - PHP_FUNCTION(sleep)
pub fn php_sleep(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("sleep() expects exactly 1 parameter".into());
    }
    let seconds = vm.arena.get(args[0]).value.to_int();
    if seconds < 0 {
        return Err("sleep(): Argument #1 ($seconds) must be greater than or equal to 0".into());
    }
    std::thread::sleep(std::time::Duration::from_secs(seconds as u64));
    Ok(vm.arena.alloc(Val::Int(0)))
}

/// hrtime(bool $as_number = false): array|int
/// Returns high resolution time
/// Reference: $PHP_SRC_PATH/ext/standard/hrtime.c
pub fn php_hrtime(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() > 1 {
        return Err("hrtime() expects at most 1 parameter".into());
    }

    let as_number = if args.len() == 1 {
        let val = vm.arena.get(args[0]);
        matches!(&val.value, Val::Bool(true) | Val::Int(1))
    } else {
        false
    };

    // Get current time with nanosecond precision
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    
    let secs = now.as_secs() as i64;
    let nanos = now.subsec_nanos() as i64;

    if as_number {
        // Return total nanoseconds as integer
        let total_nanos = secs * 1_000_000_000 + nanos;
        Ok(vm.arena.alloc(Val::Int(total_nanos)))
    } else {
        // Return array [seconds, nanoseconds]
        let mut result_arr = indexmap::IndexMap::new();
        result_arr.insert(
            crate::core::value::ArrayKey::Int(0),
            vm.arena.alloc(Val::Int(secs)),
        );
        result_arr.insert(
            crate::core::value::ArrayKey::Int(1),
            vm.arena.alloc(Val::Int(nanos)),
        );
        Ok(vm.arena.alloc(Val::Array(
            crate::core::value::ArrayData::from(result_arr).into(),
        )))
    }
}

/// mktime(int $hour, ?int $minute = null, ?int $second = null, ?int $month = null, ?int $day = null, ?int $year = null): int|false
pub fn php_mktime(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 6 {
        return Err("mktime() expects 0 to 6 parameters".into());
    }

    let now = Local::now();

    let hour = if !args.is_empty() {
        get_int_arg(vm, args[0])? as u32
    } else {
        now.hour()
    };

    let minute = if args.len() > 1 {
        get_int_arg(vm, args[1])? as u32
    } else {
        now.minute()
    };

    let second = if args.len() > 2 {
        get_int_arg(vm, args[2])? as u32
    } else {
        now.second()
    };

    let month = if args.len() > 3 {
        get_int_arg(vm, args[3])? as u32
    } else {
        now.month()
    };

    let day = if args.len() > 4 {
        get_int_arg(vm, args[4])? as u32
    } else {
        now.day()
    };

    let year = if args.len() > 5 {
        get_int_arg(vm, args[5])? as i32
    } else {
        now.year()
    };

    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => match NaiveTime::from_hms_opt(hour, minute, second) {
            Some(time) => {
                let dt = NaiveDateTime::new(date, time);
                let timestamp = dt.and_utc().timestamp();
                Ok(vm.arena.alloc(Val::Int(timestamp)))
            }
            None => Ok(vm.arena.alloc(Val::Bool(false))),
        },
        None => Ok(vm.arena.alloc(Val::Bool(false))),
    }
}

/// gmmktime(int $hour, ?int $minute = null, ?int $second = null, ?int $month = null, ?int $day = null, ?int $year = null): int|false
pub fn php_gmmktime(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // Same as mktime but always uses UTC
    php_mktime(vm, args)
}

/// strtotime(string $datetime, ?int $baseTimestamp = null): int|false
pub fn php_strtotime(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("strtotime() expects 1 or 2 parameters".into());
    }

    let datetime_str = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();

    let base_timestamp = if args.len() == 2 {
        get_int_arg(vm, args[1])?
    } else {
        Utc::now().timestamp()
    };

    match parse_strtotime(&datetime_str, base_timestamp) {
        Some(timestamp) => Ok(vm.arena.alloc(Val::Int(timestamp))),
        None => Ok(vm.arena.alloc(Val::Bool(false))),
    }
}

// ============================================================================
// strtotime Parser Implementation
// ============================================================================

fn parse_strtotime(input: &str, base_timestamp: i64) -> Option<i64> {
    let input = input.trim().to_lowercase();
    
    if input.is_empty() {
        return None;
    }

    let base_dt = match Utc.timestamp_opt(base_timestamp, 0) {
        chrono::LocalResult::Single(dt) => dt.naive_utc(),
        _ => return None,
    };

    // Unix timestamp format (@timestamp)
    if let Some(stripped) = input.strip_prefix('@') {
        if let Ok(ts) = stripped.trim().parse::<i64>() {
            return Some(ts);
        }
    }

    // Special keywords
    if let Some(ts) = parse_special_keyword(&input, &base_dt) {
        return Some(ts);
    }

    // Relative time formats (+1 day, -2 weeks, etc.)
    if let Some(ts) = parse_relative_time(&input, &base_dt) {
        return Some(ts);
    }

    // Weekday references (next monday, last friday, etc.)
    if let Some(ts) = parse_weekday_reference(&input, &base_dt) {
        return Some(ts);
    }

    // Special phrases (first day of next month, last day of this month, etc.)
    if let Some(ts) = parse_special_phrase(&input, &base_dt) {
        return Some(ts);
    }

    // Absolute date/time formats
    parse_absolute_datetime(&input, &base_dt)
}

fn parse_special_keyword(input: &str, base_dt: &NaiveDateTime) -> Option<i64> {
    match input {
        "now" => Some(base_dt.and_utc().timestamp()),
        "today" => {
            let date = base_dt.date();
            Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
        }
        "tomorrow" => {
            let date = base_dt.date().succ_opt()?;
            Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
        }
        "yesterday" => {
            let date = base_dt.date().pred_opt()?;
            Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
        }
        "midnight" => {
            let date = base_dt.date();
            Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
        }
        "noon" => {
            let date = base_dt.date();
            Some(date.and_hms_opt(12, 0, 0)?.and_utc().timestamp())
        }
        _ => None,
    }
}

fn parse_relative_time(input: &str, base_dt: &NaiveDateTime) -> Option<i64> {
    // Pattern: [+/-]number unit[s] [ago]
    if let Some(caps) = RELATIVE_TIME_RE.captures(input) {
        let mut amount: i64 = caps.get(1)?.as_str().parse().ok()?;
        let unit = caps.get(2)?.as_str();
        let is_ago = caps.get(3).is_some();

        if is_ago {
            amount = -amount;
        }

        let result_dt = match unit {
            "year" => add_years(base_dt, amount)?,
            "month" => add_months(base_dt, amount)?,
            "week" => *base_dt + chrono::Duration::try_weeks(amount)?,
            "fortnight" => *base_dt + chrono::Duration::try_weeks(amount * 2)?,
            "day" => *base_dt + chrono::Duration::try_days(amount)?,
            "hour" => *base_dt + chrono::Duration::try_hours(amount)?,
            "minute" => *base_dt + chrono::Duration::try_minutes(amount)?,
            "second" => *base_dt + chrono::Duration::try_seconds(amount)?,
            _ => return None,
        };

        return Some(result_dt.and_utc().timestamp());
    }

    None
}

fn parse_weekday_reference(input: &str, base_dt: &NaiveDateTime) -> Option<i64> {
    // Pattern: (next|last|this) weekday
    if let Some(caps) = WEEKDAY_REFERENCE_RE.captures(input) {
        let direction = caps.get(1)?.as_str();
        let weekday_str = caps.get(2)?.as_str();
        
        let target_weekday = parse_weekday(weekday_str)?;
        let current_weekday = base_dt.date().weekday();
        
        let result_date = match direction {
            "next" => {
                // PHP's "next" means: if target day hasn't occurred this week, use it; otherwise next week
                let days_diff = target_weekday.num_days_from_monday() as i64
                    - current_weekday.num_days_from_monday() as i64;
                let days_ahead = if days_diff > 0 {
                    days_diff
                } else {
                    7 + days_diff
                };
                base_dt.date() + chrono::Duration::try_days(days_ahead)?
            }
            "last" | "previous" => {
                // PHP's "last" means: if target day already occurred this week, use it; otherwise last week
                let days_diff = current_weekday.num_days_from_monday() as i64
                    - target_weekday.num_days_from_monday() as i64;
                let days_behind = if days_diff > 0 {
                    days_diff
                } else {
                    7 + days_diff
                };
                base_dt.date() - chrono::Duration::try_days(days_behind)?
            }
            "this" => {
                let days_diff = target_weekday.num_days_from_monday() as i64
                    - current_weekday.num_days_from_monday() as i64;
                if days_diff >= 0 {
                    base_dt.date() + chrono::Duration::try_days(days_diff)?
                } else {
                    base_dt.date() + chrono::Duration::try_days(7 + days_diff)?
                }
            }
            _ => return None,
        };

        return Some(result_date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }

    None
}

fn parse_special_phrase(input: &str, base_dt: &NaiveDateTime) -> Option<i64> {
    // Pattern: (first|last) day of (next|this|last) month
    if let Some(caps) = SPECIAL_PHRASE_RE.captures(input) {
        let first_last = caps.get(1)?.as_str();
        let which_month = caps.get(2)?.as_str();
        
        let target_date = match which_month {
            "next" => {
                let next_month_dt = add_months(base_dt, 1)?;
                next_month_dt.date()
            }
            "last" => {
                let last_month_dt = add_months(base_dt, -1)?;
                last_month_dt.date()
            }
            "this" => base_dt.date(),
            _ => return None,
        };

        let result_date = match first_last {
            "first" => {
                NaiveDate::from_ymd_opt(target_date.year(), target_date.month(), 1)?
            }
            "last" => {
                // Last day of month
                let next_month = if target_date.month() == 12 {
                    NaiveDate::from_ymd_opt(target_date.year() + 1, 1, 1)?
                } else {
                    NaiveDate::from_ymd_opt(target_date.year(), target_date.month() + 1, 1)?
                };
                next_month.pred_opt()?
            }
            _ => return None,
        };

        return Some(result_date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }

    None
}

fn parse_absolute_datetime(input: &str, base_dt: &NaiveDateTime) -> Option<i64> {
    // Try various datetime formats
    
    // Strip optional 't' or 'T' prefix for time formats (gnunocolon, iso8601nocolon)
    let (time_input, has_time_prefix) = if input.starts_with('t') || input.starts_with('T') {
        (&input[1..], true)
    } else {
        (input, false)
    };
    
    // gnunocolon with optional 't': t?HHMM (4-5 chars total)
    // HHMM (4 digits): 1530 → 15:30:00 today, t1530 → same
    if has_time_prefix && time_input.len() == 4 && time_input.chars().all(|c| c.is_ascii_digit()) {
        if let (Ok(hour), Ok(minute)) = (
            time_input[..2].parse::<u32>(),
            time_input[2..4].parse::<u32>()
        ) {
            if hour < 24 && minute < 60 {
                let today = base_dt.date();
                if let Some(time) = NaiveTime::from_hms_opt(hour, minute, 0) {
                    let dt = NaiveDateTime::new(today, time);
                    return Some(dt.and_utc().timestamp());
                }
            }
        }
    }
    
    // iso8601nocolon with optional 't': t?HHMMSS (6-7 chars total)
    // HHMMSS (6 digits): 202613 → 20:26:13 today, t202613 → same
    if has_time_prefix && time_input.len() == 6 && time_input.chars().all(|c| c.is_ascii_digit()) {
        if let (Ok(hour), Ok(minute), Ok(second)) = (
            time_input[..2].parse::<u32>(),
            time_input[2..4].parse::<u32>(),
            time_input[4..6].parse::<u32>()
        ) {
            if hour < 24 && minute < 60 && second < 60 {
                let today = base_dt.date();
                if let Some(time) = NaiveTime::from_hms_opt(hour, minute, second) {
                    let dt = NaiveDateTime::new(today, time);
                    return Some(dt.and_utc().timestamp());
                }
            }
        }
    }
    
    // Only try bare digit time formats if no 't' prefix
    if !has_time_prefix {
        // gnunocolon: HHMM (4 digits) → time today
        if input.len() == 4 && input.chars().all(|c| c.is_ascii_digit()) {
            if let (Ok(hour), Ok(minute)) = (
                input[..2].parse::<u32>(),
                input[2..4].parse::<u32>()
            ) {
                if hour < 24 && minute < 60 {
                    let today = base_dt.date();
                    if let Some(time) = NaiveTime::from_hms_opt(hour, minute, 0) {
                        let dt = NaiveDateTime::new(today, time);
                        return Some(dt.and_utc().timestamp());
                    }
                }
                // If HHMM validation fails (hour >= 24 or minute >= 60), try as year4
                // This handles cases like '2560', '2461' which should be years
                if let Ok(year) = input.parse::<i32>() {
                    let today = base_dt.date();
                    let month = today.month();
                    let day = today.day();
                    if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
                    }
                }
            }
        }
        
        // iso8601nocolon: HHMMSS (6 digits) → time today
        if input.len() == 6 && input.chars().all(|c| c.is_ascii_digit()) {
            if let (Ok(hour), Ok(minute), Ok(second)) = (
                input[..2].parse::<u32>(),
                input[2..4].parse::<u32>(),
                input[4..6].parse::<u32>()
            ) {
                if hour < 24 && minute < 60 && second < 60 {
                    let today = base_dt.date();
                    if let Some(time) = NaiveTime::from_hms_opt(hour, minute, second) {
                        let dt = NaiveDateTime::new(today, time);
                        return Some(dt.and_utc().timestamp());
                    }
                }
            }
        }
        
        // pgydotd: YYYY-DDD or YYYY.DDD (8 chars with separator)
        if input.len() == 8 {
            let sep_pos = input.chars().position(|c| c == '-' || c == '.');
            if let Some(4) = sep_pos {
                let year_part = &input[..4];
                let doy_part = &input[5..];
                if year_part.chars().all(|c| c.is_ascii_digit()) && doy_part.chars().all(|c| c.is_ascii_digit()) {
                    if let (Ok(year), Ok(day_of_year)) = (
                        year_part.parse::<i32>(),
                        doy_part.parse::<u32>()
                    ) {
                        if day_of_year >= 1 && day_of_year <= 366 {
                            if let Some(date) = NaiveDate::from_yo_opt(year, day_of_year) {
                                return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
                            }
                        }
                    }
                }
            }
        }
        
        // pgydotd: YYYYDDD (7 digits), e.g., 2026113 = 2026 day 113 = April 23
        if input.len() == 7 && input.chars().all(|c| c.is_ascii_digit()) {
            if let (Ok(year), Ok(day_of_year)) = (
                input[..4].parse::<i32>(),
                input[4..].parse::<u32>()
            ) {
                if day_of_year >= 1 && day_of_year <= 366 {
                    if let Some(date) = NaiveDate::from_yo_opt(year, day_of_year) {
                        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
                    }
                }
            }
        }
        
        // datenocolon: YYYYMMDD (8 digits), e.g., 20260113 = 2026-01-13
        if input.len() == 8 && input.chars().all(|c| c.is_ascii_digit()) {
            if let (Ok(year), Ok(month), Ok(day)) = (
                input[..4].parse::<i32>(),
                input[4..6].parse::<u32>(),
                input[6..8].parse::<u32>()
            ) {
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                    return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
                }
            }
        }
        
        // MySQL format: YYYYMMDDHHMMSS (14 digits)
        if input.len() == 14 && input.chars().all(|c| c.is_ascii_digit()) {
            if let (Ok(year), Ok(month), Ok(day), Ok(hour), Ok(minute), Ok(second)) = (
                input[..4].parse::<i32>(),
                input[4..6].parse::<u32>(),
                input[6..8].parse::<u32>(),
                input[8..10].parse::<u32>(),
                input[10..12].parse::<u32>(),
                input[12..14].parse::<u32>()
            ) {
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                    if let Some(time) = NaiveTime::from_hms_opt(hour, minute, second) {
                        let dt = NaiveDateTime::new(date, time);
                        return Some(dt.and_utc().timestamp());
                    }
                }
            }
        }
    }
    
    // Time-only formats (HH:MM:SS, HH:MM) - apply to base date
    // Try HH:MM:SS first
    if let Ok(time) = NaiveTime::parse_from_str(input, "%H:%M:%S") {
        let date = base_dt.date();
        let dt = NaiveDateTime::new(date, time);
        return Some(dt.and_utc().timestamp());
    }
    
    // Try HH:MM
    if let Ok(time) = NaiveTime::parse_from_str(input, "%H:%M") {
        let date = base_dt.date();
        let dt = NaiveDateTime::new(date, time);
        return Some(dt.and_utc().timestamp());
    }
    
    // ISO 8601: 2024-01-15T14:30:00Z or 2024-01-15T14:30:00+00:00
    if let Ok(dt) = ChronoDateTime::parse_from_rfc3339(input) {
        return Some(dt.timestamp());
    }

    // Standard datetime: 2024-01-15 14:30:00
    if let Ok(dt) = NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp());
    }

    // Date only: 2024-01-15
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }

    // US format: 01/15/2024
    if let Ok(date) = NaiveDate::parse_from_str(input, "%m/%d/%Y") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }

    // Day Month Year: 15 Jan 2024
    if let Ok(date) = NaiveDate::parse_from_str(input, "%d %b %Y") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }

    // Month Day Year: Jan 15 2024
    if let Ok(date) = NaiveDate::parse_from_str(input, "%b %d %Y") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }

    // Day-Month-Year: 15-Jan-2024
    if let Ok(date) = NaiveDate::parse_from_str(input, "%d-%b-%Y") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }

    // RFC 2822
    if let Ok(dt) = ChronoDateTime::parse_from_rfc2822(input) {
        return Some(dt.timestamp());
    }

    None
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    match s {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn add_months(dt: &NaiveDateTime, months: i64) -> Option<NaiveDateTime> {
    let date = dt.date();
    let mut year = date.year();
    let mut month = date.month() as i32;
    
    month += months as i32;
    
    while month > 12 {
        month -= 12;
        year += 1;
    }
    
    while month < 1 {
        month += 12;
        year -= 1;
    }
    
    // PHP allows day overflow - if day is invalid for the month, it overflows to next month
    // e.g., Jan 31 + 1 month = March 2 (or March 3 depending on leap year)
    let day = date.day();
    
    // Try to create the date; if it fails, add the excess days to the next month
    if let Some(new_date) = NaiveDate::from_ymd_opt(year, month as u32, day) {
        return Some(NaiveDateTime::new(new_date, dt.time()));
    }
    
    // Day is too large for this month, so overflow
    // Get the last valid day of the target month
    let mut test_day = day;
    while test_day > 1 {
        if let Some(_) = NaiveDate::from_ymd_opt(year, month as u32, test_day) {
            break;
        }
        test_day -= 1;
    }
    
    // Calculate overflow days
    let overflow_days = (day - test_day) as i64;
    
    // Create date with last valid day of month, then add overflow
    if let Some(base_date) = NaiveDate::from_ymd_opt(year, month as u32, test_day) {
        let result_date = base_date + chrono::Duration::try_days(overflow_days)?;
        return Some(NaiveDateTime::new(result_date, dt.time()));
    }
    
    None
}

fn add_years(dt: &NaiveDateTime, years: i64) -> Option<NaiveDateTime> {
    let date = dt.date();
    let new_year = date.year() + years as i32;
    
    // PHP allows day overflow for leap year handling
    // e.g., Feb 29 2024 + 1 year = March 1 2025
    let day = date.day();
    let month = date.month();
    
    if let Some(new_date) = NaiveDate::from_ymd_opt(new_year, month, day) {
        return Some(NaiveDateTime::new(new_date, dt.time()));
    }
    
    // Day doesn't exist in the target year (e.g., Feb 29 on non-leap year)
    // Calculate overflow
    let mut test_day = day;
    while test_day > 1 {
        if let Some(_) = NaiveDate::from_ymd_opt(new_year, month, test_day) {
            break;
        }
        test_day -= 1;
    }
    
    let overflow_days = (day - test_day) as i64;
    
    if let Some(base_date) = NaiveDate::from_ymd_opt(new_year, month, test_day) {
        let result_date = base_date + chrono::Duration::try_days(overflow_days)?;
        return Some(NaiveDateTime::new(result_date, dt.time()));
    }
    
    None
}

/// getdate(?int $timestamp = null): array
pub fn php_getdate(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() > 1 {
        return Err("getdate() expects at most 1 parameter".into());
    }

    let timestamp = if args.len() == 1 {
        get_int_arg(vm, args[0])?
    } else {
        Utc::now().timestamp()
    };

    let dt = Local.timestamp_opt(timestamp, 0).unwrap();

    let mut map = IndexMap::new();
    map.insert(
        make_array_key("seconds"),
        vm.arena.alloc(Val::Int(dt.second() as i64)),
    );
    map.insert(
        make_array_key("minutes"),
        vm.arena.alloc(Val::Int(dt.minute() as i64)),
    );
    map.insert(
        make_array_key("hours"),
        vm.arena.alloc(Val::Int(dt.hour() as i64)),
    );
    map.insert(
        make_array_key("mday"),
        vm.arena.alloc(Val::Int(dt.day() as i64)),
    );
    map.insert(
        make_array_key("wday"),
        vm.arena
            .alloc(Val::Int(dt.weekday().number_from_sunday() as i64)),
    );
    map.insert(
        make_array_key("mon"),
        vm.arena.alloc(Val::Int(dt.month() as i64)),
    );
    map.insert(
        make_array_key("year"),
        vm.arena.alloc(Val::Int(dt.year() as i64)),
    );
    map.insert(
        make_array_key("yday"),
        vm.arena.alloc(Val::Int(dt.ordinal0() as i64)),
    );

    let weekday = match dt.weekday() {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    };
    map.insert(
        make_array_key("weekday"),
        vm.arena
            .alloc(Val::String(weekday.as_bytes().to_vec().into())),
    );

    let month = match dt.month() {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "",
    };
    map.insert(
        make_array_key("month"),
        vm.arena
            .alloc(Val::String(month.as_bytes().to_vec().into())),
    );

    map.insert(make_array_key("0"), vm.arena.alloc(Val::Int(timestamp)));

    Ok(vm
        .arena
        .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
            map,
            next_free: 0,
        }))))
}

/// idate(string $format, ?int $timestamp = null): int|false
pub fn php_idate(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("idate() expects 1 or 2 parameters".into());
    }

    let format = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();
    if format.len() != 1 {
        return Err("idate() format must be exactly one character".into());
    }

    let timestamp = if args.len() == 2 {
        get_int_arg(vm, args[1])?
    } else {
        Utc::now().timestamp()
    };

    let dt = Local.timestamp_opt(timestamp, 0).unwrap();

    let result = match format.chars().next().unwrap() {
        'B' => {
            let seconds = (dt.hour() * 3600 + dt.minute() * 60 + dt.second()) as f64;
            ((seconds + 3600.0) / 86.4).floor() as i64 % 1000
        }
        'd' => dt.day() as i64,
        'h' => {
            let hour = dt.hour();
            (if hour == 0 || hour == 12 {
                12
            } else {
                hour % 12
            }) as i64
        }
        'H' => dt.hour() as i64,
        'i' => dt.minute() as i64,
        'I' => 0, // Simplified
        'L' => {
            if NaiveDate::from_ymd_opt(dt.year(), 2, 29).is_some() {
                1
            } else {
                0
            }
        }
        'm' => dt.month() as i64,
        's' => dt.second() as i64,
        't' => {
            let days_in_month = NaiveDate::from_ymd_opt(dt.year(), dt.month() + 1, 1)
                .unwrap_or(NaiveDate::from_ymd_opt(dt.year() + 1, 1, 1).unwrap())
                .signed_duration_since(NaiveDate::from_ymd_opt(dt.year(), dt.month(), 1).unwrap())
                .num_days();
            days_in_month
        }
        'U' => timestamp,
        'w' => dt.weekday().number_from_sunday() as i64,
        'W' => dt.iso_week().week() as i64,
        'y' => (dt.year() % 100) as i64,
        'Y' => dt.year() as i64,
        'z' => dt.ordinal0() as i64,
        'Z' => dt.offset().fix().local_minus_utc() as i64,
        _ => return Err("idate(): Invalid format character".into()),
    };

    Ok(vm.arena.alloc(Val::Int(result)))
}

/// gettimeofday(bool $as_float = false): array|float
pub fn php_gettimeofday(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() > 1 {
        return Err("gettimeofday() expects at most 1 parameter".into());
    }

    let as_float = if args.len() == 1 {
        let val = vm.arena.get(args[0]);
        matches!(&val.value, Val::Bool(true) | Val::Int(1))
    } else {
        false
    };

    let now = Utc::now();
    let secs = now.timestamp();
    let usecs = now.timestamp_subsec_micros();

    if as_float {
        let float_time = secs as f64 + (usecs as f64 / 1_000_000.0);
        Ok(vm.arena.alloc(Val::Float(float_time)))
    } else {
        let mut map = IndexMap::new();
        map.insert(make_array_key("sec"), vm.arena.alloc(Val::Int(secs)));
        map.insert(
            make_array_key("usec"),
            vm.arena.alloc(Val::Int(usecs as i64)),
        );
        map.insert(make_array_key("minuteswest"), vm.arena.alloc(Val::Int(0)));
        map.insert(make_array_key("dsttime"), vm.arena.alloc(Val::Int(0)));

        Ok(vm
            .arena
            .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
                map,
                next_free: 0,
            }))))
    }
}

/// localtime(?int $timestamp = null, bool $associative = false): array
pub fn php_localtime(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() > 2 {
        return Err("localtime() expects at most 2 parameters".into());
    }

    let timestamp = if !args.is_empty() {
        get_int_arg(vm, args[0])?
    } else {
        Utc::now().timestamp()
    };

    let associative = if args.len() == 2 {
        let val = vm.arena.get(args[1]);
        matches!(&val.value, Val::Bool(true) | Val::Int(1))
    } else {
        false
    };

    let dt = Local.timestamp_opt(timestamp, 0).unwrap();

    let mut map = IndexMap::new();

    if associative {
        map.insert(
            make_array_key("tm_sec"),
            vm.arena.alloc(Val::Int(dt.second() as i64)),
        );
        map.insert(
            make_array_key("tm_min"),
            vm.arena.alloc(Val::Int(dt.minute() as i64)),
        );
        map.insert(
            make_array_key("tm_hour"),
            vm.arena.alloc(Val::Int(dt.hour() as i64)),
        );
        map.insert(
            make_array_key("tm_mday"),
            vm.arena.alloc(Val::Int(dt.day() as i64)),
        );
        map.insert(
            make_array_key("tm_mon"),
            vm.arena.alloc(Val::Int((dt.month() - 1) as i64)),
        );
        map.insert(
            make_array_key("tm_year"),
            vm.arena.alloc(Val::Int((dt.year() - 1900) as i64)),
        );
        map.insert(
            make_array_key("tm_wday"),
            vm.arena
                .alloc(Val::Int(dt.weekday().number_from_sunday() as i64)),
        );
        map.insert(
            make_array_key("tm_yday"),
            vm.arena.alloc(Val::Int(dt.ordinal0() as i64)),
        );
        map.insert(make_array_key("tm_isdst"), vm.arena.alloc(Val::Int(0)));
    } else {
        map.insert(
            make_array_key("0"),
            vm.arena.alloc(Val::Int(dt.second() as i64)),
        );
        map.insert(
            make_array_key("1"),
            vm.arena.alloc(Val::Int(dt.minute() as i64)),
        );
        map.insert(
            make_array_key("2"),
            vm.arena.alloc(Val::Int(dt.hour() as i64)),
        );
        map.insert(
            make_array_key("3"),
            vm.arena.alloc(Val::Int(dt.day() as i64)),
        );
        map.insert(
            make_array_key("4"),
            vm.arena.alloc(Val::Int((dt.month() - 1) as i64)),
        );
        map.insert(
            make_array_key("5"),
            vm.arena.alloc(Val::Int((dt.year() - 1900) as i64)),
        );
        map.insert(
            make_array_key("6"),
            vm.arena
                .alloc(Val::Int(dt.weekday().number_from_sunday() as i64)),
        );
        map.insert(
            make_array_key("7"),
            vm.arena.alloc(Val::Int(dt.ordinal0() as i64)),
        );
        map.insert(make_array_key("8"), vm.arena.alloc(Val::Int(0)));
    }

    Ok(vm
        .arena
        .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
            map,
            next_free: if associative { 0 } else { 9 },
        }))))
}

// ============================================================================
// Timezone Functions
// ============================================================================

/// date_default_timezone_get(): string
pub fn php_date_default_timezone_get(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::String(
        vm.context.config.timezone.as_bytes().to_vec().into(),
    )))
}

/// date_default_timezone_set(string $timezoneId): bool
pub fn php_date_default_timezone_set(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("date_default_timezone_set() expects exactly 1 parameter".into());
    }

    let tz_str = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();

    // Validate timezone
    match parse_timezone(&tz_str) {
        Ok(_) => {
            vm.context.config.timezone = tz_str;
            Ok(vm.arena.alloc(Val::Bool(true)))
        }
        Err(_) => Ok(vm.arena.alloc(Val::Bool(false))),
    }
}

// ============================================================================
// Sun Functions (Simplified - deprecated in PHP 8.4)
// ============================================================================

/// date_sunrise(int $timestamp, int $returnFormat = SUNFUNCS_RET_STRING, ?float $latitude = null, ?float $longitude = null, ?float $zenith = null, ?float $utcOffset = null): string|int|float|false
pub fn php_date_sunrise(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 6 {
        return Err("date_sunrise() expects 1 to 6 parameters".into());
    }

    // Simplified implementation - just return a fixed sunrise time
    let return_format = if args.len() > 1 {
        get_int_arg(vm, args[1])?
    } else {
        SUNFUNCS_RET_STRING
    };

    match return_format {
        0 => Ok(vm.arena.alloc(Val::Int(1234567890))), // SUNFUNCS_RET_TIMESTAMP
        1 => Ok(vm
            .arena
            .alloc(Val::String("06:00".as_bytes().to_vec().into()))), // SUNFUNCS_RET_STRING
        2 => Ok(vm.arena.alloc(Val::Float(6.0))),      // SUNFUNCS_RET_DOUBLE
        _ => Ok(vm.arena.alloc(Val::Bool(false))),
    }
}

/// date_sunset(int $timestamp, int $returnFormat = SUNFUNCS_RET_STRING, ?float $latitude = null, ?float $longitude = null, ?float $zenith = null, ?float $utcOffset = null): string|int|float|false
pub fn php_date_sunset(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 6 {
        return Err("date_sunset() expects 1 to 6 parameters".into());
    }

    // Simplified implementation
    let return_format = if args.len() > 1 {
        get_int_arg(vm, args[1])?
    } else {
        SUNFUNCS_RET_STRING
    };

    match return_format {
        0 => Ok(vm.arena.alloc(Val::Int(1234567890))),
        1 => Ok(vm
            .arena
            .alloc(Val::String("18:00".as_bytes().to_vec().into()))),
        2 => Ok(vm.arena.alloc(Val::Float(18.0))),
        _ => Ok(vm.arena.alloc(Val::Bool(false))),
    }
}

/// date_sun_info(int $timestamp, float $latitude, float $longitude): array
pub fn php_date_sun_info(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 3 {
        return Err("date_sun_info() expects exactly 3 parameters".into());
    }

    let _timestamp = get_int_arg(vm, args[0])?;
    let _latitude = get_float_arg(vm, args[1])?;
    let _longitude = get_float_arg(vm, args[2])?;

    // Simplified implementation - return placeholder data
    let mut map = IndexMap::new();
    map.insert(
        make_array_key("sunrise"),
        vm.arena.alloc(Val::Int(1234567890)),
    );
    map.insert(
        make_array_key("sunset"),
        vm.arena.alloc(Val::Int(1234567890)),
    );
    map.insert(
        make_array_key("transit"),
        vm.arena.alloc(Val::Int(1234567890)),
    );
    map.insert(
        make_array_key("civil_twilight_begin"),
        vm.arena.alloc(Val::Int(1234567890)),
    );
    map.insert(
        make_array_key("civil_twilight_end"),
        vm.arena.alloc(Val::Int(1234567890)),
    );
    map.insert(
        make_array_key("nautical_twilight_begin"),
        vm.arena.alloc(Val::Int(1234567890)),
    );
    map.insert(
        make_array_key("nautical_twilight_end"),
        vm.arena.alloc(Val::Int(1234567890)),
    );
    map.insert(
        make_array_key("astronomical_twilight_begin"),
        vm.arena.alloc(Val::Int(1234567890)),
    );
    map.insert(
        make_array_key("astronomical_twilight_end"),
        vm.arena.alloc(Val::Int(1234567890)),
    );

    Ok(vm
        .arena
        .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
            map,
            next_free: 0,
        }))))
}

/// date_create(string $datetime = "now", ?DateTimeZone $timezone = null): DateTime|false
pub fn php_date_create(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let datetime_sym = vm.context.interner.intern(b"DateTime");
    vm.instantiate_class(datetime_sym, args)
}

/// date_create_immutable(string $datetime = "now", ?DateTimeZone $timezone = null): DateTimeImmutable|false
pub fn php_date_create_immutable(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let datetime_immutable_sym = vm.context.interner.intern(b"DateTimeImmutable");
    vm.instantiate_class(datetime_immutable_sym, args)
}

/// date_format(DateTimeInterface $object, string $format): string
pub fn php_date_format(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("date_format() expects at least 2 parameters".into());
    }

    // We can't easily call the method on the object here without more VM infrastructure
    // But we can just call the internal implementation
    let data = get_internal_data::<DateTimeData>(vm, args[0])?;
    let format = String::from_utf8_lossy(&get_string_arg(vm, args[1])?).to_string();
    let formatted = format_php_date(&data.dt, &format);

    Ok(vm.arena.alloc(Val::String(formatted.into_bytes().into())))
}

// ============================================================================
// Date Parsing Functions
// ============================================================================

/// date_parse(string $datetime): array
pub fn php_date_parse(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("date_parse() expects exactly 1 parameter".into());
    }

    let datetime_str = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();

    // Simplified parsing - in real PHP this is very complex
    let mut map = IndexMap::new();

    // Try to parse and extract components
    if let Ok(dt) = NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S") {
        map.insert(
            make_array_key("year"),
            vm.arena.alloc(Val::Int(dt.year() as i64)),
        );
        map.insert(
            make_array_key("month"),
            vm.arena.alloc(Val::Int(dt.month() as i64)),
        );
        map.insert(
            make_array_key("day"),
            vm.arena.alloc(Val::Int(dt.day() as i64)),
        );
        map.insert(
            make_array_key("hour"),
            vm.arena.alloc(Val::Int(dt.hour() as i64)),
        );
        map.insert(
            make_array_key("minute"),
            vm.arena.alloc(Val::Int(dt.minute() as i64)),
        );
        map.insert(
            make_array_key("second"),
            vm.arena.alloc(Val::Int(dt.second() as i64)),
        );
    } else {
        // Return false values
        map.insert(make_array_key("year"), vm.arena.alloc(Val::Bool(false)));
        map.insert(make_array_key("month"), vm.arena.alloc(Val::Bool(false)));
        map.insert(make_array_key("day"), vm.arena.alloc(Val::Bool(false)));
        map.insert(make_array_key("hour"), vm.arena.alloc(Val::Bool(false)));
        map.insert(make_array_key("minute"), vm.arena.alloc(Val::Bool(false)));
        map.insert(make_array_key("second"), vm.arena.alloc(Val::Bool(false)));
    }

    map.insert(make_array_key("fraction"), vm.arena.alloc(Val::Float(0.0)));
    map.insert(make_array_key("warning_count"), vm.arena.alloc(Val::Int(0)));
    map.insert(
        make_array_key("warnings"),
        vm.arena
            .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
                map: IndexMap::new(),
                next_free: 0,
            }))),
    );
    map.insert(make_array_key("error_count"), vm.arena.alloc(Val::Int(0)));
    map.insert(
        make_array_key("errors"),
        vm.arena
            .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
                map: IndexMap::new(),
                next_free: 0,
            }))),
    );
    map.insert(
        make_array_key("is_localtime"),
        vm.arena.alloc(Val::Bool(false)),
    );

    Ok(vm
        .arena
        .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
            map,
            next_free: 0,
        }))))
}

/// date_parse_from_format(string $format, string $datetime): array
pub fn php_date_parse_from_format(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("date_parse_from_format() expects exactly 2 parameters".into());
    }

    let _format = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();
    let _datetime_str = String::from_utf8_lossy(&get_string_arg(vm, args[1])?).to_string();

    // Simplified implementation - return basic structure
    let mut map = IndexMap::new();
    map.insert(
        ArrayKey::Str(Rc::new("year".as_bytes().to_vec())),
        vm.arena.alloc(Val::Bool(false)),
    );
    map.insert(
        ArrayKey::Str(Rc::new("month".as_bytes().to_vec())),
        vm.arena.alloc(Val::Bool(false)),
    );
    map.insert(
        ArrayKey::Str(Rc::new("day".as_bytes().to_vec())),
        vm.arena.alloc(Val::Bool(false)),
    );
    map.insert(
        ArrayKey::Str(Rc::new("hour".as_bytes().to_vec())),
        vm.arena.alloc(Val::Bool(false)),
    );
    map.insert(
        ArrayKey::Str(Rc::new("minute".as_bytes().to_vec())),
        vm.arena.alloc(Val::Bool(false)),
    );
    map.insert(
        ArrayKey::Str(Rc::new("second".as_bytes().to_vec())),
        vm.arena.alloc(Val::Bool(false)),
    );
    map.insert(make_array_key("fraction"), vm.arena.alloc(Val::Float(0.0)));
    map.insert(make_array_key("warning_count"), vm.arena.alloc(Val::Int(0)));
    map.insert(
        make_array_key("warnings"),
        vm.arena
            .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
                map: IndexMap::new(),
                next_free: 0,
            }))),
    );
    map.insert(make_array_key("error_count"), vm.arena.alloc(Val::Int(0)));
    map.insert(
        make_array_key("errors"),
        vm.arena
            .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
                map: IndexMap::new(),
                next_free: 0,
            }))),
    );

    Ok(vm
        .arena
        .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
            map,
            next_free: 0,
        }))))
}

// ============================================================================
// Procedural Functions
// ============================================================================

fn get_internal_data_mut<T: 'static>(vm: &mut VM, handle: Handle) -> Result<&mut T, String> {
    let val = vm.arena.get(handle);
    let payload_handle = match &val.value {
        Val::Object(h) => *h,
        _ => return Err("Expected object".into()),
    };

    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        if let Some(internal) = &mut obj_data.internal {
            if let Some(data) = Rc::get_mut(internal) {
                if let Some(typed_data) = data.downcast_mut::<T>() {
                    return Ok(typed_data);
                }
            }
            return Err("Failed to get mutable internal data (shared reference)".into());
        }
    }
    Err("Object has no internal data".into())
}

fn get_obj_data_mut<'a>(
    vm: &'a mut VM,
    handle: Handle,
) -> Result<&'a mut crate::core::value::ObjectData, String> {
    let val = vm.arena.get(handle);
    let payload_handle = match &val.value {
        Val::Object(h) => *h,
        _ => return Err("Expected object".into()),
    };

    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        return Ok(obj_data);
    }
    Err("Expected object payload".into())
}

pub fn php_date_add(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("date_add() expects 2 parameters".into());
    }
    let obj = args[0];
    let interval = args[1];

    let interval_data = get_internal_data::<DateIntervalData>(vm, interval)?;
    let data_mut = get_internal_data_mut::<DateTimeData>(vm, obj)?;
    data_mut.dt = add_interval(&data_mut.dt, &interval_data, false);

    Ok(obj)
}

pub fn php_date_sub(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("date_sub() expects 2 parameters".into());
    }
    let obj = args[0];
    let interval = args[1];

    let interval_data = get_internal_data::<DateIntervalData>(vm, interval)?;
    let data_mut = get_internal_data_mut::<DateTimeData>(vm, obj)?;
    data_mut.dt = add_interval(&data_mut.dt, &interval_data, true);

    Ok(obj)
}

pub fn php_date_diff(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("date_diff() expects at least 2 parameters".into());
    }
    let obj1 = args[0];
    let obj2 = args[1];
    let absolute = if args.len() > 2 {
        let val = vm.arena.get(args[2]);
        matches!(val.value, Val::Bool(true))
    } else {
        false
    };

    let data1 = get_internal_data::<DateTimeData>(vm, obj1)?;
    let data2 = get_internal_data::<DateTimeData>(vm, obj2)?;

    let diff = data2.dt.signed_duration_since(data1.dt);
    let mut seconds = diff.num_seconds();
    if absolute && seconds < 0 {
        seconds = -seconds;
    }

    let invert = if seconds < 0 { 1 } else { 0 };
    let abs_seconds = seconds.abs();

    let days = abs_seconds / 86400;
    let rem = abs_seconds % 86400;
    let h = rem / 3600;
    let rem = rem % 3600;
    let i = rem / 60;
    let s = rem % 60;

    let interval_data = DateIntervalData {
        y: 0,
        m: 0,
        d: days,
        h,
        i,
        s,
        f: 0.0,
        invert,
        days: Some(days),
    };

    let interval_sym = vm.context.interner.intern(b"DateInterval");
    let p0d = vm.arena.alloc(Val::String(b"P0D".to_vec().into()));
    let interval_handle = vm.instantiate_class(interval_sym, &[p0d])?;
    let obj_data = get_obj_data_mut(vm, interval_handle)?;
    obj_data.internal = Some(Rc::new(interval_data));

    Ok(interval_handle)
}

pub fn php_date_modify(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("date_modify() expects 2 parameters".into());
    }
    let obj = args[0];
    let modifier_bytes = get_string_arg(vm, args[1])?;
    let modifier = String::from_utf8_lossy(&modifier_bytes);

    let data_mut = get_internal_data_mut::<DateTimeData>(vm, obj)?;
    if modifier == "+1 day" {
        data_mut.dt = data_mut.dt + chrono::Duration::days(1);
    } else if modifier == "-1 day" {
        data_mut.dt = data_mut.dt - chrono::Duration::days(1);
    } else {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }

    Ok(obj)
}

pub fn php_timezone_open(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let tz_sym = vm.context.interner.intern(b"DateTimeZone");
    vm.instantiate_class(tz_sym, args)
}

pub fn php_date_interval_create_from_date_string(
    vm: &mut VM,
    args: &[Handle],
) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("date_interval_create_from_date_string() expects 1 parameter".into());
    }
    let interval_sym = vm.context.interner.intern(b"DateInterval");
    vm.instantiate_class(interval_sym, args)
}

// ============================================================================
// Additional Procedural Wrappers
// ============================================================================

/// date_create_immutable_from_format(string $format, string $datetime, ?DateTimeZone $timezone = null): DateTimeImmutable|false
pub fn php_date_create_immutable_from_format(
    vm: &mut VM,
    args: &[Handle],
) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("date_create_immutable_from_format() expects at least 2 parameters".into());
    }

    let format = String::from_utf8_lossy(&get_string_arg(vm, args[0])?).to_string();
    let datetime_str = String::from_utf8_lossy(&get_string_arg(vm, args[1])?).to_string();

    let chrono_format = convert_php_to_chrono_format(&format);

    let tz: Tz = if args.len() > 2 {
        let tz_data = get_internal_data::<DateTimeZoneData>(vm, args[2])?;
        tz_data.tz
    } else {
        vm.context.config.timezone.parse().unwrap_or(Tz::UTC)
    };

    // Try parsing as NaiveDateTime first, if that fails try NaiveDate
    let dt = if let Ok(naive) = NaiveDateTime::parse_from_str(&datetime_str, &chrono_format) {
        tz.from_utc_datetime(&naive)
    } else if let Ok(naive_date) = NaiveDate::parse_from_str(&datetime_str, &chrono_format) {
        let naive = naive_date.and_hms_opt(0, 0, 0).unwrap();
        tz.from_utc_datetime(&naive)
    } else {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    };

    let datetime_sym = vm.context.interner.intern(b"DateTimeImmutable");
    let obj_handle = vm.instantiate_class(datetime_sym, &[])?;

    let payload_handle = match &vm.arena.get(obj_handle).value {
        Val::Object(h) => *h,
        _ => return Err("Failed to create DateTimeImmutable".into()),
    };

    if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(payload_handle).value {
        obj_data.internal = Some(Rc::new(DateTimeData { dt }));
    }

    Ok(obj_handle)
}

/// date_timestamp_get(DateTimeInterface $object): int
pub fn php_date_timestamp_get(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("date_timestamp_get() expects exactly 1 parameter".into());
    }

    let data = get_internal_data::<DateTimeData>(vm, args[0])?;
    Ok(vm.arena.alloc(Val::Int(data.dt.timestamp())))
}

/// date_timestamp_set(DateTime $object, int $timestamp): DateTime
pub fn php_date_timestamp_set(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("date_timestamp_set() expects exactly 2 parameters".into());
    }

    let obj = args[0];
    let timestamp = get_int_arg(vm, args[1])?;

    let data = get_internal_data::<DateTimeData>(vm, obj)?;
    let new_dt = data.dt.timezone().timestamp_opt(timestamp, 0).unwrap();

    if let Val::Object(payload_handle) = &vm.arena.get(obj).value {
        let payload = vm.arena.get_mut(*payload_handle);
        if let Val::ObjPayload(ref mut obj_data) = payload.value {
            obj_data.internal = Some(Rc::new(DateTimeData { dt: new_dt }));
        }
    }

    Ok(obj)
}

/// date_timezone_get(DateTimeInterface $object): DateTimeZone|false
pub fn php_date_timezone_get(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("date_timezone_get() expects exactly 1 parameter".into());
    }

    let data = get_internal_data::<DateTimeData>(vm, args[0])?;

    // Create a new DateTimeZone object with the timezone name
    let dtz_sym = vm.context.interner.intern(b"DateTimeZone");
    let tz_name = data.dt.timezone().name().as_bytes().to_vec();
    let tz_handle = vm.arena.alloc(Val::String(Rc::new(tz_name)));
    let dtz_handle = vm.instantiate_class(dtz_sym, &[tz_handle])?;

    Ok(dtz_handle)
}

/// date_timezone_set(DateTime $object, DateTimeZone $timezone): DateTime
pub fn php_date_timezone_set(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("date_timezone_set() expects exactly 2 parameters".into());
    }

    let obj = args[0];
    let tz_data = get_internal_data::<DateTimeZoneData>(vm, args[1])?;

    let data = get_internal_data::<DateTimeData>(vm, obj)?;
    let new_dt = data.dt.with_timezone(&tz_data.tz);

    if let Val::Object(payload_handle) = &vm.arena.get(obj).value {
        let payload = vm.arena.get_mut(*payload_handle);
        if let Val::ObjPayload(ref mut obj_data) = payload.value {
            obj_data.internal = Some(Rc::new(DateTimeData { dt: new_dt }));
        }
    }

    Ok(obj)
}

/// timezone_name_get(DateTimeZone $object): string
pub fn php_timezone_name_get(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("timezone_name_get() expects exactly 1 parameter".into());
    }

    let data = get_internal_data::<DateTimeZoneData>(vm, args[0])?;
    Ok(vm
        .arena
        .alloc(Val::String(data.tz.name().as_bytes().to_vec().into())))
}

/// timezone_identifiers_list(int $timezoneGroup = DateTimeZone::ALL, ?string $countryCode = null): array
pub fn php_timezone_identifiers_list(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let mut map = IndexMap::new();
    for (i, tz) in chrono_tz::TZ_VARIANTS.iter().enumerate() {
        map.insert(
            ArrayKey::Int(i as i64),
            vm.arena
                .alloc(Val::String(tz.name().as_bytes().to_vec().into())),
        );
    }

    Ok(vm
        .arena
        .alloc(Val::Array(Rc::new(crate::core::value::ArrayData {
            map,
            next_free: chrono_tz::TZ_VARIANTS.len() as i64,
        }))))
}

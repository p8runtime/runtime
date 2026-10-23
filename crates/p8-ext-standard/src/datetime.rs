//! The date classes: DateTimeInterface, DateTime, DateTimeImmutable,
//! DateTimeZone, DateInterval, the date exceptions, and their procedural
//! forms (date_create, date_diff, ...).
//!
//! A DateTime keeps its state in hidden properties (the instant, the
//! microseconds, the zone's kind and name), so clone copies it as PHP's
//! does; var_dump shows PHP's date/timezone_type/timezone instead.
//!
//! Reference: $PHP_SRC_PATH/ext/date/php_date.c, $PHP_SRC_PATH/ext/date/lib/interval.c

use crate::dates::{Zone, civil, default_zone, format, resolve_zone};
use crate::{arg, str_arg};
use chrono::{Datelike, Timelike};
use chrono_tz::Tz;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::{CLASS_INTERFACE, Native, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

const TS: &str = "\0ts";
const US: &str = "\0us";
const ZTYPE: &str = "\0ztype";
const ZONE: &str = "\0zone";

const STATE: &[(&str, Constant, Visibility)] = &[
    (TS, Constant::Int(0), Visibility::Public),
    (US, Constant::Int(0), Visibility::Public),
    (ZTYPE, Constant::Int(3), Visibility::Public),
    (ZONE, Constant::Str(b"UTC"), Visibility::Public),
];

const DATETIME_METHODS: &[(&str, Option<Native>)] = &[
    ("__construct", Some(Native::Method(construct))),
    ("format", Some(Native::Method(m_format))),
    ("getTimestamp", Some(Native::Method(get_timestamp))),
    ("setTimestamp", Some(Native::Method(set_timestamp))),
    ("getTimezone", Some(Native::Method(get_timezone))),
    ("setTimezone", Some(Native::Method(set_timezone))),
    ("getOffset", Some(Native::Method(get_offset))),
    ("getMicrosecond", Some(Native::Method(get_microsecond))),
    ("modify", Some(Native::Method(modify))),
    ("setDate", Some(Native::Method(set_date))),
    ("setISODate", Some(Native::Method(set_iso_date))),
    ("setTime", Some(Native::Method(set_time))),
    ("add", Some(Native::Method(add))),
    ("sub", Some(Native::Method(sub))),
    ("diff", Some(Native::Method(diff))),
    ("createFromFormat", Some(Native::Static(create_from_format))),
    ("createFromInterface", Some(Native::Static(create_from_interface))),
    ("createFromImmutable", Some(Native::Static(create_from_interface))),
    ("createFromMutable", Some(Native::Static(create_from_interface))),
    ("getLastErrors", Some(Native::Static(get_last_errors))),
];

const FORMATS: &[(&str, Constant)] = &[
    ("ATOM", Constant::Str(b"Y-m-d\\TH:i:sP")),
    ("COOKIE", Constant::Str(b"l, d-M-Y H:i:s T")),
    ("ISO8601", Constant::Str(b"Y-m-d\\TH:i:sO")),
    ("ISO8601_EXPANDED", Constant::Str(b"X-m-d\\TH:i:sP")),
    ("RFC822", Constant::Str(b"D, d M y H:i:s O")),
    ("RFC850", Constant::Str(b"l, d-M-y H:i:s T")),
    ("RFC1036", Constant::Str(b"D, d M y H:i:s O")),
    ("RFC1123", Constant::Str(b"D, d M Y H:i:s O")),
    ("RFC7231", Constant::Str(b"D, d M Y H:i:s \\G\\M\\T")),
    ("RFC2822", Constant::Str(b"D, d M Y H:i:s O")),
    ("RFC3339", Constant::Str(b"Y-m-d\\TH:i:sP")),
    ("RFC3339_EXTENDED", Constant::Str(b"Y-m-d\\TH:i:s.vP")),
    ("RSS", Constant::Str(b"D, d M Y H:i:s O")),
    ("W3C", Constant::Str(b"Y-m-d\\TH:i:sP")),
];

pub static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "DateTimeInterface",
        parent: None,
        interfaces: &[],
        flags: CLASS_INTERFACE,
        properties: &[],
        methods: &[("format", None), ("getTimestamp", None), ("getTimezone", None), ("getOffset", None), ("diff", None)],
        constants: FORMATS,
        debug_info: None,
    },
    ClassSpec {
        name: "DateTime",
        parent: None,
        interfaces: &["DateTimeInterface"],
        flags: 0,
        properties: STATE,
        methods: DATETIME_METHODS,
        constants: FORMATS,
        debug_info: Some(datetime_info),
    },
    ClassSpec {
        name: "DateTimeImmutable",
        parent: None,
        interfaces: &["DateTimeInterface"],
        flags: 0,
        properties: STATE,
        methods: DATETIME_METHODS,
        constants: FORMATS,
        debug_info: Some(datetime_info),
    },
    ClassSpec {
        name: "DateTimeZone",
        parent: None,
        interfaces: &[],
        flags: 0,
        properties: &[(ZTYPE, Constant::Int(3), Visibility::Public), (ZONE, Constant::Str(b"UTC"), Visibility::Public)],
        methods: &[
            ("__construct", Some(Native::Method(zone_construct))),
            ("getName", Some(Native::Method(zone_name))),
            ("getOffset", Some(Native::Method(zone_offset))),
            ("listIdentifiers", Some(Native::Static(list_identifiers))),
        ],
        constants: &[
            ("AFRICA", Constant::Int(1)),
            ("AMERICA", Constant::Int(2)),
            ("ANTARCTICA", Constant::Int(4)),
            ("ARCTIC", Constant::Int(8)),
            ("ASIA", Constant::Int(16)),
            ("ATLANTIC", Constant::Int(32)),
            ("AUSTRALIA", Constant::Int(64)),
            ("EUROPE", Constant::Int(128)),
            ("INDIAN", Constant::Int(256)),
            ("PACIFIC", Constant::Int(512)),
            ("UTC", Constant::Int(1024)),
            ("ALL", Constant::Int(2047)),
            ("ALL_WITH_BC", Constant::Int(4095)),
            ("PER_COUNTRY", Constant::Int(4096)),
        ],
        debug_info: Some(zone_info),
    },
    ClassSpec {
        name: "DateInterval",
        parent: None,
        interfaces: &[],
        flags: 0,
        properties: &[
            ("y", Constant::Int(0), Visibility::Public),
            ("m", Constant::Int(0), Visibility::Public),
            ("d", Constant::Int(0), Visibility::Public),
            ("h", Constant::Int(0), Visibility::Public),
            ("i", Constant::Int(0), Visibility::Public),
            ("s", Constant::Int(0), Visibility::Public),
            ("f", Constant::Float(0.0), Visibility::Public),
            ("invert", Constant::Int(0), Visibility::Public),
            ("days", Constant::Bool(false), Visibility::Public),
            ("from_string", Constant::Bool(false), Visibility::Public),
        ],
        methods: &[
            ("__construct", Some(Native::Method(interval_construct))),
            ("format", Some(Native::Method(interval_format))),
            ("createFromDateString", Some(Native::Static(interval_from_string))),
        ],
        constants: &[],
        debug_info: None,
    },
    ClassSpec { name: "DateException", parent: Some("Exception"), interfaces: &[], flags: 0, properties: &[], methods: &[], constants: &[], debug_info: None },
    ClassSpec { name: "DateMalformedStringException", parent: Some("DateException"), interfaces: &[], flags: 0, properties: &[], methods: &[], constants: &[], debug_info: None },
    ClassSpec { name: "DateInvalidTimeZoneException", parent: Some("DateException"), interfaces: &[], flags: 0, properties: &[], methods: &[], constants: &[], debug_info: None },
    ClassSpec { name: "DateMalformedIntervalStringException", parent: Some("DateException"), interfaces: &[], flags: 0, properties: &[], methods: &[], constants: &[], debug_info: None },
];

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "date_create", run: date_create, by_ref: &[] },
    Builtin { name: "date_create_immutable", run: date_create_immutable, by_ref: &[] },
    Builtin { name: "date_create_from_format", run: date_create_from_format, by_ref: &[] },
    Builtin { name: "date_format", run: date_format, by_ref: &[] },
    Builtin { name: "date_diff", run: date_diff, by_ref: &[] },
    Builtin { name: "date_add", run: date_add, by_ref: &[] },
    Builtin { name: "date_sub", run: date_sub, by_ref: &[] },
    Builtin { name: "date_modify", run: date_modify, by_ref: &[] },
    Builtin { name: "date_timestamp_get", run: date_timestamp_get, by_ref: &[] },
    Builtin { name: "date_timestamp_set", run: date_timestamp_set, by_ref: &[] },
    Builtin { name: "date_timezone_get", run: date_timezone_get, by_ref: &[] },
    Builtin { name: "date_timezone_set", run: date_timezone_set, by_ref: &[] },
    Builtin { name: "date_offset_get", run: date_offset_get, by_ref: &[] },
    Builtin { name: "date_date_set", run: date_date_set, by_ref: &[] },
    Builtin { name: "date_time_set", run: date_time_set, by_ref: &[] },
    Builtin { name: "date_interval_format", run: date_interval_format, by_ref: &[] },
    Builtin { name: "timezone_open", run: timezone_open, by_ref: &[] },
    Builtin { name: "timezone_name_get", run: timezone_name_get, by_ref: &[] },
    Builtin { name: "timezone_identifiers_list", run: list_identifiers, by_ref: &[] },
];

/// A DateTime's moment: the instant, microseconds, and its zone (PHP's
/// timezone_type 1 offset, 2 abbreviation, 3 identifier, and the name).
#[derive(Clone)]
struct State {
    ts: i64,
    us: i64,
    ztype: i64,
    zone: String,
}

fn read(o: &Object, name: &str) -> Value {
    match o.class.slot(name.as_bytes(), None) {
        Some(i) => o.slots.borrow()[i].clone(),
        None => Value::Null,
    }
}

fn write(o: &Object, name: &str, v: Value) {
    if let Some(i) = o.class.slot(name.as_bytes(), None) {
        o.slots.borrow_mut()[i] = v;
    }
}

fn state(o: &Object) -> State {
    State {
        ts: read(o, TS).to_int(),
        us: read(o, US).to_int(),
        ztype: read(o, ZTYPE).to_int(),
        zone: String::from_utf8_lossy(read(o, ZONE).to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned(),
    }
}

fn put(o: &Object, s: &State) {
    write(o, TS, Value::Int(s.ts));
    write(o, US, Value::Int(s.us));
    write(o, ZTYPE, Value::Int(s.ztype));
    write(o, ZONE, Value::Str(PhpStr::from(s.zone.as_str())));
}

/// The zone a state names.
fn zone_of(s: &State) -> Zone {
    match s.ztype {
        3 => s.zone.parse::<Tz>().map_or(Zone::Fixed(0), Zone::Named),
        _ => Zone::Fixed(offset_of(&s.zone)),
    }
}

/// "+02:00" / "-0530" / abbreviations: seconds east of UTC.
fn offset_of(z: &str) -> i32 {
    match z.to_ascii_uppercase().as_str() {
        "UTC" | "GMT" | "Z" => return 0,
        "EST" => return -5 * 3600,
        "EDT" | "CDT" => return if z.eq_ignore_ascii_case("EDT") { -4 * 3600 } else { -5 * 3600 },
        "CST" => return -6 * 3600,
        "MST" => return -7 * 3600,
        "MDT" => return -6 * 3600,
        "PST" => return -8 * 3600,
        "PDT" => return -7 * 3600,
        "CET" | "BST" => return 3600,
        "CEST" => return 7200,
        _ => {}
    }
    let sign = if z.starts_with('-') { -1 } else { 1 };
    let digits: String = z.chars().filter(char::is_ascii_digit).collect();
    let (h, m) = match digits.len() {
        1 | 2 => (digits.parse::<i32>().unwrap_or(0), 0),
        3 | 4 => (digits[..digits.len() - 2].parse::<i32>().unwrap_or(0), digits[digits.len() - 2..].parse::<i32>().unwrap_or(0)),
        _ => (0, 0),
    };
    sign * (h * 3600 + m * 60)
}

/// The zone a DateTimeZone object (or null: the default) names.
fn zone_arg(ctx: &mut dyn Ctx, v: &Value) -> (i64, String) {
    match v.deref() {
        Value::Object(o) if o.class.is_a(b"datetimezone") => (
            read(&o, ZTYPE).to_int(),
            String::from_utf8_lossy(read(&o, ZONE).to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned(),
        ),
        _ => (3, default_zone(ctx).1),
    }
}

fn now_us() -> (i64, i64) {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as i64, i64::from(d.subsec_micros()))
}

/// The state `text` names, in the zone (ztype, name) unless it names one.
fn parse_state(text: &str, base: i64, ztype: i64, zone: &str) -> Option<State> {
    let s = State { ts: base, us: 0, ztype, zone: zone.to_string() };
    let (ts, named) = resolve_zone(text, base, zone_of(&s))?;
    Some(match named {
        Some((_, t, name)) => State { ts, us: 0, ztype: t, zone: name },
        None => State { ts, ..s },
    })
}

fn class_name(o: &Object) -> String {
    String::from_utf8_lossy(o.class.name.as_bytes()).into_owned()
}

/// DateTime::__construct(string $datetime = "now", ?DateTimeZone $timezone = null)
fn construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let text = if args.is_empty() { "now".to_string() } else { String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned() };
    let (ztype, zone) = zone_arg(ctx, &arg(args, 1));
    let (now, us) = now_us();
    match parse_state(if text.trim().is_empty() { "now" } else { &text }, now, ztype, &zone) {
        Some(mut s) => {
            if text.trim().is_empty() || text.trim().eq_ignore_ascii_case("now") {
                s.us = us;
            }
            put(this, &s);
            Ok(Value::Null)
        }
        None => Err(ctx.throw(
            "DateMalformedStringException",
            format!("Failed to parse time string ({text}) at position 0 ({}): The timezone could not be found in the database", text.chars().next().unwrap_or(' ')),
        )),
    }
}

fn local_of(s: &State) -> crate::dates::Local {
    let mut l = zone_of(s).at(s.ts);
    if s.ztype != 3 {
        l.abbr = if s.ztype == 1 { s.zone.clone() } else { s.zone.to_ascii_uppercase() };
    }
    l
}

fn format_state(s: &State, fmt: &[u8]) -> Vec<u8> {
    let l = local_of(s);
    // 'u' and 'v': the microseconds; 'e': the zone's name.
    let mut expanded = Vec::with_capacity(fmt.len());
    let mut i = 0;
    while i < fmt.len() {
        match fmt[i] {
            b'\\' if i + 1 < fmt.len() => {
                expanded.extend_from_slice(&fmt[i..i + 2]);
                i += 2;
                continue;
            }
            b'u' => escape_into(&mut expanded, &format!("{:06}", s.us)),
            b'v' => escape_into(&mut expanded, &format!("{:03}", s.us / 1000)),
            b'e' => escape_into(&mut expanded, &s.zone),
            b'T' if s.ztype == 1 => escape_into(&mut expanded, &s.zone),
            c => expanded.push(c),
        }
        i += 1;
    }
    format(&expanded, s.ts, &l, &s.zone)
}

fn escape_into(out: &mut Vec<u8>, text: &str) {
    for c in text.bytes() {
        out.push(b'\\');
        out.push(c);
    }
}

fn m_format(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Str(PhpStr::new(format_state(&state(this), str_arg(args, 0).as_bytes()))))
}

fn get_timestamp(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(state(this).ts))
}

fn get_microsecond(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(state(this).us))
}

fn get_offset(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let s = state(this);
    Ok(Value::Int(i64::from(local_of(&s).offset)))
}

/// The object a change applies to: `this` for a DateTime, a copy for a
/// DateTimeImmutable (the change goes there and it is the result).
fn target(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Rc<Object> {
    if this.class.is_a(b"datetimeimmutable") {
        let copy = ctx.new_object(&this.class);
        put(&copy, &state(this));
        copy
    } else {
        Rc::clone(this)
    }
}

fn set_timestamp(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = target(ctx, this);
    let mut s = state(&o);
    s.ts = arg(args, 0).to_int();
    s.us = 0;
    put(&o, &s);
    Ok(Value::Object(o))
}

fn get_timezone(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let s = state(this);
    let Some(class) = ctx.class(b"DateTimeZone") else { return Ok(Value::Bool(false)) };
    let z = ctx.new_object(&class);
    write(&z, ZTYPE, Value::Int(s.ztype));
    write(&z, ZONE, Value::Str(PhpStr::from(s.zone.as_str())));
    Ok(Value::Object(z))
}

fn set_timezone(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = target(ctx, this);
    let mut s = state(&o);
    (s.ztype, s.zone) = zone_arg(ctx, &arg(args, 0));
    put(&o, &s);
    Ok(Value::Object(o))
}

fn modify(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let text = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let s = state(this);
    let Some(mut new) = parse_state(&text, s.ts, s.ztype, &s.zone) else {
        return Err(ctx.throw(
            "DateMalformedStringException",
            format!("{}::modify(): Failed to parse time string ({text}) at position 0 ({})", class_name(this), text.chars().next().unwrap_or(' ')),
        ));
    };
    new.us = s.us;
    let o = target(ctx, this);
    put(&o, &new);
    Ok(Value::Object(o))
}

/// The wall-clock parts of a state.
fn parts(s: &State) -> (i64, i64, i64, i64, i64, i64) {
    let d = local_of(s).dt;
    (i64::from(d.year()), i64::from(d.month()), i64::from(d.day()), i64::from(d.hour()), i64::from(d.minute()), i64::from(d.second()))
}

fn set_parts(s: &mut State, p: (i64, i64, i64, i64, i64, i64)) {
    if let Some(dt) = civil(p.0, p.1, p.2, p.3, p.4, p.5) {
        s.ts = zone_of(s).timestamp(dt);
    }
}

fn set_date(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = target(ctx, this);
    let mut s = state(&o);
    let (_, _, _, h, i, sec) = parts(&s);
    set_parts(&mut s, (arg(args, 0).to_int(), arg(args, 1).to_int(), arg(args, 2).to_int(), h, i, sec));
    put(&o, &s);
    Ok(Value::Object(o))
}

fn set_iso_date(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = target(ctx, this);
    let mut s = state(&o);
    let (_, _, _, h, i, sec) = parts(&s);
    let (year, week) = (arg(args, 0).to_int(), arg(args, 1).to_int());
    let day = if args.len() > 2 { arg(args, 2).to_int() } else { 1 };
    // Monday of ISO week 1 is the Monday on or before January 4th.
    let jan4 = chrono::NaiveDate::from_ymd_opt(year as i32, 1, 4).unwrap_or_default();
    let monday = jan4 - chrono::Duration::days(i64::from(jan4.weekday().num_days_from_monday()));
    let date = monday + chrono::Duration::days((week - 1) * 7 + (day - 1));
    set_parts(&mut s, (i64::from(date.year()), i64::from(date.month()), i64::from(date.day()), h, i, sec));
    put(&o, &s);
    Ok(Value::Object(o))
}

fn set_time(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let o = target(ctx, this);
    let mut s = state(&o);
    let (y, m, d, _, _, _) = parts(&s);
    let sec = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    set_parts(&mut s, (y, m, d, arg(args, 0).to_int(), arg(args, 1).to_int(), sec));
    s.us = if args.len() > 3 { arg(args, 3).to_int() } else { 0 };
    put(&o, &s);
    Ok(Value::Object(o))
}

/// An interval's parts (sign applied when `invert`).
fn interval(o: &Object) -> [i64; 6] {
    let sign = if read(o, "invert").to_int() != 0 { -1 } else { 1 };
    ["y", "m", "d", "h", "i", "s"].map(|k| read(o, k).to_int() * sign)
}

fn shift(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &[Value], dir: i64) -> Result<Value, Throw> {
    let Value::Object(iv) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    let o = target(ctx, this);
    let mut s = state(&o);
    let p = parts(&s);
    let d = interval(&iv);
    // Whole days counted by diff() ("days") move by days.
    let days = read(&iv, "days");
    if let Value::Int(total) = days
        && read(&iv, "from_string").to_bool()
    {
        let _ = total;
    }
    set_parts(&mut s, (p.0 + dir * d[0], p.1 + dir * d[1], p.2 + dir * d[2], p.3 + dir * d[3], p.4 + dir * d[4], p.5 + dir * d[5]));
    put(&o, &s);
    Ok(Value::Object(o))
}

fn add(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    shift(ctx, this, args, 1)
}

fn sub(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    shift(ctx, this, args, -1)
}

/// A new DateInterval with these parts.
fn new_interval(ctx: &mut dyn Ctx, p: [i64; 6], invert: bool, days: Value) -> Value {
    let Some(class) = ctx.class(b"DateInterval") else { return Value::Null };
    let o = ctx.new_object(&class);
    for (k, v) in ["y", "m", "d", "h", "i", "s"].iter().zip(p) {
        write(&o, k, Value::Int(v));
    }
    write(&o, "f", Value::Float(0.0));
    write(&o, "invert", Value::Int(i64::from(invert)));
    write(&o, "days", days);
    Value::Object(o)
}

/// DateTimeInterface::diff(DateTimeInterface $targetObject, bool $absolute = false):
/// the calendar difference (as timelib_diff, in the first one's zone).
fn diff(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(other) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    let absolute = args.len() > 1 && arg(args, 1).to_bool();
    let a = state(this);
    let mut b = state(&other);
    let invert = b.ts < a.ts;
    // Both seen in the first one's zone.
    b.ztype = a.ztype;
    b.zone = a.zone.clone();
    let (from, to) = if invert { (b, a) } else { (a, b) };
    let (mut y1, mut m1, mut d1, mut h1, mut i1, mut s1) = parts(&from);
    let (y2, m2, d2, h2, i2, s2) = parts(&to);
    let _ = (&mut y1, &mut m1, &mut d1, &mut h1, &mut i1, &mut s1);
    let mut p = [y2 - y1, m2 - m1, d2 - d1, h2 - h1, i2 - i1, s2 - s1];
    if p[5] < 0 {
        p[5] += 60;
        p[4] -= 1;
    }
    if p[4] < 0 {
        p[4] += 60;
        p[3] -= 1;
    }
    if p[3] < 0 {
        p[3] += 24;
        p[2] -= 1;
    }
    if p[2] < 0 {
        // Borrow the days of the month before the later date's month.
        let (py, pm) = if m2 == 1 { (y2 - 1, 12) } else { (y2, m2 - 1) };
        let prev = civil(py, pm + 1, 0, 0, 0, 0).map_or(30, |d| i64::from(d.day()));
        p[2] += prev;
        p[1] -= 1;
    }
    if p[1] < 0 {
        p[1] += 12;
        p[0] -= 1;
    }
    let days = (civil(parts(&to).0, parts(&to).1, parts(&to).2, 0, 0, 0), civil(y1, m1, d1, 0, 0, 0));
    let total = match days {
        (Some(t), Some(f)) => {
            let whole = (t - f).num_days();
            // Not a whole day yet when the time of day is earlier.
            if (h2, i2, s2) < (h1, i1, s1) { whole - 1 } else { whole }
        }
        _ => 0,
    };
    Ok(new_interval(ctx, p, invert && !absolute, Value::Int(total)))
}

fn get_last_errors(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(false))
}

fn create_from_interface(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(src) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    let class = ctx.native_class().or_else(|| ctx.class(b"DateTime")).expect("a date class");
    let o = ctx.new_object(&class);
    put(&o, &state(&src));
    Ok(Value::Object(o))
}

/// createFromFormat(string $format, string $datetime, ?DateTimeZone $timezone = null)
fn create_from_format(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let class = ctx.native_class().or_else(|| ctx.class(b"DateTime")).expect("a date class");
    from_format(ctx, &class, args)
}

fn from_format(ctx: &mut dyn Ctx, class: &Rc<p8_runtime::object::Class>, args: &[Value]) -> Result<Value, Throw> {
    let fmt = str_arg(args, 0);
    let text = str_arg(args, 1);
    let (ztype, zone) = zone_arg(ctx, &arg(args, 2));
    let (now, _) = now_us();
    let mut s = State { ts: now, us: 0, ztype, zone };
    let (ny, nm, nd, nh, ni, ns) = parts(&s);
    // Parts not given come from now; after '!' or '|' they are reset.
    let (mut y, mut m, mut d, mut h, mut i, mut sec) = (None, None, None, None, None, None);
    let mut us = 0;
    let mut reset = false;
    let mut pm: Option<bool> = None;
    let t = text.as_bytes();
    let mut at = 0;
    let mut stamp = None;
    let num = |at: &mut usize, max: usize| -> Option<i64> {
        let start = *at;
        if *at < t.len() && (t[*at] == b'-' || t[*at] == b'+') {
            *at += 1;
        }
        while *at < t.len() && *at - start < max && t[*at].is_ascii_digit() {
            *at += 1;
        }
        std::str::from_utf8(&t[start..*at]).ok()?.parse().ok()
    };
    let f = fmt.as_bytes();
    let mut k = 0;
    while k < f.len() {
        let c = f[k];
        k += 1;
        let ok = match c {
            b'd' | b'j' => num(&mut at, 2).map(|v| d = Some(v)).is_some(),
            b'm' | b'n' => num(&mut at, 2).map(|v| m = Some(v)).is_some(),
            b'Y' => num(&mut at, 4).map(|v| y = Some(v)).is_some(),
            b'y' => num(&mut at, 2).map(|v| y = Some(if v < 70 { v + 2000 } else { v + 1900 })).is_some(),
            b'H' | b'G' | b'h' | b'g' => num(&mut at, 2).map(|v| h = Some(v)).is_some(),
            b'i' => num(&mut at, 2).map(|v| i = Some(v)).is_some(),
            b's' => num(&mut at, 2).map(|v| sec = Some(v)).is_some(),
            b'u' | b'v' => {
                let start = at;
                while at < t.len() && t[at].is_ascii_digit() && at - start < if c == b'u' { 6 } else { 3 } {
                    at += 1;
                }
                let digits = std::str::from_utf8(&t[start..at]).unwrap_or("0");
                us = format!("{digits:0<6}").parse().unwrap_or(0);
                at > start
            }
            b'a' | b'A' => {
                let w = t.get(at..at + 2).map(|x| x.to_ascii_lowercase());
                at += 2;
                match w.as_deref() {
                    Some(b"am") => {
                        pm = Some(false);
                        true
                    }
                    Some(b"pm") => {
                        pm = Some(true);
                        true
                    }
                    _ => false,
                }
            }
            b'D' | b'l' => {
                while at < t.len() && t[at].is_ascii_alphabetic() {
                    at += 1;
                }
                true
            }
            b'M' | b'F' => {
                let start = at;
                while at < t.len() && t[at].is_ascii_alphabetic() {
                    at += 1;
                }
                let w = String::from_utf8_lossy(&t[start..at]).to_ascii_lowercase();
                match ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|x| w.starts_with(x)) {
                    Some(p) => {
                        m = Some(p as i64 + 1);
                        true
                    }
                    None => false,
                }
            }
            b'U' => num(&mut at, 20).map(|v| stamp = Some(v)).is_some(),
            b'e' | b'T' | b'P' | b'O' | b'p' => {
                let start = at;
                while at < t.len() && !t[at].is_ascii_whitespace() {
                    at += 1;
                }
                let z = String::from_utf8_lossy(&t[start..at]).into_owned();
                if z.parse::<Tz>().is_ok() {
                    s.ztype = 3;
                    s.zone = z;
                } else if z.starts_with(['+', '-']) {
                    s.ztype = 1;
                    let o = offset_of(&z);
                    s.zone = format!("{}{:02}:{:02}", if o < 0 { '-' } else { '+' }, o.abs() / 3600, o.abs() / 60 % 60);
                } else {
                    s.ztype = 2;
                    s.zone = z.to_ascii_uppercase();
                }
                at > start
            }
            b'!' | b'|' => {
                reset = true;
                true
            }
            b'*' => {
                while at < t.len() && !matches!(t[at], b' ' | b',' | b';' | b':' | b'/' | b'.' | b'-' | b'(' | b')') {
                    at += 1;
                }
                true
            }
            b'+' => {
                at = t.len();
                true
            }
            b'\\' => {
                let lit = f.get(k).copied();
                k += 1;
                let hit = lit.is_some() && t.get(at).copied() == lit;
                at += 1;
                hit
            }
            b'?' => {
                at += 1;
                true
            }
            lit => {
                let hit = t.get(at) == Some(&lit);
                at += 1;
                hit
            }
        };
        if !ok {
            return Ok(Value::Bool(false));
        }
    }
    if at < t.len() {
        return Ok(Value::Bool(false));
    }
    if let Some(ts) = stamp {
        s.ts = ts;
        s.ztype = 1;
        s.zone = "+00:00".into();
    } else {
        let base = if reset { (1970, 1, 1, 0, 0, 0) } else { (ny, nm, nd, nh, ni, ns) };
        // A date without a time takes the time of now (unless reset).
        let any_time = h.is_some() || i.is_some() || sec.is_some();
        let mut hour = h.unwrap_or(if any_time { 0 } else { base.3 });
        if let Some(p) = pm {
            hour = if p { if hour == 12 { 12 } else { hour + 12 } } else if hour == 12 { 0 } else { hour };
        }
        set_parts(
            &mut s,
            (
                y.unwrap_or(base.0),
                m.unwrap_or(base.1),
                d.unwrap_or(base.2),
                hour,
                i.unwrap_or(if any_time { 0 } else { base.4 }),
                sec.unwrap_or(if any_time { 0 } else { base.5 }),
            ),
        );
        s.us = us;
    }
    let o = ctx.new_object(class);
    put(&o, &s);
    Ok(Value::Object(o))
}

/// var_dump of a DateTime: date, timezone_type, timezone.
fn datetime_info(o: &Object) -> PhpArray {
    let s = state(o);
    let mut out = PhpArray::new();
    let date = format!("{}.{:06}", String::from_utf8_lossy(&format_state(&s, b"Y-m-d H:i:s")), s.us);
    out.insert(Key::from_string(PhpStr::from("date")), Value::Str(PhpStr::from(date)));
    out.insert(Key::from_string(PhpStr::from("timezone_type")), Value::Int(s.ztype));
    out.insert(Key::from_string(PhpStr::from("timezone")), Value::Str(PhpStr::from(s.zone.as_str())));
    out
}

fn zone_info(o: &Object) -> PhpArray {
    let mut out = PhpArray::new();
    out.insert(Key::from_string(PhpStr::from("timezone_type")), read(o, ZTYPE));
    out.insert(Key::from_string(PhpStr::from("timezone")), read(o, ZONE));
    out
}

/// DateTimeZone::__construct(string $timezone)
fn zone_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let name = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let found = name.parse::<Tz>().ok().or_else(|| chrono_tz::TZ_VARIANTS.iter().find(|z| z.name().eq_ignore_ascii_case(&name)).copied());
    let (ztype, zone) = match found {
        Some(tz) if !name.eq_ignore_ascii_case("utc") || name == "UTC" => (3, tz.name().to_string()),
        Some(_) => (3, "UTC".to_string()),
        None if name.starts_with(['+', '-']) => {
            let o = offset_of(&name);
            (1, format!("{}{:02}:{:02}", if o < 0 { '-' } else { '+' }, o.abs() / 3600, o.abs() / 60 % 60))
        }
        None if matches!(name.to_ascii_uppercase().as_str(), "EST" | "EDT" | "CST" | "CDT" | "MST" | "MDT" | "PST" | "PDT" | "GMT" | "CET" | "CEST" | "BST") => {
            (2, name.to_ascii_uppercase())
        }
        None => {
            return Err(ctx.throw("DateInvalidTimeZoneException", format!("DateTimeZone::__construct(): Unknown or bad timezone ({name})")));
        }
    };
    write(this, ZTYPE, Value::Int(ztype));
    write(this, ZONE, Value::Str(PhpStr::from(zone)));
    Ok(Value::Null)
}

fn zone_name(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(read(this, ZONE))
}

fn zone_offset(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(dt) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    let s = State {
        ts: state(&dt).ts,
        us: 0,
        ztype: read(this, ZTYPE).to_int(),
        zone: String::from_utf8_lossy(read(this, ZONE).to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned(),
    };
    Ok(Value::Int(i64::from(local_of(&s).offset)))
}

fn list_identifiers(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let mut names: Vec<&str> = chrono_tz::TZ_VARIANTS
        .iter()
        .map(|z| z.name())
        .filter(|n| n.contains('/') && !n.starts_with("Etc/") && !n.starts_with("US/") && !n.starts_with("SystemV/") || *n == "UTC")
        .collect();
    names.sort_unstable();
    Ok(Value::Array(Rc::new(PhpArray::from_list(names.into_iter().map(Value::str).collect()))))
}

/// DateInterval::__construct(string $duration): ISO 8601 (P1Y2M3DT4H5M6S, P2W).
fn interval_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let spec = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let bad = |ctx: &mut dyn Ctx| ctx.throw("DateMalformedIntervalStringException", format!("Unknown or bad format ({spec})"));
    let Some(body) = spec.strip_prefix('P') else { return Err(bad(ctx)) };
    let mut p = [0i64; 6];
    let mut in_time = false;
    let mut digits = String::new();
    for c in body.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => digits.push(c),
            unit => {
                let Ok(n) = digits.parse::<i64>() else { return Err(bad(ctx)) };
                digits.clear();
                match (unit, in_time) {
                    ('Y', false) => p[0] = n,
                    ('M', false) => p[1] = n,
                    ('W', false) => p[2] += n * 7,
                    ('D', false) => p[2] += n,
                    ('H', true) => p[3] = n,
                    ('M', true) => p[4] = n,
                    ('S', true) => p[5] = n,
                    _ => return Err(bad(ctx)),
                }
            }
        }
    }
    if !digits.is_empty() || body.is_empty() {
        return Err(bad(ctx));
    }
    for (k, v) in ["y", "m", "d", "h", "i", "s"].iter().zip(p) {
        write(this, k, Value::Int(v));
    }
    Ok(Value::Null)
}

/// DateInterval::format: %y %Y %m %M %d %D %h %H %i %I %s %S %a %R %r %f %F %%.
fn interval_format(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let f = str_arg(args, 0);
    let get = |k: &str| read(this, k).to_int();
    let invert = get("invert") != 0;
    let mut out = String::new();
    let b = f.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' || i + 1 >= b.len() {
            out.push(b[i] as char);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        i += 2;
        let s = match c {
            b'y' => get("y").to_string(),
            b'Y' => format!("{:04}", get("y")),
            b'm' => get("m").to_string(),
            b'M' => format!("{:02}", get("m")),
            b'd' => get("d").to_string(),
            b'D' => format!("{:02}", get("d")),
            b'h' => get("h").to_string(),
            b'H' => format!("{:02}", get("h")),
            b'i' => get("i").to_string(),
            b'I' => format!("{:02}", get("i")),
            b's' => get("s").to_string(),
            b'S' => format!("{:02}", get("s")),
            b'f' => "0".into(),
            b'F' => "000000".into(),
            b'a' => match read(this, "days") {
                Value::Int(n) => n.to_string(),
                _ => "(unknown)".into(),
            },
            b'R' => if invert { "-" } else { "+" }.into(),
            b'r' => if invert { "-" } else { "" }.into(),
            b'%' => "%".into(),
            other => format!("%{}", other as char),
        };
        out.push_str(&s);
    }
    Ok(Value::Str(PhpStr::from(out)))
}

/// DateInterval::createFromDateString(string $datetime): "1 day", "2 weeks"...
fn interval_from_string(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let text = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    // The parts a relative text moves, measured from a fixed date.
    let base = 0;
    let Some(after) = crate::dates::resolve(&text, base, Zone::Fixed(0)) else {
        return Err(ctx.throw("DateMalformedIntervalStringException", format!("Unknown or bad format ({text})")));
    };
    let mut p = [0i64; 6];
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut k = 0;
    while k + 1 < words.len() {
        if let Ok(n) = words[k].trim_start_matches('+').parse::<i64>() {
            let unit = words[k + 1].to_ascii_lowercase();
            let unit = unit.trim_end_matches('s');
            match unit {
                "year" => p[0] += n,
                "month" => p[1] += n,
                "week" => p[2] += 7 * n,
                "day" => p[2] += n,
                "hour" => p[3] += n,
                "minute" | "min" => p[4] += n,
                "second" | "sec" => p[5] += n,
                _ => {}
            }
            k += 2;
        } else {
            k += 1;
        }
    }
    let _ = after;
    let v = new_interval(ctx, p, false, Value::Bool(false));
    if let Value::Object(o) = &v {
        write(o, "from_string", Value::Bool(true));
    }
    Ok(v)
}

fn new_date(ctx: &mut dyn Ctx, class: &[u8], args: &mut [Value]) -> Result<Value, Throw> {
    let Some(class) = ctx.class(class) else { return Ok(Value::Bool(false)) };
    let o = ctx.new_object(&class);
    match construct(ctx, &o, args) {
        Ok(_) => Ok(Value::Object(o)),
        // The procedural form gives false for a bad string.
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn date_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    new_date(ctx, b"DateTime", args)
}

fn date_create_immutable(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    new_date(ctx, b"DateTimeImmutable", args)
}

fn date_create_from_format(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(class) = ctx.class(b"DateTime") else { return Ok(Value::Bool(false)) };
    from_format(ctx, &class, args)
}

/// The procedural form of a method: the object first, then the method's arguments.
type Method = fn(&mut dyn Ctx, &Rc<Object>, &mut [Value]) -> Result<Value, Throw>;

fn with_object(ctx: &mut dyn Ctx, args: &mut [Value], method: Method) -> Result<Value, Throw> {
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    let mut rest: Vec<Value> = args[1..].to_vec();
    method(ctx, &o, &mut rest)
}

fn date_format(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, m_format)
}

fn date_diff(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, diff)
}

fn date_add(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, add)
}

fn date_sub(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, sub)
}

fn date_modify(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, modify)
}

fn date_timestamp_get(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, get_timestamp)
}

fn date_timestamp_set(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, set_timestamp)
}

fn date_timezone_get(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, get_timezone)
}

fn date_timezone_set(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, set_timezone)
}

fn date_offset_get(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, get_offset)
}

fn date_date_set(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, set_date)
}

fn date_time_set(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, set_time)
}

fn date_interval_format(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, interval_format)
}

fn timezone_open(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(class) = ctx.class(b"DateTimeZone") else { return Ok(Value::Bool(false)) };
    let o = ctx.new_object(&class);
    match zone_construct(ctx, &o, args) {
        Ok(_) => Ok(Value::Object(o)),
        Err(_) => {
            let name = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
            ctx.warn(&format!("timezone_open(): Unknown or bad timezone ({name})"));
            Ok(Value::Bool(false))
        }
    }
}

fn timezone_name_get(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    with_object(ctx, args, zone_name)
}

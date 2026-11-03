//! IntlTimeZone and intltz_* (timezone/timezone_methods.cpp), the part a
//! formatter's time zone needs: creation from IDs and DateTimeZone, the
//! ID, offsets, daylight saving, conversion back to DateTimeZone, and
//! display names (CLDR via ICU4X). Over the IANA database the date
//! functions use (chrono-tz).
//!
//! Not available: the ID enumerations, equivalent IDs and the Windows ID
//! mappings (ICU's own tables).

use crate::datefmt::{Zone, fields_at, zone_from_id};
use crate::{IntlError, R, U_ILLEGAL_ARGUMENT_ERROR, arg, b, lossy, s, str_arg};
use p8_runtime::ext::{Builtin, ClassSpec, Ctx, Registry, Throw};
use p8_runtime::object::Object;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
pub struct State {
    pub zone: RefCell<Option<Zone>>,
    /// ICU's "Etc/Unknown".
    pub unknown: std::cell::Cell<bool>,
    pub err: IntlError,
}

fn state(o: &Object) -> &State {
    o.native::<State>().expect("IntlTimeZone state")
}

pub fn register(registry: &mut Registry) {
    registry.add_data("IntlTimeZone", || Rc::new(State::default()));
}

/// The zone an IntlTimeZone object holds (UTC when it is the unknown zone).
pub fn zone_of(o: &Object) -> Zone {
    state(o).zone.borrow().clone().unwrap_or(Zone::Iana(chrono_tz::Tz::UTC))
}

pub fn new_object(ctx: &mut dyn Ctx, zone: Zone) -> Value {
    let class = ctx.class(b"IntlTimeZone").expect("IntlTimeZone");
    let o = ctx.new_object(&class);
    *state(&o).zone.borrow_mut() = Some(zone);
    Value::Object(o)
}

fn unknown(ctx: &mut dyn Ctx) -> Value {
    let v = new_object(ctx, Zone::Fixed(0));
    if let Value::Object(o) = &v {
        state(o).unknown.set(true);
    }
    v
}

pub static BUILTINS: &[Builtin] = &[
    b!("intltz_create_time_zone", f_create_time_zone),
    b!("intltz_create_default", f_create_default),
    b!("intltz_get_gmt", f_get_gmt),
    b!("intltz_get_unknown", f_get_unknown),
    b!("intltz_from_date_time_zone", f_from_date_time_zone),
    b!("intltz_get_id", f_get_id),
    b!("intltz_get_raw_offset", f_get_raw_offset),
    b!("intltz_get_dst_savings", f_get_dst_savings),
    b!("intltz_use_daylight_time", f_use_daylight_time),
    Builtin { name: "intltz_get_offset", run: f_get_offset, by_ref: &[3, 4] },
    b!("intltz_has_same_rules", f_has_same_rules),
    b!("intltz_to_date_time_zone", f_to_date_time_zone),
    b!("intltz_get_display_name", f_get_display_name),
    Builtin { name: "intltz_get_canonical_id", run: f_get_canonical_id, by_ref: &[1] },
    b!("intltz_get_tz_data_version", f_get_tz_data_version),
    b!("intltz_get_error_code", f_get_error_code),
    b!("intltz_get_error_message", f_get_error_message),
];

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "IntlTimeZone",
    parent: None,
    interfaces: &[],
    flags: p8_runtime::object::CLASS_NOT_SERIALIZABLE,
    properties: &[],
    methods: crate::methods! {
        "IntlTimeZone";
        statics: [
            ("createTimeZone", s_create_time_zone),
            ("createDefault", s_create_default),
            ("getGMT", s_get_gmt),
            ("getUnknown", s_get_unknown),
            ("fromDateTimeZone", s_from_date_time_zone),
            ("getCanonicalID", s_get_canonical_id),
            ("getTZDataVersion", s_get_tz_data_version)
        ];
        "getID" => f_get_id;
        "getRawOffset" => f_get_raw_offset;
        "getDSTSavings" => f_get_dst_savings;
        "useDaylightTime" => f_use_daylight_time;
        "getOffset" => f_get_offset;
        "hasSameRules" => f_has_same_rules;
        "toDateTimeZone" => f_to_date_time_zone;
        "getDisplayName" => f_get_display_name;
        "getErrorCode" => f_get_error_code;
        "getErrorMessage" => f_get_error_message;
    },
    constants: crate::consts::INTL_TIME_ZONE,
    debug_info: Some(debug_info),
}];

/// What var_dump and print_r show (IntlTimeZone_get_debug_info).
fn debug_info(o: &Object) -> p8_runtime::array::PhpArray {
    use p8_runtime::array::{Key, PhpArray};
    let mut a = PhpArray::new();
    let put = |a: &mut PhpArray, k: &str, v: Value| a.insert(Key::from_string(p8_runtime::string::PhpStr::from(k.as_bytes())), v);
    let st = state(o);
    let Some(z) = st.zone.borrow().clone() else {
        put(&mut a, "valid", Value::Bool(false));
        return a;
    };
    put(&mut a, "valid", Value::Bool(true));
    let id = if st.unknown.get() { "Etc/Unknown".to_string() } else { z.id() };
    put(&mut a, "id", s(id));
    let now = now_ms();
    let (raw, dst) = offsets(&z, now);
    put(&mut a, "rawOffset", Value::Int(raw));
    put(&mut a, "currentOffset", Value::Int(raw + dst));
    a
}

macro_rules! static_of {
    ($name:ident, $func:ident, $stub:literal) => {
        fn $name(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
            crate::prep(ctx, $stub, args)?;
            $func(ctx, args)
        }
    };
}
static_of!(s_create_time_zone, f_create_time_zone, "IntlTimeZone::createTimeZone");
static_of!(s_create_default, f_create_default, "IntlTimeZone::createDefault");
static_of!(s_get_gmt, f_get_gmt, "IntlTimeZone::getGMT");
static_of!(s_get_unknown, f_get_unknown, "IntlTimeZone::getUnknown");
static_of!(s_from_date_time_zone, f_from_date_time_zone, "IntlTimeZone::fromDateTimeZone");
static_of!(s_get_canonical_id, f_get_canonical_id, "IntlTimeZone::getCanonicalID");
static_of!(s_get_tz_data_version, f_get_tz_data_version, "IntlTimeZone::getTZDataVersion");

fn f_create_time_zone(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::reset_global(ctx);
    let id = str_arg(args, 0);
    Ok(match zone_from_id(&id) {
        Some(z) => new_object(ctx, z),
        // ICU gives the unknown zone for an ID it does not know
        None => unknown(ctx),
    })
}

fn f_create_default(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let name = ctx.call(&s("date_default_timezone_get"), &mut []).ok().and_then(|v| v.to_php_string()).map(|s| lossy(s.as_bytes())).unwrap_or_else(|| "UTC".into());
    let z = zone_from_id(&name).unwrap_or(Zone::Iana(chrono_tz::Tz::UTC));
    Ok(new_object(ctx, z))
}

fn f_get_gmt(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(new_object(ctx, Zone::Iana(chrono_tz::Tz::GMT)))
}

fn f_get_unknown(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(unknown(ctx))
}

fn f_from_date_time_zone(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::reset_global(ctx);
    let tz = arg(args, 0);
    let name = ctx.call(&Value::Array(Rc::new(p8_runtime::array::PhpArray::from_list(vec![tz, s("getName")]))), &mut [])?;
    let name = name.to_php_string().map(|s| lossy(s.as_bytes())).unwrap_or_default();
    let id = if name.starts_with(['+', '-']) { format!("GMT{name}") } else { name };
    match zone_from_id(&id) {
        Some(z) => Ok(new_object(ctx, z)),
        None => {
            crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("intltz_from_date_time_zone: time zone id '{id}' extracted from ext/date DateTimeZone not recognized"))?;
            Ok(Value::Null)
        }
    }
}

fn obj(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Rc<Object>, Throw> {
    let Value::Object(o) = arg(args, 0) else { unreachable!("checked against the stub") };
    if state(&o).zone.borrow().is_none() {
        return Err(ctx.throw("Error", "Found unconstructed IntlTimeZone".into()));
    }
    state(&o).err.reset();
    Ok(o)
}

fn f_get_id(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    if state(&o).unknown.get() {
        return Ok(s("Etc/Unknown"));
    }
    Ok(s(zone_of(&o).id()))
}

fn now_ms() -> i64 {
    crate::datefmt::now_utc().timestamp_millis()
}

/// The standard (raw) offset and the DST part at an instant, in ms.
fn offsets(z: &Zone, ms: i64) -> (i64, i64) {
    let (off, dst) = z.offsets_at(ms);
    ((off - dst) as i64 * 1000, dst as i64 * 1000)
}

/// Whether the zone observes daylight saving now (this year or next).
fn uses_dst(z: &Zone) -> Option<i64> {
    let now = now_ms();
    let year = 365 * 86_400_000i64;
    (0..24).map(|k| now + k * year / 24).map(|t| z.offsets_at(t).1).find(|&d| d != 0).map(|d| d as i64 * 1000)
}

fn f_get_raw_offset(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(Value::Int(offsets(&zone_of(&o), now_ms()).0))
}

fn f_get_dst_savings(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(Value::Int(uses_dst(&zone_of(&o)).unwrap_or(0)))
}

fn f_use_daylight_time(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(Value::Bool(uses_dst(&zone_of(&o)).is_some()))
}

fn set_ref(args: &mut [Value], i: usize, v: Value) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = v;
    }
}

fn f_get_offset(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let z = zone_of(&o);
    let t = arg(args, 1).to_float() as i64;
    let local = arg(args, 2).to_bool();
    let at = if local { t - z.offsets_at(t - z.offsets_at(t).0 as i64 * 1000).0 as i64 * 1000 } else { t };
    let (raw, dst) = offsets(&z, at);
    set_ref(args, 3, Value::Int(raw));
    set_ref(args, 4, Value::Int(dst));
    Ok(Value::Bool(true))
}

fn f_has_same_rules(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let Value::Object(other) = arg(args, 1) else { return Ok(Value::Bool(false)) };
    let (a, b) = (zone_of(&o), zone_of(&other));
    let now = now_ms();
    let day = 86_400_000i64;
    Ok(Value::Bool((0..800).all(|k| a.offsets_at(now + k * day) == b.offsets_at(now + k * day))))
}

fn f_to_date_time_zone(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let id = match zone_of(&o) {
        Zone::Iana(tz) => tz.name().to_string(),
        Zone::Fixed(off) => {
            let a = off.unsigned_abs();
            format!("{}{:02}:{:02}", if off < 0 { '-' } else { '+' }, a / 3600, a / 60 % 60)
        }
    };
    let class = ctx.class(b"DateTimeZone").expect("DateTimeZone");
    let mut a = [s(id)];
    let tz = ctx.new_instance(&class, &mut a, true)?;
    Ok(Value::Object(tz))
}

fn f_get_display_name(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let dst = arg(args, 1).to_bool();
    let style = if args.len() > 2 { arg(args, 2).to_int() } else { 2 };
    let locale = crate::opt_str(args, 3).unwrap_or_else(|| crate::default_locale(ctx));
    let z = zone_of(&o);
    let pattern = match style {
        1 => "z",
        2 => "zzzz",
        3 => "v",
        4 => "vvvv",
        5 => "O",
        6 => "OOOO",
        7 => "z",
        8 => "VVVV",
        _ => {
            crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "intltz_get_display_name: wrong display type")?;
            return Ok(Value::Bool(false));
        }
    };
    // an instant in this year's standard (or daylight) time
    let now = now_ms();
    let year = 365 * 86_400_000i64;
    let at = (0..24).map(|k| now + k * year / 24).find(|&t| (z.offsets_at(t).1 != 0) == dst).unwrap_or(now);
    let f = crate::datefmt::Formatter::new(&crate::uloc::canonicalize(&locale), -2, -2, z.clone(), 1, Some(pattern)).ok();
    let _ = fields_at(at, &z);
    Ok(f.and_then(|f| f.format_ms(at)).map_or(Value::Bool(false), s))
}

fn f_get_canonical_id(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::reset_global(ctx);
    let id = str_arg(args, 0);
    match zone_from_id(&id) {
        Some(z) => {
            set_ref(args, 1, Value::Bool(true));
            // the canonical (CLDR) form of an alias is ICU's; chrono-tz keeps the IANA name
            Ok(s(match z {
                Zone::Iana(tz) => tz.name().to_string(),
                Zone::Fixed(_) => z.id(),
            }))
        }
        None => {
            crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "intltz_get_canonical_id: error obtaining canonical ID")?;
            Ok(Value::Bool(false))
        }
    }
}

fn f_get_tz_data_version(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(s(chrono_tz::IANA_TZDB_VERSION))
}

fn f_get_error_code(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(o) = arg(args, 0) else { unreachable!() };
    Ok(Value::Int(state(&o).err.code.get()))
}

fn f_get_error_message(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(o) = arg(args, 0) else { unreachable!() };
    Ok(s(state(&o).err.message()))
}

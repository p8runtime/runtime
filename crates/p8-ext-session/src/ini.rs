//! The session.* settings and what changing one checks (the OnUpdate
//! handlers of session.c).
//!
//! Reference: $PHP_SRC_PATH/ext/session/session.c (PHP_INI_BEGIN)

use crate::state::{Module, Serializer, Status, atol, deprecated, flag, headers_sent, int, parse_bool, peek, state, text, warn};
use p8_runtime::ext::{Ctx, Registry};
use p8_runtime::string::PhpStr;

/// Each setting and its default.
pub(crate) const DEFAULTS: &[(&str, &[u8])] = &[
    ("session.save_path", b""),
    ("session.name", b"PHPSESSID"),
    ("session.save_handler", b"files"),
    ("session.auto_start", b"0"),
    ("session.gc_probability", b"1"),
    ("session.gc_divisor", b"100"),
    ("session.gc_maxlifetime", b"1440"),
    ("session.serialize_handler", b"php"),
    ("session.cookie_lifetime", b"0"),
    ("session.cookie_path", b"/"),
    ("session.cookie_domain", b""),
    ("session.cookie_secure", b"0"),
    ("session.cookie_httponly", b"0"),
    ("session.cookie_samesite", b""),
    ("session.use_cookies", b"1"),
    ("session.use_only_cookies", b"1"),
    ("session.use_strict_mode", b"0"),
    ("session.referer_check", b""),
    ("session.cache_limiter", b"nocache"),
    ("session.cache_expire", b"180"),
    ("session.use_trans_sid", b"0"),
    ("session.sid_length", b"32"),
    ("session.sid_bits_per_character", b"4"),
    ("session.lazy_write", b"1"),
    ("session.upload_progress.enabled", b"1"),
    ("session.upload_progress.cleanup", b"1"),
    ("session.upload_progress.prefix", b"upload_progress_"),
    ("session.upload_progress.name", b"PHP_SESSION_UPLOAD_PROGRESS"),
    ("session.upload_progress.freq", b"1%"),
    ("session.upload_progress.min_freq", b"1"),
];

/// Those that may be set only by the host (PHP_INI_PERDIR).
const PERDIR: &[&str] = &[
    "session.auto_start",
    "session.upload_progress.enabled",
    "session.upload_progress.cleanup",
    "session.upload_progress.prefix",
    "session.upload_progress.name",
    "session.upload_progress.freq",
    "session.upload_progress.min_freq",
];

/// Those whose handler is only OnUpdateSessionLong/String/Bool, which check
/// the state and nothing else.
const PLAIN: &[&str] = &[
    "session.gc_maxlifetime",
    "session.cookie_path",
    "session.cookie_domain",
    "session.cookie_secure",
    "session.cookie_httponly",
    "session.cookie_samesite",
    "session.use_cookies",
    "session.use_strict_mode",
    "session.cache_limiter",
    "session.cache_expire",
    "session.lazy_write",
];

const CHECKED: &[&str] = &[
    "session.save_path",
    "session.name",
    "session.save_handler",
    "session.serialize_handler",
    "session.cookie_lifetime",
    "session.use_only_cookies",
    "session.use_trans_sid",
    "session.referer_check",
    "session.sid_length",
    "session.sid_bits_per_character",
    "session.gc_probability",
    "session.gc_divisor",
    "session.trans_sid_tags",
    "session.trans_sid_hosts",
];

pub(crate) fn register(registry: &mut Registry) {
    for (name, default) in DEFAULTS {
        registry.add_ini(name, Some(default));
    }
    for name in PERDIR.iter().chain(PLAIN).chain(CHECKED) {
        registry.add_ini_check(name, check);
    }
    registry.add_startup_check(startup);
}

/// What PHP says once the host's settings are made: the deprecated ones
/// (in the order session.c registers them).
fn startup(ctx: &mut dyn Ctx) {
    // The modules the settings name (none that exists disables the session).
    let s = state(ctx);
    let module = Module::find(&text(ctx, "session.save_handler"));
    let usable = module.is_some() && Serializer::find(&text(ctx, "session.serialize_handler")).is_some();
    {
        let mut st = s.borrow_mut();
        st.module = module;
        st.status = if usable { Status::None } else { Status::Disabled };
    }
    if !flag(ctx, "session.use_only_cookies") {
        deprecated(ctx, "Disabling session.use_only_cookies INI setting is deprecated");
    }
    if !text(ctx, "session.referer_check").is_empty() {
        deprecated(ctx, "Usage of session.referer_check INI setting is deprecated");
    }
    if flag(ctx, "session.use_trans_sid") {
        deprecated(ctx, "Enabling session.use_trans_sid INI setting is deprecated");
    }
    if int(ctx, "session.sid_length") != 32 {
        deprecated(ctx, "session.sid_length INI setting is deprecated");
    }
    if int(ctx, "session.sid_bits_per_character") != 4 {
        deprecated(ctx, "session.sid_bits_per_character INI setting is deprecated");
    }
    if text(ctx, "session.trans_sid_tags") != b"a=href,area=href,frame=src,form=" {
        deprecated(ctx, "Usage of session.trans_sid_tags INI setting is deprecated");
    }
    if !text(ctx, "session.trans_sid_hosts").is_empty() {
        deprecated(ctx, "Usage of session.trans_sid_hosts INI setting is deprecated");
    }
}

/// Whether the host is making its settings (no script runs).
fn at_startup(ctx: &dyn Ctx) -> bool {
    ctx.location().0.is_empty()
}

/// SESSION_CHECK_ACTIVE_STATE and SESSION_CHECK_OUTPUT_STATE
fn settable(ctx: &mut dyn Ctx) -> bool {
    // (the host's settings are made before anything runs)
    if at_startup(ctx) {
        return true;
    }
    if peek(ctx).is_some_and(|s| s.borrow().status == Status::Active) {
        warn(ctx, "Session ini settings cannot be changed when a session is active");
        return false;
    }
    if headers_sent(ctx) {
        warn(ctx, "Session ini settings cannot be changed after headers have already been sent");
        return false;
    }
    true
}

/// "=,;.[ \t\r\n\013\014"
const FORBIDDEN: &[u8] = b"=,;.[ \t\r\n\x0b\x0c";

/// ini_set() of a session.* setting: whether the new value is taken.
fn check(ctx: &mut dyn Ctx, name: &[u8], new: &PhpStr) -> bool {
    let name = std::str::from_utf8(name).unwrap_or("");
    let value = new.as_bytes();
    if PERDIR.contains(&name) {
        // only the host (no script running) may set these
        if !at_startup(ctx) {
            return false;
        }
        if name == "session.upload_progress.freq" {
            let tmp = atol(value);
            if tmp < 0 {
                warn(ctx, "session.upload_progress.freq must be greater than or equal to 0");
                return false;
            }
            if value.ends_with(b"%") && tmp > 100 {
                warn(ctx, "session.upload_progress.freq must be less than or equal to 100%");
                return false;
            }
        }
        return true;
    }
    if PLAIN.contains(&name) {
        return settable(ctx);
    }
    // (the URL rewriter's, not the session's: nothing to check but its use)
    if name == "session.trans_sid_tags" || name == "session.trans_sid_hosts" {
        // (only a value other than the default is used)
        let default: &[u8] = if name == "session.trans_sid_tags" { b"a=href,area=href,frame=src,form=" } else { b"" };
        if !at_startup(ctx) && value != default {
            deprecated(ctx, &format!("Usage of {name} INI setting is deprecated"));
        }
        return true;
    }
    // a setting with a handler of its own
    if !settable(ctx) {
        return false;
    }
    match name {
        "session.save_path" => !value.contains(&0),
        "session.name" => {
            let numeric = p8_runtime::numeric::numeric_string(value).is_some();
            if value.is_empty() || value.contains(&0) || numeric || value.iter().any(|c| FORBIDDEN.contains(c)) {
                warn(
                    ctx,
                    &format!("session.name \"{}\" must not be numeric, empty, contain null bytes or any of the following characters \"=,;.[ \\t\\r\\n\\013\\014\"", String::from_utf8_lossy(&value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())])),
                );
                return false;
            }
            true
        }
        "session.save_handler" => {
            let Some(m) = Module::find(value) else {
                // (a name not known yet is taken at startup: the session is
                // disabled then, and starting it says why)
                if at_startup(ctx) {
                    return true;
                }
                warn(ctx, &format!("Session save handler \"{}\" cannot be found", String::from_utf8_lossy(value)));
                return false;
            };
            let s = state(ctx);
            // "user" is not for ini_set() to choose
            if !s.borrow().set_handler && m == Module::User {
                warn(ctx, "Session save handler \"user\" cannot be set by ini_set()");
                return false;
            }
            let mut st = s.borrow_mut();
            st.default_mod = st.module;
            st.module = Some(m);
            true
        }
        "session.serialize_handler" => {
            if Serializer::find(value).is_none() {
                if at_startup(ctx) {
                    return true;
                }
                warn(ctx, &format!("Serialization handler \"{}\" cannot be found", String::from_utf8_lossy(value)));
                return false;
            }
            true
        }
        "session.cookie_lifetime" => {
            if atol(value) < 0 {
                warn(ctx, "CookieLifetime cannot be negative");
                return false;
            }
            true
        }
        "session.use_only_cookies" => {
            if !parse_bool(value) && !at_startup(ctx) {
                deprecated(ctx, "Disabling session.use_only_cookies INI setting is deprecated");
            }
            true
        }
        "session.use_trans_sid" => {
            if parse_bool(value) && !at_startup(ctx) {
                deprecated(ctx, "Enabling session.use_trans_sid INI setting is deprecated");
            }
            true
        }
        "session.referer_check" => {
            if !value.is_empty() && !at_startup(ctx) {
                deprecated(ctx, "Usage of session.referer_check INI setting is deprecated");
            }
            true
        }
        "session.sid_length" => {
            let v = atol(value);
            if v != 32 && !at_startup(ctx) {
                deprecated(ctx, "session.sid_length INI setting is deprecated");
            }
            if is_integer(value) && (22..=crate::PS_MAX_SID_LENGTH as i64).contains(&v) {
                return true;
            }
            warn(ctx, "session.configuration \"session.sid_length\" must be between 22 and 256");
            false
        }
        "session.sid_bits_per_character" => {
            let v = atol(value);
            if v != 4 && !at_startup(ctx) {
                deprecated(ctx, "session.sid_bits_per_character INI setting is deprecated");
            }
            if is_integer(value) && (4..=6).contains(&v) {
                return true;
            }
            warn(ctx, "session.configuration \"session.sid_bits_per_character\" must be between 4 and 6");
            false
        }
        "session.gc_probability" => {
            if atol(value) < 0 {
                warn(ctx, "session.gc_probability must be greater than or equal to 0");
                return false;
            }
            true
        }
        "session.gc_divisor" => {
            if atol(value) <= 0 {
                warn(ctx, "session.gc_divisor must be greater than 0");
                return false;
            }
            true
        }
        _ => true,
    }
}

/// strtol() read all of it: optional sign and digits only.
fn is_integer(v: &[u8]) -> bool {
    let digits = v.strip_prefix(b"-").or_else(|| v.strip_prefix(b"+")).unwrap_or(v);
    let lead = v.iter().take_while(|c| c.is_ascii_whitespace()).count();
    let digits = if lead > 0 { &v[lead..] } else { digits };
    let digits = digits.strip_prefix(b"-").or_else(|| digits.strip_prefix(b"+")).unwrap_or(digits);
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

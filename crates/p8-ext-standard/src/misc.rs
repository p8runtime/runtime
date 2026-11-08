//! Runtime information and control: error levels, ini settings,
//! trigger_error, the environment, time, the garbage collector's switches.
//! Arguments arrive already checked and converted (p8_runtime::args).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_builtin_functions.c,
//! $PHP_SRC_PATH/main/main.c, $PHP_SRC_PATH/ext/standard/basic_functions.c,
//! $PHP_SRC_PATH/ext/standard/versioning.c

use crate::{arg, str_arg};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "getopt", run: getopt, by_ref: &[2] },
    Builtin { name: "get_cfg_var", run: get_cfg_var, by_ref: &[] },
    Builtin { name: "assert_options", run: assert_options, by_ref: &[] },
    Builtin { name: "error_reporting", run: error_reporting, by_ref: &[] },
    Builtin { name: "ini_get", run: ini_get, by_ref: &[] },
    Builtin { name: "ini_get_all", run: ini_get_all, by_ref: &[] },
    Builtin { name: "php_ini_loaded_file", run: no_ini_file, by_ref: &[] },
    Builtin { name: "php_ini_scanned_files", run: no_ini_file, by_ref: &[] },
    Builtin { name: "ini_set", run: ini_set, by_ref: &[] },
    Builtin { name: "ini_alter", run: ini_set, by_ref: &[] },
    Builtin { name: "trigger_error", run: trigger_error, by_ref: &[] },
    Builtin { name: "error_get_last", run: error_get_last, by_ref: &[] },
    Builtin { name: "error_clear_last", run: error_clear_last, by_ref: &[] },
    Builtin { name: "user_error", run: trigger_error, by_ref: &[] },
    Builtin { name: "set_time_limit", run: set_time_limit, by_ref: &[] },
    Builtin { name: "php_sapi_name", run: php_sapi_name, by_ref: &[] },
    Builtin { name: "phpversion", run: phpversion, by_ref: &[] },
    Builtin { name: "version_compare", run: version_compare, by_ref: &[] },
    Builtin { name: "error_log", run: error_log, by_ref: &[] },
    Builtin { name: "get_defined_functions", run: get_defined_functions, by_ref: &[] },
    Builtin { name: "get_defined_constants", run: get_defined_constants, by_ref: &[] },
    Builtin { name: "zend_version", run: zend_version, by_ref: &[] },
    Builtin { name: "quoted_printable_encode", run: quoted_printable_encode, by_ref: &[] },
    Builtin { name: "quoted_printable_decode", run: quoted_printable_decode, by_ref: &[] },
    Builtin { name: "getenv", run: getenv, by_ref: &[] },
    Builtin { name: "time", run: time, by_ref: &[] },
    Builtin { name: "gc_collect_cycles", run: gc_collect_cycles, by_ref: &[] },
    Builtin { name: "gc_mem_caches", run: gc_mem_caches, by_ref: &[] },
    Builtin { name: "gc_status", run: gc_status, by_ref: &[] },
    Builtin { name: "gc_enable", run: gc_switch, by_ref: &[] },
    Builtin { name: "gc_disable", run: gc_switch, by_ref: &[] },
    Builtin { name: "gc_enabled", run: gc_enabled, by_ref: &[] },
    Builtin { name: "usleep", run: usleep, by_ref: &[] },
    Builtin { name: "sleep", run: sleep, by_ref: &[] },
];

fn error_reporting(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let new = match args.first().map(Value::deref) {
        Some(v) if !v.is_null() => Some(v.to_int()),
        _ => None,
    };
    Ok(Value::int(ctx.error_level(new)))
}

fn ini_get(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(match ctx.ini_get(str_arg(args, 0).as_bytes()) {
        Some(v) if v.is_null() => Value::str(PhpStr::empty()),
        Some(v) => v,
        None => Value::FALSE,
    })
}

/// php_ini_loaded_file() / php_ini_scanned_files(): p8 reads no php.ini.
fn no_ini_file(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::FALSE)
}

/// ini_get_all(?string $extension = null, bool $details = true): every
/// directive (or one extension's), sorted by name; with details, each as
/// [global_value, local_value, access].
fn ini_get_all(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let ext = match arg(args, 0) {
        v if v.is_null() => None,
        _ => Some(String::from_utf8_lossy(str_arg(args, 0).as_bytes()).to_ascii_lowercase()),
    };
    let details = args.len() < 2 || arg(args, 1).to_bool();
    let mut rows: Vec<_> = crate::defaults::INI_DEFAULTS.iter().filter(|(e, _, _)| ext.as_deref().is_none_or(|x| x == *e)).collect();
    if let Some(x) = &ext
        && !ctx.modules().iter().any(|m| m.eq_ignore_ascii_case(x))
    {
        ctx.warn(&format!("ini_get_all(): Extension \"{x}\" cannot be found"));
        return Ok(Value::FALSE);
    }
    // zend_ini_sort_entries: names compared ignoring case; PHP's table
    // stays sorted after.
    rows.sort_by_key(|(_, name, _)| name.to_ascii_lowercase());
    *ctx.state(p8_runtime::info::INI_SORTED) = Value::TRUE;
    let text = |v: Option<Value>| match v {
        Some(v) if !v.is_null() => v,
        _ => Value::NULL,
    };
    let mut out = PhpArray::new();
    for (_, name, default) in rows {
        let local = text(ctx.ini_get(name.as_bytes()));
        let v = if details {
            let access = crate::defaults::INI_ACCESS.iter().find(|(n, _)| n == name).map_or(7, |(_, a)| i64::from(*a));
            let mut d = PhpArray::new();
            d.insert(Key::str(PhpStr::from("global_value")), default.map_or(Value::NULL, |b| Value::str(PhpStr::from(b))));
            d.insert(Key::str(PhpStr::from("local_value")), local);
            d.insert(Key::str(PhpStr::from("access")), Value::int(access));
            Value::array(d)
        } else {
            local
        };
        out.insert(Key::str(PhpStr::from(*name)), v);
    }
    Ok(Value::array(out))
}

fn ini_set(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let value = match arg(args, 1) {
        v if v.is_null() => Value::str(PhpStr::empty()),
        v => match v.as_bool() {
            Some(b) => Value::str(PhpStr::from(if b { "1" } else { "" })),
            None => v,
        },
    };
    let name = str_arg(args, 0);
    // zend_alter_ini_entry_ex: a script may change only what PHP_INI_USER
    // (1) allows; a PHP_INI_SYSTEM or PERDIR directive is fixed by then.
    if crate::defaults::INI_ACCESS.iter().any(|(n, access)| n.as_bytes() == name.as_bytes() && access & 1 == 0) {
        return Ok(Value::FALSE);
    }
    Ok(match ctx.ini_set(name.as_bytes(), value) {
        Some(old) if old.is_null() => Value::str(PhpStr::empty()),
        Some(old) => old,
        None => Value::FALSE,
    })
}

const E_USER_ERROR: i64 = 256;
const E_USER_WARNING: i64 = 512;
const E_USER_NOTICE: i64 = 1024;
const E_USER_DEPRECATED: i64 = 16384;

/// trigger_error($message, $error_level = E_USER_NOTICE)
/// error_get_last(): ["type", "message", "file", "line"], or null.
fn error_get_last(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let Some((level, message, file, line)) = ctx.last_error(false) else { return Ok(Value::NULL) };
    let mut a = PhpArray::new();
    a.insert(Key::str(PhpStr::from("type")), Value::int(level));
    a.insert(Key::str(PhpStr::from("message")), Value::str(PhpStr::from(message.as_str())));
    a.insert(Key::str(PhpStr::from("file")), Value::str(PhpStr::from(file.as_str())));
    a.insert(Key::str(PhpStr::from("line")), Value::int(line as i64));
    Ok(Value::array(a))
}

/// error_log($message, $message_type = 0, $destination = null, $headers = null)
/// (_php_error_log_ex): 0 the error log, 3 append to a file, 4 the
/// SAPI's log (stderr); 1 is mail, which p8 cannot send.
fn error_log(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let message = str_arg(args, 0);
    let kind = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    let destination = match arg(args, 2) {
        v if v.is_null() => None,
        v => Some(v.to_php_string().unwrap_or_else(PhpStr::empty)),
    };
    Ok(Value::bool(match kind {
        1 => false,
        2 => return Err(ctx.throw("ValueError", "TCP/IP option is not available for error logging".into())),
        3 => {
            let path = String::from_utf8_lossy(destination.as_ref().map_or(&b""[..], |d| d.as_bytes())).into_owned();
            append_log(ctx, &path, message.as_bytes())
        }
        4 => {
            sapi_log(message.as_bytes());
            true
        }
        _ => {
            log_err(ctx, message.as_bytes());
            true
        }
    }))
}

/// The CLI's log_message: the message and a newline on stderr.
fn sapi_log(message: &[u8]) {
    use std::io::Write;
    let mut e = std::io::stderr().lock();
    let _ = e.write_all(message);
    let _ = e.write_all(b"\n");
}

/// php_log_err(): to the error_log ini's file, dated, when it is set and
/// can be opened; else the SAPI's log.
pub(crate) fn log_err(ctx: &mut dyn Ctx, message: &[u8]) {
    let file = ctx.ini_get(b"error_log").and_then(|v| v.to_php_string()).filter(|f| !f.is_empty() && f.as_bytes() != b"syslog");
    if let Some(file) = file {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let (zone, name) = crate::dates::default_zone(ctx);
        let date = crate::dates::format(b"d-M-Y H:i:s e", ts, &zone.at(ts), &name);
        let mut line = b"[".to_vec();
        line.extend_from_slice(&date);
        line.extend_from_slice(b"] ");
        line.extend_from_slice(message);
        line.push(b'\n');
        let path = String::from_utf8_lossy(file.as_bytes()).into_owned();
        let opened = std::fs::OpenOptions::new().create(true).append(true).open(&path);
        if let Ok(mut f) = opened {
            use std::io::Write;
            let _ = f.write_all(&line);
            return;
        }
    }
    sapi_log(message);
}

/// error_log() type 3: append to `path` (php://stderr and php://stdout
/// too), warning as fopen() does when it cannot.
fn append_log(ctx: &mut dyn Ctx, path: &str, message: &[u8]) -> bool {
    use std::io::Write;
    match path {
        "php://stderr" => std::io::stderr().write_all(message).is_ok(),
        "php://stdout" | "php://output" => {
            ctx.write_stdout(message);
            true
        }
        _ => match std::fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(mut f) => f.write_all(message).is_ok(),
            Err(e) => {
                let reason = match e.kind() {
                    std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
                    std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
                    _ => e.to_string(),
                };
                ctx.warn(&format!("error_log({path}): Failed to open stream: {reason}"));
                false
            }
        },
    }
}

fn version_compare(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    // The C code reads each version up to its first NUL byte.
    let c_str = |v: PhpStr| {
        let b = v.as_bytes();
        b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())].to_vec()
    };
    let compare = compare_versions(&c_str(str_arg(args, 0)), &c_str(str_arg(args, 1)));
    if args.len() < 3 || arg(args, 2).is_null() {
        return Ok(Value::int(compare as i64));
    }
    let op = str_arg(args, 2);
    Ok(Value::bool(match op.as_bytes() {
        b"<" | b"lt" => compare == -1,
        b"<=" | b"le" => compare != 1,
        b">" | b"gt" => compare == 1,
        b">=" | b"ge" => compare != -1,
        b"==" | b"=" | b"eq" => compare == 0,
        b"!=" | b"<>" | b"ne" => compare != 0,
        _ => return Err(ctx.throw("ValueError", "version_compare(): Argument #3 ($operator) must be a valid comparison operator".into())),
    }))
}

/// php_canonicalize_version(): `-`, `_`, `+` and other non-alphanumerics
/// become `.`, and a `.` goes between a digit and a non-digit; no
/// trailing `.`.
fn canonicalize_version(v: &[u8]) -> Vec<u8> {
    let digit = |c: u8| c.is_ascii_digit();
    let non_digit = |c: u8| !c.is_ascii_digit() && c != b'.';
    let mut out = Vec::with_capacity(v.len() * 2);
    let Some((&first, rest)) = v.split_first() else { return out };
    out.push(first);
    let mut lp = first;
    for &c in rest {
        let need_dot = out[out.len() - 1] != b'.';
        if matches!(c, b'-' | b'_' | b'+') {
            if need_dot {
                out.push(b'.');
            }
        } else if non_digit(lp) && digit(c) || digit(lp) && non_digit(c) {
            if need_dot {
                out.push(b'.');
            }
            out.push(c);
        } else if !c.is_ascii_alphanumeric() {
            if need_dot {
                out.push(b'.');
            }
        } else {
            out.push(c);
        }
        lp = c;
    }
    if out.last() == Some(&b'.') {
        out.pop();
    }
    out
}

/// compare_special_version_forms(): by the first form each one starts with.
fn special_form_order(form: &[u8]) -> i32 {
    const FORMS: [(&[u8], i32); 10] =
        [(b"dev", 0), (b"alpha", 1), (b"a", 1), (b"beta", 2), (b"b", 2), (b"RC", 3), (b"rc", 3), (b"#", 4), (b"pl", 5), (b"p", 5)];
    FORMS.iter().find(|(name, _)| form.starts_with(name)).map_or(-1, |&(_, order)| order)
}

/// php_version_compare(): -1, 0 or 1.
fn compare_versions(a: &[u8], b: &[u8]) -> i32 {
    if a.is_empty() || b.is_empty() {
        return (!a.is_empty()) as i32 - (!b.is_empty()) as i32;
    }
    let canonical = |v: &[u8]| if v[0] == b'#' { v.to_vec() } else { canonicalize_version(v) };
    let (v1, v2) = (canonical(a), canonical(b));
    // strtol(): the leading digits, saturating.
    let number = |part: &[u8]| part.iter().take_while(|c| c.is_ascii_digit()).fold(0i64, |n, &c| n.saturating_mul(10).saturating_add((c - b'0') as i64));
    let starts_digit = |part: &[u8]| part.first().is_some_and(u8::is_ascii_digit);
    let (mut p1, mut p2) = (0, 0);
    let (mut n1, mut n2) = (Some(0), Some(0));
    let mut compare = 0;
    while p1 < v1.len() && p2 < v2.len() && n1.is_some() && n2.is_some() {
        n1 = v1[p1..].iter().position(|&c| c == b'.').map(|i| p1 + i);
        n2 = v2[p2..].iter().position(|&c| c == b'.').map(|i| p2 + i);
        let part1 = &v1[p1..n1.unwrap_or(v1.len())];
        let part2 = &v2[p2..n2.unwrap_or(v2.len())];
        compare = match (starts_digit(part1), starts_digit(part2)) {
            (true, true) => number(part1).cmp(&number(part2)) as i32,
            (false, false) => special_form_order(part1).cmp(&special_form_order(part2)) as i32,
            (true, false) => special_form_order(b"#N#").cmp(&special_form_order(part2)) as i32,
            (false, true) => special_form_order(part1).cmp(&special_form_order(b"#N#")) as i32,
        };
        if compare != 0 {
            break;
        }
        if let Some(n) = n1 {
            p1 = n + 1;
        }
        if let Some(n) = n2 {
            p2 = n + 1;
        }
    }
    if compare == 0 {
        if n1.is_some() {
            compare = if starts_digit(&v1[p1..]) { 1 } else { compare_versions(&v1[p1..], b"#N#") };
        } else if n2.is_some() {
            compare = if starts_digit(&v2[p2..]) { -1 } else { compare_versions(b"#N#", &v2[p2..]) };
        }
    }
    compare
}

fn error_clear_last(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.last_error(true);
    Ok(Value::NULL)
}

fn trigger_error(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let message = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let level = if args.len() > 1 { arg(args, 1).to_int() } else { E_USER_NOTICE };
    match level {
        E_USER_ERROR => {
            ctx.deprecated(
                "Passing E_USER_ERROR to trigger_error() is deprecated since 8.4, throw an exception or call exit with a string message instead",
            );
            // A user handler takes it (and may throw, or decline: fatal
            // then); without one it is fatal now.
            if !ctx.user_handles(E_USER_ERROR) {
                return Err(ctx.fatal(message));
            }
            ctx.report(E_USER_ERROR, &message);
            Ok(Value::TRUE)
        }
        E_USER_WARNING | E_USER_NOTICE | E_USER_DEPRECATED => {
            ctx.report(level, &message);
            Ok(Value::TRUE)
        }
        _ => Err(ctx.throw(
            "ValueError",
            "trigger_error(): Argument #2 ($error_level) must be one of E_USER_ERROR, E_USER_WARNING, E_USER_NOTICE, or E_USER_DEPRECATED".into(),
        )),
    }
}

/// set_time_limit: the host's watchdog enforces max_execution_time; this
/// records the setting (and the CLI's default is no limit).
fn set_time_limit(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let seconds = arg(args, 0).to_int();
    ctx.ini_set(b"max_execution_time", Value::str(PhpStr::from(seconds.to_string())));
    Ok(Value::TRUE)
}

fn php_sapi_name(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    // "cli", or "cli-server" when the host serves web requests.
    Ok(Value::str(if ctx.response().server { "cli-server" } else { "cli" }))
}

/// phpversion($extension = null): PHP's version, or a loaded extension's
/// (zend_get_module_version, any case); false for one p8 lacks or one
/// without a version.
fn phpversion(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    match args.first().map(Value::deref) {
        Some(ext) if !ext.is_null() => {
            let name = ext.to_php_string().unwrap_or_else(PhpStr::empty);
            let version = p8_runtime::info::loaded_module(ctx, name.as_bytes()).and_then(p8_runtime::info::module_version);
            Ok(version.map_or(Value::FALSE, Value::str))
        }
        _ => Ok(Value::str(crate::PHP_VERSION)),
    }
}

/// getenv($name = null, $local_only = false): one variable, or all of them.
fn getenv(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    match args.first().map(Value::deref) {
        Some(name) if !name.is_null() => {
            let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
            Ok(crate::process::env_var(ctx, name.as_bytes()).map_or(Value::FALSE, |v| Value::str(PhpStr::new(v))))
        }
        _ => Ok(Value::array(crate::process::env_all(ctx))),
    }
}

fn time(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    Ok(Value::int(now.as_secs() as i64))
}

/// gc_collect_cycles(): free the garbage cycles reference counting can't
/// (their destructors run first); how many values they held.
fn gc_collect_cycles(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::int(ctx.gc_collect()?))
}

/// gc_mem_caches(): bytes handed back from the memory manager's caches;
/// p8 keeps no such caches (Rust's allocator owns memory), so none.
fn gc_mem_caches(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::int(0))
}

/// gc_status(): the collector's counters, as PHP 8.4 reports them.
fn gc_status(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let (runs, collected) = ctx.gc_counts();
    let mut out = PhpArray::hashed(12);
    for (k, v) in [
        ("running", Value::FALSE),
        ("protected", Value::FALSE),
        ("full", Value::FALSE),
        ("runs", Value::int(runs)),
        ("collected", Value::int(collected)),
        ("threshold", Value::int(10001)),
        ("buffer_size", Value::int(16384)),
        ("roots", Value::int(0)),
        // No collector runs (reference counting frees what it can), so all
        // the time is the application's.
        ("application_time", Value::float(ctx.running_for().as_secs_f64())),
        ("collector_time", Value::float(0.0)),
        ("destructor_time", Value::float(0.0)),
        ("free_time", Value::float(0.0)),
    ] {
        out.insert(Key::from_string(PhpStr::from(k)), v);
    }
    Ok(Value::array(out))
}

fn gc_switch(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::NULL)
}

fn gc_enabled(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::TRUE)
}

fn usleep(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    std::thread::sleep(std::time::Duration::from_micros(arg(args, 0).to_int().max(0) as u64));
    Ok(Value::NULL)
}

fn sleep(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let seconds = arg(args, 0).to_int();
    if seconds < 0 {
        return Err(ctx.throw("ValueError", "sleep(): Argument #1 ($seconds) must be greater than or equal to 0".into()));
    }
    std::thread::sleep(std::time::Duration::from_secs(seconds as u64));
    Ok(Value::int(0))
}

/// assert_options($option, $value): an assert.* setting's old value (and
/// the new one set): ASSERT_ACTIVE, _CALLBACK, _BAIL, _WARNING, _EXCEPTION.
fn assert_options(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name: &[u8] = match arg(args, 0).to_int() {
        1 => b"assert.active",
        2 => b"assert.callback",
        3 => b"assert.bail",
        4 => b"assert.warning",
        5 => b"assert.exception",
        _ => return Err(ctx.throw("ValueError", "assert_options(): Argument #1 ($option) must be an ASSERT_* constant".into())),
    };
    let callback = name == b"assert.callback";
    let old = ctx.ini_get(name).unwrap_or(Value::NULL);
    if args.len() > 1 {
        let new = match arg(args, 1) {
            v if callback => v,
            v => match v.as_bool() {
                Some(b) => Value::str(PhpStr::from(if b { "1" } else { "0" })),
                None => match v.to_php_string() {
                    Some(s) => Value::str(s),
                    None => return Err(ctx.throw("TypeError", "assert_options(): Argument #2 ($value) must be of type string|int|bool|null".into())),
                },
            },
        };
        ctx.ini_set(name, new);
    }
    Ok(if callback {
        match old {
            v if v.as_str().is_some_and(|s| s.is_empty()) => Value::NULL,
            v => v,
        }
    } else {
        Value::int(old.to_int())
    })
}

/// get_cfg_var($option): a setting as the host gave it (`-d name=value`),
/// a string; `name[key]=value` ones make an array; false if not given.
fn get_cfg_var(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    let mut found = None;
    let mut entries = PhpArray::new();
    for (n, v) in ctx.cfg() {
        if n == name.as_bytes() {
            found = Some(Value::str(PhpStr::from(v.as_str())));
        } else if let Some(rest) = n.strip_prefix(name.as_bytes())
            && let Some(inner) = rest.strip_prefix(b"[").and_then(|r| r.strip_suffix(b"]"))
        {
            let key = std::str::from_utf8(inner).ok().map(str::trim).unwrap_or("");
            let value = Value::str(PhpStr::from(v.as_str()));
            match Key::from_value(&Value::str(PhpStr::from(key))) {
                Ok((k, _)) if !key.is_empty() => {
                    entries.insert(k, value);
                }
                _ => {
                    let _ = entries.push(value);
                }
            }
        }
    }
    Ok(match found {
        Some(v) => v,
        None if !entries.is_empty() => Value::array(entries),
        None => Value::FALSE,
    })
}

/// One option getopt() knows: a letter or a long name, and whether it
/// takes a value (1: required, 2: optional).
struct Opt {
    short: u8,
    long: Option<Vec<u8>>,
    need: u8,
}

/// getopt($short_options, $long_options = [], &$rest_index = null): the
/// options in the command line (or $argv), as php_getopt() reads them.
fn getopt(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    if let Some(c) = args.get(2).and_then(Value::as_reference) {
        *c.borrow_mut() = Value::int(1);
    }
    // The request's $_SERVER['argv'] (edits to $_SERVER don't reach it),
    // else the global $argv.
    let argv = match ctx.request_argv() {
        Some(a) => Some(Value::array(PhpArray::from_list(a.iter().map(|s| Value::str(PhpStr::from(s.as_str()))).collect()))),
        None => ctx.global_value(b"argv"),
    };
    let argv: Vec<Vec<u8>> = match argv.as_ref().and_then(Value::as_array) {
        Some(a) =>a.iter().map(|(_, v)| v.deref().to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()).collect(),
        _ => return Ok(Value::FALSE),
    };
    // Short options: letters and digits, each with ':' (value) or '::' (optional).
    let short = str_arg(args, 0);
    let mut opts: Vec<Opt> = Vec::new();
    let mut s = short.as_bytes();
    while let Some((&c, rest)) = s.split_first() {
        if !c.is_ascii_alphanumeric() {
            break;
        }
        s = rest;
        let mut need = 0;
        if s.first() == Some(&b':') {
            need = 1;
            s = &s[1..];
            if s.first() == Some(&b':') {
                need = 2;
                s = &s[1..];
            }
        }
        opts.push(Opt { short: c, long: None, need });
    }
    if let Some(longs) = arg(args, 1).as_array() {
        for (_, v) in longs.iter() {
            let mut name = v.deref().to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default();
            let mut need = 0;
            if name.last() == Some(&b':') {
                name.pop();
                need = 1;
                if name.last() == Some(&b':') {
                    name.pop();
                    need = 2;
                }
            }
            opts.push(Opt { short: 0, long: Some(name), need });
        }
    }
    let mut out = PhpArray::new();
    let mut add = |name: &[u8], value: Value| {
        let key = match Key::from_value(&Value::str(PhpStr::from(name))) {
            Ok((k @ Key::Int(_), _)) if !(name.len() > 1 && name[0] == b'0') => k,
            _ => Key::str(PhpStr::from(name)),
        };
        match out.get(&key).cloned().map(Value::into_array) {
            Some(Ok(a)) => {
                let mut a = (*a).clone();
                let _ = a.push(value);
                out.insert(key, Value::array(a));
            }
            Some(Err(old)) => {
                out.insert(key, Value::array(PhpArray::from_list(vec![old, value])));
            }
            None => {
                out.insert(key, value);
            }
        }
    };
    let byte = |a: &[u8], i: usize| a.get(i).copied().unwrap_or(0);
    let (mut optind, mut optchr, mut dash) = (1usize, 0usize, false);
    while optind < argv.len() {
        let a = &argv[optind];
        if !dash && (byte(a, 0) != b'-' || byte(a, 1) == 0) {
            break;
        }
        let found: Option<usize>;
        let arg_start: usize;
        if byte(a, 0) == b'-' && byte(a, 1) == b'-' {
            if byte(a, 2) == 0 {
                optind += 1;
                break;
            }
            // --name or --name=value
            let body = &a[2..];
            let (name, mut start) = match body.iter().position(|&c| c == b'=') {
                Some(p) => (&body[..p], 3),
                None => (body, 2),
            };
            match opts.iter().position(|o| o.long.as_deref() == Some(name)) {
                Some(i) => {
                    found = Some(i);
                    start += name.len();
                    arg_start = start;
                    optchr = 0;
                    dash = false;
                }
                None => {
                    optind += 1;
                    continue;
                }
            }
        } else {
            if !dash {
                dash = true;
                optchr = 1;
            }
            if byte(a, optchr) == b':' {
                dash = false;
                optind += 1;
                continue;
            }
            arg_start = 1 + optchr;
            match opts.iter().position(|o| o.short != 0 && o.short == byte(a, optchr)) {
                Some(i) => found = Some(i),
                None => {
                    if byte(a, optchr + 1) == 0 {
                        dash = false;
                        optind += 1;
                    } else {
                        optchr += 1;
                    }
                    continue;
                }
            }
        }
        let Some(i) = found else { break };
        let o = &opts[i];
        let name: Vec<u8> = match &o.long {
            Some(n) => n.clone(),
            None => vec![o.short],
        };
        if o.need > 0 {
            dash = false;
            let value;
            if byte(a, arg_start) == 0 {
                optind += 1;
                if optind == argv.len() {
                    if o.need == 1 {
                        continue;
                    }
                    value = Value::FALSE;
                } else if o.need == 1 {
                    value = Value::str(PhpStr::from(argv[optind].as_slice()));
                    optind += 1;
                } else {
                    value = Value::FALSE;
                }
            } else if byte(a, arg_start) == b'=' {
                value = Value::str(PhpStr::from(&a[arg_start + 1..]));
                optind += 1;
            } else {
                value = Value::str(PhpStr::from(&a[arg_start..]));
                optind += 1;
            }
            add(&name, value);
        } else {
            if arg_start >= 2 && !(byte(a, 0) == b'-' && byte(a, 1) == b'-') {
                if byte(a, optchr + 1) == 0 {
                    dash = false;
                    optind += 1;
                } else {
                    optchr += 1;
                }
            } else {
                optind += 1;
            }
            add(&name, Value::FALSE);
        }
    }
    if let Some(c) = args.get(2).and_then(Value::as_reference) {
        *c.borrow_mut() = Value::int(optind as i64);
    }
    Ok(Value::array(out))
}

/// get_defined_constants(bool $categorize = false): name => value for
/// every constant, builtins first in the order their extensions made them,
/// then the script's; categorized, grouped by module ("Core", "pcre", ...,
/// "user").
fn get_defined_constants(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let categorize = args.first().is_some_and(|v| v.deref().to_bool());
    let mut flat = PhpArray::new();
    let mut groups: Vec<(String, PhpArray)> = Vec::new();
    for (module, name, value) in ctx.defined_constants() {
        let key = Key::from_string(name);
        if categorize {
            match groups.iter_mut().find(|(m, _)| *m == module) {
                Some((_, g)) => g.insert(key, value),
                None => {
                    let mut g = PhpArray::new();
                    g.insert(key, value);
                    groups.push((module, g));
                }
            }
        } else {
            flat.insert(key, value);
        }
    }
    if !categorize {
        return Ok(Value::array(flat));
    }
    let mut out = PhpArray::new();
    for (module, g) in groups {
        out.insert(Key::from_string(PhpStr::from(module.as_str())), Value::array(g));
    }
    Ok(Value::array(out))
}

/// get_defined_functions(): ['internal' => builtins, 'user' => the
/// script's, lowercase, in the order declared].
fn get_defined_functions(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let (internal, user) = ctx.defined_functions();
    let mut out = PhpArray::new();
    let list = |names: Vec<Value>| Value::array(PhpArray::from_list(names));
    out.insert(Key::from_string(PhpStr::from("internal")), list(internal.into_iter().map(Value::str).collect()));
    out.insert(Key::from_string(PhpStr::from("user")), list(user.into_iter().map(|n| Value::str(PhpStr::new(n))).collect()));
    Ok(Value::array(out))
}

/// zend_version(): the Zend Engine's version (4.6.x for PHP 8.6.x).
fn zend_version(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str(PhpStr::from(crate::zend_engine_version())))
}

/// quoted_printable_encode() (php_quot_print_encode): RFC 2045 lines of
/// at most 76, broken softly before a UTF-8 sequence would cross the end.
fn quoted_printable_encode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    const MAXL: usize = 75;
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let s = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let s = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3);
    let mut lp = 0;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        let next = s.get(i + 1).copied();
        if c == b'\r' && next == Some(b'\n') && i + 1 < s.len() {
            out.extend_from_slice(b"\r\n");
            i += 2;
            lp = 0;
            continue;
        }
        if c.is_ascii_control() || c == 0x7f || c & 0x80 != 0 || c == b'=' || (c == b' ' && next == Some(b'\r')) {
            lp += 3;
            // A UTF-8 sequence's lead byte breaks the line early when the
            // whole sequence would not fit.
            let breaks = match c {
                0x00..=0x7f => lp > MAXL,
                0x80..=0xdf => lp + 3 > MAXL,
                0xe0..=0xef => lp + 6 > MAXL,
                0xf0..=0xf4 => lp + 9 > MAXL,
                _ => false,
            };
            if breaks {
                out.extend_from_slice(b"=\r\n");
                lp = 3;
            }
            out.extend_from_slice(&[b'=', HEX[(c >> 4) as usize], HEX[(c & 15) as usize]]);
        } else {
            lp += 1;
            if lp > MAXL {
                out.extend_from_slice(b"=\r\n");
                lp = 1;
            }
            out.push(c);
        }
        i += 1;
    }
    Ok(Value::str(PhpStr::new(out)))
}

/// quoted_printable_decode(): =XX bytes, soft line breaks (`=` then
/// optional blanks then CR, LF or CRLF) removed; stops at a NUL byte.
fn quoted_printable_decode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let s = s.as_bytes();
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while at(i) != 0 {
        if at(i) != b'=' {
            out.push(at(i));
            i += 1;
            continue;
        }
        if let (Some(h), Some(l)) = (hex(at(i + 1)), hex(at(i + 2)))
            && at(i + 1) != 0
            && at(i + 2) != 0
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        let mut k = 1;
        while matches!(at(i + k), b' ' | b'\t') {
            k += 1;
        }
        match (at(i + k), at(i + k + 1)) {
            (0, _) => i += k,
            (b'\r', b'\n') => i += k + 2,
            (b'\r' | b'\n', _) => i += k + 1,
            _ => {
                out.push(b'=');
                i += 1;
            }
        }
    }
    Ok(Value::str(PhpStr::new(out)))
}

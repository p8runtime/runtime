//! Runtime information and control: error levels, ini settings,
//! trigger_error, the environment, time, the garbage collector's switches.
//! Arguments arrive already checked and converted (p8_runtime::args).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_builtin_functions.c,
//! $PHP_SRC_PATH/main/main.c, $PHP_SRC_PATH/ext/standard/basic_functions.c

use crate::{arg, str_arg};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "get_cfg_var", run: get_cfg_var, by_ref: &[] },
    Builtin { name: "assert_options", run: assert_options, by_ref: &[] },
    Builtin { name: "error_reporting", run: error_reporting, by_ref: &[] },
    Builtin { name: "ini_get", run: ini_get, by_ref: &[] },
    Builtin { name: "ini_set", run: ini_set, by_ref: &[] },
    Builtin { name: "ini_alter", run: ini_set, by_ref: &[] },
    Builtin { name: "trigger_error", run: trigger_error, by_ref: &[] },
    Builtin { name: "user_error", run: trigger_error, by_ref: &[] },
    Builtin { name: "set_time_limit", run: set_time_limit, by_ref: &[] },
    Builtin { name: "php_sapi_name", run: php_sapi_name, by_ref: &[] },
    Builtin { name: "phpversion", run: phpversion, by_ref: &[] },
    Builtin { name: "getenv", run: getenv, by_ref: &[] },
    Builtin { name: "time", run: time, by_ref: &[] },
    Builtin { name: "gc_collect_cycles", run: gc_collect_cycles, by_ref: &[] },
    Builtin { name: "gc_status", run: gc_status, by_ref: &[] },
    Builtin { name: "gc_enable", run: gc_switch, by_ref: &[] },
    Builtin { name: "gc_disable", run: gc_switch, by_ref: &[] },
    Builtin { name: "gc_enabled", run: gc_enabled, by_ref: &[] },
    Builtin { name: "usleep", run: usleep, by_ref: &[] },
    Builtin { name: "sleep", run: sleep, by_ref: &[] },
];

fn error_reporting(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let new = match args.first().map(Value::deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.to_int()),
    };
    Ok(Value::Int(ctx.error_level(new)))
}

fn ini_get(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(match ctx.ini_get(str_arg(args, 0).as_bytes()) {
        Some(Value::Null) => Value::Str(PhpStr::empty()),
        Some(v) => v,
        None => Value::Bool(false),
    })
}

fn ini_set(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let value = match arg(args, 1) {
        Value::Null => Value::Str(PhpStr::empty()),
        Value::Bool(b) => Value::Str(PhpStr::from(if b { "1" } else { "" })),
        v => v,
    };
    Ok(match ctx.ini_set(str_arg(args, 0).as_bytes(), value) {
        Some(Value::Null) => Value::Str(PhpStr::empty()),
        Some(old) => old,
        None => Value::Bool(false),
    })
}

const E_USER_ERROR: i64 = 256;
const E_USER_WARNING: i64 = 512;
const E_USER_NOTICE: i64 = 1024;
const E_USER_DEPRECATED: i64 = 16384;

/// trigger_error($message, $error_level = E_USER_NOTICE)
fn trigger_error(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let message = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let level = if args.len() > 1 { arg(args, 1).to_int() } else { E_USER_NOTICE };
    match level {
        E_USER_ERROR => {
            ctx.deprecated(
                "Passing E_USER_ERROR to trigger_error() is deprecated since 8.4, throw an exception or call exit with a string message instead",
            );
            Err(ctx.fatal(message))
        }
        E_USER_WARNING | E_USER_NOTICE | E_USER_DEPRECATED => {
            ctx.report(level, &message);
            Ok(Value::Bool(true))
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
    ctx.ini_set(b"max_execution_time", Value::Str(PhpStr::from(seconds.to_string())));
    Ok(Value::Bool(true))
}

fn php_sapi_name(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str("cli"))
}

/// phpversion($extension = null): PHP's version, or an extension's (which
/// for bundled extensions is PHP's own); false for an extension p8 lacks.
fn phpversion(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let version = ctx.constant(b"PHP_VERSION").unwrap_or(Value::Bool(false));
    match args.first().map(Value::deref) {
        None | Some(Value::Null) => Ok(version),
        Some(ext) => {
            let name = ext.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_ascii_lowercase();
            Ok(if matches!(name.as_slice(), b"standard" | b"core") { version } else { Value::Bool(false) })
        }
    }
}

/// getenv($name = null, $local_only = false): one variable, or all of them.
fn getenv(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    match args.first().map(Value::deref) {
        None | Some(Value::Null) => Ok(Value::Array(Rc::new(crate::process::env_all(ctx)))),
        Some(name) => {
            let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
            Ok(crate::process::env_var(ctx, name.as_bytes()).map_or(Value::Bool(false), |v| Value::Str(PhpStr::new(v))))
        }
    }
}

fn time(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    Ok(Value::Int(now.as_secs() as i64))
}

/// gc_collect_cycles(): free the garbage cycles reference counting can't
/// (their destructors run first); how many values they held.
fn gc_collect_cycles(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(ctx.gc_collect()?))
}

/// gc_status(): the collector's counters, as PHP 8.4 reports them.
fn gc_status(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let (runs, collected) = ctx.gc_counts();
    let mut out = PhpArray::new();
    for (k, v) in [
        ("running", Value::Bool(false)),
        ("protected", Value::Bool(false)),
        ("full", Value::Bool(false)),
        ("runs", Value::Int(runs)),
        ("collected", Value::Int(collected)),
        ("threshold", Value::Int(10001)),
        ("buffer_size", Value::Int(16384)),
        ("roots", Value::Int(0)),
        ("application_time", Value::Float(0.0)),
        ("collector_time", Value::Float(0.0)),
        ("dtor_time", Value::Float(0.0)),
        ("free_time", Value::Float(0.0)),
    ] {
        out.insert(Key::from_string(PhpStr::from(k)), v);
    }
    Ok(Value::Array(Rc::new(out)))
}

fn gc_switch(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

fn gc_enabled(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(true))
}

fn usleep(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    std::thread::sleep(std::time::Duration::from_micros(arg(args, 0).to_int().max(0) as u64));
    Ok(Value::Null)
}

fn sleep(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let seconds = arg(args, 0).to_int();
    if seconds < 0 {
        return Err(ctx.throw("ValueError", "sleep(): Argument #1 ($seconds) must be greater than or equal to 0".into()));
    }
    std::thread::sleep(std::time::Duration::from_secs(seconds as u64));
    Ok(Value::Int(0))
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
    let old = ctx.ini_get(name).unwrap_or(Value::Null);
    if args.len() > 1 {
        let new = match arg(args, 1) {
            v if callback => v,
            Value::Bool(b) => Value::Str(PhpStr::from(if b { "1" } else { "0" })),
            v => match v.to_php_string() {
                Some(s) => Value::Str(s),
                None => return Err(ctx.throw("TypeError", "assert_options(): Argument #2 ($value) must be of type string|int|bool|null".into())),
            },
        };
        ctx.ini_set(name, new);
    }
    Ok(if callback {
        match old {
            Value::Str(s) if s.as_bytes().is_empty() => Value::Null,
            v => v,
        }
    } else {
        Value::Int(old.to_int())
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
            found = Some(Value::Str(PhpStr::from(v.as_str())));
        } else if let Some(rest) = n.strip_prefix(name.as_bytes())
            && let Some(inner) = rest.strip_prefix(b"[").and_then(|r| r.strip_suffix(b"]"))
        {
            let key = std::str::from_utf8(inner).ok().map(str::trim).unwrap_or("");
            let value = Value::Str(PhpStr::from(v.as_str()));
            match Key::from_value(&Value::Str(PhpStr::from(key))) {
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
        None if !entries.is_empty() => Value::Array(Rc::new(entries)),
        None => Value::Bool(false),
    })
}

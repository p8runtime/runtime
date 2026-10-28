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
    Builtin { name: "getopt", run: getopt, by_ref: &[2] },
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
    if let Some(Value::Ref(c)) = args.get(2) {
        *c.borrow_mut() = Value::Int(1);
    }
    // The request's $_SERVER['argv'] (edits to $_SERVER don't reach it),
    // else the global $argv.
    let argv = match ctx.request_argv() {
        Some(a) => Some(Value::Array(Rc::new(PhpArray::from_list(a.iter().map(|s| Value::Str(PhpStr::from(s.as_str()))).collect())))),
        None => ctx.global_value(b"argv"),
    };
    let argv: Vec<Vec<u8>> = match argv {
        Some(Value::Array(a)) => a.iter().map(|(_, v)| v.deref().to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()).collect(),
        _ => return Ok(Value::Bool(false)),
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
    if let Value::Array(longs) = arg(args, 1) {
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
        let key = match Key::from_value(&Value::Str(PhpStr::from(name))) {
            Ok((k @ Key::Int(_), _)) if !(name.len() > 1 && name[0] == b'0') => k,
            _ => Key::Str(PhpStr::from(name)),
        };
        match out.get(&key).cloned() {
            Some(Value::Array(a)) => {
                let mut a = (*a).clone();
                let _ = a.push(value);
                out.insert(key, Value::Array(Rc::new(a)));
            }
            Some(old) => {
                out.insert(key, Value::Array(Rc::new(PhpArray::from_list(vec![old, value]))));
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
                    value = Value::Bool(false);
                } else if o.need == 1 {
                    value = Value::Str(PhpStr::from(argv[optind].as_slice()));
                    optind += 1;
                } else {
                    value = Value::Bool(false);
                }
            } else if byte(a, arg_start) == b'=' {
                value = Value::Str(PhpStr::from(&a[arg_start + 1..]));
                optind += 1;
            } else {
                value = Value::Str(PhpStr::from(&a[arg_start..]));
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
            add(&name, Value::Bool(false));
        }
    }
    if let Some(Value::Ref(c)) = args.get(2) {
        *c.borrow_mut() = Value::Int(optind as i64);
    }
    Ok(Value::Array(Rc::new(out)))
}

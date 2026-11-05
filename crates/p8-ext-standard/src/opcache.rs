//! Zend OPcache's functions, over the compile cache the host keeps
//! (`Ctx::code_cache`: p8's kept files and file cache). What they report is
//! that cache: its files, hits and misses, and its memory (each file's
//! source and compiled code). OPcache runs as PHP 8.6 starts it: on with
//! opcache.enable, in the CLI only with opcache.enable_cli too (off by
//! default, so opcache_get_status() is false there, as in PHP). p8 has no
//! JIT: it is reported as PHP reports a disabled one, and the jit settings
//! are kept and shown but change nothing.
//!
//! Reference: $PHP_SRC_PATH/ext/opcache/zend_accelerator_module.c

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::opcache::{accelerator_enabled as enabled, ini_on, ini_text, max_cached_keys};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "opcache_reset", run: opcache_reset, by_ref: &[] },
    Builtin { name: "opcache_get_status", run: opcache_get_status, by_ref: &[] },
    Builtin { name: "opcache_compile_file", run: opcache_compile_file, by_ref: &[] },
    Builtin { name: "opcache_invalidate", run: opcache_invalidate, by_ref: &[] },
    Builtin { name: "opcache_jit_blacklist", run: opcache_jit_blacklist, by_ref: &[] },
    Builtin { name: "opcache_get_configuration", run: opcache_get_configuration, by_ref: &[] },
    Builtin { name: "opcache_is_script_cached", run: opcache_is_script_cached, by_ref: &[] },
    Builtin { name: "opcache_is_script_cached_in_file_cache", run: opcache_is_script_cached_in_file_cache, by_ref: &[] },
];

type R = Result<Value, Throw>;

fn key(k: &str) -> Key {
    Key::from_string(PhpStr::from(k))
}

fn assoc(entries: Vec<(&str, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in entries {
        a.insert(key(k), v);
    }
    Value::Array(Rc::new(a))
}

/// validate_api_restriction: with opcache.restrict_api set, only a script
/// whose path starts with it may use the API.
fn api_allowed(ctx: &mut dyn Ctx) -> bool {
    let restrict = ini_text(ctx, "opcache.restrict_api");
    if restrict.is_empty() {
        return true;
    }
    let script = ctx.included_files().into_iter().next().unwrap_or_default();
    if script.starts_with(&restrict) {
        return true;
    }
    ctx.warn("Zend OPcache API is restricted by \"restrict_api\" configuration directive");
    false
}

fn opcache_reset(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    if !api_allowed(ctx) || !enabled(ctx) {
        return Ok(Value::Bool(false));
    }
    if let Some(cache) = ctx.code_cache() {
        cache.reset();
    }
    Ok(Value::Bool(true))
}

fn opcache_invalidate(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let path = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let force = args.len() > 1 && arg(args, 1).to_bool();
    if !api_allowed(ctx) || !enabled(ctx) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(ctx.code_cache().is_some_and(|c| c.invalidate(&path, force))))
}

fn opcache_compile_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let path = str_arg(args, 0);
    if !ctx.code_cache().is_some_and(|c| c.started()) {
        ctx.notice("Zend OPcache has not been properly started, can't compile file");
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(ctx.compile_file(path.as_bytes())?))
}

fn opcache_is_script_cached(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let path = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    if !api_allowed(ctx) || !enabled(ctx) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(ctx.code_cache().is_some_and(|c| c.is_cached(&path))))
}

fn opcache_is_script_cached_in_file_cache(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let path = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    if !api_allowed(ctx) {
        return Ok(Value::Bool(false));
    }
    if !(enabled(ctx) || ini_on(ctx, "opcache.file_cache_only")) || ini_text(ctx, "opcache.file_cache").is_empty() {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(ctx.code_cache().is_some_and(|c| c.in_file_cache(&path))))
}

/// p8 has no JIT: there is nothing to keep a function out of.
fn opcache_jit_blacklist(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match arg(args, 0) {
        Value::Object(o) if o.class.lower == b"closure" => Ok(Value::Null),
        other => Err(ctx.throw(
            "TypeError",
            format!("opcache_jit_blacklist(): Argument #1 ($closure) must be of type Closure, {} given", other.type_name()),
        )),
    }
}


/// asctime(localtime(t)) without its newline: "Tue Nov  4 16:21:34 2026".
fn asctime(t: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(t, 0).single().map_or_else(String::new, |d| d.format("%a %b %e %H:%M:%S %Y").to_string())
}

fn opcache_get_status(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let scripts = args.is_empty() || arg(args, 0).to_bool();
    if !api_allowed(ctx) {
        return Ok(Value::Bool(false));
    }
    let Some(cache) = ctx.code_cache().filter(|c| c.started()) else { return Ok(Value::Bool(false)) };
    let on = enabled(ctx);
    let mut out = vec![("opcache_enabled", Value::Bool(on))];
    let file_cache = ini_text(ctx, "opcache.file_cache");
    if !file_cache.is_empty() {
        out.push(("file_cache", Value::str(file_cache.as_str())));
    }
    if ini_on(ctx, "opcache.file_cache_only") {
        out.push(("file_cache_only", Value::Bool(true)));
        return Ok(assoc(out));
    }
    let status = cache.status(scripts && on);
    let consumption = p8_runtime::ini::quantity(ini_text(ctx, "opcache.memory_consumption").as_bytes()).saturating_mul(1 << 20);
    let used = status.used_memory as i64;
    out.extend([
        ("cache_full", Value::Bool(false)),
        ("restart_pending", Value::Bool(status.restart_pending)),
        ("restart_in_progress", Value::Bool(false)),
        (
            "memory_usage",
            assoc(vec![
                ("used_memory", Value::Int(used)),
                ("free_memory", Value::Int((consumption - used).max(0))),
                ("wasted_memory", Value::Int(0)),
                ("current_wasted_percentage", Value::Float(0.0)),
            ]),
        ),
    ]);
    let requests = status.hits + status.misses + status.blacklist_misses;
    let ratio = |n: u64| if requests == 0 { 0.0 } else { n as f64 / requests as f64 * 100.0 };
    let max_files = p8_runtime::ini::quantity(ini_text(ctx, "opcache.max_accelerated_files").as_bytes());
    out.push((
        "opcache_statistics",
        assoc(vec![
            ("num_cached_scripts", Value::Int(status.num_cached_scripts as i64)),
            ("num_cached_keys", Value::Int(status.num_cached_scripts as i64)),
            ("max_cached_keys", Value::Int(max_cached_keys(max_files))),
            ("hits", Value::Int(status.hits as i64)),
            ("start_time", Value::Int(status.start_time)),
            ("last_restart_time", Value::Int(status.last_restart_time)),
            ("oom_restarts", Value::Int(0)),
            ("hash_restarts", Value::Int(0)),
            ("manual_restarts", Value::Int(status.manual_restarts as i64)),
            ("misses", Value::Int(status.misses as i64)),
            ("blacklist_misses", Value::Int(status.blacklist_misses as i64)),
            ("blacklist_miss_ratio", Value::Float(ratio(status.blacklist_misses))),
            ("opcache_hit_rate", Value::Float(ratio(status.hits))),
        ]),
    ));
    if scripts && on {
        let mut list = PhpArray::new();
        let mut entries = status.scripts;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        for s in entries {
            let mut e = vec![
                ("full_path", Value::str(s.path.as_str())),
                ("hits", Value::Int(s.hits as i64)),
                ("memory_consumption", Value::Int(s.memory as i64)),
                ("last_used", Value::str(asctime(s.last_used).as_str())),
                ("last_used_timestamp", Value::Int(s.last_used)),
            ];
            if let Some(t) = s.timestamp {
                e.push(("timestamp", Value::Int(t)));
            }
            e.push(("revalidate", Value::Int(s.revalidate)));
            list.insert(key(&s.path), assoc(e));
        }
        out.push(("scripts", Value::Array(Rc::new(list))));
    }
    // zend_jit_status with the JIT off.
    out.push((
        "jit",
        assoc(vec![
            ("enabled", Value::Bool(false)),
            ("on", Value::Bool(false)),
            ("kind", Value::Int(0)),
            ("opt_level", Value::Int(0)),
            ("opt_flags", Value::Int(0)),
            ("buffer_size", Value::Int(0)),
            ("buffer_free", Value::Int(0)),
        ]),
    ));
    Ok(assoc(out))
}

/// A directive as opcache_get_configuration() shows it: its type as real
/// PHP gives it (OPCACHE_DIRECTIVE_TYPES), memory_consumption in bytes
/// (it is set in megabytes), max_wasted_percentage as a fraction.
fn directive(ctx: &dyn Ctx, name: &str, kind: char) -> Value {
    let text = ini_text(ctx, name);
    match (name, kind) {
        ("opcache.memory_consumption", _) => Value::Int(p8_runtime::ini::quantity(text.as_bytes()).saturating_mul(1 << 20)),
        ("opcache.max_wasted_percentage", _) => Value::Float(text.trim().parse::<f64>().unwrap_or(0.0) / 100.0),
        (_, 'b') => Value::Bool(matches!(text.trim().to_ascii_lowercase().as_str(), "1" | "on" | "yes" | "true")),
        (_, 'i') => Value::Int(p8_runtime::ini::quantity(text.as_bytes())),
        (_, 'f') => Value::Float(text.trim().parse::<f64>().unwrap_or(0.0)),
        _ => Value::str(text.as_str()),
    }
}

fn opcache_get_configuration(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    if !api_allowed(ctx) {
        return Ok(Value::Bool(false));
    }
    let mut directives = PhpArray::new();
    for (name, kind) in crate::defaults::OPCACHE_DIRECTIVE_TYPES {
        directives.insert(key(name), directive(ctx, name, *kind));
    }
    let blacklist: Vec<Value> = ctx.code_cache().map(|c| c.blacklist()).unwrap_or_default().iter().map(|p| Value::str(p.as_str())).collect();
    Ok(assoc(vec![
        ("directives", Value::Array(Rc::new(directives))),
        ("version", assoc(vec![("version", Value::str(crate::PHP_VERSION)), ("opcache_product_name", Value::str("Zend OPcache"))])),
        ("blacklist", Value::Array(Rc::new(PhpArray::from_list(blacklist)))),
    ]))
}

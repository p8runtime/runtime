#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's standard extension: PHP 8.4's constants, and the builtin functions
//! ported so far (the rest move here module by module, each gated by its
//! php-src directories).

use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Ob, Builtin, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

mod array;
mod array2;
mod range;
mod callback;
mod context;
mod dates;
mod datetime;
mod file;
mod filter;
mod hash;
mod hash_algos;
mod hash_tables;
mod image;
mod info;
mod random;
mod scan;
mod spl;
mod spl_iter;
mod spl_dir;
mod json;
mod mail;
mod serial;
pub use serial::{Serializer, Unserializer};
mod class;
pub mod defaults;
mod math;
mod head;
pub use head::{cookie_date, header_line};
mod misc;
mod net;
mod password;
mod phpinfo;
pub(crate) mod printf;
mod proc;
mod process;
mod csv;
mod ctype;
mod string;
mod string2;
pub mod server;
mod var;
mod weak;
mod zlib;

static BUILTINS: &[Builtin] = &[
    Builtin { name: "strlen", run: strlen, by_ref: &[] },
    Builtin { name: "str_repeat", run: str_repeat, by_ref: &[] },
    Builtin { name: "count", run: count, by_ref: &[] },
    Builtin { name: "dechex", run: dechex, by_ref: &[] },
    Builtin { name: "hrtime", run: hrtime, by_ref: &[] },
    Builtin { name: "microtime", run: microtime, by_ref: &[] },
    Builtin { name: "number_format", run: number_format, by_ref: &[] },
    Builtin { name: "printf", run: printf, by_ref: &[] },
    Builtin { name: "sprintf", run: sprintf, by_ref: &[] },
    Builtin { name: "ob_start", run: ob_start, by_ref: &[] },
    Builtin { name: "ob_end_clean", run: ob_end_clean, by_ref: &[] },
    Builtin { name: "ob_end_flush", run: ob_end_flush, by_ref: &[] },
    Builtin { name: "ob_clean", run: ob_clean, by_ref: &[] },
    Builtin { name: "ob_flush", run: ob_flush, by_ref: &[] },
    Builtin { name: "ob_get_contents", run: ob_get_contents, by_ref: &[] },
    Builtin { name: "ob_get_length", run: ob_get_length, by_ref: &[] },
    Builtin { name: "ob_get_level", run: ob_get_level, by_ref: &[] },
    Builtin { name: "ob_get_clean", run: ob_get_clean, by_ref: &[] },
    Builtin { name: "ob_get_flush", run: ob_get_flush, by_ref: &[] },
    Builtin { name: "ob_list_handlers", run: ob_list_handlers, by_ref: &[] },
    Builtin { name: "ob_get_status", run: ob_get_status, by_ref: &[] },
    Builtin { name: "ob_implicit_flush", run: ob_implicit_flush, by_ref: &[] },
    Builtin { name: "flush", run: flush, by_ref: &[] },
    Builtin { name: "function_exists", run: function_exists, by_ref: &[] },
    Builtin { name: "extension_loaded", run: extension_loaded, by_ref: &[] },
    Builtin { name: "get_loaded_extensions", run: get_loaded_extensions, by_ref: &[] },
    Builtin { name: "define", run: define, by_ref: &[] },
    Builtin { name: "defined", run: defined, by_ref: &[] },
    Builtin { name: "func_get_args", run: func_get_args, by_ref: &[] },
    Builtin { name: "array_keys", run: array_keys, by_ref: &[] },
    Builtin { name: "range", run: range::range, by_ref: &[] },
];

/// The standard extension: its functions and PHP 8.4's constants (the
/// table examples/gen_defaults.php generates from real PHP).
pub const EXTENSION: Extension = Extension { name: "standard", register };

/// The PHP release p8 reports (PHP_VERSION).
pub const PHP_VERSION: &str = "8.4.27";

/// p8's own version (P8_VERSION, php2 -v, X-Powered-By, phpinfo()): the
/// release p8 is built as, from Cargo.
pub const P8_VERSION: &str = env!("CARGO_PKG_VERSION");

/// X-Powered-By's value: PHP's product token, then p8's (as `Server:` names
/// more than one), so what matches `PHP/%s` still does.
pub fn powered_by() -> String {
    format!("PHP/{PHP_VERSION} p8/{P8_VERSION}")
}

/// Whether this is a debug build (ZEND_DEBUG): PHP_DEBUG, ZEND_DEBUG_BUILD
/// and phpinfo()'s "Debug Build" all report it.
pub const DEBUG_BUILD: bool = cfg!(debug_assertions);

/// The constants examples/gen_defaults.php leaves out because they describe
/// the build and the host rather than PHP: here, p8's own. The version is
/// the release whose php-src suite p8 is measured against.
const HOST_CONSTANTS: &[(&str, Constant)] = &[
    ("PHP_VERSION", Constant::Str(PHP_VERSION.as_bytes())),
    ("P8_VERSION", Constant::Str(P8_VERSION.as_bytes())),
    ("PHP_VERSION_ID", Constant::Int(80427)),
    ("PHP_MAJOR_VERSION", Constant::Int(8)),
    ("PHP_MINOR_VERSION", Constant::Int(4)),
    ("PHP_RELEASE_VERSION", Constant::Int(27)),
    ("PHP_EXTRA_VERSION", Constant::Str(b"")),
    ("PHP_OS", Constant::Str(if cfg!(target_os = "macos") { b"Darwin" } else { b"Linux" })),
    ("PHP_OS_FAMILY", Constant::Str(if cfg!(target_os = "macos") { b"Darwin" } else { b"Linux" })),
    ("PHP_SAPI", Constant::Str(b"cli")),
    ("PHP_EOL", Constant::Str(b"\n")),
    ("DIRECTORY_SEPARATOR", Constant::Str(b"/")),
    ("PATH_SEPARATOR", Constant::Str(b":")),
    ("PHP_MAXPATHLEN", Constant::Int(p8_runtime::stream::MAXPATHLEN as i64)),
    ("PHP_FD_SETSIZE", Constant::Int(1024)),
    ("PHP_SHLIB_SUFFIX", Constant::Str(b"so")),
    ("PHP_ZTS", Constant::Bool(false)),
    ("PHP_DEBUG", Constant::Bool(DEBUG_BUILD)),
    ("ZEND_THREAD_SAFE", Constant::Bool(false)),
    ("ZEND_DEBUG_BUILD", Constant::Bool(DEBUG_BUILD)),
];

fn register(registry: &mut Registry) {
    registry.set_error_logger(misc::log_err);
    for (name, value) in HOST_CONSTANTS {
        registry.add_constant(name.as_bytes(), *value);
    }
    for (_, name, default) in defaults::INI_DEFAULTS {
        registry.add_ini(name, *default);
    }
    registry.add_functions(BUILTINS);
    registry.add_functions(array::BUILTINS);
    // basic_functions.stub.php: `@prefer-ref $array`, `@prefer-ref $rest`.
    registry.mark_prefer_ref(&["array_multisort"]);
    registry.add_functions(callback::BUILTINS);
    registry.add_functions(ctype::BUILTINS);
    registry.add_functions(zlib::BUILTINS);
    registry.add_classes(zlib::CLASSES);
    registry.add_classes(file::CLASSES);
    registry.add_data("DeflateContext", zlib::make_deflate);
    registry.add_data("InflateContext", zlib::make_inflate);
    for (name, value) in zlib::CONSTANTS {
        registry.add_constant(name.as_bytes(), *value);
    }
    // The PHP modules this extension provides, in PHP's order.
    for module in ["Core", "date", "filter", "hash", "json", "random", "Reflection", "SPL", "standard", "ctype", "zlib"] {
        registry.add_module(module);
    }
    registry.add_functions(file::BUILTINS);
    // The streams a user-space wrapper opened that are still open close
    // when the request ends (stream_close), as PHP frees its resources.
    registry.add_request_end(p8_runtime::user_stream::close_open);
    registry.add_functions(proc::BUILTINS);
    registry.mark_reenters(file::REENTER);
    registry.add_functions(json::BUILTINS);
    registry.add_functions(hash::BUILTINS);
    registry.add_classes(hash::CLASSES);
    registry.add_functions(info::BUILTINS);
    registry.add_functions(string2::BUILTINS);
    registry.add_functions(random::BUILTINS);
    registry.add_functions(scan::BUILTINS);
    registry.add_functions(dates::BUILTINS);
    registry.add_functions(datetime::BUILTINS);
    registry.add_functions(spl::BUILTINS);
    registry.add_classes(spl::CLASSES);
    registry.add_classes(spl_iter::CLASSES);
    registry.add_hooks("IteratorIterator", spl_iter::DUAL_HOOKS);
    registry.add_hooks("RecursiveIteratorIterator", spl_iter::RECURSIVE_HOOKS);
    registry.add_classes(spl_dir::CLASSES);
    registry.add_data("SplFileInfo", spl_dir::make);
    registry.add_classes(weak::CLASSES);
    registry.add_data("WeakReference", weak::make_weakref);
    registry.add_data("WeakMap", weak::make_weakmap);
    registry.add_hooks("WeakMap", weak::WEAKMAP_HOOKS);
    registry.add_data("InternalIterator", weak::make_internal);
    registry.mark_reenters(spl::REENTER);
    registry.add_classes(datetime::CLASSES);
    registry.add_functions(serial::BUILTINS);
    registry.add_functions(array2::BUILTINS);
    registry.add_functions(class::BUILTINS);
    registry.add_functions(math::BUILTINS);
    registry.add_functions(head::BUILTINS);
    registry.add_functions(misc::BUILTINS);
    registry.add_functions(phpinfo::BUILTINS);
    registry.mark_reenters(phpinfo::REENTER);
    registry.add_functions(password::BUILTINS);
    registry.add_functions(context::BUILTINS);
    registry.add_functions(process::BUILTINS);
    registry.add_functions(mail::BUILTINS);
    registry.add_functions(net::BUILTINS);
    registry.add_functions(filter::BUILTINS);
    registry.add_functions(image::BUILTINS);
    registry.mark_reenters(filter::REENTERING);
    registry.add_functions(csv::BUILTINS);
    // The binary running this script (PHP_BINARY), for tests that run PHP again.
    let binary = std::env::current_exe().map(|p| p.into_os_string().into_encoded_bytes()).unwrap_or_default();
    registry.add_constant(b"PHP_BINARY", Constant::Str(Box::leak(binary.into_boxed_slice())));
    registry.add_functions(string::BUILTINS);
    registry.add_functions(var::BUILTINS);
    // These reach the caller's variables or run PHP code (__serialize,
    // jsonSerialize) though they take no callable.
    registry.mark_reenters(array2::REENTER);
    registry.mark_reenters(info::REENTER);
    registry.mark_reenters(callback::REENTER);
    registry.unchecked(&["assert"]);
    // (count: a Countable's count() is PHP code.)
    // (The ob_* functions run output handlers, PHP code.)
    registry.mark_reenters(&["ob_end_clean", "ob_end_flush", "ob_clean", "ob_flush", "ob_get_clean", "ob_get_flush"]);
    // (The printf family calls __toString on the values a v* array holds.)
    registry.mark_reenters(&["vsprintf", "vprintf", "vfprintf"]);
    registry.mark_reenters(class::REENTER);
    registry.mark_reenters(&["implode", "join", "getopt", "assert", "gc_collect_cycles", "is_callable", "defined", "func_get_args", "serialize", "unserialize", "json_encode", "get_object_vars", "var_export", "count", "sizeof"]);
    for (module, name, value, _) in defaults::CONSTANTS {
        let value = match value {
            defaults::ConstValue::Int(i) => Constant::Int(*i),
            defaults::ConstValue::Float(bits) => Constant::Float(f64::from_bits(*bits)),
            defaults::ConstValue::Str(s) => Constant::Str(s),
            defaults::ConstValue::Bool(b) => Constant::Bool(*b),
            defaults::ConstValue::Null => Constant::Null,
        };
        registry.add_constant_in(module, name, value);
    }
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

fn str_arg(args: &[Value], i: usize) -> PhpStr {
    arg(args, i).to_php_string().unwrap_or_else(PhpStr::empty)
}

fn strlen(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(str_arg(args, 0).len() as i64))
}

fn str_repeat(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let n = arg(args, 1).to_int().max(0) as usize;
    Ok(Value::Str(PhpStr::new(s.as_bytes().repeat(n))))
}

fn count(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mode = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    Ok(Value::Int(match arg(args, 0) {
        Value::Array(a) if mode == 1 => count_recursive(&a),
        Value::Array(a) => a.len() as i64,
        // A Countable: its count().
        Value::Object(o) if o.class.is_a(b"countable") => {
            let callable = Value::Array(std::rc::Rc::new(PhpArray::from_list(vec![Value::Object(o), Value::str("count")])));
            ctx.call(&callable, &mut [])?.to_int()
        }
        _ => 1,
    }))
}

/// count($a, COUNT_RECURSIVE)
fn count_recursive(a: &PhpArray) -> i64 {
    a.values().map(|v| match v.deref() {
        Value::Array(inner) => 1 + count_recursive(&inner),
        _ => 1,
    }).sum()
}

fn dechex(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut digits = Vec::with_capacity(16);
    p8_runtime::numeric::push_digits(&mut digits, arg(args, 0).to_int() as u64, 16, false);
    Ok(Value::Str(PhpStr::new(digits)))
}

fn hrtime(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    thread_local!(static START: std::time::Instant = std::time::Instant::now());
    let ns = START.with(|s| s.elapsed().as_nanos()) as i64;
    if arg(args, 0).to_bool() {
        return Ok(Value::Int(ns));
    }
    Ok(Value::Array(Rc::new(PhpArray::from_list(vec![
        Value::Int(ns / 1_000_000_000),
        Value::Int(ns % 1_000_000_000),
    ]))))
}

fn microtime(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    if arg(args, 0).to_bool() {
        return Ok(Value::Float(now.as_secs_f64()));
    }
    Ok(Value::str(format!("{:.8} {}", now.subsec_nanos() as f64 / 1e9, now.as_secs())))
}

/// number_format(float $num, int $decimals = 0, ?string $decimal_separator = ".", ?string $thousands_separator = ",")
fn number_format(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let num = arg(args, 0).to_float();
    let decimals = arg(args, 1).to_int().max(0) as usize;
    // ?string $decimal_separator = ".", ?string $thousands_separator = ",":
    // null (or not given) is the default (math.c, PHP_FUNCTION(number_format)).
    let given = |args: &[Value], i: usize| args.get(i).is_some_and(|v| !matches!(v.deref(), Value::Null));
    let point = if given(args, 2) { str_arg(args, 2).as_bytes().to_vec() } else { b".".to_vec() };
    let sep = if given(args, 3) { str_arg(args, 3).as_bytes().to_vec() } else { b",".to_vec() };
    // _php_math_round: round half away from zero at `decimals` places
    let factor = 10f64.powi(decimals as i32);
    let rounded = (num.abs() * factor).round() / factor;
    let text = format!("{:.*}", decimals, rounded);
    let (int_part, frac) = text.split_once('.').unwrap_or((&text, ""));
    let mut out = Vec::new();
    if num < 0.0 && rounded != 0.0 {
        out.push(b'-');
    }
    for (i, d) in int_part.bytes().enumerate() {
        if i > 0 && (int_part.len() - i) % 3 == 0 {
            out.extend_from_slice(&sep);
        }
        out.push(d);
    }
    if decimals > 0 {
        out.extend_from_slice(&point);
        out.extend_from_slice(frac.as_bytes());
    }
    Ok(Value::Str(PhpStr::new(out)))
}

fn printf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let out = printf::formatted(ctx, "printf", str_arg(args, 0).as_bytes(), &args[1..], printf::Missing::Count(1))?;
    ctx.echo(&out);
    Ok(Value::Int(out.len() as i64))
}

fn sprintf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let out = printf::formatted(ctx, "sprintf", str_arg(args, 0).as_bytes(), &args[1..], printf::Missing::Count(1))?;
    Ok(Value::Str(PhpStr::new(out)))
}

/// ob_start($callback = null, $chunk_size = 0, $flags = PHP_OUTPUT_HANDLER_STDFLAGS)
fn ob_start(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let handler = arg(args, 0);
    let chunk = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    ctx.ob(Ob::Start { handler, chunk })
}

/// An ob_* operation that needs a buffer: PHP's notice when there is none.
fn ob_or_notice(ctx: &mut dyn Ctx, op: Ob, notice: &str) -> Result<Value, Throw> {
    let r = ctx.ob(op)?;
    if matches!(r, Value::Bool(false)) {
        ctx.notice(notice);
    }
    Ok(r)
}

fn ob_end_clean(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ob_or_notice(ctx, Ob::End { flush: false }, "ob_end_clean(): Failed to delete buffer. No buffer to delete")
}

fn ob_end_flush(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ob_or_notice(ctx, Ob::End { flush: true }, "ob_end_flush(): Failed to delete and flush buffer. No buffer to delete or flush")
}

fn ob_clean(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ob_or_notice(ctx, Ob::Clean, "ob_clean(): Failed to delete buffer. No buffer to delete")
}

fn ob_flush(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ob_or_notice(ctx, Ob::Flush, "ob_flush(): Failed to flush buffer. No buffer to flush")
}

fn ob_get_contents(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.ob(Ob::Contents)
}

fn ob_get_length(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.ob(Ob::Length)
}

fn ob_get_level(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.ob(Ob::Level)
}

/// ob_get_clean(): the contents, and the buffer removed (false without one).
fn ob_get_clean(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let contents = ctx.ob(Ob::Contents)?;
    if matches!(contents, Value::Bool(false)) {
        return Ok(contents);
    }
    ctx.ob(Ob::End { flush: false })?;
    Ok(contents)
}

/// ob_get_flush(): the contents, and the buffer flushed and removed.
fn ob_get_flush(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let contents = ctx.ob(Ob::Contents)?;
    if matches!(contents, Value::Bool(false)) {
        ctx.notice("ob_get_flush(): Failed to delete and flush buffer. No buffer to delete or flush");
        return Ok(contents);
    }
    ctx.ob(Ob::End { flush: true })?;
    Ok(contents)
}

fn ob_list_handlers(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.ob(Ob::Handlers)
}

fn ob_get_status(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctx.ob(Ob::Status { full: arg(args, 0).to_bool() })
}

fn ob_implicit_flush(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

fn flush(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.flush_output();
    Ok(Value::Null)
}

fn function_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(ctx.function_exists(str_arg(args, 0).as_bytes())))
}

/// extension_loaded(string $extension): any case.
fn extension_loaded(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    Ok(Value::Bool(ctx.modules().iter().any(|m| m.as_bytes().eq_ignore_ascii_case(name.as_bytes()))))
}

/// get_loaded_extensions(bool $zend_extensions = false): p8 has no Zend
/// extensions (no OPcache).
fn get_loaded_extensions(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let zend = args.first().is_some_and(|v| v.to_bool());
    let names = if zend { Vec::new() } else { ctx.modules().iter().map(|m| Value::str(*m)).collect() };
    Ok(Value::Array(std::rc::Rc::new(PhpArray::from_list(names))))
}

fn define(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    if !ctx.define(name.as_bytes(), arg(args, 1)) {
        ctx.warn(&format!("Constant {} already defined", String::from_utf8_lossy(name.as_bytes())));
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

/// defined($name): a constant, or a class constant (`A::B`, the class
/// autoloaded) the running code can see.
fn defined(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    let text = name.as_bytes();
    if let Some(at) = text.windows(2).position(|w| w == b"::") {
        return class::class_constant_defined(ctx, &text[..at], &text[at + 2..]).map(Value::Bool);
    }
    Ok(Value::Bool(ctx.constant(text).is_some()))
}


fn func_get_args(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Array(Rc::new(PhpArray::from_list(ctx.call_args()))))
}

/// range() for int bounds and step (the float and character forms come with the port).
/// array_keys($array, $filter_value, $strict = false): with a filter value
/// (even null), only the keys whose values match it.
fn array_keys(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let filter = (args.len() > 1).then(|| (arg(args, 1), args.len() > 2 && arg(args, 2).to_bool()));
    Ok(match arg(args, 0) {
        Value::Array(a) => {
            // Sized once: every key when there is no filter.
            let mut keys = Vec::with_capacity(if filter.is_none() { a.len() } else { 0 });
            keys.extend(a.iter().filter(|(_, v)| filter.as_ref().is_none_or(|(needle, strict)| array::matches(needle, &v.deref(), *strict))).map(|(k, _)| k.to_value()));
            Value::Array(Rc::new(PhpArray::from_list(keys)))
        }
        _ => Value::Null,
    })
}


#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's standard extension: PHP 8.4's constants, and the builtin functions
//! ported so far (the rest move here module by module, each gated by its
//! php-src directories).

use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Ob, Builtin, Constant, Ctx, Extension, Fault, FastBuiltin, OrFault, Registry, Throw};
use p8_runtime::lent;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

mod array;
mod array2;
mod opcache;
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

/// Builtins on the borrowed ABI ([`p8_runtime::ext::Fast`]).
static FAST: &[FastBuiltin] = &[
    FastBuiltin { name: "strlen", run: strlen },
    FastBuiltin { name: "count", run: count },
    FastBuiltin { name: "str_repeat", run: str_repeat },
    FastBuiltin { name: "dechex", run: dechex },
    FastBuiltin { name: "hrtime", run: hrtime },
    FastBuiltin { name: "microtime", run: microtime },
    FastBuiltin { name: "sprintf", run: sprintf },
    FastBuiltin { name: "function_exists", run: function_exists },
    FastBuiltin { name: "array_keys", run: array_keys },
];

static BUILTINS: &[Builtin] = &[
    Builtin { name: "number_format", run: number_format, by_ref: &[] },
    Builtin { name: "printf", run: printf, by_ref: &[] },
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
    Builtin { name: "extension_loaded", run: extension_loaded, by_ref: &[] },
    Builtin { name: "get_loaded_extensions", run: get_loaded_extensions, by_ref: &[] },
    Builtin { name: "define", run: define, by_ref: &[] },
    Builtin { name: "defined", run: defined, by_ref: &[] },
    Builtin { name: "func_get_args", run: func_get_args, by_ref: &[] },
    Builtin { name: "range", run: range::range, by_ref: &[] },
];

/// The standard extension: its functions and PHP 8.4's constants (the
/// table examples/gen_defaults.php generates from real PHP).
pub const EXTENSION: Extension = Extension { name: "standard", register };

/// The PHP release p8 reports (PHP_VERSION) and the numbers derived from
/// it: defined once in p8-stubs, next to the extension table that reports
/// the version too.
pub use p8_runtime::stubs::{PHP_MAJOR_VERSION, PHP_MINOR_VERSION, PHP_RELEASE_VERSION, PHP_VERSION, PHP_VERSION_ID};

/// The Zend Engine's version for this PHP: 4.<minor>.<release> (PHP 8.x).
pub fn zend_engine_version() -> String {
    format!("4.{PHP_MINOR_VERSION}.{PHP_RELEASE_VERSION}")
}

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
    ("PHP_VERSION_ID", Constant::Int(PHP_VERSION_ID)),
    ("PHP_MAJOR_VERSION", Constant::Int(PHP_MAJOR_VERSION)),
    ("PHP_MINOR_VERSION", Constant::Int(PHP_MINOR_VERSION)),
    ("PHP_RELEASE_VERSION", Constant::Int(PHP_RELEASE_VERSION)),
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
    registry.add_fast(FAST);
    registry.add_functions(array::BUILTINS);
    registry.add_fast(array::FAST);
    registry.add_classes(array::CLASSES);
    registry.add_classes(math::CLASSES);
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
    // The PHP modules this extension provides, in PHP's order. Zend OPcache
    // is built into PHP since 8.5; its functions act on the host's
    // compile cache (opcache.rs).
    for module in ["Core", "date", "filter", "hash", "json", "Zend OPcache", "random", "Reflection", "SPL", "standard", "ctype", "zlib"] {
        registry.add_module(module);
    }
    registry.add_functions(opcache::BUILTINS);
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
    registry.add_fast(string2::FAST);
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
    registry.add_fast(class::FAST);
    registry.add_functions(math::BUILTINS);
    registry.add_fast(math::FAST);
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
    // PHP_BUILD_DATE (8.5+): what phpinfo() shows as "Build Date". (PHP
    // defines PHP_BUILD_PROVIDER only when the build names one; p8's doesn't.)
    let built = phpinfo::build_date().into_bytes();
    registry.add_constant(b"PHP_BUILD_DATE", Constant::Str(Box::leak(built.into_boxed_slice())));
    registry.add_functions(string::BUILTINS);
    registry.add_fast(string::FAST);
    registry.add_functions(var::BUILTINS);
    registry.add_fast(var::FAST);
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
    registry.mark_reenters(&["clone", "implode", "join", "getopt", "assert", "gc_collect_cycles", "is_callable", "defined", "func_get_args", "serialize", "unserialize", "json_encode", "get_object_vars", "var_export"]);
    // (assert runs PHP code only for an assertion that fails.)
    registry.quick("assert", scan::assert_holds);
    // (count: a Countable's count() is PHP code; on an array it is not,
    // so a call with no object argument may still be frameless.)
    registry.mark_reenters_on_objects(&["count", "sizeof"]);
    // Only an object's class, id or kind is looked at: frameless with one.
    registry.objects_inert(&[
        "get_class", "spl_object_id", "spl_object_hash", "is_object", "is_null", "is_bool", "is_int", "is_integer", "is_long", "is_float",
        "is_double", "is_string", "is_array", "is_scalar", "is_numeric", "is_iterable", "is_countable", "is_resource", "gettype",
        "get_debug_type",
    ]);
    for (module, name, value, deprecation) in defaults::CONSTANTS {
        let value = match value {
            defaults::ConstValue::Int(i) => Constant::Int(*i),
            defaults::ConstValue::Float(bits) => Constant::Float(f64::from_bits(*bits)),
            defaults::ConstValue::Str(s) => Constant::Str(s),
            defaults::ConstValue::Bool(b) => Constant::Bool(*b),
            defaults::ConstValue::Null => Constant::Null,
        };
        registry.add_constant_in(module, name, value);
        if let Some(message) = deprecation {
            registry.deprecate_constant(name, message);
        }
    }
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

fn str_arg(args: &[Value], i: usize) -> PhpStr {
    arg(args, i).to_php_string().unwrap_or_else(PhpStr::empty)
}

fn strlen(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::int(match lent::str(args, 0) {
        Some(s) => s.len(),
        None => lent::string(args, 0).len(),
    } as i64))
}

fn str_repeat(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let n = lent::int(args, 1).max(0) as usize;
    Ok(Value::str(PhpStr::new(lent::bytes(args, 0).repeat(n))))
}

fn count(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let mode = lent::int(args, 1);
    if let Some(a) = lent::array(args, 0) {
        return Ok(Value::int(if mode == 1 { count_recursive(a) } else { a.len() as i64 }));
    }
    let v = lent::at(args, 0).deref();
    Ok(Value::int(if let Some(a) = v.as_array() {
        if mode == 1 { count_recursive(a) } else { a.len() as i64 }
    } else if let Some(o) = v.as_object().filter(|o| o.class.is_a(b"countable")) {
        // A Countable: its count() (PHP code: count is marked as one
        // that may run it, so it is never frameless on an object).
        let callable = Value::array(PhpArray::from_list(vec![Value::object(o.clone()), Value::str("count")]));
        ctx.call(&callable, &mut []).or_fault(ctx)?.to_int()
    } else {
        1
    }))
}

/// count($a, COUNT_RECURSIVE)
fn count_recursive(a: &PhpArray) -> i64 {
    a.values().map(|v| match v.deref().as_array() {
        Some(inner) => 1 + count_recursive(inner),
        None => 1,
    }).sum()
}

fn dechex(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let mut digits = Vec::with_capacity(16);
    p8_runtime::numeric::push_digits(&mut digits, lent::int(args, 0) as u64, 16, false);
    Ok(Value::str(PhpStr::new(digits)))
}

fn hrtime(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    thread_local!(static START: std::time::Instant = std::time::Instant::now());
    let ns = START.with(|s| s.elapsed().as_nanos()) as i64;
    if lent::bool(args, 0) {
        return Ok(Value::int(ns));
    }
    Ok(Value::array(Rc::new(PhpArray::from_list(vec![
        Value::int(ns / 1_000_000_000),
        Value::int(ns % 1_000_000_000),
    ]))))
}

fn microtime(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    if lent::bool(args, 0) {
        return Ok(Value::float(now.as_secs_f64()));
    }
    Ok(Value::str(format!("{:.8} {}", now.subsec_nanos() as f64 / 1e9, now.as_secs())))
}

/// number_format(float $num, int $decimals = 0, ?string $decimal_separator = ".", ?string $thousands_separator = ",")
fn number_format(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let num = arg(args, 0).to_float();
    let decimals = arg(args, 1).to_int().max(0) as usize;
    // ?string $decimal_separator = ".", ?string $thousands_separator = ",":
    // null (or not given) is the default (math.c, PHP_FUNCTION(number_format)).
    let given = |args: &[Value], i: usize| args.get(i).is_some_and(|v| !v.deref().is_null());
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
    Ok(Value::str(PhpStr::new(out)))
}

fn printf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let out = printf::formatted(ctx, "printf", str_arg(args, 0).as_bytes(), &args[1..], printf::Missing::Count(1))?;
    ctx.echo(&out);
    Ok(Value::int(out.len() as i64))
}

fn sprintf(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let out = printf::formatted(ctx, "sprintf", lent::bytes(args, 0), args.get(1..).unwrap_or_default(), printf::Missing::Count(1)).or_fault(ctx)?;
    Ok(Value::str(PhpStr::new(out)))
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
    if r.as_bool() == Some(false) {
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
    if contents.as_bool() == Some(false) {
        return Ok(contents);
    }
    ctx.ob(Ob::End { flush: false })?;
    Ok(contents)
}

/// ob_get_flush(): the contents, and the buffer flushed and removed.
fn ob_get_flush(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let contents = ctx.ob(Ob::Contents)?;
    if contents.as_bool() == Some(false) {
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
    Ok(Value::NULL)
}

fn flush(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.flush_output();
    Ok(Value::NULL)
}

fn function_exists(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(ctx.function_exists(lent::bytes(args, 0))))
}

/// extension_loaded(string $extension): any case.
fn extension_loaded(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    Ok(Value::bool(ctx.modules().iter().any(|m| m.as_bytes().eq_ignore_ascii_case(name.as_bytes()))))
}

/// get_loaded_extensions(bool $zend_extensions = false): with
/// $zend_extensions, the one Zend extension, OPcache (a module too).
fn get_loaded_extensions(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let zend = args.first().is_some_and(|v| v.to_bool());
    let names = if zend {
        ctx.modules().iter().filter(|m| **m == "Zend OPcache").map(|m| Value::str(*m)).collect()
    } else {
        ctx.modules().iter().map(|m| Value::str(*m)).collect()
    };
    Ok(Value::array(PhpArray::from_list(names)))
}

fn define(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    // ZEND_FUNCTION(define): the third argument is deprecated (8.6), and
    // has been ignored since 8.0.
    if args.len() == 3 {
        ctx.deprecated(
            "define(): Argument #3 ($case_insensitive) is ignored and treated as false since declaration of case-insensitive constants is no longer supported, passing the argument explicitly is unnecessary",
        );
        if arg(args, 2).to_bool() {
            ctx.warn("define(): Argument #3 ($case_insensitive) is ignored since declaration of case-insensitive constants is no longer supported, this will be an error in PHP 9.0");
        }
    }
    // zend_register_constant: "_", "let" and "is" (8.6)
    if let Some((word, _)) = p8_runtime::symbol::deprecated_name(name.as_bytes()) {
        ctx.deprecated(&format!("Calling a constant \"{word}\" is deprecated since 8.6"));
    }
    if !ctx.define(name.as_bytes(), arg(args, 1)) {
        // zend_register_constant (8.5: redeclaring is on its way to an error)
        ctx.warn(&format!("Constant {} already defined, this will be an error in PHP 9", String::from_utf8_lossy(name.as_bytes())));
        return Ok(Value::FALSE);
    }
    Ok(Value::TRUE)
}

/// defined($name): a constant, or a class constant (`A::B`, the class
/// autoloaded) the running code can see.
fn defined(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    let text = name.as_bytes();
    if let Some(at) = text.windows(2).position(|w| w == b"::") {
        return class::class_constant_defined(ctx, &text[..at], &text[at + 2..]).map(Value::bool);
    }
    Ok(Value::bool(ctx.constant(text).is_some()))
}


fn func_get_args(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::array(PhpArray::from_list(ctx.call_args())))
}

/// range() for int bounds and step (the float and character forms come with the port).
/// array_keys($array, $filter_value, $strict = false): with a filter value
/// (even null), only the keys whose values match it.
fn array_keys(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let filter = (args.len() > 1).then(|| (lent::at(args, 1), lent::bool(args, 2)));
    Ok(match lent::array(args, 0) {
        Some(a) => {
            // Sized once: every key when there is no filter.
            let mut keys = Vec::with_capacity(if filter.is_none() { a.len() } else { 0 });
            keys.extend(a.iter().filter(|(_, v)| filter.is_none_or(|(needle, strict)| array::matches(needle, v, strict))).map(|(k, _)| k.to_value()));
            Value::array(PhpArray::from_list(keys))
        }
        None => Value::NULL,
    })
}

#[cfg(test)]
mod tests {
    use p8_runtime::ext::{Imp, Registry};

    fn registry() -> Registry {
        Registry::new(&[super::EXTENSION])
    }

    /// Which call path each builtin may take: frameless only for one with
    /// a signature that cannot run PHP code and is not deprecated; the
    /// borrowed ABI for the migrated ones, the adapter keeping the rest.
    #[test]
    fn frameless_calls_are_the_plain_builtins() {
        let r = registry();
        let call = |name: &str| *r.call(r.lookup(name.as_bytes()).unwrap_or_else(|| panic!("{name} registered")));
        for name in ["strlen", "abs", "in_array", "is_array", "str_contains", "substr", "explode", "sprintf", "count", "get_class"] {
            let c = call(name);
            assert!(matches!(c.frameless, Some(Imp::Lent(_))), "{name}: frameless on the borrowed ABI");
        }
        // Not migrated: frameless through the adapter, its moved ABI.
        for name in ["array_merge", "str_pad", "number_format"] {
            assert!(matches!(call(name).frameless, Some(Imp::Moved(_))), "{name}: frameless on the moved ABI");
        }
        // They call PHP code (a callback, a __toString on an element, an
        // autoloader), or the checked path has a notice to give.
        for name in ["array_map", "usort", "implode", "class_exists", "call_user_func", "is_callable", "array_multisort"] {
            assert!(call(name).frameless.is_none(), "{name}: never frameless");
        }
        // count may run a Countable's count(): lent registers on the
        // checked path, frameless (on no object) still.
        assert!(call("count").reenters && call("count").frameless.is_some());
        assert!(call("get_class").objects_ok && !call("strlen").objects_ok);
    }

    /// The call site's test: plainly typed arguments, the right count, no
    /// reference, no undefined value, an object only where it is inert.
    #[test]
    fn plain_call_takes_only_arguments_needing_no_check() {
        use p8_runtime::value::Value;
        let r = registry();
        let plain = |name: &str, args: &[Value]| r.call(r.lookup(name.as_bytes()).expect("registered")).plain(args);
        assert!(plain("strlen", &[Value::str("abc")]));
        assert!(!plain("strlen", &[Value::int(1)]), "an int to a string parameter is converted");
        assert!(!plain("strlen", &[]), "too few");
        assert!(!plain("strlen", &[Value::str("a"), Value::str("b")]), "too many");
        assert!(!plain("strlen", &[Value::new_reference(Value::str("a"))]), "a reference");
        assert!(!plain("strlen", &[Value::UNDEF]), "undefined");
        assert!(plain("substr", &[Value::str("abc"), Value::int(1)]), "an optional parameter left out");
        assert!(plain("substr", &[Value::str("abc"), Value::int(1), Value::NULL]), "?int takes null");
        assert!(!plain("substr", &[Value::str("abc"), Value::float(1.0)]), "a float to an int parameter is converted");
        assert!(plain("in_array", &[Value::int(1), Value::array(p8_runtime::array::PhpArray::new())]));
        assert!(plain("sprintf", &[Value::str("%d %d"), Value::int(1), Value::int(2)]), "variadic");
        let five = [Value::str("%d%d%d%d"), Value::int(1), Value::int(2), Value::int(3), Value::int(4)];
        assert!(plain("sprintf", &five), "past the precomputed four: from the signature");
        assert!(!plain("sprintf", &[Value::str("%s"), Value::int(1), Value::int(2), Value::int(3), Value::UNDEF]));
        // (Objects: the recorded script builtin_frameless.php.)
        assert!(plain("is_bool", &[Value::FALSE]) && plain("is_bool", &[Value::TRUE]));
    }
}


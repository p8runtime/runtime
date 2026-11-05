//! A module's phpinfo() section in the text form the CLI prints, shared by
//! phpinfo() and ReflectionExtension::info(), and the module facts both
//! report (its version, its ini directives).
//!
//! The sections' rows are PHP's (p8_stubs::ModuleSpec::info, generated
//! from real PHP); a `{source}` in a row is a value p8 computes here:
//!
//! - `const:NAME` / `const:Class::NAME`: a constant's value
//! - `ini:name`: a directive's current value
//! - `call:function/key/key`: what a function returns (or a static method,
//!   `Class::method(int)`), indexed by the keys
//!
//! then a modifier after `|`: `yesno` (Yes / No), `enabled` (enabled, or no
//! row), `endis` (enabled / disabled), `nonempty` (no row for ""),
//! `terms` (each item and a space), `join=SEP`. A source that gives nothing
//! (an undefined constant, a function p8 lacks, false) drops its row.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/info.c (php_info_print_module,
//! php_info_print_table_row), main/php_ini.c (display_ini_entries,
//! php_ini_displayer_cb), Zend/zend_ini.c (the displayers)

use crate::ext::{Ctx, Throw};
use crate::string::PhpStr;
use crate::value::Value;
use p8_stubs::{IniDisplay, ModuleSpec};
use std::rc::Rc;

/// The ext state key set once phpinfo() or ini_get_all() has sorted the
/// directives (zend_ini_sort_entries): from then on a module's directives
/// are listed by name, as PHP's table stays sorted.
pub const INI_SORTED: &str = "info.ini_sorted";

/// The module the running process provides by `name` (any case), as PHP
/// names it.
pub fn loaded_module(ctx: &dyn Ctx, name: &[u8]) -> Option<&'static str> {
    ctx.modules().iter().copied().find(|m| m.as_bytes().eq_ignore_ascii_case(name))
}

/// A loaded module's version (phpversion($extension)), None when it has none.
pub fn module_version(module: &str) -> Option<&'static str> {
    p8_stubs::module_spec(module.as_bytes()).and_then(|s| s.version)
}

/// The directives of `spec` p8 has, in the order PHP lists them: as the
/// module registered them, or by name once they have been sorted.
pub fn ini_entries(ctx: &mut dyn Ctx, spec: &ModuleSpec) -> Vec<(&'static str, u8, IniDisplay)> {
    let mut entries: Vec<_> = spec.ini.iter().copied().filter(|(name, _, _)| ctx.ini_get(name.as_bytes()).is_some()).collect();
    if ctx.state(INI_SORTED).to_bool() {
        // ini_key_compare: zend_binary_strcasecmp.
        entries.sort_by_key(|e| e.0.to_ascii_lowercase());
    }
    entries
}

fn bytes(v: &Value) -> Vec<u8> {
    v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()
}

/// atoi(): the leading number, 0 when there is none.
fn atoi(text: &[u8]) -> i64 {
    let text = String::from_utf8_lossy(text);
    let t = text.trim_start();
    let end = t.char_indices().find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))).map_or(t.len(), |(i, _)| i);
    t[..end].parse().unwrap_or(0)
}

/// zend_ini_parse_bool
fn parse_bool(text: &[u8]) -> bool {
    let lower = text.to_ascii_lowercase();
    matches!(lower.as_slice(), b"true" | b"yes" | b"on") || atoi(text) != 0
}

/// How phpinfo() shows a directive's value (None: PHP has no value).
pub fn shown(ctx: &mut dyn Ctx, display: IniDisplay, value: Option<&[u8]>) -> Vec<u8> {
    match display {
        IniDisplay::Plain => match value {
            Some(v) if !v.is_empty() => v.to_vec(),
            _ => b"no value".to_vec(),
        },
        IniDisplay::Bool => (if value.is_some_and(parse_bool) { "On" } else { "Off" }).into(),
        IniDisplay::Color => match value {
            Some(v) => [b"<span style=\"color: ", v, b"\">", v, b"</span>"].concat(),
            None => b"no value".to_vec(),
        },
        IniDisplay::DisplayErrors => {
            // php_get_display_errors_mode: 1 stdout, 2 stderr, 0 off.
            let mode = match value.map(<[u8]>::to_ascii_lowercase) {
                None => 1,
                Some(v) if matches!(v.as_slice(), b"on" | b"yes" | b"true" | b"stdout") => 1,
                Some(v) if v == b"stderr" => 2,
                Some(v) => match atoi(&v) & 0xff {
                    0 => 0,
                    2 => 2,
                    _ => 1,
                },
            };
            // The CLI says where; another SAPI just On.
            let cli = !ctx.response().server;
            (match mode {
                0 => "Off",
                2 if cli => "STDERR",
                1 if cli => "STDOUT",
                _ => "On",
            })
            .into()
        }
        IniDisplay::LinkNumbers => match value {
            Some(v) if atoi(v) == -1 => b"Unlimited".to_vec(),
            Some(v) => v.to_vec(),
            None => Vec::new(),
        },
    }
}

/// display_ini_entries(): the module's directives with their local and
/// master values, after a blank line; nothing when it has none.
pub fn print_ini_entries(ctx: &mut dyn Ctx, spec: &ModuleSpec) {
    let entries = ini_entries(ctx, spec);
    if entries.is_empty() {
        return;
    }
    ctx.echo(b"\nDirective => Local Value => Master Value\n");
    for (name, _, display) in entries {
        let local = ctx.ini_get(name.as_bytes()).filter(|v| !matches!(v, Value::Null)).map(|v| bytes(&v));
        let master = ctx.ini_master(name.as_bytes()).filter(|v| !matches!(v, Value::Null)).map(|v| bytes(&v));
        let local = shown(ctx, display, local.as_deref());
        let master = shown(ctx, display, master.as_deref());
        ctx.echo(&[name.as_bytes(), b" => ", &local, b" => ", &master, b"\n"].concat());
    }
}

/// php_info_print_module(): the module's title and section (a module PHP
/// knows nothing of is just named, as one without an info function or a
/// version is, under "Additional Modules").
pub fn print_module(ctx: &mut dyn Ctx, module: &str) -> Result<(), Throw> {
    let Some(spec) = p8_stubs::module_spec(module.as_bytes()) else {
        ctx.echo(format!("{module}\n").as_bytes());
        return Ok(());
    };
    ctx.echo(format!("\n{}\n", spec.name).as_bytes());
    for line in spec.info {
        if *line == "{ini}" {
            print_ini_entries(ctx, spec);
        } else if *line == "{accel_info}" {
            accel_info(ctx)?;
        } else if let Some(text) = render(ctx, line)? {
            ctx.echo(&text);
            ctx.echo(b"\n");
        }
    }
    Ok(())
}

/// zend_accel_info(): Zend OPcache's rows, from the compile cache
/// (`Ctx::code_cache`). Two of PHP's rows are not printed: "Shared memory
/// model" (p8's cache is the Platform's memory, not a shared segment) and
/// the interned strings' (p8 keeps no interned string buffer; PHP prints
/// them only with one).
fn accel_info(ctx: &mut dyn Ctx) -> Result<(), Throw> {
    use crate::opcache::{accelerator_enabled, ini_on, ini_text, max_cached_keys};
    let row = |ctx: &mut dyn Ctx, label: &str, value: &str| ctx.echo(format!("{label} => {value}\n").as_bytes());
    let file_cache_only = ini_on(ctx, "opcache.file_cache_only");
    let cache = ctx.code_cache();
    let started = cache.as_ref().is_some_and(|c| c.started());
    let enable = ini_on(ctx, "opcache.enable");
    let caching = if accelerator_enabled(ctx) || file_cache_only { "Up and Running" } else { "Disabled" };
    row(ctx, "Opcode Caching", caching);
    let optimizing = enable && started && crate::ini::quantity(ini_text(ctx, "opcache.optimization_level").as_bytes()) != 0;
    row(ctx, "Optimization", if optimizing { "Enabled" } else { "Disabled" });
    row(ctx, "SHM Cache", if file_cache_only { "Disabled" } else { "Enabled" });
    let file_cache = if ini_text(ctx, "opcache.file_cache").is_empty() { "Disabled" } else { "Enabled" };
    row(ctx, "File Cache", file_cache);
    // JIT_G(enabled) is false: p8 compiles no machine code.
    row(ctx, "JIT", "Disabled");
    let Some(cache) = cache else { return Ok(()) };
    if !file_cache_only && !enable {
        return Ok(());
    }
    if !started {
        // accel_startup(): the one reason p8 does not start it.
        row(ctx, "Startup Failed", "Opcode Caching is disabled for CLI");
        return Ok(());
    }
    row(ctx, "Startup", "OK");
    if file_cache_only {
        return Ok(());
    }
    let status = cache.status(false);
    let consumption = crate::ini::quantity(ini_text(ctx, "opcache.memory_consumption").as_bytes()).saturating_mul(1 << 20);
    let max_keys = max_cached_keys(crate::ini::quantity(ini_text(ctx, "opcache.max_accelerated_files").as_bytes()));
    let used = status.used_memory as i64;
    for (label, value) in [
        ("Cache hits", status.hits as i64),
        ("Cache misses", status.misses as i64),
        ("Used memory", used),
        ("Free memory", (consumption - used).max(0)),
        ("Wasted memory", 0),
        ("Cached scripts", status.num_cached_scripts as i64),
        ("Cached keys", status.num_cached_scripts as i64),
        ("Max keys", max_keys),
        ("OOM restarts", 0),
        ("Hash keys restarts", 0),
        ("Manual restarts", status.manual_restarts as i64),
    ] {
        row(ctx, label, &value.to_string());
    }
    // php_format_date(DATE_ISO8601, t, localtime): date() in the default timezone.
    let format = ctx.constant(b"DATE_ISO8601").unwrap_or(Value::str("Y-m-d\\TH:i:sO"));
    let date = |ctx: &mut dyn Ctx, t: i64| -> Result<String, Throw> {
        let v = ctx.call(&Value::str("date"), &mut [format.clone(), Value::Int(t)])?;
        Ok(String::from_utf8_lossy(&bytes(&v)).into_owned())
    };
    let start = date(ctx, status.start_time)?;
    row(ctx, "Start time", &start);
    let last = if status.last_restart_time == 0 { "none".to_string() } else { date(ctx, status.last_restart_time)? };
    row(ctx, "Last restart time", &last);
    row(ctx, "Last force restart time", "none");
    Ok(())
}

/// A section line with its sources filled in; None when one gives nothing.
fn render(ctx: &mut dyn Ctx, line: &str) -> Result<Option<Vec<u8>>, Throw> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}').expect("a source's closing brace");
        out.extend_from_slice(&rest.as_bytes()[..open]);
        match source(ctx, &rest[open + 1..close])? {
            Some(v) => out.extend_from_slice(&v),
            None => return Ok(None),
        }
        rest = &rest[close + 1..];
    }
    out.extend_from_slice(rest.as_bytes());
    Ok(Some(out))
}

/// What a source gives (see the module documentation).
fn source(ctx: &mut dyn Ctx, spec: &str) -> Result<Option<Vec<u8>>, Throw> {
    let (expr, modifier) = spec.split_once('|').unwrap_or((spec, ""));
    let (kind, arg) = expr.split_once(':').expect("a source's kind");
    let value = match kind {
        "const" => match arg.split_once("::") {
            Some((class, name)) => match ctx.class(class.as_bytes()) {
                Some(c) => ctx.class_constant(&c, name.as_bytes())?,
                None => None,
            },
            None => ctx.constant(arg.as_bytes()),
        },
        "ini" => ctx.ini_get(arg.as_bytes()).map(|v| if matches!(v, Value::Null) { Value::Str(PhpStr::empty()) } else { v }),
        "call" => {
            let mut path = arg.split('/');
            let callee = path.next().expect("a function to call");
            let (callee, mut args) = match callee.strip_suffix(')').and_then(|c| c.split_once('(')) {
                Some((f, n)) => (f, vec![Value::Int(n.parse().expect("an int argument"))]),
                None => (callee, Vec::new()),
            };
            let callable = match callee.split_once("::") {
                Some((class, method)) => Value::Array(Rc::new(crate::array::PhpArray::from_list(vec![Value::str(class), Value::str(method)]))),
                None => Value::str(callee),
            };
            if !ctx.is_callable(&callable) {
                return Ok(None);
            }
            let mut v = ctx.call(&callable, &mut args)?;
            for key in path {
                v = match &v {
                    Value::Array(a) => a.get(&crate::array::Key::from_string(PhpStr::from(key))).cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                };
            }
            Some(v)
        }
        _ => unreachable!("a source of kind {kind}"),
    };
    let Some(v) = value.map(|v| v.deref()) else { return Ok(None) };
    if matches!(v, Value::Null) || (matches!(v, Value::Bool(false)) && modifier.is_empty()) {
        return Ok(None);
    }
    let list = |v: &Value| match v {
        Value::Array(a) => a.values().map(bytes).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    Ok(match modifier {
        "yesno" => Some((if v.to_bool() { "Yes" } else { "No" }).into()),
        "enabled" => v.to_bool().then(|| b"enabled".to_vec()),
        "endis" => Some((if v.to_bool() { "enabled" } else { "disabled" }).into()),
        "nonempty" => Some(bytes(&v)).filter(|b| !b.is_empty()),
        "terms" => Some(list(&v).iter().flat_map(|t| [t.as_slice(), b" "].concat()).collect()),
        "" => Some(bytes(&v)),
        m => match m.strip_prefix("join=") {
            Some(sep) => Some(list(&v).join(sep.as_bytes())),
            None => unreachable!("a source modifier {m}"),
        },
    })
}

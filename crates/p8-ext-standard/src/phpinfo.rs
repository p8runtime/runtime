//! phpinfo(), phpcredits(), php_uname() and get_extension_funcs(): what PHP
//! says about itself, in the text form the CLI prints. The modules'
//! sections are p8_runtime::info's (ReflectionExtension::info() prints
//! them too); a fact of the build or the host is p8's own.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/info.c, ext/standard/credits.c,
//! Zend/zend_builtin_functions.c (get_extension_funcs)

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::info;
use p8_runtime::stubs;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "phpinfo", run: phpinfo, by_ref: &[] },
    Builtin { name: "phpcredits", run: phpcredits, by_ref: &[] },
    Builtin { name: "php_uname", run: php_uname, by_ref: &[] },
    Builtin { name: "get_extension_funcs", run: get_extension_funcs, by_ref: &[] },
];

/// phpinfo() calls what the modules' rows come from (curl_version(), ...)
/// and reads the request's variables.
pub const REENTER: &[&str] = &["phpinfo"];

const INFO_GENERAL: i64 = 1;
const INFO_CREDITS: i64 = 2;
const INFO_CONFIGURATION: i64 = 4;
const INFO_MODULES: i64 = 8;
const INFO_ENVIRONMENT: i64 = 16;
const INFO_VARIABLES: i64 = 32;
const INFO_LICENSE: i64 = 64;
const CREDITS_ALL: i64 = 0xFFFF_FFFF;
const CREDITS_FULLPAGE: i64 = 32;

/// php_info_print_table_row(): the cells joined by " => " (an empty one
/// as a space).
fn row(ctx: &mut dyn Ctx, cells: &[&[u8]]) {
    let cells: Vec<&[u8]> = cells.iter().map(|c| if c.is_empty() { &b" "[..] } else { c }).collect();
    ctx.echo(&[cells.join(&b" => "[..]), b"\n".to_vec()].concat());
}

/// SECTION(): a table with the one header.
fn section(ctx: &mut dyn Ctx, name: &str) {
    ctx.echo(format!("\n{name}\n").as_bytes());
}

/// php_info_print_hr()
fn hr(ctx: &mut dyn Ctx) {
    ctx.echo(b"\n\n _______________________________________________________________________\n\n");
}

/// phpinfo(int $flags = INFO_ALL)
fn phpinfo(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let flags = if args.is_empty() { -1 } else { arg(args, 0).to_int() };
    ctx.echo(b"phpinfo()\n");
    if flags & INFO_GENERAL != 0 {
        general(ctx);
    }
    // zend_ini_sort_entries(): the directives are listed by name from now on.
    *ctx.state(info::INI_SORTED) = Value::Bool(true);
    if flags & INFO_CONFIGURATION != 0 {
        hr(ctx);
        section(ctx, "Configuration");
        if flags & INFO_MODULES == 0 {
            section(ctx, "PHP Core");
            if let Some(core) = stubs::module_spec(b"Core") {
                info::print_ini_entries(ctx, core);
            }
        }
    }
    if flags & INFO_MODULES != 0 {
        // module_name_cmp: by name, ignoring case.
        let mut modules = ctx.modules().to_vec();
        modules.sort_by_key(|m| m.to_ascii_lowercase());
        let (known, additional): (Vec<&str>, Vec<&str>) = modules.into_iter().partition(|m| stubs::module_spec(m.as_bytes()).is_some());
        for m in known {
            info::print_module(ctx, m)?;
        }
        section(ctx, "Additional Modules");
        ctx.echo(b"\nModule Name\n");
        for m in additional {
            info::print_module(ctx, m)?;
        }
    }
    if flags & INFO_ENVIRONMENT != 0 {
        section(ctx, "Environment");
        ctx.echo(b"\nVariable => Value\n");
        for (name, value) in crate::process::env_all(ctx).iter() {
            let value = value.to_php_string().unwrap_or_else(PhpStr::empty);
            row(ctx, &[key_bytes(&name).as_slice(), value.as_bytes()]);
        }
    }
    if flags & INFO_VARIABLES != 0 {
        variables(ctx);
    }
    if flags & INFO_CREDITS != 0 {
        hr(ctx);
        credits(ctx, CREDITS_ALL & !CREDITS_FULLPAGE);
    }
    if flags & INFO_LICENSE != 0 {
        ctx.echo(LICENSE.as_bytes());
    }
    Ok(Value::Bool(true))
}

const LICENSE: &str = "
PHP License
This program is free software; you can redistribute it and/or modify
it under the terms of the PHP License as published by the PHP Group
and included in the distribution in the file:  LICENSE

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.

If you did not receive a copy of the PHP license, or have any
questions about PHP licensing, please contact license@php.net.
";

fn key_bytes(k: &Key) -> Vec<u8> {
    match k {
        Key::Int(i) => i.to_string().into_bytes(),
        Key::Str(s) => s.as_bytes().to_vec(),
    }
}

/// PHP_INFO_GENERAL: the version, the build, the host and the APIs. What
/// PHP decides when it is configured is p8's: no php.ini read, no Zend
/// memory manager, signals or multibyte scanner.
fn general(ctx: &mut dyn Ctx) {
    row(ctx, &[b"PHP Version", crate::PHP_VERSION.as_bytes()]);
    ctx.echo(b"\n");
    row(ctx, &[b"System", uname('a').as_bytes()]);
    row(ctx, &[b"Build Date", build_date().as_bytes()]);
    let server = if ctx.response().server { "Built-in HTTP server" } else { "Command Line Interface" };
    row(ctx, &[b"Server API", server.as_bytes()]);
    let rows: [(&str, &str); 19] = [
        ("Virtual Directory Support", "disabled"),
        ("Configuration File (php.ini) Path", "(none)"),
        ("Loaded Configuration File", "(none)"),
        ("Scan this dir for additional .ini files", "(none)"),
        ("Additional .ini files parsed", "(none)"),
        ("PHP API", "20240924"),
        ("PHP Extension", "20240924"),
        ("Zend Extension", "420240924"),
        ("Zend Extension Build", "API420240924,NTS"),
        ("PHP Extension Build", "API20240924,NTS"),
        ("PHP Integer Size", "64 bits"),
        ("Debug Build", if cfg!(debug_assertions) { "yes" } else { "no" }),
        ("Thread Safety", "disabled"),
        ("Zend Signal Handling", "disabled"),
        ("Zend Memory Manager", "disabled"),
        ("Zend Multibyte Support", "disabled"),
        ("Zend Max Execution Timers", "disabled"),
        ("IPv6 Support", "enabled"),
        ("DTrace Support", "disabled"),
    ];
    for (name, value) in rows {
        row(ctx, &[name.as_bytes(), value.as_bytes()]);
    }
    let wrappers = p8_runtime::user_stream::schemes(ctx);
    stream_hash(ctx, "PHP Streams", &wrappers);
    // fsockopen()'s one transport; p8 has no stream filters.
    stream_hash(ctx, "Stream Socket Transports", &[b"tcp".to_vec()]);
    stream_hash(ctx, "Stream Filters", &[]);
    ctx.echo(b"\n\n");
    let zend = format!("Zend Engine v4.4.{}, Copyright (c) Zend Technologies\n", crate::PHP_VERSION.rsplit('.').next().unwrap_or("0"));
    ctx.echo(b"This program makes use of the Zend Scripting Language Engine:\n");
    ctx.echo(zend.as_bytes());
}

/// php_info_print_stream_hash(): "Registered <name> => a, b" (no line end:
/// the next row starts with one), or a row saying none are.
fn stream_hash(ctx: &mut dyn Ctx, name: &str, names: &[Vec<u8>]) {
    if names.is_empty() {
        row(ctx, &[format!("Registered {name}").as_bytes(), b"none registered"]);
        return;
    }
    ctx.echo(format!("\nRegistered {name} => ").as_bytes());
    ctx.echo(&names.join(&b", "[..]));
}

/// When the running binary was built (PHP's __DATE__ " " __TIME__): its
/// modification time, in UTC.
fn build_date() -> String {
    let built = std::env::current_exe().and_then(std::fs::metadata).and_then(|m| m.modified()).ok();
    let secs = built.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    chrono::DateTime::from_timestamp(secs, 0).map_or_else(String::new, |t| t.format("%b %e %Y %H:%M:%S").to_string())
}

/// PHP_INFO_VARIABLES: the request's variables, each array's entries a row
/// (an array value as print_r() prints it).
fn variables(ctx: &mut dyn Ctx) {
    section(ctx, "PHP Variables");
    ctx.echo(b"\nVariable => Value\n");
    for name in ["PHP_SELF", "PHP_AUTH_TYPE", "PHP_AUTH_USER", "PHP_AUTH_PW"] {
        if let Some(Value::Str(v)) = ctx.global_value(name.as_bytes()) {
            row(ctx, &[name.as_bytes(), v.as_bytes()]);
        }
    }
    for name in ["_REQUEST", "_GET", "_POST", "_FILES", "_COOKIE", "_SERVER", "_ENV"] {
        let Some(Value::Array(a)) = ctx.global_value(name.as_bytes()) else { continue };
        for (k, v) in a.iter() {
            let mut out = format!("${name}['").into_bytes();
            out.extend_from_slice(&key_bytes(&k));
            out.extend_from_slice(b"'] => ");
            match v.deref() {
                v @ Value::Array(_) => crate::var::print_r_to(&v, 0, &mut out, &mut Vec::new()),
                v => out.extend_from_slice(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()),
            }
            out.push(b'\n');
            ctx.echo(&out);
        }
    }
}

/// php_print_credits(): the parts `flags` selects (CREDITS_GROUP, ...).
fn credits(ctx: &mut dyn Ctx, flags: i64) {
    ctx.echo(b"PHP Credits\n");
    for (flag, text) in stubs::CREDITS {
        if flags & i64::from(*flag) != 0 {
            ctx.echo(text.as_bytes());
        }
    }
}

/// phpcredits(int $flags = CREDITS_ALL)
fn phpcredits(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let flags = if args.is_empty() { CREDITS_ALL } else { arg(args, 0).to_int() };
    credits(ctx, flags);
    Ok(Value::Bool(true))
}

/// uname(2), as php_get_uname() reads it: one field, or all five ('a').
#[cfg(unix)]
fn uname(mode: char) -> String {
    let Ok(u) = nix::sys::utsname::uname() else { return String::new() };
    let field = |s: &std::ffi::OsStr| s.to_string_lossy().into_owned();
    match mode {
        's' => field(u.sysname()),
        'n' => field(u.nodename()),
        'r' => field(u.release()),
        'v' => field(u.version()),
        'm' => field(u.machine()),
        _ => format!("{} {} {} {} {}", field(u.sysname()), field(u.nodename()), field(u.release()), field(u.version()), field(u.machine())),
    }
}

/// What wasi-libc's uname() says: sysname, nodename, release, version,
/// machine.
#[cfg(target_os = "wasi")]
pub(crate) const WASI_UNAME: [&str; 5] = ["wasi", "(none)", "0.0.0", "0.0.0", "wasm32"];

/// uname(2), as php_get_uname() reads it: one field, or all five ('a').
#[cfg(target_os = "wasi")]
fn uname(mode: char) -> String {
    let [sysname, nodename, release, version, machine] = WASI_UNAME;
    match mode {
        's' => sysname.into(),
        'n' => nodename.into(),
        'r' => release.into(),
        'v' => version.into(),
        'm' => machine.into(),
        _ => format!("{sysname} {nodename} {release} {version} {machine}"),
    }
}

/// php_uname(string $mode = "a")
fn php_uname(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mode = if args.is_empty() { PhpStr::from("a") } else { str_arg(args, 0) };
    let &[m] = mode.as_bytes() else {
        return Err(ctx.throw("ValueError", "php_uname(): Argument #1 ($mode) must be a single character".into()));
    };
    if !b"amnrsv".contains(&m) {
        return Err(ctx.throw("ValueError", "php_uname(): Argument #1 ($mode) must be one of \"a\", \"m\", \"n\", \"r\", \"s\", or \"v\"".into()));
    }
    Ok(Value::str(uname(char::from(m))))
}

/// get_extension_funcs(string $extension): the loaded module's functions
/// ("zend" is Core); false for one p8 lacks, or one that declares none.
fn get_extension_funcs(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    let name = if name.as_bytes().eq_ignore_ascii_case(b"zend") { PhpStr::from("core") } else { name };
    let Some(spec) = info::loaded_module(ctx, name.as_bytes()).and_then(|m| stubs::module_spec(m.as_bytes())) else {
        return Ok(Value::Bool(false));
    };
    if spec.functions.is_empty() {
        return Ok(Value::Bool(false));
    }
    let builtins = ctx.defined_functions().0;
    let names = spec.functions.iter().filter(|f| builtins.contains(f)).map(|f| Value::str(*f)).collect();
    Ok(Value::Array(Rc::new(PhpArray::from_list(names))))
}

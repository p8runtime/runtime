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
        } else if let Some(text) = render(ctx, line)? {
            ctx.echo(&text);
            ctx.echo(b"\n");
        }
    }
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

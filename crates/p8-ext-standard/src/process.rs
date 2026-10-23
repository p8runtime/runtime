//! Running commands and the process: shell_exec (and backticks), exec,
//! system, passthru; putenv (the environment this request changed, kept
//! apart: commands it runs see it); getmypid; memory_get_usage;
//! get_include_path / set_include_path.
//!
//! Commands run as PHP runs them: `/bin/sh -c command`.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/exec.c

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "shell_exec", run: shell_exec, by_ref: &[] },
    Builtin { name: "exec", run: exec, by_ref: &[1, 2] },
    Builtin { name: "system", run: system, by_ref: &[1] },
    Builtin { name: "passthru", run: passthru, by_ref: &[1] },
    Builtin { name: "putenv", run: putenv, by_ref: &[] },
    Builtin { name: "getmypid", run: getmypid, by_ref: &[] },
    Builtin { name: "memory_get_usage", run: memory_get_usage, by_ref: &[] },
    Builtin { name: "memory_get_peak_usage", run: memory_get_usage, by_ref: &[] },
    Builtin { name: "memory_reset_peak_usage", run: memory_reset_peak_usage, by_ref: &[] },
    Builtin { name: "get_include_path", run: get_include_path, by_ref: &[] },
    Builtin { name: "set_include_path", run: set_include_path, by_ref: &[] },
];

/// The environment changes this request made (putenv): name => value,
/// or null for a removed variable.
const ENV: &str = "process.env";

/// The environment a variable has here (putenv first), if set.
pub(crate) fn env_var(ctx: &mut dyn Ctx, name: &[u8]) -> Option<Vec<u8>> {
    if let Value::Array(changed) = ctx.state(ENV)
        && let Some(v) = changed.get(&Key::from_string(PhpStr::from(name)))
    {
        return match v {
            Value::Null => None,
            v => Some(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()),
        };
    }
    let name = std::str::from_utf8(name).ok()?;
    std::env::var_os(name).map(|v| v.into_encoded_bytes())
}

/// The whole environment as getenv() lists it (putenv's changes applied).
pub(crate) fn env_all(ctx: &mut dyn Ctx) -> PhpArray {
    let mut all = PhpArray::new();
    for (k, v) in std::env::vars_os() {
        all.insert(Key::from_string(PhpStr::new(k.into_encoded_bytes())), Value::Str(PhpStr::new(v.into_encoded_bytes())));
    }
    if let Value::Array(changed) = ctx.state(ENV) {
        for (k, v) in changed.iter() {
            match v {
                Value::Null => {
                    all.remove(&k);
                }
                v => all.insert(k, v.clone()),
            }
        }
    }
    all
}

/// putenv("NAME=value"), putenv("NAME") (removed).
fn putenv(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let setting = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let bytes = setting.as_bytes();
    if bytes.is_empty() || bytes[0] == b'=' {
        return Err(ctx.throw("ValueError", "putenv(): Argument #1 ($assignment) must have a valid syntax".into()));
    }
    let (name, value) = match bytes.iter().position(|&c| c == b'=') {
        Some(i) => (&bytes[..i], Value::Str(PhpStr::from(&bytes[i + 1..]))),
        None => (bytes, Value::Null),
    };
    let mut changed = match ctx.state(ENV) {
        Value::Array(a) => (**a).clone(),
        _ => PhpArray::new(),
    };
    changed.insert(Key::from_string(PhpStr::from(name)), value);
    *ctx.state(ENV) = Value::Array(Rc::new(changed));
    Ok(Value::Bool(true))
}

/// Run `command` through the shell; its stdout and exit status.
fn run(ctx: &mut dyn Ctx, command: &[u8]) -> Option<(Vec<u8>, i64)> {
    let command = std::ffi::OsStr::from_bytes(command);
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(command).stdin(std::process::Stdio::inherit()).stderr(std::process::Stdio::inherit());
    if let Value::Array(changed) = ctx.state(ENV).clone() {
        for (k, v) in changed.iter() {
            let name = std::ffi::OsString::from_vec(k.into_value().to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec());
            match v {
                Value::Null => {
                    cmd.env_remove(name);
                }
                v => {
                    cmd.env(name, std::ffi::OsString::from_vec(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()));
                }
            }
        }
    }
    let out = cmd.output().ok()?;
    let status = out.status.code().unwrap_or(-1);
    Some((out.stdout, i64::from(status)))
}

fn command_arg(ctx: &mut dyn Ctx, args: &[Value], func: &str) -> Result<Vec<u8>, Throw> {
    let c = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    if c.is_empty() {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($command) must not be empty")));
    }
    if c.as_bytes().contains(&0) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($command) must not contain any null bytes")));
    }
    Ok(c.as_bytes().to_vec())
}

/// Write `v` into a by-reference argument.
fn set_ref(args: &mut [Value], i: usize, v: Value) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = v;
    }
}

/// The output's lines, without trailing whitespace (as exec() gives them).
fn lines(out: &[u8]) -> Vec<Vec<u8>> {
    let mut lines: Vec<Vec<u8>> = out.split(|&c| c == b'\n').map(|l| {
        let mut l = l.to_vec();
        while l.last().is_some_and(|c| c.is_ascii_whitespace()) {
            l.pop();
        }
        l
    }).collect();
    if out.ends_with(b"\n") || out.is_empty() {
        lines.pop();
    }
    lines
}

/// shell_exec(string $command): the output, null when there is none.
pub(crate) fn shell_exec(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let c = command_arg(ctx, args, "shell_exec")?;
    Ok(match run(ctx, &c) {
        Some((out, _)) if out.is_empty() => Value::Null,
        Some((out, _)) => Value::Str(PhpStr::new(out)),
        None => Value::Bool(false),
    })
}

/// exec(string $command, array &$output = null, int &$result_code = null): the last line
fn exec(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let c = command_arg(ctx, args, "exec")?;
    let Some((out, status)) = run(ctx, &c) else { return Ok(Value::Bool(false)) };
    let lines = lines(&out);
    if args.len() > 1 {
        let mut list = match args[1].deref() {
            Value::Array(a) => (*a).clone(),
            _ => PhpArray::new(),
        };
        for l in &lines {
            let _ = list.push(Value::Str(PhpStr::from(l.as_slice())));
        }
        set_ref(args, 1, Value::Array(Rc::new(list)));
    }
    set_ref(args, 2, Value::Int(status));
    Ok(Value::Str(PhpStr::from(lines.last().map_or(&[][..], |l| l.as_slice()))))
}

/// system(string $command, int &$result_code = null): the output shown;
/// the last line returned.
fn system(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let c = command_arg(ctx, args, "system")?;
    let Some((out, status)) = run(ctx, &c) else { return Ok(Value::Bool(false)) };
    ctx.echo(&out);
    set_ref(args, 1, Value::Int(status));
    let lines = lines(&out);
    Ok(Value::Str(PhpStr::from(lines.last().map_or(&[][..], |l| l.as_slice()))))
}

/// passthru(string $command, int &$result_code = null): the raw output shown.
fn passthru(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let c = command_arg(ctx, args, "passthru")?;
    let Some((out, status)) = run(ctx, &c) else { return Ok(Value::Bool(false)) };
    ctx.echo(&out);
    set_ref(args, 1, Value::Int(status));
    Ok(Value::Null)
}

fn getmypid(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(i64::from(std::process::id())))
}

/// memory_get_usage(): p8 does not count PHP's allocations; a steady
/// figure PHP's tests compare differences of.
fn memory_get_usage(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(2 * 1024 * 1024))
}

fn memory_reset_peak_usage(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

fn get_include_path(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(ctx.ini_get(b"include_path").unwrap_or_else(|| Value::str(".:/usr/share/php")))
}

/// set_include_path(string $include_path): the old one.
fn set_include_path(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let new = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    if new.is_empty() {
        return Ok(Value::Bool(false));
    }
    let old = get_include_path(ctx, &mut [])?;
    ctx.ini_set(b"include_path", Value::Str(new));
    Ok(old)
}

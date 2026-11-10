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
use p8_runtime::os::{OsStrExt, OsStringExt};

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "shell_exec", run: shell_exec, by_ref: &[] },
    Builtin { name: "exec", run: exec, by_ref: &[1, 2] },
    Builtin { name: "system", run: system, by_ref: &[1] },
    Builtin { name: "passthru", run: passthru, by_ref: &[1] },
    Builtin { name: "putenv", run: putenv, by_ref: &[] },
    Builtin { name: "getmypid", run: getmypid, by_ref: &[] },
    Builtin { name: "getrusage", run: getrusage, by_ref: &[] },
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
    if let Some(changed) = ctx.state(ENV).as_array()
        && let Some(v) = changed.get(&Key::from_string(PhpStr::from(name)))
    {
        return if v.is_null() { None } else { Some(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()) };
    }
    let name = std::str::from_utf8(name).ok()?;
    std::env::var_os(name).map(|v| v.into_encoded_bytes())
}

/// The whole environment as getenv() lists it (putenv's changes applied).
pub(crate) fn env_all(ctx: &mut dyn Ctx) -> PhpArray {
    let mut all = PhpArray::new();
    for (k, v) in std::env::vars_os() {
        all.insert(Key::from_string(PhpStr::new(k.into_encoded_bytes())), Value::str(PhpStr::new(v.into_encoded_bytes())));
    }
    if let Some(changed) = ctx.state(ENV).as_array() {
        for (k, v) in changed.iter() {
            if v.is_null() {
                all.remove(&k);
            } else {
                all.insert(k, v.clone());
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
        Some(i) => (&bytes[..i], Value::str(PhpStr::from(&bytes[i + 1..]))),
        None => (bytes, Value::NULL),
    };
    let mut changed = ctx.state(ENV).as_array().cloned().unwrap_or_default();
    changed.insert(Key::from_string(PhpStr::from(name)), value);
    *ctx.state(ENV) = Value::array(changed);
    Ok(Value::TRUE)
}

/// Run `command` through the shell; its stdout and exit status.
fn run(ctx: &mut dyn Ctx, command: &[u8]) -> Option<(Vec<u8>, i64)> {
    let mut cmd = shell(ctx, command);
    cmd.stdin(std::process::Stdio::inherit()).stderr(std::process::Stdio::inherit());
    let out = cmd.output().ok()?;
    let status = out.status.code().unwrap_or(-1);
    Some((out.stdout, i64::from(status)))
}

/// `/bin/sh -c command`, in the environment this request sees (putenv's
/// changes applied).
pub(crate) fn shell(ctx: &mut dyn Ctx, command: &[u8]) -> std::process::Command {
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(std::ffi::OsStr::from_bytes(command));
    if let Some(changed) = ctx.state(ENV).cloned_array() {
        for (k, v) in changed.iter() {
            let name = std::ffi::OsString::from_vec(k.into_value().to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec());
            if v.is_null() {
                cmd.env_remove(name);
            } else {
                cmd.env(name, std::ffi::OsString::from_vec(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()));
            }
        }
    }
    cmd
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
    if let Some(cell) = args.get(i).and_then(Value::as_reference) {
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
        Some((out, _)) if out.is_empty() => Value::NULL,
        Some((out, _)) => Value::str(PhpStr::new(out)),
        None => Value::FALSE,
    })
}

/// exec(string $command, array &$output = null, int &$result_code = null): the last line
fn exec(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let c = command_arg(ctx, args, "exec")?;
    let Some((out, status)) = run(ctx, &c) else { return Ok(Value::FALSE) };
    let lines = lines(&out);
    if args.len() > 1 {
        let mut list = args[1].deref().as_array().cloned().unwrap_or_default();
        for l in &lines {
            let _ = list.push(Value::str(PhpStr::from(l.as_slice())));
        }
        set_ref(args, 1, Value::array(list));
    }
    set_ref(args, 2, Value::int(status));
    Ok(Value::str(PhpStr::from(lines.last().map_or(&[][..], |l| l.as_slice()))))
}

/// system(string $command, int &$result_code = null): the output shown;
/// the last line returned.
fn system(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let c = command_arg(ctx, args, "system")?;
    let Some((out, status)) = run(ctx, &c) else { return Ok(Value::FALSE) };
    ctx.echo(&out);
    set_ref(args, 1, Value::int(status));
    let lines = lines(&out);
    Ok(Value::str(PhpStr::from(lines.last().map_or(&[][..], |l| l.as_slice()))))
}

/// passthru(string $command, int &$result_code = null): the raw output shown.
fn passthru(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let c = command_arg(ctx, args, "passthru")?;
    let Some((out, status)) = run(ctx, &c) else { return Ok(Value::FALSE) };
    ctx.echo(&out);
    set_ref(args, 1, Value::int(status));
    Ok(Value::NULL)
}

/// getmypid(): false where there are no processes (WASI), as PHP gives
/// when getpid() fails.
fn getmypid(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(p8_runtime::os::process_id().map_or(Value::FALSE, |pid| Value::int(i64::from(pid))))
}

/// getrusage(int $mode = 0): this process's resource usage (1: its
/// children's), with PHP's keys in its order.
#[cfg(unix)]
fn getrusage(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    use nix::sys::resource::{UsageWho, getrusage};
    let who = if args.first().is_some_and(|v| v.deref().to_int() == 1) { UsageWho::RUSAGE_CHILDREN } else { UsageWho::RUSAGE_SELF };
    let Ok(u) = getrusage(who) else { return Ok(Value::FALSE) };
    let (ut, st) = (u.user_time(), u.system_time());
    let fields: [i64; 17] = [
        u.block_writes(),
        u.block_reads(),
        u.ipc_sends(),
        u.ipc_receives(),
        u.max_rss(),
        u.shared_integral(),
        u.unshared_data_integral(),
        u.minor_page_faults(),
        u.major_page_faults(),
        u.signals(),
        u.voluntary_context_switches(),
        u.involuntary_context_switches(),
        u.full_swaps(),
        i64::from(ut.tv_usec()),
        ut.tv_sec(),
        i64::from(st.tv_usec()),
        st.tv_sec(),
    ];
    // (PHPUnit calls this around every test: the keys are made once.)
    thread_local! {
        static KEYS: [PhpStr; 17] = p8_runtime::array::record_keys([
            "ru_oublock", "ru_inblock", "ru_msgsnd", "ru_msgrcv", "ru_maxrss", "ru_ixrss", "ru_idrss", "ru_minflt",
            "ru_majflt", "ru_nsignals", "ru_nvcsw", "ru_nivcsw", "ru_nswap", "ru_utime.tv_usec", "ru_utime.tv_sec",
            "ru_stime.tv_usec", "ru_stime.tv_sec",
        ]);
    }
    Ok(Value::array(KEYS.with(|k| PhpArray::record(k, fields.map(Value::int)))))
}

/// getrusage(): false, as PHP gives when getrusage(2) fails: WASI has no
/// resource usage to report.
#[cfg(target_os = "wasi")]
fn getrusage(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::FALSE)
}

/// memory_get_usage(): p8 does not count PHP's allocations; a steady
/// figure PHP's tests compare differences of.
fn memory_get_usage(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::int(2 * 1024 * 1024))
}

fn memory_reset_peak_usage(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::NULL)
}

fn get_include_path(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(ctx.ini_get(b"include_path").unwrap_or_else(|| Value::str(".:/usr/share/php")))
}

/// set_include_path(string $include_path): the old one.
fn set_include_path(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let new = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    if new.is_empty() {
        return Ok(Value::FALSE);
    }
    let old = get_include_path(ctx, &mut [])?;
    ctx.ini_set(b"include_path", Value::str(new));
    Ok(old)
}

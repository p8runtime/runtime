//! Child processes and sockets: proc_open() and its family, fsockopen(),
//! stream_set_blocking() and stream_set_timeout(). A child's pipes and a
//! TCP connection are streams (p8_runtime::stream::Target), so fgets(),
//! fwrite() and fclose() work on them as on files.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/proc_open.c,
//! $PHP_SRC_PATH/ext/standard/fsock.c

use crate::file::{new_stream, stream_arg};
use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::stream::{PipeOut, Resource, Stream, Target, resource, resource_of};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "proc_open", run: proc_open, by_ref: &[2] },
    Builtin { name: "proc_get_status", run: proc_get_status, by_ref: &[] },
    Builtin { name: "proc_terminate", run: proc_terminate, by_ref: &[] },
    Builtin { name: "proc_close", run: proc_close, by_ref: &[] },
    Builtin { name: "fsockopen", run: fsockopen, by_ref: &[2, 3] },
    Builtin { name: "stream_set_blocking", run: stream_set_blocking, by_ref: &[] },
    Builtin { name: "stream_set_timeout", run: stream_set_timeout, by_ref: &[] },
];

fn text(v: &Value) -> String {
    String::from_utf8_lossy(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned()
}

/// The Stdio a descriptor specification asks for (fd 0, 1 or 2), and
/// whether it is a pipe to hand back.
fn descriptor(ctx: &mut dyn Ctx, fd: i64, spec: &Value) -> Result<Option<(Stdio, bool)>, Throw> {
    if let Some(o) = resource_of(spec) {
        let Some(r) = resource(&o) else { return Ok(Some((Stdio::inherit(), false))) };
        let s = r.stream.borrow();
        return Ok(Some(match &s.target {
            Target::File(f) => (f.try_clone().map_or_else(|_| Stdio::inherit(), Stdio::from), false),
            _ => (Stdio::inherit(), false),
        }));
    }
    let Value::Array(parts) = spec.deref() else {
        return Err(ctx.throw("ValueError", "proc_open(): Argument #2 ($descriptor_spec) must only contain arrays and streams".into()));
    };
    let item = |i: i64| parts.get(&Key::Int(i)).map(Value::deref);
    let kind = item(0).map(|v| text(&v)).unwrap_or_default();
    Ok(Some(match kind.as_str() {
        "pipe" => (Stdio::piped(), true),
        "null" => (Stdio::null(), false),
        "file" => {
            let path = item(1).map(|v| text(&v)).unwrap_or_default();
            let mode = item(2).map(|v| text(&v)).unwrap_or_else(|| "r".into());
            let mut open = std::fs::OpenOptions::new();
            match mode.chars().next() {
                Some('a') => open.append(true).create(true),
                Some('w') => open.write(true).create(true).truncate(true),
                _ if fd == 0 => open.read(true),
                _ => open.write(true).create(true),
            };
            match open.open(&path) {
                Ok(f) => (Stdio::from(f), false),
                Err(_) => (Stdio::null(), false),
            }
        }
        _ => return Ok(None),
    }))
}

/// proc_open(array|string $command, array $descriptor_spec, &$pipes, ?string $cwd = null, ?array $env_vars = null, ?array $options = null)
fn proc_open(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let command = arg(args, 0);
    let mut cmd = match &command {
        Value::Array(parts) => {
            let words: Vec<Vec<u8>> = parts.values().map(|v| v.deref().to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()).collect();
            let Some((program, rest)) = words.split_first() else {
                return Err(ctx.throw("ValueError", "proc_open(): Argument #1 ($command) must have at least one element".into()));
            };
            let mut c = Command::new(std::ffi::OsStr::from_bytes(program));
            c.args(rest.iter().map(|a| std::ffi::OsStr::from_bytes(a)));
            c
        }
        v => crate::process::shell(ctx, v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()),
    };
    let display = match &command {
        Value::Array(parts) => parts.values().map(text).collect::<Vec<_>>().join(" "),
        v => text(v),
    };
    if let Some(Value::Str(cwd)) = args.get(3).map(Value::deref) {
        cmd.current_dir(std::ffi::OsStr::from_bytes(cwd.as_bytes()));
    }
    if let Some(Value::Array(env)) = args.get(4).map(Value::deref) {
        cmd.env_clear();
        for (k, v) in env.iter() {
            cmd.env(std::ffi::OsStr::from_bytes(k.into_value().to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()), std::ffi::OsStr::from_bytes(v.deref().to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()));
        }
    }
    let Value::Array(spec) = arg(args, 1) else {
        return Err(ctx.throw("TypeError", "proc_open(): Argument #2 ($descriptor_spec) must be of type array".into()));
    };
    let mut piped = Vec::new();
    for (key, value) in spec.iter() {
        let Key::Int(fd) = key else {
            return Err(ctx.throw("ValueError", "proc_open(): Argument #2 ($descriptor_spec) must be an integer indexed array".into()));
        };
        let Some((stdio, is_pipe)) = descriptor(ctx, fd, value)? else {
            return Err(ctx.throw("ValueError", format!("proc_open(): Argument #2 ($descriptor_spec) must only contain arrays and streams")));
        };
        match fd {
            0 => cmd.stdin(stdio),
            1 => cmd.stdout(stdio),
            2 => cmd.stderr(stdio),
            _ => &mut cmd,
        };
        if is_pipe && fd <= 2 {
            piped.push(fd);
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            ctx.warn(&format!("proc_open(): Exec failed: {}", crate::file::os_error(&e)));
            return Ok(Value::Bool(false));
        }
    };
    let mut pipes = PhpArray::new();
    for fd in piped {
        let stream = match fd {
            0 => child.stdin.take().map(|p| Stream::new(Target::PipeIn(p), "", "w")),
            1 => child.stdout.take().map(|p| Stream::new(Target::PipeOut(PipeOut::new(p)), "", "r")),
            _ => child.stderr.take().map(|p| Stream::new(Target::PipeOut(PipeOut::new(p)), "", "r")),
        };
        if let Some(s) = stream {
            pipes.insert(Key::Int(fd), new_stream(ctx, s));
        }
    }
    if let Some(Value::Ref(cell)) = args.get(2) {
        *cell.borrow_mut() = Value::Array(Rc::new(pipes));
    }
    let process = Stream::new(Target::Process(Box::new(child)), &display, "r");
    Ok(ctx.new_resource(Resource { kind: "process", closed: Cell::new(false), stream: RefCell::new(process) }))
}

/// The process behind a proc_* argument.
fn process_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<Rc<p8_runtime::object::Object>, Throw> {
    let v = arg(args, 0);
    match resource_of(&v) {
        Some(o) if resource(&o).is_some_and(|r| r.kind == "process" && !r.closed.get()) => Ok(o),
        _ => Err(ctx.throw("TypeError", format!("{func}(): supplied resource is not a valid process resource"))),
    }
}

/// proc_get_status(resource $process): command, pid, running, signaled,
/// stopped, exitcode, termsig, stopsig. The exit code is given once.
fn proc_get_status(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = process_arg(ctx, "proc_get_status", args)?;
    let r = resource(&o).expect("a process");
    let mut s = r.stream.borrow_mut();
    let command = s.path.clone();
    let reported = s.pos != 0;
    let Target::Process(child) = &mut s.target else { return Ok(Value::Bool(false)) };
    let pid = i64::from(child.id());
    let status = child.try_wait().ok().flatten();
    let (running, signaled, termsig, mut exitcode) = match status {
        None => (true, false, 0, -1),
        Some(st) => (false, st.signal().is_some(), st.signal().map_or(0, i64::from), st.code().map_or(-1, i64::from)),
    };
    if !running {
        if reported {
            exitcode = -1;
        }
        s.pos = 1;
    }
    let mut out = PhpArray::new();
    let mut put = |k: &str, v: Value| out.insert(Key::from_string(PhpStr::from(k)), v);
    put("command", Value::Str(PhpStr::from(command.as_str())));
    put("pid", Value::Int(pid));
    put("running", Value::Bool(running));
    put("signaled", Value::Bool(signaled));
    put("stopped", Value::Bool(false));
    put("exitcode", Value::Int(exitcode));
    put("termsig", Value::Int(termsig));
    put("stopsig", Value::Int(0));
    Ok(Value::Array(Rc::new(out)))
}

/// proc_terminate(resource $process, int $signal = 15): the process is
/// killed whatever the signal.
fn proc_terminate(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = process_arg(ctx, "proc_terminate", args)?;
    let r = resource(&o).expect("a process");
    let mut s = r.stream.borrow_mut();
    let Target::Process(child) = &mut s.target else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(child.kill().is_ok()))
}

/// proc_close(resource $process): waits for the process; its exit code.
fn proc_close(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = process_arg(ctx, "proc_close", args)?;
    let r = resource(&o).expect("a process");
    let code = {
        let mut s = r.stream.borrow_mut();
        let Target::Process(child) = &mut s.target else { return Ok(Value::Int(-1)) };
        drop(child.stdin.take());
        child.wait().ok().and_then(|st| st.code()).map_or(-1, i64::from)
    };
    r.closed.set(true);
    Ok(Value::Int(code))
}

/// fsockopen(string $hostname, int $port = -1, &$error_code = null, &$error_message = null, ?float $timeout = null)
fn fsockopen(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    use std::net::ToSocketAddrs;
    let hostname = text(&arg(args, 0));
    let port = if args.len() > 1 { arg(args, 1).to_int() } else { -1 };
    let rest = match hostname.split_once("://") {
        Some(("tcp", rest)) => rest.to_string(),
        Some((scheme, _)) => {
            ctx.warn(&format!("fsockopen(): Unable to find the socket transport \"{scheme}\" - did you forget to enable it when you configured PHP?"));
            return Ok(Value::Bool(false));
        }
        None => hostname.clone(),
    };
    let target = if port >= 0 { if rest.contains(':') && !rest.starts_with('[') { format!("[{rest}]:{port}") } else { format!("{rest}:{port}") } } else { rest };
    let shown = format!("tcp://{target}");
    let timeout = match args.get(4).map(Value::deref) {
        Some(Value::Null) | None => 60.0,
        Some(v) => v.to_float(),
    };
    let mut last = None;
    let mut stream = None;
    match target.to_socket_addrs() {
        Ok(addrs) => {
            for addr in addrs {
                match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs_f64(timeout.max(0.001))) {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(e) => last = Some(e),
                }
            }
        }
        Err(e) => last = Some(e),
    }
    let Some(sock) = stream else {
        let (code, message) = match &last {
            Some(e) => (i64::from(e.raw_os_error().unwrap_or(0)), crate::file::os_error(e)),
            None => (0, String::new()),
        };
        if let Some(Value::Ref(r)) = args.get(2) {
            *r.borrow_mut() = Value::Int(code);
        }
        if let Some(Value::Ref(r)) = args.get(3) {
            *r.borrow_mut() = Value::Str(PhpStr::from(message.as_str()));
        }
        ctx.warn(&format!("fsockopen(): Unable to connect to {shown} ({message})"));
        return Ok(Value::Bool(false));
    };
    if let Some(Value::Ref(r)) = args.get(2) {
        *r.borrow_mut() = Value::Int(0);
    }
    if let Some(Value::Ref(r)) = args.get(3) {
        *r.borrow_mut() = Value::Str(PhpStr::empty());
    }
    Ok(new_stream(ctx, Stream::new(Target::Tcp(sock), &shown, "r+")))
}

/// stream_set_blocking(resource $stream, bool $enable): reads wait or do not.
fn stream_set_blocking(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_set_blocking", args, 0)?;
    let enable = arg(args, 1).to_bool();
    let r = resource(&o).expect("a stream");
    r.stream.borrow_mut().nonblocking = !enable;
    Ok(Value::Bool(true))
}

/// stream_set_timeout(resource $stream, int $seconds, int $microseconds = 0)
fn stream_set_timeout(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_set_timeout", args, 0)?;
    let seconds = arg(args, 1).to_int().max(0) as u64;
    let micros = if args.len() > 2 { arg(args, 2).to_int().max(0) as u64 } else { 0 };
    let r = resource(&o).expect("a stream");
    let s = r.stream.borrow();
    if let Target::Tcp(sock) = &s.target {
        let d = Duration::from_secs(seconds) + Duration::from_micros(micros);
        let _ = sock.set_read_timeout((!d.is_zero()).then_some(d));
        return Ok(Value::Bool(true));
    }
    let _ = str_arg(args, 0);
    Ok(Value::Bool(false))
}

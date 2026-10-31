//! Child processes and sockets: proc_open() and its family, fsockopen(),
//! stream_set_blocking() and stream_set_timeout(). A child's pipes and a
//! TCP connection are streams (p8_runtime::stream::Target), so fgets(),
//! fwrite() and fclose() work on them as on files.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/proc_open.c,
//! $PHP_SRC_PATH/ext/standard/fsock.c

use crate::file::{is_user, new_stream, stream_arg, with_user};
use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::stream::{PipeOut, Resource, Stream, Target, resource, resource_of};
use p8_runtime::string::PhpStr;
use p8_runtime::user_stream;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use p8_runtime::os::OsStrExt;
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
    Builtin { name: "stream_set_chunk_size", run: stream_set_chunk_size, by_ref: &[] },
    Builtin { name: "stream_select", run: stream_select, by_ref: &[0, 1, 2] },
    Builtin { name: "stream_set_timeout", run: stream_set_timeout, by_ref: &[] },
    Builtin { name: "stream_set_write_buffer", run: stream_set_write_buffer, by_ref: &[] },
    Builtin { name: "set_file_buffer", run: stream_set_write_buffer, by_ref: &[] },
    Builtin { name: "stream_set_read_buffer", run: stream_set_read_buffer, by_ref: &[] },
    Builtin { name: "stream_supports_lock", run: stream_supports_lock, by_ref: &[] },
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
            return Err(ctx.throw("ValueError", "proc_open(): Argument #2 ($descriptor_spec) must only contain arrays and streams".into()));
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
        Some(st) => (false, p8_runtime::os::exit_signal(&st).is_some(), p8_runtime::os::exit_signal(&st).map_or(0, i64::from), st.code().map_or(-1, i64::from)),
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
    if is_user(&o) {
        let mode = Value::Int(i64::from(enable));
        return Ok(Value::Bool(with_user(ctx, &o, false, |ctx, s| user_stream::set_option(ctx, "", s, user_stream::OPTION_BLOCKING, mode, None))?));
    }
    let r = resource(&o).expect("a stream");
    r.stream.borrow_mut().nonblocking = !enable;
    Ok(Value::Bool(true))
}

/// stream_set_chunk_size(resource $stream, int $size): the size before.
fn stream_set_chunk_size(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_set_chunk_size", args, 0)?;
    let size = arg(args, 1).to_int();
    if size <= 0 {
        return Err(ctx.throw("ValueError", "stream_set_chunk_size(): Argument #2 ($size) must be greater than 0".into()));
    }
    let r = resource(&o).expect("a stream");
    let old = std::mem::replace(&mut r.stream.borrow_mut().chunk_size, size);
    Ok(Value::Int(old))
}

/// stream_select(?array &$read, ?array &$write, ?array &$except, ?int
/// $seconds, ?int $microseconds = null): the arrays keep the streams that
/// are ready (writes always are; nothing is exceptional), waiting up to the
/// timeout (forever when null) for a read to be. Pipes are read on threads
/// and sockets peeked, so the wait is a poll every millisecond.
fn stream_select(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let list = |v: &Value| match v.deref() {
        Value::Array(a) => Some(a),
        _ => None,
    };
    let (read, write, except) = (args.first().and_then(list), args.get(1).and_then(list), args.get(2).and_then(list));
    if read.is_none() && write.is_none() && except.is_none() {
        return Err(ctx.throw("ValueError", "No stream arrays were passed".into()));
    }
    // A user-space stream is waited on through the stream its stream_cast
    // gives (asked for each place it is listed, as PHP builds its fd sets);
    // one that gives none is left out, as PHP cannot select on it.
    let mut stand_ins: Vec<(Rc<p8_runtime::object::Object>, Option<Value>)> = Vec::new();
    let mut usable = 0;
    for v in [&read, &write, &except].into_iter().flatten().flat_map(|a| a.values()) {
        let Some(o) = resource_of(v) else { continue };
        if !is_user(&o) {
            usable += 1;
            continue;
        }
        let cast = with_user(ctx, &o, None, |ctx, s| user_stream::cast(ctx, "stream_select", s, &o))?;
        if cast.is_none() {
            ctx.warn("stream_select(): Cannot represent a stream of type user-space as a select()able descriptor");
        } else {
            usable += 1;
        }
        stand_ins.retain(|(s, _)| !Rc::ptr_eq(s, &o));
        stand_ins.push((o, cast));
    }
    if usable == 0 && !stand_ins.is_empty() {
        return Err(ctx.throw("ValueError", "No stream arrays were passed".into()));
    }
    // What a value in the arrays is waited on as (None: left out).
    let waited = |v: &Value| -> Option<Value> {
        let o = resource_of(v)?;
        match stand_ins.iter().find(|(s, _)| Rc::ptr_eq(s, &o)) {
            Some((_, cast)) => cast.clone(),
            None => Some(v.clone()),
        }
    };
    let timeout = match args.get(3).map(Value::deref) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let seconds = v.to_int();
            let micros = args.get(4).map(Value::deref).map_or(0, |v| v.to_int());
            if seconds < 0 {
                return Err(ctx.throw("ValueError", "stream_select(): Argument #4 ($seconds) must be greater than or equal to 0".into()));
            }
            if micros < 0 {
                return Err(ctx.throw("ValueError", "stream_select(): Argument #5 ($microseconds) must be greater than or equal to 0".into()));
            }
            Some(Duration::from_secs(seconds as u64) + Duration::from_micros(micros as u64))
        }
    };
    let deadline = timeout.map(|t| std::time::Instant::now() + t);
    let mut writes = PhpArray::new();
    for (k, v) in write.iter().flat_map(|a| a.iter()).filter(|(_, v)| waited(v).is_some()) {
        writes.insert(k.clone(), v.clone());
    }
    let writable = writes.len();
    let ready_reads = loop {
        let mut ready = PhpArray::new();
        for (k, v) in read.iter().flat_map(|a| a.iter()) {
            let Some(o) = waited(v).as_ref().and_then(resource_of) else { continue };
            let Some(r) = resource(&o) else { continue };
            // A user-space stream with bytes read ahead stands in for itself.
            let ahead = r.stream.try_borrow().is_ok_and(|s| s.user().is_some() && !s.buffer.is_empty());
            if !r.closed.get() && (ahead || r.stream.try_borrow_mut().is_ok_and(|mut s| s.read_ready())) {
                ready.insert(k.clone(), v.clone());
            }
        }
        if !ready.is_empty() || writable > 0 || deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            break ready;
        }
        let pause = Duration::from_millis(1);
        std::thread::sleep(deadline.map_or(pause, |d| pause.min(d.saturating_duration_since(std::time::Instant::now()))));
    };
    // PHP asks each user-space stream it waited on again, finding its
    // descriptor among the ready ones.
    for v in [&read, &write, &except].into_iter().flatten().flat_map(|a| a.values()) {
        if let Some(o) = resource_of(v).filter(|o| is_user(o) && stand_ins.iter().any(|(s, c)| Rc::ptr_eq(s, o) && c.is_some())) {
            with_user(ctx, &o, None, |ctx, s| user_stream::cast(ctx, "stream_select", s, &o))?;
        }
    }
    let count = ready_reads.len() + writable;
    if let (Some(Value::Ref(cell)), Some(_)) = (args.first(), &read) {
        *cell.borrow_mut() = Value::Array(Rc::new(ready_reads));
    }
    if let (Some(Value::Ref(cell)), Some(_)) = (args.get(1), &write) {
        *cell.borrow_mut() = Value::Array(Rc::new(writes));
    }
    if let (Some(Value::Ref(cell)), Some(_)) = (args.get(2), &except) {
        *cell.borrow_mut() = Value::Array(Rc::new(PhpArray::new()));
    }
    Ok(Value::Int(count as i64))
}

/// stream_set_timeout(resource $stream, int $seconds, int $microseconds = 0)
fn stream_set_timeout(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_set_timeout", args, 0)?;
    if is_user(&o) {
        // The wrapper is told the seconds and microseconds as given.
        let seconds = arg(args, 1).to_int();
        let micros = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
        let set = with_user(ctx, &o, false, |ctx, s| user_stream::set_option(ctx, "", s, user_stream::OPTION_READ_TIMEOUT, Value::Int(seconds), Some(micros)))?;
        return Ok(Value::Bool(set));
    }
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

/// stream_set_write_buffer / stream_set_read_buffer for a user-space
/// stream: stream_set_option (PHP_STREAM_BUFFER_NONE 0 for size 0, else
/// PHP_STREAM_BUFFER_FULL 2 and the size); 0 when it agrees, else EOF (-1).
fn user_buffer(ctx: &mut dyn Ctx, o: &p8_runtime::object::Object, option: i64, size: i64) -> Result<Value, Throw> {
    let (mode, size) = if size == 0 { (0, None) } else { (2, Some(size)) };
    let set = with_user(ctx, o, false, |ctx, s| user_stream::set_option(ctx, "", s, option, Value::Int(mode), size))?;
    Ok(Value::Int(if set { 0 } else { -1 }))
}

/// stream_set_write_buffer(resource $stream, int $size) (and set_file_buffer):
/// 0 when the buffering was set. p8 writes through: a file opened by
/// descriptor refuses (EOF, -1), as PHP's does; the standard streams take it.
fn stream_set_write_buffer(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_set_write_buffer", args, 0)?;
    let size = arg(args, 1).to_int();
    if is_user(&o) {
        return user_buffer(ctx, &o, user_stream::OPTION_WRITE_BUFFER, size);
    }
    let r = resource(&o).expect("a stream");
    let standard = matches!(r.stream.borrow().target, Target::Stdin | Target::Stdout | Target::Stderr);
    Ok(Value::Int(if standard { 0 } else { -1 }))
}

/// stream_set_read_buffer(resource $stream, int $size): 0 (p8's streams
/// read what is asked for; a user-space wrapper is asked).
fn stream_set_read_buffer(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_set_read_buffer", args, 0)?;
    let size = arg(args, 1).to_int();
    if is_user(&o) {
        return user_buffer(ctx, &o, user_stream::OPTION_READ_BUFFER, size);
    }
    Ok(Value::Int(0))
}

/// stream_supports_lock(resource $stream): files and the standard streams
/// do, memory does not; a user-space wrapper is asked (stream_lock(0)).
fn stream_supports_lock(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_supports_lock", args, 0)?;
    if is_user(&o) {
        return Ok(Value::Bool(with_user(ctx, &o, false, |ctx, s| user_stream::lock(ctx, s, 0))?));
    }
    let r = resource(&o).expect("a stream");
    let lockable = matches!(r.stream.borrow().target, Target::File(_) | Target::Stdin | Target::Stdout | Target::Stderr);
    Ok(Value::Bool(lockable))
}

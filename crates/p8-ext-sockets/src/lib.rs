//! p8's sockets extension: the Socket class and the socket_* functions
//! over the standard library's TCP and Unix-domain sockets.
//!
//! A socket is created unbound; socket_bind on a stream socket binds and
//! listens at once (the standard library has no bound-but-not-listening
//! TCP socket), so socket_listen only checks it is bound, and its backlog
//! is the system's. Not implemented: datagram sockets, socket_select,
//! the stream import/export functions and multicast options; options are
//! remembered, not applied.
//!
//! Reference: $PHP_SRC_PATH/ext/sockets/sockets.c

#![forbid(unsafe_code)]

use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_FINAL, CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "sockets", register };

type R = Result<Value, Throw>;

const AF_UNIX: i64 = 1;
const AF_INET: i64 = 2;
#[cfg(target_os = "macos")]
const AF_INET6: i64 = 30;
#[cfg(not(target_os = "macos"))]
const AF_INET6: i64 = 10;
const SOCK_STREAM: i64 = 1;
const PHP_NORMAL_READ: i64 = 1;
const PHP_BINARY_READ: i64 = 2;

static CONSTANTS: &[(&str, i64)] = &[
    ("AF_UNIX", AF_UNIX),
    ("AF_INET", AF_INET),
    ("AF_INET6", AF_INET6),
    ("SOCK_STREAM", SOCK_STREAM),
    ("SOCK_DGRAM", 2),
    ("SOCK_RAW", 3),
    ("SOCK_SEQPACKET", 5),
    ("SOCK_RDM", 4),
    ("SOL_SOCKET", if cfg!(target_os = "macos") { 0xffff } else { 1 }),
    ("SOL_TCP", 6),
    ("SOL_UDP", 17),
    ("SO_REUSEADDR", if cfg!(target_os = "macos") { 4 } else { 2 }),
    ("SO_KEEPALIVE", if cfg!(target_os = "macos") { 8 } else { 9 }),
    ("SO_BROADCAST", if cfg!(target_os = "macos") { 0x20 } else { 6 }),
    ("SO_LINGER", if cfg!(target_os = "macos") { 0x80 } else { 13 }),
    ("SO_SNDBUF", if cfg!(target_os = "macos") { 0x1001 } else { 7 }),
    ("SO_RCVBUF", if cfg!(target_os = "macos") { 0x1002 } else { 8 }),
    ("SO_SNDTIMEO", if cfg!(target_os = "macos") { 0x1005 } else { 21 }),
    ("SO_RCVTIMEO", if cfg!(target_os = "macos") { 0x1006 } else { 20 }),
    ("SO_ERROR", if cfg!(target_os = "macos") { 0x1007 } else { 4 }),
    ("SO_TYPE", if cfg!(target_os = "macos") { 0x1008 } else { 3 }),
    ("TCP_NODELAY", 1),
    ("SOMAXCONN", 128),
    ("MSG_OOB", 1),
    ("MSG_PEEK", 2),
    ("MSG_WAITALL", if cfg!(target_os = "macos") { 0x40 } else { 0x100 }),
    ("MSG_DONTWAIT", if cfg!(target_os = "macos") { 0x80 } else { 0x40 }),
    ("PHP_NORMAL_READ", PHP_NORMAL_READ),
    ("PHP_BINARY_READ", PHP_BINARY_READ),
    ("SOCKET_EAGAIN", if cfg!(target_os = "macos") { 35 } else { 11 }),
    ("SOCKET_EWOULDBLOCK", if cfg!(target_os = "macos") { 35 } else { 11 }),
    ("SOCKET_EINTR", 4),
    ("SOCKET_EADDRINUSE", if cfg!(target_os = "macos") { 48 } else { 98 }),
    ("SOCKET_ECONNREFUSED", if cfg!(target_os = "macos") { 61 } else { 111 }),
    ("SOCKET_EINPROGRESS", if cfg!(target_os = "macos") { 36 } else { 115 }),
    ("SOCKET_ECONNRESET", if cfg!(target_os = "macos") { 54 } else { 104 }),
    ("SOCKET_ETIMEDOUT", if cfg!(target_os = "macos") { 60 } else { 110 }),
];

macro_rules! b {
    ($name:literal, $f:ident) => {
        Builtin { name: $name, run: $f, by_ref: &[] }
    };
    ($name:literal, $f:ident, $r:expr) => {
        Builtin { name: $name, run: $f, by_ref: $r }
    };
}

static BUILTINS: &[Builtin] = &[
    b!("socket_create", socket_create),
    b!("socket_create_listen", socket_create_listen),
    b!("socket_bind", socket_bind),
    b!("socket_listen", socket_listen),
    b!("socket_accept", socket_accept),
    b!("socket_connect", socket_connect),
    b!("socket_getsockname", socket_getsockname, &[1, 2]),
    b!("socket_getpeername", socket_getpeername, &[1, 2]),
    b!("socket_read", socket_read),
    b!("socket_write", socket_write),
    b!("socket_send", socket_send),
    b!("socket_recv", socket_recv, &[1]),
    b!("socket_shutdown", socket_shutdown),
    b!("socket_close", socket_close),
    b!("socket_set_option", socket_set_option),
    b!("setsockopt", socket_set_option),
    b!("socket_get_option", socket_get_option),
    b!("getsockopt", socket_get_option),
    b!("socket_set_nonblock", socket_set_nonblock),
    b!("socket_set_block", socket_set_block),
    b!("socket_last_error", socket_last_error),
    b!("socket_clear_error", socket_clear_error),
    b!("socket_strerror", socket_strerror),
];

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "Socket",
    parent: None,
    interfaces: &[],
    flags: CLASS_FINAL | CLASS_NOT_SERIALIZABLE | CLASS_UNCLONEABLE,
    properties: &[],
    methods: &[],
    constants: &[],
    debug_info: None,
}];

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_data("Socket", || Rc::new(Sock::default()));
    for (name, value) in CONSTANTS {
        registry.add_constant(name.as_bytes(), Constant::Int(*value));
    }
    registry.add_module("sockets");
}

thread_local! {
    /// socket_last_error() without a socket: the last error of any.
    static LAST_ERROR: Cell<i64> = const { Cell::new(0) };
}

#[derive(Default)]
enum State {
    #[default]
    Unbound,
    TcpListener(TcpListener),
    Tcp(TcpStream),
    UnixListener(UnixListener),
    Unix(UnixStream),
    Closed,
}

/// A Socket's native state.
#[derive(Default)]
struct Sock {
    domain: Cell<i64>,
    kind: Cell<i64>,
    state: RefCell<State>,
    error: Cell<i64>,
    nonblocking: Cell<bool>,
    options: RefCell<HashMap<(i64, i64), Value>>,
}

// ---- helpers ----

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

fn text(v: &Value) -> String {
    String::from_utf8_lossy(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned()
}

fn set_ref(args: &[Value], i: usize, v: Value) {
    if let Some(r) = args.get(i).and_then(Value::as_reference) {
        *r.borrow_mut() = v;
    }
}

/// strerror(3) text, without std's " (os error N)".
fn strerror(code: i64) -> String {
    let s = std::io::Error::from_raw_os_error(code as i32).to_string();
    match s.find(" (os error") {
        Some(at) => s[..at].to_string(),
        None => s,
    }
}

fn sock_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value], i: usize) -> Result<Rc<Object>, Throw> {
    let v = arg(args, i);
    match v.as_object() {
        Some(o) if o.native::<Sock>().is_some() => {
            if matches!(*o.native::<Sock>().expect("a Socket").state.borrow(), State::Closed) {
                return Err(ctx.throw("Error", format!("{func}(): Argument #{} ($socket) has already been closed", i + 1)));
            }
            Ok(Rc::clone(o))
        }
        _ => Err(ctx.throw("TypeError", format!("{func}(): Argument #{} ($socket) must be of type Socket, {} given", i + 1, v.type_name()))),
    }
}

fn sock(o: &Object) -> &Sock {
    o.native::<Sock>().expect("a Socket")
}

/// Record `e` on the socket and warn as PHP does ("socket_bind(): Unable
/// to bind address [48]: Address already in use").
fn failed(ctx: &mut dyn Ctx, s: &Sock, what: &str, e: &std::io::Error) -> Value {
    let code = i64::from(e.raw_os_error().unwrap_or(0));
    s.error.set(code);
    LAST_ERROR.with(|l| l.set(code));
    ctx.warn(&format!("{what} [{code}]: {}", strerror(code)));
    Value::FALSE
}

fn new_socket(ctx: &mut dyn Ctx, domain: i64, kind: i64, state: State) -> Value {
    let class = ctx.class(b"Socket").expect("Socket");
    let o = ctx.new_object(&class);
    let s = Sock::default();
    s.domain.set(domain);
    s.kind.set(kind);
    *s.state.borrow_mut() = state;
    Value::object(o.sibling(class, Some(Rc::new(s))))
}

fn inet_addr(host: &str, port: i64) -> std::io::Result<SocketAddr> {
    let port = u16::try_from(port).map_err(|_| std::io::Error::from_raw_os_error(22))?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (host, port).to_socket_addrs()?.next().ok_or_else(|| std::io::Error::from_raw_os_error(49))
}

fn would_block(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::WouldBlock
}

// ---- functions ----

fn socket_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (domain, kind) = (arg(args, 0).to_int(), arg(args, 1).to_int());
    if ![AF_UNIX, AF_INET, AF_INET6].contains(&domain) {
        return Err(ctx.throw("ValueError", "socket_create(): Argument #1 ($domain) must be one of AF_UNIX, AF_INET6, or AF_INET".into()));
    }
    if kind != SOCK_STREAM {
        ctx.warn("socket_create(): Unable to create socket [43]: Protocol not supported");
        return Ok(Value::FALSE);
    }
    Ok(new_socket(ctx, domain, kind, State::Unbound))
}

fn socket_create_listen(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let port = arg(args, 0).to_int();
    match u16::try_from(port).map_err(|_| std::io::Error::from_raw_os_error(22)).and_then(|p| TcpListener::bind(("0.0.0.0", p))) {
        Ok(l) => Ok(new_socket(ctx, AF_INET, SOCK_STREAM, State::TcpListener(l))),
        Err(e) => Ok(failed(ctx, &Sock::default(), "socket_create_listen(): unable to bind to given address", &e)),
    }
}

fn socket_bind(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_bind", args, 0)?;
    let s = sock(&o);
    let address = text(&arg(args, 1));
    let bound = if s.domain.get() == AF_UNIX {
        UnixListener::bind(&address).map(State::UnixListener)
    } else {
        inet_addr(&address, if args.len() > 2 { arg(args, 2).to_int() } else { 0 }).and_then(TcpListener::bind).map(State::TcpListener)
    };
    match bound {
        Ok(state) => {
            *s.state.borrow_mut() = state;
            Ok(Value::TRUE)
        }
        Err(e) => Ok(failed(ctx, s, "socket_bind(): Unable to bind address", &e)),
    }
}

fn socket_listen(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_listen", args, 0)?;
    let s = sock(&o);
    let listening = matches!(*s.state.borrow(), State::TcpListener(_) | State::UnixListener(_));
    if !listening {
        return Ok(failed(ctx, s, "socket_listen(): Unable to listen on socket", &std::io::Error::from_raw_os_error(22)));
    }
    Ok(Value::TRUE)
}

fn socket_accept(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_accept", args, 0)?;
    let s = sock(&o);
    let accepted = match &*s.state.borrow() {
        State::TcpListener(l) => l.accept().map(|(c, _)| State::Tcp(c)),
        State::UnixListener(l) => l.accept().map(|(c, _)| State::Unix(c)),
        _ => Err(std::io::Error::from_raw_os_error(22)),
    };
    match accepted {
        Ok(state) => Ok(new_socket(ctx, s.domain.get(), s.kind.get(), state)),
        Err(e) if would_block(&e) => {
            s.error.set(i64::from(e.raw_os_error().unwrap_or(35)));
            Ok(Value::FALSE)
        }
        Err(e) => Ok(failed(ctx, s, "socket_accept(): unable to accept incoming connection", &e)),
    }
}

fn socket_connect(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_connect", args, 0)?;
    let s = sock(&o);
    let address = text(&arg(args, 1));
    let connected = if s.domain.get() == AF_UNIX {
        UnixStream::connect(&address).map(State::Unix)
    } else {
        if args.len() < 3 || arg(args, 2).is_null() {
            return Err(ctx.throw("ValueError", "socket_connect(): Argument #3 ($port) cannot be null when the socket type is AF_INET".into()));
        }
        inet_addr(&address, arg(args, 2).to_int()).and_then(TcpStream::connect).map(State::Tcp)
    };
    match connected {
        Ok(state) => {
            if let State::Tcp(t) = &state {
                let _ = t.set_nonblocking(s.nonblocking.get());
            }
            *s.state.borrow_mut() = state;
            Ok(Value::TRUE)
        }
        Err(e) => Ok(failed(ctx, s, "socket_connect(): unable to connect", &e)),
    }
}

fn name_of(ctx: &mut dyn Ctx, args: &[Value], func: &str, peer: bool) -> R {
    let o = sock_arg(ctx, func, args, 0)?;
    let s = sock(&o);
    let inet = |a: std::io::Result<SocketAddr>| a.map(|a| (a.ip().to_string(), Some(i64::from(a.port()))));
    let found = match &*s.state.borrow() {
        State::TcpListener(l) if !peer => inet(l.local_addr()),
        State::Tcp(t) => inet(if peer { t.peer_addr() } else { t.local_addr() }),
        State::UnixListener(l) if !peer => l.local_addr().map(|a| (a.as_pathname().map(|p| p.display().to_string()).unwrap_or_default(), None)),
        State::Unix(u) => (if peer { u.peer_addr() } else { u.local_addr() }).map(|a| (a.as_pathname().map(|p| p.display().to_string()).unwrap_or_default(), None)),
        _ => Err(std::io::Error::from_raw_os_error(if peer { 57 } else { 22 })),
    };
    match found {
        Ok((address, port)) => {
            set_ref(args, 1, Value::str(PhpStr::from(address.as_str())));
            if let Some(port) = port {
                set_ref(args, 2, Value::int(port));
            }
            Ok(Value::TRUE)
        }
        Err(e) => Ok(failed(ctx, s, &format!("{func}(): unable to retrieve {} name", if peer { "peer" } else { "socket" }), &e)),
    }
}

fn socket_getsockname(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    name_of(ctx, args, "socket_getsockname", false)
}

fn socket_getpeername(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    name_of(ctx, args, "socket_getpeername", true)
}

/// Read up to `len` bytes from a connected socket (one line at most with
/// `line`).
fn read_some(s: &Sock, len: usize, line: bool) -> std::io::Result<Vec<u8>> {
    let mut state = s.state.borrow_mut();
    let stream: &mut dyn Read = match &mut *state {
        State::Tcp(t) => t,
        State::Unix(u) => u,
        _ => return Err(std::io::Error::from_raw_os_error(57)),
    };
    if !line {
        let mut buf = vec![0; len];
        let n = stream.read(&mut buf)?;
        buf.truncate(n);
        return Ok(buf);
    }
    let mut out = Vec::new();
    let mut byte = [0u8];
    while out.len() < len {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                out.push(byte[0]);
                if byte[0] == b'\n' || byte[0] == b'\r' {
                    break;
                }
            }
            Err(e) if out.is_empty() => return Err(e),
            Err(_) => break,
        }
    }
    Ok(out)
}

fn socket_read(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_read", args, 0)?;
    let s = sock(&o);
    let len = arg(args, 1).to_int();
    if len < 1 {
        return Err(ctx.throw("ValueError", "socket_read(): Argument #2 ($length) must be greater than 0".into()));
    }
    let line = args.len() > 2 && arg(args, 2).to_int() == PHP_NORMAL_READ;
    match read_some(s, len as usize, line) {
        Ok(b) => Ok(Value::str(PhpStr::from(&b[..]))),
        Err(e) if would_block(&e) => {
            s.error.set(i64::from(e.raw_os_error().unwrap_or(35)));
            Ok(Value::FALSE)
        }
        Err(e) => Ok(failed(ctx, s, "socket_read(): unable to read from socket", &e)),
    }
}

fn write_some(s: &Sock, data: &[u8]) -> std::io::Result<usize> {
    let mut state = s.state.borrow_mut();
    let stream: &mut dyn Write = match &mut *state {
        State::Tcp(t) => t,
        State::Unix(u) => u,
        _ => return Err(std::io::Error::from_raw_os_error(57)),
    };
    stream.write(data)
}

fn send(ctx: &mut dyn Ctx, args: &[Value], func: &str, len_at: usize) -> R {
    let o = sock_arg(ctx, func, args, 0)?;
    let s = sock(&o);
    let data = arg(args, 1).to_php_string().unwrap_or_else(PhpStr::empty);
    let mut bytes = data.as_bytes();
    if let Some(v) = args.get(len_at).map(Value::deref).filter(|v| !v.is_null()) {
        let n = v.to_int().max(0) as usize;
        bytes = &bytes[..n.min(bytes.len())];
    }
    match write_some(s, bytes) {
        Ok(n) => Ok(Value::int(n as i64)),
        Err(e) => Ok(failed(ctx, s, &format!("{func}(): unable to write to socket"), &e)),
    }
}

fn socket_write(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    send(ctx, args, "socket_write", 2)
}

fn socket_send(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    send(ctx, args, "socket_send", 2)
}

fn socket_recv(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_recv", args, 0)?;
    let s = sock(&o);
    let len = arg(args, 2).to_int().max(1) as usize;
    match read_some(s, len, false) {
        Ok(b) => {
            let n = b.len() as i64;
            set_ref(args, 1, if b.is_empty() { Value::NULL } else { Value::str(PhpStr::from(&b[..])) });
            Ok(Value::int(n))
        }
        Err(e) => {
            set_ref(args, 1, Value::NULL);
            Ok(failed(ctx, s, "socket_recv(): unable to read from socket", &e))
        }
    }
}

fn socket_shutdown(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_shutdown", args, 0)?;
    let s = sock(&o);
    let how = match if args.len() > 1 { arg(args, 1).to_int() } else { 2 } {
        0 => std::net::Shutdown::Read,
        1 => std::net::Shutdown::Write,
        _ => std::net::Shutdown::Both,
    };
    let done = match &*s.state.borrow() {
        State::Tcp(t) => t.shutdown(how),
        State::Unix(u) => u.shutdown(how),
        _ => Err(std::io::Error::from_raw_os_error(57)),
    };
    match done {
        Ok(()) => Ok(Value::TRUE),
        Err(e) => Ok(failed(ctx, s, "socket_shutdown(): Unable to shutdown socket", &e)),
    }
}

fn socket_close(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_close", args, 0)?;
    *sock(&o).state.borrow_mut() = State::Closed;
    Ok(Value::NULL)
}

fn socket_set_option(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_set_option", args, 0)?;
    let s = sock(&o);
    let (level, name) = (arg(args, 1).to_int(), arg(args, 2).to_int());
    if name == 1
        && level == 6
        && let State::Tcp(t) = &*s.state.borrow()
    {
        let _ = t.set_nodelay(arg(args, 3).to_bool());
    }
    s.options.borrow_mut().insert((level, name), arg(args, 3));
    Ok(Value::TRUE)
}

fn socket_get_option(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = sock_arg(ctx, "socket_get_option", args, 0)?;
    let s = sock(&o);
    let (level, name) = (arg(args, 1).to_int(), arg(args, 2).to_int());
    let so_type = if cfg!(target_os = "macos") { 0x1008 } else { 3 };
    let so_error = if cfg!(target_os = "macos") { 0x1007 } else { 4 };
    if name == so_type {
        return Ok(Value::int(s.kind.get()));
    }
    if name == so_error {
        return Ok(Value::int(s.error.get()));
    }
    Ok(s.options.borrow().get(&(level, name)).cloned().unwrap_or(Value::int(0)))
}

fn blocking(ctx: &mut dyn Ctx, args: &[Value], func: &str, nonblocking: bool) -> R {
    let o = sock_arg(ctx, func, args, 0)?;
    let s = sock(&o);
    s.nonblocking.set(nonblocking);
    let _ = match &*s.state.borrow() {
        State::TcpListener(l) => l.set_nonblocking(nonblocking),
        State::Tcp(t) => t.set_nonblocking(nonblocking),
        State::UnixListener(l) => l.set_nonblocking(nonblocking),
        State::Unix(u) => u.set_nonblocking(nonblocking),
        _ => Ok(()),
    };
    Ok(Value::TRUE)
}

fn socket_set_nonblock(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    blocking(ctx, args, "socket_set_nonblock", true)
}

fn socket_set_block(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    blocking(ctx, args, "socket_set_block", false)
}

fn socket_last_error(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    if args.is_empty() || arg(args, 0).is_null() {
        return Ok(Value::int(LAST_ERROR.with(Cell::get)));
    }
    let o = sock_arg(ctx, "socket_last_error", args, 0)?;
    Ok(Value::int(sock(&o).error.get()))
}

fn socket_clear_error(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    if args.is_empty() || arg(args, 0).is_null() {
        LAST_ERROR.with(|l| l.set(0));
        return Ok(Value::NULL);
    }
    let o = sock_arg(ctx, "socket_clear_error", args, 0)?;
    sock(&o).error.set(0);
    Ok(Value::NULL)
}

fn socket_strerror(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::str(PhpStr::from(strerror(arg(args, 0).to_int()).as_str())))
}

//! `php -S`: PHP's built-in web server (sapi/cli/php_cli_server.c). One
//! request runs at a time, as in PHP: connections are read on their own
//! threads (so a client that connects and says nothing blocks no one), and
//! the requests they produce run one after the other on the thread that
//! owns the isolate. Every response closes its connection.
//!
//! `-S <addr> --stdio <client>` serves one request the same way, read from
//! stdin with its response written to stdout: how a host without sockets
//! (a WASI runtime) hands a connection's bytes over.

use crate::http::{self, HttpRequest, Parse};
use crate::mime_types::MIME_TYPES;
use crate::status_phrases::STATUS_PHRASES;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// php_cli_server_log_level: 1 process, 2 errors, 3 everything (default).
static LOG_LEVEL: AtomicU8 = AtomicU8::new(3);
const LOG_PROCESS: u8 = 1;
const LOG_ERROR: u8 = 2;
const LOG_MESSAGE: u8 = 3;

pub struct Config {
    pub address: String,
    pub docroot: Option<String>,
    pub router: Option<String>,
    /// `-q` given this many times.
    pub quiet: u8,
    /// `-d name=value` settings, in order.
    pub ini: Vec<(String, String)>,
    /// `p8 serve`: the project served (the document root) from its
    /// package; none for `php -S`.
    pub project: Option<Project>,
}

/// What `p8 serve` adds to the built-in server: the project's package,
/// which files are taken from (compiled ahead by `p8 compile`, for this
/// server's engine), and a log that says how many files each request
/// compiled from source.
pub struct Project {
    /// The package (`<dir>/.p8/package`); missing or not usable, the
    /// project is served from source, and stderr says so once.
    pub package: PathBuf,
}

impl Project {
    /// `platform` taking compiled files from the package, when it can.
    /// The default check (each file's source hashed and compared with the
    /// package's) is the one a server needs: it reads each source anyway,
    /// and an edited or new file runs its new code (#382, #383).
    fn attach(&self, platform: p8::Platform) -> p8::Platform {
        let shown = self.package.display();
        match platform.open_package(&self.package, p8::PackageCheck::Hash) {
            Ok(package) => return platform.with_package(package),
            Err(p8::PackageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("p8: there is no compiled package at {shown}, so every file is compiled from source (`p8 compile` writes it).");
            }
            Err(p8::PackageError::Io(e)) => {
                eprintln!("p8: the package at {shown} could not be read ({}), so every file is compiled from source.", strerror(&e));
            }
            Err(p8::PackageError::OtherEngine) => {
                eprintln!("p8: the package at {shown} was written by another build of p8 or for other extensions, so every file is compiled from source (`p8 compile` writes it again).");
            }
            Err(e) => {
                eprintln!("p8: the package at {shown} cannot be used ({e}), so every file is compiled from source (`p8 compile` writes it again).");
            }
        }
        platform
    }
}

/// What `php -S` sets that differs from the CLI's own defaults: the CLI
/// hardcodes html_errors=0 and max_execution_time=0; the server keeps
/// PHP's (HTML errors, 30 seconds).
const SERVER_INI: &[(&str, &str)] = &[("html_errors", "1"), ("max_execution_time", "30")];

/// "Thu Oct 29 06:47:19 2026" (asctime) for a time in seconds, in UTC.
fn asctime(secs: u64) -> String {
    let (y, mo, d, h, mi, s, wd) = civil(secs);
    format!("{} {} {:2} {:02}:{:02}:{:02} {}", DAYS[wd], MONTHS[mo - 1], d, h, mi, s, y)
}

/// "Thu, 29 Oct 2026 06:47:19 GMT"
fn http_date(secs: u64) -> String {
    let (y, mo, d, h, mi, s, wd) = civil(secs);
    format!("{}, {:02} {} {} {:02}:{:02}:{:02} GMT", DAYS[wd], d, MONTHS[mo - 1], y, h, mi, s)
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// (year, month, day, hour, minute, second, weekday with Sunday 0) in UTC.
fn civil(secs: u64) -> (i64, usize, i64, u64, u64, u64, usize) {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let wd = ((days + 4).rem_euclid(7)) as usize;
    (y, m as usize, d, rem / 3600, rem / 60 % 60, rem % 60, wd)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// sapi_cli_server_log_write(): `[date] message` on stderr when the level allows.
fn log(level: u8, message: &str) {
    if LOG_LEVEL.load(Ordering::Relaxed) < level {
        return;
    }
    let _ = writeln!(std::io::stderr(), "[{}] {}", asctime(now_secs()), message);
}

/// strerror() text of an I/O error (std appends " (os error N)").
fn strerror(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.find(" (os error") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

/// php_cli_server_parse_addr(): `host:port` or `[host]:port`.
fn parse_addr(addr: &str) -> Option<(String, u16)> {
    let (host, rest) = if let Some(inner) = addr.strip_prefix('[') {
        let end = inner.find(']')?;
        let rest = inner[end + 1..].strip_prefix(':')?;
        (inner[..end].to_string(), rest)
    } else {
        let colon = addr.find(':')?;
        (addr[..colon].to_string(), &addr[colon + 1..])
    };
    // strtol(): a number, nothing after it.
    let digits = rest.strip_prefix(['-', '+']).unwrap_or(rest);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let port: i64 = rest.parse().ok()?;
    if !(0..=65535).contains(&port) {
        return None;
    }
    Some((host, port as u16))
}

/// A connection with a request read off it (no stream: stdin and stdout).
struct Incoming {
    stream: Option<TcpStream>,
    peer: String,
    request: Box<HttpRequest>,
}

/// The limit the watchdog reports when it fires.
static LIMIT_SECONDS: AtomicU32 = AtomicU32::new(0);

/// Stops the request that runs past max_execution_time, and leaves one
/// that finished alone.
struct Watchdog {
    state: Arc<(Mutex<Option<Instant>>, Condvar)>,
}

impl Watchdog {
    /// Where there are no threads (WASI) nothing watches, and the limit is
    /// not kept, as in a PHP built without timers.
    fn start(handle: p8::InterruptHandle) -> Watchdog {
        let state: Arc<(Mutex<Option<Instant>>, Condvar)> = Arc::new((Mutex::new(None), Condvar::new()));
        let shared = Arc::clone(&state);
        let _ = std::thread::Builder::new().spawn(move || {
            let (lock, cv) = &*shared;
            let mut deadline = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                match *deadline {
                    None => deadline = cv.wait(deadline).unwrap_or_else(|e| e.into_inner()),
                    Some(at) => {
                        let now = Instant::now();
                        if now >= at {
                            *deadline = None;
                            handle.timeout(u64::from(LIMIT_SECONDS.load(Ordering::Relaxed)));
                        } else {
                            deadline = cv.wait_timeout(deadline, at - now).unwrap_or_else(|e| e.into_inner()).0;
                        }
                    }
                }
            }
        });
        Watchdog { state }
    }

    /// Stop the request in `seconds` (none: not at all) unless disarmed first.
    fn arm(&self, seconds: u64) {
        let (lock, cv) = &*self.state;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = (seconds > 0).then(|| Instant::now() + Duration::from_secs(seconds));
        cv.notify_all();
    }
}

/// What a request needs from its connection while the script runs: the
/// response head waits until the first output or the end, so a router
/// that declines can still discard it.
struct Conn {
    out: std::io::BufWriter<Box<dyn Write>>,
    host: Option<Vec<u8>>,
    head_only: bool,
    protocol: u32,
    pending: Option<Vec<u8>>,
    sent: bool,
}

impl Conn {
    /// "HTTP/1.1 200 OK\r\n"
    fn status_line(protocol: u32, code: i64) -> String {
        let code = if code == 0 { 200 } else { code };
        format!("HTTP/{}.{} {} {}\r\n", protocol / 100, protocol % 100, code, status_phrase(code))
    }

    /// The headers every response starts with (append_essential_headers()).
    fn essential(&self, has_date: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(host) = &self.host {
            out.extend_from_slice(b"Host: ");
            out.extend_from_slice(host);
            out.extend_from_slice(b"\r\n");
        }
        if !has_date {
            out.extend_from_slice(format!("Date: {}\r\n", http_date(now_secs())).as_bytes());
        }
        out.extend_from_slice(b"Connection: close\r\n");
        out
    }

    fn send_pending(&mut self) {
        if let Some(head) = self.pending.take() {
            let _ = self.out.write_all(&head);
            self.sent = true;
        }
    }

    /// The end of the response: a head nobody sent yet goes now.
    fn finish(&mut self) {
        self.send_pending();
        let _ = self.out.flush();
    }
}

impl p8::Host for Conn {
    fn write(&mut self, bytes: &[u8]) {
        self.send_pending();
        if !self.head_only {
            let _ = self.out.write_all(bytes);
        }
    }

    fn flush(&mut self) {
        self.send_pending();
        let _ = self.out.flush();
    }

    fn head(&mut self, response: &p8::Response) {
        if self.sent {
            return;
        }
        let mut out = match &response.status_line {
            Some(line) => {
                let mut l = line.clone();
                l.extend_from_slice(b"\r\n");
                l
            }
            None => Conn::status_line(self.protocol, response.status).into_bytes(),
        };
        let has_date = response.headers.iter().any(|h| h.len() > 5 && h[..5].eq_ignore_ascii_case(b"Date:"));
        out.extend_from_slice(&self.essential(has_date));
        for h in response.headers.iter().filter(|h| !h.is_empty()) {
            out.extend_from_slice(h);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        self.pending = Some(out);
    }
}

fn status_phrase(code: i64) -> &'static str {
    STATUS_PHRASES.iter().find(|(c, _)| i64::from(*c) == code).map_or("Unknown Status Code", |(_, p)| p)
}

/// normalize_vpath(): percent-decode, collapse slashes, resolve . and ..
fn normalize_vpath(raw: &[u8]) -> Vec<u8> {
    // php_raw_url_decode
    let mut s = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() && raw[i + 1].is_ascii_hexdigit() && raw[i + 2].is_ascii_hexdigit() {
            let h = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            s.push(h(raw[i + 1]) << 4 | h(raw[i + 2]));
            i += 3;
        } else {
            s.push(raw[i]);
            i += 1;
        }
    }
    let mut end = s.len();
    let mut p = 0;
    if p < end && s[p] == b'/' {
        let mut n = p;
        while n < end && s[n] == b'/' {
            n += 1;
        }
        p += 1;
        s.copy_within(n..end, p);
        end -= n - p;
    }
    while p < end {
        let mut n = p;
        while n < end && s[n] != b'/' {
            n += 1;
        }
        if n - p == 2 && s[p] == b'.' && s[p + 1] == b'.' {
            if p > 0 {
                p -= 1;
                loop {
                    if p == 0 {
                        if s[p] == b'/' {
                            p += 1;
                        }
                        break;
                    }
                    p -= 1;
                    if s[p] == b'/' {
                        p += 1;
                        break;
                    }
                }
            }
            while n < end && s[n] == b'/' {
                n += 1;
            }
            s.copy_within(n..end, p);
            end -= n - p;
        } else if n - p == 1 && s[p] == b'.' {
            while n < end && s[n] == b'/' {
                n += 1;
            }
            s.copy_within(n..end, p);
            end -= n - p;
        } else if n < end {
            let mut nn = n;
            while nn < end && s[nn] == b'/' {
                nn += 1;
            }
            p = n + 1;
            s.copy_within(nn..end, p);
            end -= nn - p;
        } else {
            p = n;
        }
    }
    s.truncate(end);
    s
}

/// What a request's path resolves to under the document root.
#[derive(Default)]
struct Resolved {
    /// The path after resolving (an index file appended): SCRIPT_NAME.
    vpath: Vec<u8>,
    /// The file it names, if it names one.
    path_translated: Option<Vec<u8>>,
    path_info: Option<Vec<u8>>,
    size: u64,
}

fn to_path(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;
    #[cfg(target_os = "wasi")]
    use std::os::wasi::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

/// php_cli_server_request_translate_vpath(): the file a path names, found by
/// walking up until something exists; what was left over is PATH_INFO. A
/// directory means its index.php or index.html.
fn translate_vpath(docroot: &[u8], vpath: Vec<u8>) -> Resolved {
    let mut buf = docroot.to_vec();
    let start = buf.len();
    if !vpath.is_empty() {
        if vpath[0] != b'/' {
            buf.push(b'/');
        }
        buf.extend_from_slice(&vpath);
    }
    let full = buf.len();
    let mut q = full;
    let mut prev: Option<Vec<u8>> = None;
    let mut size = 0;
    while q > 0 {
        if let Ok(m) = std::fs::metadata(to_path(&buf[..q])) {
            if m.is_dir() {
                let mut dir = buf[..q].to_vec();
                if dir.last() != Some(&b'/') {
                    dir.push(b'/');
                }
                let mut found = None;
                for index in ["index.php", "index.html"] {
                    let mut candidate = dir.clone();
                    candidate.extend_from_slice(index.as_bytes());
                    if let Ok(im) = std::fs::metadata(to_path(&candidate))
                        && im.is_file()
                    {
                        found = Some((candidate, im.len()));
                        break;
                    }
                }
                let Some((candidate, len)) = found else {
                    return Resolved { vpath, ..Resolved::default() };
                };
                buf = candidate;
                q = buf.len();
                size = len;
            } else {
                size = m.len();
            }
            break;
        }
        // One component up; the rest is path info.
        loop {
            if q == 0 {
                break;
            }
            q -= 1;
            if buf[q] == b'/' {
                break;
            }
        }
        prev = Some(buf[q..full].to_vec());
    }
    let q = q.max(start);
    Resolved { vpath: buf[start..q].to_vec(), path_translated: Some(buf[..q].to_vec()), path_info: prev, size }
}

/// The extension a path ends in, the way the server takes it (after the last '.').
fn extension(vpath: &[u8]) -> &[u8] {
    match vpath.iter().rposition(|&c| c == b'.') {
        Some(i) => &vpath[i + 1..],
        None => &vpath[vpath.len()..],
    }
}

/// htmlspecialchars(ENT_QUOTES, double_encode=false) of a request URI; text
/// that is not UTF-8 gives nothing.
fn html_escape(uri: &[u8]) -> String {
    let Ok(s) = std::str::from_utf8(uri) else { return String::new() };
    let mut out = String::new();
    for (i, c) in s.char_indices() {
        match c {
            '&' if !is_entity(&s.as_bytes()[i..]) => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#039;"),
            c => out.push(c),
        }
    }
    out
}

/// Whether `s` starts with a complete entity (`&amp;`, `&#38;`, `&#x26;`):
/// double_encode=0 leaves those alone.
fn is_entity(s: &[u8]) -> bool {
    let Some(semi) = s.iter().position(|&c| c == b';') else { return false };
    match &s[1..semi] {
        [] => false,
        [b'#', b'x' | b'X', hex @ ..] => !hex.is_empty() && hex.iter().all(u8::is_ascii_hexdigit),
        [b'#', dec @ ..] => !dec.is_empty() && dec.iter().all(u8::is_ascii_digit),
        name => name.iter().all(u8::is_ascii_alphanumeric),
    }
}

const ERROR_CSS: &str = "<style>\nbody { background-color: #fcfcfc; color: #333333; margin: 0; padding:0; }\nh1 { font-size: 1.5em; font-weight: normal; background-color: #9999cc; min-height:2em; line-height:2em; border-bottom: 1px inset black; margin: 0; }\nh1, p { padding-left: 10px; }\ncode.url { background-color: #eeeeee; font-family:monospace; padding:0 2px;}\n</style>\n";

/// The address and the document root of `config`; none (after saying why)
/// when either is wrong.
fn address_and_docroot(config: &Config) -> Option<(String, u16, String)> {
    LOG_LEVEL.store(3u8.saturating_sub(config.quiet).max(1), Ordering::Relaxed);
    let Some((host, port)) = parse_addr(&config.address) else {
        eprintln!("Invalid address: {}", config.address);
        return None;
    };
    let docroot = match &config.docroot {
        Some(dir) => {
            let Ok(meta) = std::fs::metadata(dir) else {
                eprintln!("Directory {dir} does not exist.");
                return None;
            };
            if !meta.is_dir() {
                eprintln!("{dir} is not a directory.");
                return None;
            }
            p8::canonicalize(dir).map_or(dir.clone(), |p| p.to_string_lossy().into_owned())
        }
        None => std::env::current_dir().map_or(".".into(), |p| p.to_string_lossy().into_owned()),
    };
    Some((host, port, docroot))
}

/// The engine and the server's state, for `serve` to run requests with.
fn with_server(config: &Config, host: String, port: u16, docroot: String, serve: impl FnOnce(&mut Server, &mut p8::Isolate)) {
    let platform = p8::Platform::new(&crate::extensions(true))
        // Each file is compiled once, and again only when its source changes.
        .keep_compiled();
    let platform = match &config.project {
        Some(project) => project.attach(platform),
        None => platform,
    };
    let mut ini: Vec<(String, String)> = SERVER_INI.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect();
    ini.extend(config.ini.iter().cloned());
    // `p8 serve`: the package and the files kept by their source are its
    // cache (APS 8: by path and content), so an edited file runs its new
    // code at the next request; OPcache's rules (a file checked by its
    // time every revalidate_freq seconds) are php -S's only.
    let platform = match config.project {
        Some(_) => platform,
        None => {
            let platform = crate::file_cache(platform, &ini, false);
            // php -S is not the CLI: OPcache runs with opcache.enable alone, as in PHP.
            crate::opcache(platform, &ini, false)
        }
    };
    let mut isolate = platform.new_isolate();
    let limit = ini.iter().rev().find(|(n, _)| n == "max_execution_time").and_then(|(_, v)| v.trim().parse::<u32>().ok()).unwrap_or(0);
    LIMIT_SECONDS.store(limit, Ordering::Relaxed);
    let watchdog = Watchdog::start(isolate.interrupt_handle());
    let counts = config.project.is_some();
    let mut server = Server { platform: &platform, host, port, docroot, router: config.router.clone(), ini, watchdog, limit: u64::from(limit), counts };
    serve(&mut server, &mut isolate);
}

/// `php -S` for one request read from stdin, as from a connection from
/// `client`; the response goes to stdout. 1 when no request could be read.
pub fn run_stdio(config: Config, client: String) -> i32 {
    let Some((host, port, docroot)) = address_and_docroot(&config) else { return 1 };
    let mut buf = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut buf) {
        log(LOG_ERROR, &format!("{client} Invalid request ({})", strerror(&e)));
        return 1;
    }
    let request = match http::parse(&buf) {
        Parse::Complete(request) => request,
        Parse::Incomplete => {
            if buf.iter().all(|&c| c == b'\r' || c == b'\n') {
                log(LOG_MESSAGE, &format!("{client} Closed without sending a request; it was probably just an unused speculative preconnection"));
            } else {
                log(LOG_ERROR, &format!("{client} Invalid request (Unexpected EOF)"));
            }
            return 1;
        }
        Parse::Malformed => {
            let why = if buf[0] & 0x80 != 0 || buf[0] == 0x16 { "Unsupported SSL request" } else { "Malformed HTTP request" };
            log(LOG_ERROR, &format!("{client} Invalid request ({why})"));
            return 1;
        }
    };
    log(LOG_MESSAGE, &format!("{client} Accepted"));
    with_server(&config, host, port, docroot, |server, isolate| server.handle(isolate, Incoming { stream: None, peer: client, request }));
    0
}

pub fn run(config: Config) -> i32 {
    let Some((host, port, docroot)) = address_and_docroot(&config) else { return 1 };
    // php_network_listen_socket(): the first address that binds.
    let shown = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    let addrs: Vec<SocketAddr> = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(e) => {
            log(LOG_ERROR, &format!("Failed to listen on {shown} (reason: {})", strerror(&e)));
            return 1;
        }
    };
    let mut listener = None;
    let mut last_error = None;
    for addr in &addrs {
        match TcpListener::bind(addr) {
            Ok(l) => {
                listener = Some(l);
                break;
            }
            Err(e) => last_error = Some(e),
        }
    }
    let Some(listener) = listener else {
        let reason = last_error.map_or_else(|| "?".to_string(), |e| strerror(&e));
        log(LOG_ERROR, &format!("Failed to listen on {shown} (reason: {reason})"));
        return 1;
    };
    let port = listener.local_addr().map_or(port, |a| a.port());
    let shown = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    match &config.project {
        Some(_) => log(LOG_PROCESS, &format!("p8 {} serving {docroot} at http://{shown}", p8::P8_VERSION)),
        None => log(LOG_PROCESS, &format!("PHP {} Development Server (http://{shown}) started", p8::PHP_VERSION)),
    }

    let (tx, rx) = mpsc::channel::<Incoming>();
    std::thread::spawn(move || accept_loop(listener, tx));

    with_server(&config, host, port, docroot, |server, isolate| {
        for incoming in rx {
            server.handle(isolate, incoming);
        }
    });
    0
}

/// Accept connections; each is read on a thread of its own.
fn accept_loop(listener: TcpListener, tx: mpsc::Sender<Incoming>) {
    loop {
        let (stream, addr) = match listener.accept() {
            Ok(c) => c,
            Err(e) => {
                log(LOG_ERROR, &format!("Failed to accept a client (reason: {})", strerror(&e)));
                continue;
            }
        };
        let peer = addr.to_string();
        log(LOG_MESSAGE, &format!("{peer} Accepted"));
        let tx = tx.clone();
        std::thread::spawn(move || read_request(stream, peer, tx));
    }
}

/// php_cli_server_client_read_request(): bytes in until a request is whole.
fn read_request(mut stream: TcpStream, peer: String, tx: mpsc::Sender<Incoming>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16383];
    loop {
        let n = match stream.read(&mut chunk) {
            Ok(n) => n,
            Err(e) => {
                log(LOG_ERROR, &format!("{peer} Invalid request ({})", strerror(&e)));
                close(&peer);
                return;
            }
        };
        if n == 0 {
            if buf.iter().all(|&c| c == b'\r' || c == b'\n') {
                log(LOG_MESSAGE, &format!("{peer} Closed without sending a request; it was probably just an unused speculative preconnection"));
            } else {
                log(LOG_ERROR, &format!("{peer} Invalid request (Unexpected EOF)"));
            }
            close(&peer);
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        match http::parse(&buf) {
            Parse::Complete(request) => {
                let _ = tx.send(Incoming { stream: Some(stream), peer, request });
                return;
            }
            Parse::Incomplete => {}
            Parse::Malformed => {
                let why = if buf[0] & 0x80 != 0 || buf[0] == 0x16 { "Unsupported SSL request" } else { "Malformed HTTP request" };
                log(LOG_ERROR, &format!("{peer} Invalid request ({why})"));
                close(&peer);
                return;
            }
        }
    }
}

fn close(peer: &str) {
    log(LOG_MESSAGE, &format!("{peer} Closing"));
}


struct Server<'p> {
    platform: &'p p8::Platform,
    host: String,
    port: u16,
    docroot: String,
    router: Option<String>,
    ini: Vec<(String, String)>,
    watchdog: Watchdog,
    limit: u64,
    /// `p8 serve`: each request's log line says how many files it
    /// compiled from source (none, for a project served from its package).
    counts: bool,
}

/// How a script run ended.
struct Ran {
    /// The router said false: what it left of the request variables.
    declined: Option<p8::Carried>,
    status: i64,
    /// " - message in file on line N" when it ended with a fatal error.
    fatal: Option<String>,
}

/// Why a script could not be run.
enum CompileFailure {
    Source(p8::Error),
    Read,
}

impl Server<'_> {
    /// The compiled form of `path` (the platform keeps what it compiled
    /// while the source is the same).
    fn compile(&mut self, path: &Path) -> Result<Rc<p8::Script>, CompileFailure> {
        let source = std::fs::read(path).map_err(|_| CompileFailure::Read)?;
        let file = p8::canonicalize(path).map_or_else(|_| path.to_string_lossy().into_owned(), |p| p.to_string_lossy().into_owned());
        Ok(Rc::new(self.platform.compile(&source, &file).map_err(CompileFailure::Source)?))
    }

    /// $_SERVER as sapi_cli_server_register_variables() builds it.
    fn p8_request(&self, incoming: &Incoming, resolved: &Resolved, router_used: bool) -> p8::Request {
        let req = &incoming.request;
        let mut server: Vec<(Vec<u8>, Vec<u8>, bool)> = Vec::new();
        let mut known = |name: &str, value: &[u8]| server.push((name.as_bytes().to_vec(), value.to_vec(), true));
        known("DOCUMENT_ROOT", self.docroot.as_bytes());
        // addr_str is "ip:port" or "[ip]:port".
        let peer = incoming.peer.as_str();
        match peer.rfind(':') {
            Some(i) => {
                let addr = peer[..i].trim_start_matches('[').trim_end_matches(']');
                known("REMOTE_ADDR", addr.as_bytes());
                known("REMOTE_PORT", &peer.as_bytes()[i + 1..]);
            }
            None => known("REMOTE_ADDR", peer.as_bytes()),
        }
        known("SERVER_SOFTWARE", format!("PHP/{} (Development Server)", p8::PHP_VERSION).as_bytes());
        known("SERVER_PROTOCOL", format!("HTTP/{}.{}", req.protocol / 100, req.protocol % 100).as_bytes());
        known("SERVER_NAME", self.host.as_bytes());
        known("SERVER_PORT", self.port.to_string().as_bytes());
        known("REQUEST_URI", &req.request_uri);
        known("REQUEST_METHOD", req.method.as_bytes());
        known("SCRIPT_NAME", &resolved.vpath);
        if let Some(p) = &resolved.path_translated {
            known("SCRIPT_FILENAME", p);
        } else if let (true, Some(router)) = (router_used, &self.router) {
            known("SCRIPT_FILENAME", router.as_bytes());
        }
        if let Some(info) = &resolved.path_info {
            known("PATH_INFO", info);
        }
        let info = resolved.path_info.as_deref().unwrap_or_default();
        known("PHP_SELF", &[resolved.vpath.as_slice(), info].concat());
        if let Some(q) = &req.query {
            known("QUERY_STRING", q);
        }
        for (name, value) in &req.headers {
            let key: Vec<u8> = name.iter().map(|&c| if c == b'-' { b'_' } else { c.to_ascii_uppercase() }).collect();
            if key == b"CONTENT_TYPE" || key == b"CONTENT_LENGTH" {
                server.push((key.clone(), value.clone(), false));
            }
            server.push(([b"HTTP_".as_slice(), &key].concat(), value.clone(), false));
        }
        p8::Request {
            method: req.method.to_string(),
            query: req.query.clone(),
            headers: req.headers.clone(),
            headers_original: req.headers_original.clone(),
            server,
            body: req.body.clone(),
        }
    }

    /// Run a PHP file as a request of its own (the router, or the target).
    fn execute(&mut self, isolate: &mut p8::Isolate, conn: &mut Conn, file: &Path, request: &p8::Request, chdir: bool, carried: Option<p8::Carried>) -> Ran {
        let script = self.compile(file);
        if chdir && let Some(dir) = file.parent() {
            let _ = std::env::set_current_dir(if dir.as_os_str().is_empty() { Path::new(".") } else { dir });
        }
        self.watchdog.arm(self.limit);
        let mut context = isolate.new_context(conn);
        for (name, value) in &self.ini {
            context.set_ini(name, value);
        }
        context.serve(request, carried);
        match script {
            Ok(script) => {
                context.run(&script);
            }
            Err(CompileFailure::Source(error)) => {
                context.fail(&error);
            }
            // PHP's "Failed opening required" is not reproduced: nothing runs.
            Err(CompileFailure::Read) => {}
        }
        self.watchdog.arm(0);
        let declined = context.declined();
        let status = context.status();
        const FATAL: i64 = 1 | 4 | 16 | 64 | 256 | 4096;
        let fatal = context.last_error().filter(|(level, ..)| level & FATAL != 0).map(|(_, m, f, l)| format!(" - {m} in {f} on line {l}"));
        Ran { declined, status, fatal }
    }

    fn handle(&mut self, isolate: &mut p8::Isolate, incoming: Incoming) {
        let peer = incoming.peer.clone();
        let write_half: Box<dyn Write> = match &incoming.stream {
            Some(stream) => {
                let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
                let Ok(write_half) = stream.try_clone() else {
                    close(&peer);
                    return;
                };
                Box::new(write_half)
            }
            None => Box::new(std::io::stdout()),
        };
        let request = &incoming.request;
        let mut conn = Conn {
            out: std::io::BufWriter::with_capacity(16 * 1024, write_half),
            host: request.header("host").map(<[u8]>::to_vec),
            head_only: request.method == "HEAD",
            protocol: request.protocol,
            pending: None,
            sent: false,
        };
        self.dispatch(isolate, &incoming, &mut conn);
        conn.finish();
        // Said before the connection closes, as PHP's server does: the
        // client may connect again as soon as it is closed, and that
        // connection's "Accepted" comes after this one's "Closing".
        close(&peer);
        if let Some(stream) = &incoming.stream {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }

    /// php_cli_server_log_response()
    fn log_response(&self, incoming: &Incoming, status: i64, message: Option<&str>, fatal: Option<&str>) {
        let req = &incoming.request;
        let uri = String::from_utf8_lossy(&req.request_uri);
        let mut line = format!("{} [{}]: {} {}", incoming.peer, status, req.method, uri);
        if let Some(m) = message {
            line.push_str(" - ");
            line.push_str(m);
        }
        if let Some(f) = fatal {
            line.push_str(f);
        }
        log(LOG_MESSAGE, &line);
    }

    /// php_cli_server_dispatch()
    fn dispatch(&mut self, isolate: &mut p8::Isolate, incoming: &Incoming, conn: &mut Conn) {
        let req = &incoming.request;
        if req.not_implemented {
            self.error_page(incoming, conn, 501);
            return;
        }
        let compiled = self.platform.compiled_files();
        let vpath = normalize_vpath(req.path.as_deref().unwrap_or_default());
        let resolved = translate_vpath(self.docroot.as_bytes(), vpath);
        let ext = extension(&resolved.vpath).to_ascii_lowercase();
        let is_static = ext != b"php" || resolved.path_translated.is_none();
        let mut carried = None;
        if let Some(router) = self.router.clone() {
            let p8req = self.p8_request(incoming, &resolved, true);
            let ran = self.execute(isolate, conn, Path::new(&router), &p8req, false, None);
            if ran.declined.is_none() {
                return;
            }
            // The router said false: serve the request itself. A head it
            // built is dropped.
            conn.pending = None;
            carried = ran.declined;
        }
        if is_static {
            self.send_static(incoming, conn, &resolved, &ext);
            return;
        }
        // A PHP script.
        let Some(path) = resolved.path_translated.clone() else { return };
        if path.contains(&0) {
            self.error_page(incoming, conn, 400);
            return;
        }
        let p8req = self.p8_request(incoming, &resolved, false);
        let ran = self.execute(isolate, conn, &to_path(&path), &p8req, true, carried);
        let counted = self.counts.then(|| match self.platform.compiled_files() - compiled {
            1 => "1 file compiled from source".to_string(),
            n => format!("{n} files compiled from source"),
        });
        self.log_response(incoming, ran.status, counted.as_deref(), ran.fatal.as_deref());
    }

    /// php_cli_server_begin_send_static()
    fn send_static(&mut self, incoming: &Incoming, conn: &mut Conn, resolved: &Resolved, ext: &[u8]) {
        let req = &incoming.request;
        if matches!(req.method, "DELETE" | "PUT" | "PATCH") {
            self.error_page(incoming, conn, 405);
            return;
        }
        let Some(path) = &resolved.path_translated else {
            self.error_page(incoming, conn, 404);
            return;
        };
        if path.contains(&0) {
            self.error_page(incoming, conn, 400);
            return;
        }
        let Ok(mut file) = std::fs::File::open(to_path(path)) else {
            self.error_page(incoming, conn, 404);
            return;
        };
        let mime = std::str::from_utf8(ext).ok().and_then(|e| MIME_TYPES.iter().find(|(x, _)| *x == e)).map(|(_, m)| *m);
        let mut head = Conn::status_line(req.protocol, 200).into_bytes();
        head.extend_from_slice(&conn.essential(false));
        if let Some(mime) = mime {
            head.extend_from_slice(format!("Content-Type: {mime}{}\r\n", if mime.starts_with("text/") { "; charset=UTF-8" } else { "" }).as_bytes());
        }
        head.extend_from_slice(format!("Content-Length: {}\r\n\r\n", resolved.size).as_bytes());
        self.log_response(incoming, 200, None, None);
        let _ = conn.out.write_all(&head);
        conn.sent = true;
        if req.method != "HEAD" {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match file.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if conn.out.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// php_cli_server_send_error_page()
    fn error_page(&mut self, incoming: &Incoming, conn: &mut Conn, status: i64) {
        let req = &incoming.request;
        let phrase = status_phrase(status);
        let mut body = String::new();
        if req.method != "HEAD" {
            let uri = html_escape(&req.request_uri);
            let content = match status {
                400 => format!("<h1>{phrase}</h1><p>Your browser sent a request that this server could not understand.</p>"),
                404 => format!("<h1>{phrase}</h1><p>The requested resource <code class=\"url\">{uri}</code> was not found on this server.</p>"),
                405 => format!("<h1>{phrase}</h1><p>Requested method not allowed.</p>"),
                500 => format!("<h1>{phrase}</h1><p>The server is temporarily unavailable.</p>"),
                _ => format!("<h1>{phrase}</h1><p>Request method not supported.</p>"),
            };
            body = format!("<!doctype html><html><head><title>{status} {phrase}</title>{ERROR_CSS}</head><body>{content}</body></html>");
        }
        let mut head = Conn::status_line(req.protocol, status).into_bytes();
        head.extend_from_slice(&conn.essential(false));
        head.extend_from_slice(format!("X-Powered-By: {}\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: {}\r\n", p8::powered_by(), body.len()).as_bytes());
        if status == 405 {
            head.extend_from_slice(b"Allow: GET, HEAD, POST\r\n");
        }
        head.extend_from_slice(b"\r\n");
        self.log_response(incoming, status, Some("No such file or directory"), None);
        let _ = conn.out.write_all(&head);
        let _ = conn.out.write_all(body.as_bytes());
        conn.sent = true;
    }
}

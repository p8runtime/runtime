//! curl_exec(): one transfer, through rsurl. The request is built here
//! (rsurl sends only the headers given, `strict_headers`) so the headers are
//! libcurl's own and CURLINFO_HEADER_OUT can show them; redirects are
//! followed here, hop by hop, so every hop's headers reach the callbacks and
//! the counts are libcurl's.

use crate::info::Info;
use crate::{HeaderSink, Req, Sink, Source, State, bytes_of, c, form, handle, lossy};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::stream::{resource, resource_of};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use rsurl::{HttpVersionPref, Request, ResponseHead};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A CURLE_* code and the message curl_error() gives.
pub(crate) struct CurlErr {
    pub code: i64,
    pub message: String,
}

fn err(code: i64, message: impl Into<String>) -> CurlErr {
    CurlErr { code, message: message.into() }
}

const CURLAUTH_BASIC: i64 = 1;
const CURLAUTH_DIGEST: i64 = 2;

fn opt_int(st: &State, option: i64) -> Option<i64> {
    st.opts.get(&option).map(Value::to_int)
}

fn opt_str(st: &State, option: i64) -> Option<String> {
    match st.opts.get(&option) {
        Some(Value::Str(s)) => Some(lossy(s.as_bytes())),
        _ => None,
    }
}

fn opt_list(st: &State, option: i64) -> Vec<String> {
    match st.opts.get(&option) {
        Some(Value::Array(a)) => a.values().map(|v| lossy(&bytes_of(v))).collect(),
        _ => Vec::new(),
    }
}

/// What the callbacks share while a transfer runs.
struct Run<'a> {
    cx: RefCell<&'a mut dyn Ctx>,
    this: Rc<Object>,
    include_header: bool,
    /// The whole transfer's limit (CURLOPT_TIMEOUT).
    deadline: Option<(Instant, Duration)>,
    nobody: bool,
    follow: bool,
    fail_on_error: bool,
    status: RefCell<RunStatus>,
}

#[derive(Default)]
struct RunStatus {
    /// Why the transfer stops (a callback's refusal, FAILONERROR).
    failure: Option<CurlErr>,
    /// A callback threw.
    thrown: Option<Throw>,
    /// CURLOPT_RETURNTRANSFER's buffer.
    body: Vec<u8>,
    size_download: i64,
    header_size: i64,
    /// This hop is a redirect that will be followed: its body is dropped.
    redirecting: bool,
    /// Where the hop's Location points, absolute.
    location: Option<String>,
    status_code: i64,
    content_type: Option<String>,
    content_length: i64,
    version: String,
}

impl Run<'_> {
    fn aborted(&self) -> bool {
        let s = self.status.borrow();
        s.failure.is_some() || s.thrown.is_some()
    }

    fn fail(&self, code: i64, message: impl Into<String>) {
        let mut s = self.status.borrow_mut();
        if s.failure.is_none() {
            s.failure = Some(err(code, message));
        }
    }

    /// Call a user callback with the handle first; its result, or None when
    /// it threw (the exception is kept to end the transfer with).
    fn call(&self, callable: Value, mut rest: Vec<Value>) -> Option<Value> {
        let mut args = vec![Value::Object(Rc::clone(&self.this))];
        args.append(&mut rest);
        let h = handle(&self.this);
        let was = h.in_callback.replace(true);
        let r = self.cx.borrow_mut().call(&callable, &mut args);
        // A callback that replaced itself is freed here, its destructor
        // still inside the guard.
        drop(callable);
        drop(args);
        h.in_callback.set(was);
        match r {
            Ok(v) => Some(v.deref()),
            Err(t) => {
                self.status.borrow_mut().thrown = Some(t);
                None
            }
        }
    }

    fn write_stream(&self, stream: &Value, data: &[u8]) -> bool {
        let Some(r) = resource_of(stream) else { return false };
        let Some(res) = resource(&r) else { return false };
        let mut cx = self.cx.borrow_mut();
        let written = res.stream.borrow_mut().write(&mut **cx, data);
        match written {
            Ok(n) => n == Some(data.len()),
            // A user-space wrapper's stream_write threw.
            Err(t) => {
                self.status.borrow_mut().thrown = Some(t);
                false
            }
        }
    }

    /// The body (or, with CURLOPT_HEADER, header) bytes to the write sink.
    fn emit_body(&self, data: &[u8]) {
        if data.is_empty() || self.aborted() {
            return;
        }
        let sink = handle(&self.this).st.borrow().write.clone();
        match sink {
            Sink::Stdout => self.cx.borrow_mut().echo(data),
            Sink::Return => self.status.borrow_mut().body.extend_from_slice(data),
            Sink::File(v) => {
                if !self.write_stream(&v, data) {
                    self.fail(23, "Failure writing output to destination");
                }
            }
            Sink::User(f) => {
                let Some(ret) = self.call(f, vec![Value::Str(PhpStr::new(data.to_vec()))]) else { return };
                let n = ret.to_int();
                if n != data.len() as i64 {
                    self.fail(23, format!("Failure writing output to destination, passed {} returned {}", data.len(), n));
                }
            }
        }
    }

    fn emit_header(&self, line: &[u8]) {
        if self.aborted() {
            return;
        }
        let header = handle(&self.this).st.borrow().header.clone();
        match header {
            HeaderSink::Ignore => {}
            HeaderSink::File(v) => {
                if !self.write_stream(&v, line) {
                    self.fail(23, "Failed writing header");
                }
            }
            HeaderSink::User(f) => {
                let Some(ret) = self.call(f, vec![Value::Str(PhpStr::new(line.to_vec()))]) else { return };
                if ret.to_int() != line.len() as i64 {
                    self.fail(23, "Failed writing header");
                }
            }
        }
        let debug = handle(&self.this).st.borrow().debug.clone();
        if let Some(d) = debug {
            // CURLINFO_HEADER_IN
            self.call(d, vec![Value::Int(1), Value::Str(PhpStr::new(line.to_vec()))]);
        }
    }

    fn on_head(&self, head: &ResponseHead, request_url: &str) {
        let mut block = Vec::new();
        let reason = if head.reason.is_empty() { String::new() } else { format!(" {}", head.reason) };
        let status_line = if head.version.starts_with("HTTP/2") || head.version.starts_with("HTTP/3") { format!("{} {} \r\n", head.version.trim_end_matches(".0"), head.status) } else { format!("{} {}{}\r\n", head.version, head.status, reason) };
        let mut lines = vec![status_line.into_bytes()];
        for (k, v) in &head.headers {
            lines.push(format!("{k}: {v}\r\n").into_bytes());
        }
        lines.push(b"\r\n".to_vec());
        {
            let mut s = self.status.borrow_mut();
            s.status_code = head.status as i64;
            s.version = head.version.clone();
            s.content_type = head.header("content-type").map(str::to_string);
            s.content_length = head.header("content-length").and_then(|v| v.trim().parse().ok()).unwrap_or(-1);
            s.location = None;
            s.redirecting = false;
            if matches!(head.status, 301 | 302 | 303 | 307 | 308)
                && let Some(loc) = head.header("location")
            {
                s.location = Some(resolve(request_url, loc.trim()));
                s.redirecting = self.follow;
            }
        }
        for l in &lines {
            block.extend_from_slice(l);
        }
        self.status.borrow_mut().header_size += block.len() as i64;
        if self.fail_on_error && head.status >= 400 {
            self.fail(22, format!("The requested URL returned error: {}", head.status));
        }
        for l in &lines {
            self.emit_header(l);
        }
        if self.include_header {
            self.emit_body(&block);
        }
    }

    fn on_chunk(&self, data: &[u8]) -> rsurl::Result<()> {
        if let Some((start, limit)) = self.deadline
            && start.elapsed() > limit
        {
            let received = self.status.borrow().size_download;
            self.fail(28, format!("Operation timed out after {} milliseconds with {received} bytes received", limit.as_millis()));
        }
        if self.aborted() {
            return Err(rsurl::Error::Cancelled);
        }
        let skip = {
            let s = self.status.borrow();
            s.redirecting || self.nobody
        };
        if skip {
            return Ok(());
        }
        self.status.borrow_mut().size_download += data.len() as i64;
        self.emit_body(data);
        if self.aborted() { Err(rsurl::Error::Cancelled) } else { Ok(()) }
    }
}

/// RFC 3986 reference resolution, enough for Location headers.
pub(crate) fn resolve(base: &str, loc: &str) -> String {
    if loc.contains("://") {
        return loc.to_string();
    }
    let Some((scheme, rest)) = base.split_once("://") else { return loc.to_string() };
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path_q) = rest.split_at(auth_end);
    if let Some(l) = loc.strip_prefix("//") {
        return format!("{scheme}://{l}");
    }
    let path_q = path_q.split('#').next().unwrap_or("");
    let (path, query) = match path_q.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_q, None),
    };
    let path = if path.is_empty() { "/" } else { path };
    let loc = loc.split('#').next().unwrap_or("");
    if loc.is_empty() {
        return format!("{scheme}://{authority}{path_q}");
    }
    if let Some(q) = loc.strip_prefix('?') {
        return format!("{scheme}://{authority}{path}?{q}");
    }
    let _ = query;
    let merged = if loc.starts_with('/') { loc.to_string() } else { format!("{}{}", &path[..path.rfind('/').map_or(0, |i| i + 1)], loc) };
    let (mpath, mquery) = match merged.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (merged.clone(), None),
    };
    let mut out: Vec<&str> = Vec::new();
    for seg in mpath.split('/').skip(1) {
        match seg {
            "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    let mut p = format!("/{}", out.join("/"));
    if (mpath.ends_with("/.") || mpath.ends_with("/..")) && !p.ends_with('/') {
        p.push('/');
    }
    match mquery {
        Some(q) => format!("{scheme}://{authority}{p}?{q}"),
        None => format!("{scheme}://{authority}{p}"),
    }
}

/// What a failed rsurl call is, in libcurl's terms.
fn map_error(e: &rsurl::Error, host: &str, port: u16, elapsed: Duration, timeout: Option<Duration>, received: i64, proxy: Option<&str>) -> CurlErr {
    use std::io::ErrorKind;
    match e {
        rsurl::Error::InvalidUrl(_) => err(3, "URL rejected: Malformed input to a URL function"),
        rsurl::Error::UnsupportedScheme(s) => err(1, format!("Protocol \"{s}\" not supported")),
        rsurl::Error::UnexpectedEof => err(52, "Empty reply from server"),
        rsurl::Error::BadResponse(m) => {
            if let Some(n) = m.strip_prefix("maximum (").and_then(|r| r.split(')').next()) {
                err(47, format!("Maximum ({n}) redirects followed"))
            } else {
                err(8, format!("Weird server reply: {m}"))
            }
        }
        rsurl::Error::Cancelled => err(42, "Operation was aborted by an application callback"),
        rsurl::Error::Status { code, .. } => err(22, format!("The requested URL returned error: {code}")),
        rsurl::Error::Io(io) => {
            let msg = io.to_string();
            let low = msg.to_ascii_lowercase();
            if low.contains("lookup address") || low.contains("name or service not known") || low.contains("nodename nor servname") || low.contains("no address associated") || low.contains("no addresses to connect") {
                return match proxy {
                    Some(p) => err(5, format!("Could not resolve proxy: {p}")),
                    None => err(6, format!("Could not resolve host: {host}")),
                };
            }
            match io.kind() {
                ErrorKind::ConnectionRefused | ErrorKind::AddrNotAvailable | ErrorKind::NetworkUnreachable | ErrorKind::HostUnreachable => err(7, format!("Failed to connect to {host} port {port} after {} ms: Could not connect to server", elapsed.as_millis())),
                ErrorKind::TimedOut | ErrorKind::WouldBlock => match timeout {
                    Some(t) => err(28, format!("Operation timed out after {} milliseconds with {received} bytes received", t.as_millis())),
                    None => err(28, format!("Connection timed out after {} milliseconds", elapsed.as_millis())),
                },
                ErrorKind::UnexpectedEof => err(52, "Empty reply from server"),
                ErrorKind::ConnectionReset | ErrorKind::BrokenPipe | ErrorKind::ConnectionAborted => err(56, "Recv failure: Connection reset by peer"),
                _ if low.contains("cert") || low.contains("verif") || low.contains("untrusted") || low.contains("unknown issuer") => err(60, format!("SSL certificate problem: {msg}")),
                _ if low.contains("tls") || low.contains("ssl") || low.contains("handshake") => err(35, format!("SSL connect error: {msg}")),
                _ => err(56, format!("Recv failure: {msg}")),
            }
        }
        other => err(56, format!("Recv failure: {other}")),
    }
}

/// libcurl's header assembly for one request, in its order.
struct Prepared {
    request: Request,
    /// The request as sent, for CURLINFO_HEADER_OUT.
    text: String,
}

struct Hop<'a> {
    st: &'a State,
    url: &'a str,
    method: &'a str,
    body: &'a [u8],
    content_type: Option<&'a str>,
    /// Credentials go along (same host as the first request, or unrestricted).
    auth: bool,
    referer: Option<&'a str>,
}

fn prepare(hop: &Hop) -> Result<Prepared, CurlErr> {
    let st = hop.st;
    let parsed = rsurl::Url::parse(hop.url).map_err(|_| err(3, "URL rejected: Malformed input to a URL function"))?;
    let default_port = if parsed.is_tls() { 443 } else { 80 };
    let host_header = if parsed.port == default_port { parsed.host.clone() } else { format!("{}:{}", parsed.host, parsed.port) };
    let custom: Vec<String> = opt_list(st, c::CURLOPT_HTTPHEADER);
    // "Name:" removes a default header; "Name;" sends an empty one.
    let mut removed: Vec<String> = Vec::new();
    let mut given: Vec<(String, String)> = Vec::new();
    for line in &custom {
        if let Some((name, value)) = line.split_once(':') {
            if value.trim().is_empty() {
                removed.push(name.trim().to_ascii_lowercase());
            } else {
                given.push((name.trim().to_string(), value.trim_start().to_string()));
            }
        } else if let Some(name) = line.strip_suffix(';') {
            given.push((name.trim().to_string(), String::new()));
        }
    }
    let has = |n: &str| given.iter().any(|(k, _)| k.eq_ignore_ascii_case(n)) || removed.iter().any(|r| r == n);
    let mut headers: Vec<(String, String)> = Vec::new();
    // Authorization
    if hop.auth {
        let (user, pass) = credentials(st, &parsed);
        if let Some(user) = user
            && !has("authorization")
        {
            let methods = opt_int(st, c::CURLOPT_HTTPAUTH).unwrap_or(CURLAUTH_BASIC);
            if methods & CURLAUTH_BASIC != 0 {
                headers.push(("Authorization".into(), format!("Basic {}", base64(format!("{user}:{pass}").as_bytes()))));
            }
        }
        if let Some(token) = opt_str(st, c::CURLOPT_XOAUTH2_BEARER)
            && !has("authorization")
        {
            headers.push(("Authorization".into(), format!("Bearer {token}")));
        }
    }
    if let Some(range) = opt_str(st, c::CURLOPT_RANGE)
        && !has("range")
    {
        headers.push(("Range".into(), format!("bytes={range}")));
    }
    if let Some(ua) = opt_str(st, c::CURLOPT_USERAGENT)
        && !has("user-agent")
    {
        headers.push(("User-Agent".into(), ua));
    }
    if !has("accept") {
        headers.push(("Accept".into(), "*/*".into()));
    }
    let encoding = opt_str(st, c::CURLOPT_ACCEPT_ENCODING);
    if let Some(e) = &encoding
        && !has("accept-encoding")
    {
        headers.push(("Accept-Encoding".into(), if e.is_empty() { "deflate, gzip, br, zstd".to_string() } else { e.clone() }));
    }
    if let Some(r) = hop.referer.map(str::to_string).or_else(|| opt_str(st, c::CURLOPT_REFERER))
        && !has("referer")
    {
        headers.push(("Referer".into(), r));
    }
    if let Some(cookie) = opt_str(st, c::CURLOPT_COOKIE)
        && !cookie.is_empty()
        && !has("cookie")
    {
        headers.push(("Cookie".into(), cookie));
    }
    let skip_ct = has("content-type");
    headers.extend(given.iter().cloned());
    let is_body_method = ["POST", "PUT", "PATCH"].contains(&hop.method.to_ascii_uppercase().as_str());
    let has_body = !hop.body.is_empty() || is_body_method;
    if has_body && !has("content-length") && !has("transfer-encoding") {
        headers.push(("Content-Length".into(), hop.body.len().to_string()));
    }
    if let Some(ct) = hop.content_type
        && !skip_ct
        && has_body
    {
        headers.push(("Content-Type".into(), ct.to_string()));
    }

    let mut request = Request::new(hop.method, hop.url).map_err(|_| err(3, "URL rejected: Malformed input to a URL function"))?;
    request = request.strict_headers(true).keep_method_case(true);
    let mut text = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", hop.method, parsed.path, host_header);
    for (k, v) in &headers {
        request = request.header(k, v);
        text.push_str(&format!("{k}: {v}\r\n"));
    }
    text.push_str("\r\n");
    if !hop.body.is_empty() {
        request = request.body(hop.body.to_vec());
    }
    Ok(Prepared { request: apply_options(request, st, encoding.is_some())?, text })
}

fn credentials(st: &State, url: &rsurl::Url) -> (Option<String>, String) {
    if let Some(up) = opt_str(st, c::CURLOPT_USERPWD) {
        return match up.split_once(':') {
            Some((u, p)) => (Some(u.to_string()), p.to_string()),
            None => (Some(up), String::new()),
        };
    }
    if let Some(u) = opt_str(st, c::CURLOPT_USERNAME) {
        return (Some(u), opt_str(st, c::CURLOPT_PASSWORD).unwrap_or_default());
    }
    match &url.userinfo {
        Some(ui) => match ui.split_once(':') {
            Some((u, p)) => (Some(u.to_string()), p.to_string()),
            None => (Some(ui.clone()), String::new()),
        },
        None => (None, String::new()),
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |a, (i, b)| a | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The options that configure the transport rather than the message.
fn apply_options(mut req: Request, st: &State, encoding: bool) -> Result<Request, CurlErr> {
    let ms = |option_ms: i64, option_s: i64| -> Option<Duration> {
        let v = opt_int(st, option_ms).map(|n| n.max(0) as u64).filter(|n| *n > 0).map(Duration::from_millis);
        v.or_else(|| opt_int(st, option_s).filter(|n| *n > 0).map(|n| Duration::from_secs(n as u64)))
    };
    // rsurl checks max_time only between requests, so the same limit also
    // bounds each read: a stalled transfer ends on time (on_chunk bounds a slow one).
    let mut read_limit = None;
    if let Some(t) = ms(c::CURLOPT_TIMEOUT_MS, c::CURLOPT_TIMEOUT) {
        req = req.max_time(t);
        read_limit = Some(t);
    }
    if let Some(t) = ms(c::CURLOPT_CONNECTTIMEOUT_MS, c::CURLOPT_CONNECTTIMEOUT) {
        req = req.connect_timeout(t);
    }
    req = req.read_timeout(read_limit).decompress(encoding);
    if opt_int(st, c::CURLOPT_SSL_VERIFYPEER).unwrap_or(1) == 0 {
        req = req.verify_tls(false);
    }
    if let Some(p) = opt_str(st, c::CURLOPT_CAINFO).filter(|p| !p.is_empty()) {
        req = req.ca_bundle(&p);
    }
    if let Some(p) = opt_str(st, c::CURLOPT_CAPATH).filter(|p| !p.is_empty()) {
        req = req.ca_path(&p);
    }
    if let Some(p) = opt_str(st, c::CURLOPT_SSLCERT).filter(|p| !p.is_empty()) {
        req = req.client_cert(&p);
    }
    if let Some(p) = opt_str(st, c::CURLOPT_SSLKEY).filter(|p| !p.is_empty()) {
        req = req.client_key(&p);
    }
    if let Some(p) = opt_str(st, c::CURLOPT_SSLKEYPASSWD) {
        req = req.client_key_pass(&p);
    }
    if let Some(p) = opt_str(st, c::CURLOPT_PINNEDPUBLICKEY).filter(|p| !p.is_empty()) {
        req = req.pinned_pubkey(&p);
    }
    if let Some(p) = opt_str(st, c::CURLOPT_SSL_CIPHER_LIST).filter(|p| !p.is_empty()) {
        req = req.ciphers(&p);
    }
    req = match opt_int(st, c::CURLOPT_HTTP_VERSION).unwrap_or(0) {
        1 | 2 => req.http11_only(),
        5 => req.http2_only(),
        30 => req.http_version(HttpVersionPref::Http3),
        31 => req.http_version(HttpVersionPref::Http3Only),
        _ => req,
    };
    req = match opt_int(st, c::CURLOPT_IPRESOLVE).unwrap_or(0) {
        1 => req.ipv4(),
        2 => req.ipv6(),
        _ => req,
    };
    for entry in opt_list(st, c::CURLOPT_RESOLVE) {
        // host:port:address[,address]
        let mut parts = entry.splitn(3, ':');
        if let (Some(h), Some(p), Some(a)) = (parts.next(), parts.next(), parts.next())
            && let (Ok(port), Some(ip)) = (p.parse::<u16>(), a.split(',').next().and_then(|a| a.trim_matches(['[', ']', '+']).parse().ok()))
        {
            req = req.resolve_addr(h.trim_start_matches('-'), port, ip);
        }
    }
    if let Some(p) = opt_str(st, c::CURLOPT_PROXY).filter(|p| !p.is_empty()) {
        let mut spec = p;
        if !spec.contains("://") {
            let scheme = match opt_int(st, c::CURLOPT_PROXYTYPE).unwrap_or(0) {
                4 => "socks4",
                5 => "socks5",
                6 => "socks4a",
                7 => "socks5h",
                2 => "https",
                _ => "http",
            };
            spec = format!("{scheme}://{spec}");
        }
        if let Some(port) = opt_int(st, c::CURLOPT_PROXYPORT).filter(|p| *p > 0)
            && !spec.rsplit("://").next().unwrap_or("").contains(':')
        {
            spec = format!("{spec}:{port}");
        }
        req = req.proxy(&spec).map_err(|_| err(5, format!("Could not resolve proxy: {spec}")))?;
        if let Some(up) = opt_str(st, c::CURLOPT_PROXYUSERPWD) {
            let (u, pw) = up.split_once(':').unwrap_or((&up, ""));
            req = req.proxy_user(u, pw).map_err(|_| err(5, "Could not resolve proxy name"))?;
        }
        if let Some(np) = opt_str(st, c::CURLOPT_NOPROXY) {
            req = req.no_proxy(np.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
        }
    }
    if opt_int(st, c::CURLOPT_HTTPAUTH).unwrap_or(CURLAUTH_BASIC) & CURLAUTH_DIGEST != 0 && opt_int(st, c::CURLOPT_HTTPAUTH).unwrap_or(0) & CURLAUTH_BASIC == 0 {
        let parsed_user = opt_str(st, c::CURLOPT_USERPWD);
        if let Some((u, p)) = parsed_user.as_deref().and_then(|s| s.split_once(':')) {
            req = req.digest_auth(true).basic_auth(u, p);
        }
    }
    Ok(req)
}

/// CURLOPT_TIMEOUT_MS, else CURLOPT_TIMEOUT: how long a whole transfer may take.
fn total_timeout(st: &State) -> Option<Duration> {
    let ms = opt_int(st, c::CURLOPT_TIMEOUT_MS).filter(|n| *n > 0).map(|n| Duration::from_millis(n as u64));
    ms.or_else(|| opt_int(st, c::CURLOPT_TIMEOUT).filter(|n| *n > 0).map(|n| Duration::from_secs(n as u64)))
}

fn http_version_number(v: &str) -> i64 {
    match v {
        "HTTP/1.0" => 1,
        "HTTP/1.1" => 2,
        "HTTP/2" | "HTTP/2.0" => 3,
        "HTTP/3" => 30,
        _ => 0,
    }
}

fn micros(d: Option<Duration>) -> i64 {
    d.map_or(0, |d| d.as_micros() as i64)
}

/// curl_easy_perform(). The outer Err is a PHP exception from a callback;
/// the inner one is the transfer's own error (curl_errno / curl_error).
pub(crate) fn perform(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<Result<(), CurlErr>, Throw> {
    let h = handle(this);
    let mut st = h.st.borrow().clone();
    h.st.borrow_mut().info = Info::default();
    let started = Instant::now();

    let Some(raw) = opt_str(&st, c::CURLOPT_URL).filter(|u| !u.is_empty()) else {
        return Ok(Err(err(3, "No URL set")));
    };
    if let Some((scheme, _)) = raw.split_once("://").filter(|(s, _)| !s.is_empty())
        && !scheme.eq_ignore_ascii_case("http")
        && !scheme.eq_ignore_ascii_case("https")
        && !scheme.eq_ignore_ascii_case("file")
    {
        h.st.borrow_mut().info.scheme = scheme.to_ascii_lowercase();
        return Ok(Err(err(1, format!("Protocol \"{}\" not supported", scheme.to_ascii_lowercase()))));
    }
    let mut url = if raw.contains("://") { raw } else { format!("http://{raw}") };
    if let Some(port) = opt_int(&st, c::CURLOPT_PORT).filter(|p| *p > 0)
        && let Ok(u) = rsurl::Url::parse(&url)
    {
        url = format!("{}://{}{}:{}{}", u.scheme, u.userinfo.as_ref().map_or(String::new(), |ui| format!("{ui}@")), u.host, port, u.path);
    }
    let parsed = match rsurl::Url::parse(&url) {
        Ok(u) => u,
        Err(_) => return Ok(Err(err(3, "URL rejected: Malformed input to a URL function"))),
    };
    {
        let mut s = h.st.borrow_mut();
        s.info.url = url.clone();
        s.info.scheme = parsed.scheme.clone();
        s.info.primary_port = parsed.port as i64;
        s.info.protocol = match parsed.scheme.as_str() {
            "http" => 1,
            "https" => 2,
            "file" => 1 << 10,
            _ => 0,
        };
        if parsed.host.parse::<std::net::IpAddr>().is_ok() {
            s.info.primary_ip = parsed.host_unbracketed().to_string();
        }
    }
    let is_file = parsed.scheme == "file";
    let basedir = ctx.ini_get(b"open_basedir").is_some_and(|v| !bytes_of(&v).is_empty());
    if is_file && basedir {
        // php_curl_option_url(): CURLOPT_PROTOCOLS excludes file.
        return Ok(Err(err(1, "Protocol \"file\" not supported")));
    }
    if parsed.scheme != "http" && parsed.scheme != "https" && !is_file {
        return Ok(Err(err(1, format!("Protocol \"{}\" not supported", parsed.scheme))));
    }

    // The method and the body.
    let custom = opt_str(&st, c::CURLOPT_CUSTOMREQUEST).filter(|m| !m.is_empty());
    let nobody = st.req == Req::Head || opt_int(&st, c::CURLOPT_NOBODY).unwrap_or(0) != 0;
    let mut method = custom.clone().unwrap_or_else(|| match st.req {
        Req::Get => "GET",
        Req::Post => "POST",
        Req::Put => "PUT",
        Req::Head => "HEAD",
    }.to_string());
    let mut body: Vec<u8> = Vec::new();
    let mut content_type: Option<String> = None;
    match st.req {
        Req::Post if !is_file => match st.opts.get(&c::CURLOPT_POSTFIELDS) {
            Some(Value::Array(fields)) => match form::build(ctx, fields)? {
                Ok(f) => {
                    content_type = Some(format!("multipart/form-data; boundary={}", f.boundary));
                    body = f.body;
                }
                Err(e) => return Ok(Err(e)),
            },
            Some(v) => {
                body = bytes_of(v);
                content_type = Some("application/x-www-form-urlencoded".into());
            }
            None => content_type = Some("application/x-www-form-urlencoded".into()),
        },
        Req::Post => {}
        Req::Put => match read_upload(ctx, this, &st)? {
            Ok(b) => body = b,
            Err(e) => return Ok(Err(e)),
        },
        _ => {}
    }
    if nobody && custom.is_none() {
        method = "HEAD".into();
    }
    h.st.borrow_mut().info.upload_content_length = if body.is_empty() && st.req != Req::Post && st.req != Req::Put { -1 } else { body.len() as i64 };
    h.st.borrow_mut().info.effective_method = method.clone();

    // The cookie engine, when asked for.
    let jar = {
        let mut s = h.st.borrow_mut();
        if s.jar.is_none() && (s.opts.contains_key(&c::CURLOPT_COOKIEFILE) || s.opts.contains_key(&c::CURLOPT_COOKIEJAR)) {
            let mut j = rsurl::CookieJar::new();
            if let Some(f) = opt_str(&s, c::CURLOPT_COOKIEFILE).filter(|f| !f.is_empty())
                && let Ok(loaded) = rsurl::CookieJar::load_netscape_or_empty(&f)
            {
                j = loaded;
            }
            s.jar = Some(Rc::new(RefCell::new(j)));
        }
        s.jar.clone()
    };

    // The callbacks are looked up in the handle as each runs (one that
    // replaces itself is freed then), not kept from here.
    st.write = Sink::Stdout;
    st.header = HeaderSink::Ignore;
    st.read = Source::default();
    st.debug = None;
    let proxy_host = opt_str(&st, c::CURLOPT_PROXY).filter(|p| !p.is_empty()).map(|p| {
        let rest = p.rsplit("://").next().unwrap_or("").to_string();
        let rest = rest.rsplit('@').next().unwrap_or("").to_string();
        rest.split(':').next().unwrap_or("").to_string()
    });
    let follow = opt_int(&st, c::CURLOPT_FOLLOWLOCATION).unwrap_or(0) != 0;
    let max_redirs = opt_int(&st, c::CURLOPT_MAXREDIRS).unwrap_or(20);
    let run = Run {
        cx: RefCell::new(ctx),
        this: Rc::clone(this),
        include_header: opt_int(&st, c::CURLOPT_HEADER).unwrap_or(0) != 0,
        deadline: total_timeout(&st).map(|t| (started, t)),
        nobody,
        follow,
        fail_on_error: opt_int(&st, c::CURLOPT_FAILONERROR).unwrap_or(0) != 0,
        status: RefCell::new(RunStatus { content_length: -1, ..RunStatus::default() }),
    };

    let first_host = (parsed.host.clone(), parsed.port);
    let unrestricted = opt_int(&st, c::CURLOPT_UNRESTRICTED_AUTH).unwrap_or(0) != 0;
    let post_redir = opt_int(&st, c::CURLOPT_POSTREDIR).unwrap_or(0);
    let auto_referer = opt_int(&st, c::CURLOPT_AUTOREFERER).unwrap_or(0) != 0;
    let timeout = total_timeout(&st);

    let mut hops = 0i64;
    let mut request_size = 0i64;
    let mut referer: Option<String> = None;
    let mut last_head_timing = rsurl::Timing::default();
    let mut request_text = String::new();
    let mut result: Result<(), CurlErr> = Ok(());
    'transfer: {
    if is_file {
        result = file_transfer(&run, &parsed, st.req == Req::Put, &body);
        break 'transfer;
    }
    loop {
        let cur = rsurl::Url::parse(&url).unwrap_or_else(|_| parsed.clone());
        let same_host = (cur.host.clone(), cur.port) == first_host;
        let prepared = match prepare(&Hop { st: &st, url: &url, method: &method, body: &body, content_type: content_type.as_deref(), auth: same_host || unrestricted, referer: referer.as_deref() }) {
            Ok(p) => p,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        request_size += prepared.text.len() as i64;
        if st.header_out {
            request_text.push_str(&prepared.text);
        }
        let debug = h.st.borrow().debug.clone();
        if let Some(d) = debug {
            // CURLINFO_HEADER_OUT
            run.call(d, vec![Value::Int(2), Value::Str(PhpStr::new(prepared.text.clone().into_bytes()))]);
        }
        run.status.borrow_mut().location = None;
        let mut jar_ref = jar.as_ref().map(|j| j.borrow_mut());
        let request_url = url.clone();
        let sent = prepared.request.send_streaming_with(jar_ref.as_deref_mut(), |head| run.on_head(head, &request_url), |chunk| run.on_chunk(chunk));
        drop(jar_ref);
        match sent {
            Ok(resp) => {
                last_head_timing = resp.timing.clone();
                if run.aborted() {
                    break;
                }
                let (redirecting, location, code) = {
                    let s = run.status.borrow();
                    (s.redirecting, s.location.clone(), s.status_code)
                };
                if redirecting && let Some(next) = location {
                    if max_redirs >= 0 && hops >= max_redirs {
                        result = Err(err(47, format!("Maximum ({max_redirs}) redirects followed")));
                        break;
                    }
                    hops += 1;
                    referer = auto_referer.then(|| url.clone());
                    // POST becomes GET on 301, 302 and 303 unless asked not to.
                    let bit = match code {
                        301 => 1,
                        302 => 2,
                        303 => 4,
                        _ => 0,
                    };
                    if bit != 0 && custom.is_none() && (method == "POST" || method == "PUT") && (code == 303 || post_redir & bit == 0) {
                        method = "GET".into();
                        body.clear();
                        content_type = None;
                    }
                    url = next;
                    continue;
                }
                break;
            }
            Err(e) => {
                if let Some(f) = run.status.borrow_mut().failure.take() {
                    result = Err(f);
                } else if !run.aborted() {
                    let received = run.status.borrow().size_download;
                    result = Err(map_error(&e, &cur.host, cur.port, started.elapsed(), timeout, received, proxy_host.as_deref()));
                }
                break;
            }
        }
    }

    }

    // Collect what the callbacks left.
    let RunStatus { failure, thrown, body: returned, size_download, header_size, status_code, content_type: ctype, content_length, version, location, .. } = run.status.take();
    drop(run);
    if let Some(t) = thrown {
        return Err(t);
    }
    if let Some(f) = failure {
        result = Err(f);
    }
    let mut s = h.st.borrow_mut();
    s.info.url = url.clone();
    s.info.http_code = status_code;
    s.info.header_size = header_size;
    s.info.request_size = request_size;
    s.info.redirect_count = hops;
    s.info.total_us = started.elapsed().as_micros() as i64;
    s.info.namelookup_us = micros(last_head_timing.namelookup);
    s.info.connect_us = micros(last_head_timing.connect);
    s.info.appconnect_us = micros(last_head_timing.appconnect);
    s.info.pretransfer_us = micros(last_head_timing.pretransfer);
    s.info.starttransfer_us = micros(last_head_timing.starttransfer);
    s.info.size_upload = if st.req == Req::Post || st.req == Req::Put { body.len() as i64 } else { 0 };
    s.info.size_download = size_download;
    s.info.download_content_length = content_length;
    s.info.content_type = ctype;
    s.info.http_version = http_version_number(&version);
    s.info.redirect_url = if follow { String::new() } else { location.unwrap_or_default() };
    s.info.request_header = if st.header_out && !request_text.is_empty() { Some(request_text) } else { None };
    if matches!(s.write, Sink::Return) {
        s.content = Some(returned);
    }
    if let Some(path) = opt_str(&s, c::CURLOPT_COOKIEJAR).filter(|p| !p.is_empty())
        && let Some(j) = &s.jar
    {
        let _ = j.borrow().save_netscape(&path);
    }
    Ok(result)
}

/// file:// URLs: the file, as the body; an upload writes the file.
fn file_transfer(run: &Run, url: &rsurl::Url, upload: bool, data: &[u8]) -> Result<(), CurlErr> {
    let path = url.path.split(['?', '#']).next().unwrap_or("");
    if upload {
        return std::fs::write(path, data).map_err(|_| err(23, format!("Failed to open/create file {path}")));
    }
    let data = std::fs::read(path).map_err(|_| err(37, format!("Couldn't open file {path}")))?;
    {
        let mut s = run.status.borrow_mut();
        s.size_download = data.len() as i64;
        s.content_length = data.len() as i64;
    }
    run.emit_body(&data);
    Ok(())
}

/// The bytes an upload sends: CURLOPT_READFUNCTION until it returns "",
/// or CURLOPT_INFILE (up to CURLOPT_INFILESIZE).
fn read_upload(ctx: &mut dyn Ctx, this: &Rc<Object>, st: &State) -> Result<Result<Vec<u8>, CurlErr>, Throw> {
    let limit = opt_int(st, c::CURLOPT_INFILESIZE).filter(|n| *n >= 0).map(|n| n as usize);
    let mut out = Vec::new();
    if let Some(f) = &st.read.func {
        let stream = st.read.file.clone().unwrap_or(Value::Null);
        let h = handle(this);
        loop {
            let want = limit.map_or(65536, |n| (n - out.len()).min(65536));
            if want == 0 {
                break;
            }
            let mut args = [Value::Object(Rc::clone(this)), stream.clone(), Value::Int(want as i64)];
            let was = h.in_callback.replace(true);
            let r = ctx.call(f, &mut args);
            h.in_callback.set(was);
            match r?.deref() {
                Value::Str(s) if !s.is_empty() => out.extend_from_slice(&s.as_bytes()[..s.len().min(want)]),
                _ => break,
            }
        }
    } else if let Some(v) = &st.read.file
        && let Some(r) = resource_of(v)
        && let Some(res) = resource(&r)
    {
        let mut s = res.stream.borrow_mut();
        out = match limit {
            Some(n) => s.read(n),
            None => s.read_all(),
        };
    }
    Ok(Ok(out))
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn location_resolution() {
        let base = "http://h:8080/a/b/c?q=1#f";
        assert_eq!(resolve(base, "https://other/x"), "https://other/x");
        assert_eq!(resolve(base, "//cdn/x"), "http://cdn/x");
        assert_eq!(resolve(base, "/root"), "http://h:8080/root");
        assert_eq!(resolve(base, "d"), "http://h:8080/a/b/d");
        assert_eq!(resolve(base, "../d?x=2"), "http://h:8080/a/d?x=2");
        assert_eq!(resolve(base, "./d/./e/../f"), "http://h:8080/a/b/d/f");
        assert_eq!(resolve(base, "?z=3"), "http://h:8080/a/b/c?z=3");
        assert_eq!(resolve(base, ""), "http://h:8080/a/b/c?q=1");
        assert_eq!(resolve("http://h", "x"), "http://h/x");
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"user:pass"), "dXNlcjpwYXNz");
    }
}

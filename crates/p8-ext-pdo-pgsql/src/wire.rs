//! The part of libpq pdo_pgsql uses, over the PostgreSQL v3 protocol on a
//! blocking socket: connecting and authenticating, PQexec, PQprepare,
//! PQexecPrepared, PQexecParams, COPY, notifications and notices. Results
//! are read whole (as libpq does), and errors are worded as libpq words
//! them (`PQerrorMessage`), since PHP shows them to the script.
//!
//! Reference: postgresql/src/interfaces/libpq (fe-connect.c, fe-exec.c,
//! fe-protocol3.c); the protocol: https://www.postgresql.org/docs/current/protocol-message-formats.html

use crate::conninfo::Params;
use postgres_protocol::authentication::{md5_hash, sasl};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// ExecStatusType.
pub const EMPTY_QUERY: i64 = 0;
pub const COMMAND_OK: i64 = 1;
pub const TUPLES_OK: i64 = 2;
pub const COPY_OUT: i64 = 3;
pub const COPY_IN: i64 = 4;
pub const FATAL_ERROR: i64 = 7;

enum Sock {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl Sock {
    fn set_read_timeout(&self, d: Option<Duration>) -> io::Result<()> {
        match self {
            Sock::Tcp(s) => s.set_read_timeout(d),
            Sock::Unix(s) => s.set_read_timeout(d),
        }
    }

    fn set_nonblocking(&self, b: bool) -> io::Result<()> {
        match self {
            Sock::Tcp(s) => s.set_nonblocking(b),
            Sock::Unix(s) => s.set_nonblocking(b),
        }
    }
}

impl Read for Sock {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Sock::Tcp(s) => s.read(buf),
            Sock::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Sock {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Sock::Tcp(s) => s.write(buf),
            Sock::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Sock::Tcp(s) => s.flush(),
            Sock::Unix(s) => s.flush(),
        }
    }
}

/// A column of a result (RowDescription).
#[derive(Debug, Clone)]
pub struct Field {
    pub name: Vec<u8>,
    pub table_oid: u32,
    pub type_oid: u32,
    pub size: i16,
    pub modifier: i32,
}

/// A PGresult: all its rows, read.
#[derive(Debug, Clone)]
pub struct PgResult {
    pub status: i64,
    pub fields: Vec<Field>,
    pub rows: Vec<Vec<Option<Vec<u8>>>>,
    /// The command tag ("INSERT 0 1").
    pub tag: Vec<u8>,
    /// The error's SQLSTATE (PG_DIAG_SQLSTATE).
    pub sqlstate: Option<String>,
}

impl PgResult {
    fn new(status: i64) -> PgResult {
        PgResult { status, fields: Vec::new(), rows: Vec::new(), tag: Vec::new(), sqlstate: None }
    }

    pub fn ok(&self) -> bool {
        self.status == COMMAND_OK || self.status == TUPLES_OK
    }

    /// PQcmdTuples(): the row count of INSERT/UPDATE/DELETE/MERGE/SELECT/
    /// MOVE/FETCH/COPY, as text; 0 for anything else.
    pub fn cmd_tuples(&self) -> i64 {
        let tag = String::from_utf8_lossy(&self.tag);
        let mut words = tag.split(' ');
        let n = match words.next() {
            Some("INSERT") => words.nth(1),
            Some("UPDATE" | "DELETE" | "MERGE" | "SELECT" | "MOVE" | "FETCH" | "COPY") => words.next(),
            _ => None,
        };
        n.and_then(|n| n.parse().ok()).unwrap_or(0)
    }

    /// A rough PQresultMemorySize().
    pub fn memory_size(&self) -> i64 {
        let cells: usize = self.rows.iter().flatten().map(|c| c.as_ref().map_or(0, |v| v.len() + 1) + 16).sum();
        (2048 + self.fields.len() * 64 + cells) as i64
    }
}

/// A NOTIFY that arrived.
#[derive(Debug, Clone)]
pub struct Notify {
    pub channel: Vec<u8>,
    pub pid: i32,
    pub payload: Vec<u8>,
}

/// What reading a COPY TO's data gives.
pub enum CopyOut {
    Row(Vec<u8>),
    Done,
    Failed,
}

/// A parameter of PQexecPrepared()/PQexecParams(): its bytes and format
/// (0 text, 1 binary); None is SQL NULL.
pub type Param = Option<(Vec<u8>, i16)>;

/// A connection (PGconn).
pub struct Wire {
    sock: Sock,
    inbuf: Vec<u8>,
    pos: usize,
    out: Vec<u8>,
    /// ParameterStatus values (server_version, client_encoding, ...).
    params: Vec<(String, Vec<u8>)>,
    pub pid: i32,
    /// ReadyForQuery's transaction status: I, T or E.
    pub txn: u8,
    pub notifies: VecDeque<Notify>,
    /// NoticeResponse messages, worded as libpq hands them to its notice processor.
    pub notices: Vec<Vec<u8>>,
    /// PQerrorMessage().
    pub error_message: Vec<u8>,
    /// CONNECTION_OK
    pub ok: bool,
}

// ---- messages ----

fn put_i16(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// A C string: what libpq sends of `s` (up to a NUL).
fn put_cstr(out: &mut Vec<u8>, s: &[u8]) {
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    out.extend_from_slice(&s[..end]);
    out.push(0);
}

fn message(out: &mut Vec<u8>, tag: u8, body: &[u8]) {
    out.push(tag);
    put_i32(out, body.len() as i32 + 4);
    out.extend_from_slice(body);
}

fn cstr_at(body: &[u8], at: &mut usize) -> Vec<u8> {
    let rest = &body[(*at).min(body.len())..];
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    *at += end + 1;
    rest[..end].to_vec()
}

fn i16_at(body: &[u8], at: &mut usize) -> i16 {
    let v = body.get(*at..*at + 2).map_or(0, |b| i16::from_be_bytes([b[0], b[1]]));
    *at += 2;
    v
}

fn i32_at(body: &[u8], at: &mut usize) -> i32 {
    let v = body.get(*at..*at + 4).map_or(0, |b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    *at += 4;
    v
}

/// The fields of an ErrorResponse or NoticeResponse.
struct ErrFields(Vec<(u8, Vec<u8>)>);

impl ErrFields {
    fn parse(body: &[u8]) -> ErrFields {
        let mut out = Vec::new();
        let mut at = 0;
        while at < body.len() && body[at] != 0 {
            let code = body[at];
            at += 1;
            out.push((code, cstr_at(body, &mut at)));
        }
        ErrFields(out)
    }

    fn get(&self, code: u8) -> Option<&[u8]> {
        self.0.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_slice())
    }
}

/// pqBuildErrorMessage3() at the default verbosity: severity, message,
/// the position (a LINE/caret display when the query is known), DETAIL,
/// HINT, QUERY and (for errors) CONTEXT.
fn build_error_message(f: &ErrFields, query: Option<&[u8]>, is_error: bool) -> Vec<u8> {
    let mut msg = Vec::new();
    if let Some(s) = f.get(b'S') {
        msg.extend_from_slice(s);
        msg.extend_from_slice(b":  ");
    }
    match f.get(b'M') {
        Some(m) => msg.extend_from_slice(m),
        None => msg.extend_from_slice(b"no primary error message"),
    }
    let mut cursor: Option<(Vec<u8>, i64)> = None;
    if let Some(p) = f.get(b'P') {
        match query {
            Some(q) => cursor = Some((q.to_vec(), atoi(p))),
            None => {
                msg.extend_from_slice(b" at character ");
                msg.extend_from_slice(p);
            }
        }
    } else if let Some(p) = f.get(b'p') {
        match f.get(b'q') {
            Some(q) => cursor = Some((q.to_vec(), atoi(p))),
            None => {
                msg.extend_from_slice(b" at character ");
                msg.extend_from_slice(p);
            }
        }
    }
    msg.push(b'\n');
    if let Some((q, pos)) = cursor
        && pos > 0
    {
        report_error_position(&mut msg, &q, pos);
    }
    for (code, label) in [(b'D', &b"DETAIL:  "[..]), (b'H', b"HINT:  "), (b'q', b"QUERY:  ")] {
        if let Some(v) = f.get(code) {
            msg.extend_from_slice(label);
            msg.extend_from_slice(v);
            msg.push(b'\n');
        }
    }
    if is_error && let Some(v) = f.get(b'W') {
        msg.extend_from_slice(b"CONTEXT:  ");
        msg.extend_from_slice(v);
        msg.push(b'\n');
    }
    msg
}

fn atoi(s: &[u8]) -> i64 {
    String::from_utf8_lossy(s).trim().parse().unwrap_or(0)
}

/// reportErrorPosition(): the line of the query holding character `loc`
/// (1-based, in characters), at most 60 columns of it, and a caret under it.
fn report_error_position(msg: &mut Vec<u8>, query: &[u8], loc: i64) {
    const DISPLAY_SIZE: usize = 60;
    const MIN_RIGHT_CUT: usize = 10;
    let loc = (loc - 1) as usize;
    let mut w: Vec<u8> = query.iter().position(|&b| b == 0).map_or(query, |n| &query[..n]).to_vec();
    // qidx: each character's byte offset; scridx: its screen column.
    let mut qidx: Vec<usize> = Vec::new();
    let mut scridx: Vec<usize> = Vec::new();
    let (mut qoffset, mut scroffset, mut loc_line, mut ibeg) = (0usize, 0usize, 1usize, 0usize);
    let mut iend: Option<usize> = None;
    let mut cno = 0usize;
    while qoffset < w.len() {
        let ch = w[qoffset];
        qidx.push(qoffset);
        scridx.push(scroffset);
        if ch == b'\t' {
            w[qoffset] = b' ';
        } else if ch == b'\r' || ch == b'\n' {
            if cno < loc {
                if ch == b'\r' || cno == 0 || w[qidx[cno - 1]] != b'\r' {
                    loc_line += 1;
                }
                ibeg = cno + 1;
            } else {
                iend = Some(cno);
                break;
            }
        }
        // UTF-8 (the usual client encoding): one column per character.
        let len = match ch {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        scroffset += 1;
        qoffset = (qoffset + len).min(w.len());
        cno += 1;
    }
    let mut iend = match iend {
        Some(e) => e,
        None => {
            qidx.push(qoffset);
            scridx.push(scroffset);
            cno
        }
    };
    if loc > cno {
        return;
    }
    let (mut beg_trunc, mut end_trunc) = (false, false);
    if scridx[iend] - scridx[ibeg] > DISPLAY_SIZE {
        if scridx[ibeg] + DISPLAY_SIZE >= scridx[loc] + MIN_RIGHT_CUT {
            while scridx[iend] - scridx[ibeg] > DISPLAY_SIZE {
                iend -= 1;
            }
            end_trunc = true;
        } else {
            while scridx[loc] + MIN_RIGHT_CUT < scridx[iend] {
                iend -= 1;
                end_trunc = true;
            }
            while scridx[iend] - scridx[ibeg] > DISPLAY_SIZE {
                ibeg += 1;
                beg_trunc = true;
            }
        }
    }
    let prefix = format!("LINE {loc_line}: {}", if beg_trunc { "..." } else { "" });
    msg.extend_from_slice(prefix.as_bytes());
    msg.extend_from_slice(&w[qidx[ibeg]..qidx[iend]]);
    if end_trunc {
        msg.extend_from_slice(b"...");
    }
    msg.push(b'\n');
    let pad = prefix.len() + scridx[loc] - scridx[ibeg];
    msg.extend(std::iter::repeat_n(b' ', pad));
    msg.extend_from_slice(b"^\n");
}

/// strerror() of a socket error, without Rust's " (os error N)".
fn strerror(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::ConnectionRefused => "Connection refused".into(),
        io::ErrorKind::NotFound => "No such file or directory".into(),
        io::ErrorKind::PermissionDenied => "Permission denied".into(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "timeout expired".into(),
        _ => {
            let s = e.to_string();
            match s.find(" (os error") {
                Some(i) => s[..i].to_string(),
                None => s,
            }
        }
    }
}

const SERVER_CLOSED: &[u8] = b"server closed the connection unexpectedly\n\tThis probably means the server terminated abnormally\n\tbefore or while processing the request.\n";

impl Wire {
    /// PQconnectdb(): try each host in turn; Err is the error message.
    pub fn connect(p: &Params) -> Result<Wire, Vec<u8>> {
        if let Some(mode) = p.get("sslmode")
            && matches!(mode, "require" | "verify-ca" | "verify-full")
        {
            return Err(format!("sslmode value \"{mode}\" invalid when SSL support is not compiled in\n").into_bytes());
        }
        let timeout = p.get("connect_timeout").and_then(|t| t.trim().parse::<i64>().ok()).filter(|&t| t > 0).map(|t| Duration::from_secs(t.max(2) as u64));
        let hosts = p.hosts();
        let mut errors = Vec::new();
        for (host, port) in hosts {
            match Self::connect_one(p, &host, &port, timeout) {
                Ok(w) => return Ok(w),
                Err(e) => errors.extend_from_slice(&e),
            }
        }
        Err(errors)
    }

    fn connect_one(p: &Params, host: &str, port: &str, timeout: Option<Duration>) -> Result<Wire, Vec<u8>> {
        let portno: u16 = match port.trim().parse() {
            Ok(n) => n,
            Err(_) => return Err(format!("invalid integer value \"{port}\" for connection option \"port\"\n").into_bytes()),
        };
        let (sock, prefix) = if host.starts_with('/') {
            let path = format!("{host}/.s.PGSQL.{portno}");
            let prefix = format!("connection to server on socket \"{path}\" failed: ");
            match UnixStream::connect(&path) {
                Ok(s) => (Sock::Unix(s), prefix),
                Err(e) => {
                    return Err(format!("{prefix}{}\n\tIs the server running locally and accepting connections on that socket?\n", strerror(&e)).into_bytes());
                }
            }
        } else {
            let numeric = host.parse::<IpAddr>().is_ok();
            let addrs: Vec<SocketAddr> = match (host, portno).to_socket_addrs() {
                Ok(a) => a.collect(),
                Err(e) => return Err(format!("could not translate host name \"{host}\" to address: {}\n", strerror(&e)).into_bytes()),
            };
            let mut errors = Vec::new();
            let mut found = None;
            for addr in addrs {
                let prefix = if numeric { format!("connection to server at \"{host}\", port {portno} failed: ") } else { format!("connection to server at \"{host}\" ({}), port {portno} failed: ", addr.ip()) };
                let r = match timeout {
                    Some(t) => TcpStream::connect_timeout(&addr, t),
                    None => TcpStream::connect(addr),
                };
                match r {
                    Ok(s) => {
                        let _ = s.set_nodelay(true);
                        found = Some((Sock::Tcp(s), prefix));
                        break;
                    }
                    Err(e) => {
                        errors.extend_from_slice(prefix.as_bytes());
                        errors.extend_from_slice(strerror(&e).as_bytes());
                        if e.kind() == io::ErrorKind::ConnectionRefused {
                            errors.extend_from_slice(b"\n\tIs the server running on that host and accepting TCP/IP connections?");
                        }
                        errors.push(b'\n');
                    }
                }
            }
            match found {
                Some(f) => f,
                None => return Err(errors),
            }
        };
        let mut w = Wire { sock, inbuf: Vec::new(), pos: 0, out: Vec::new(), params: Vec::new(), pid: 0, txn: b'I', notifies: VecDeque::new(), notices: Vec::new(), error_message: Vec::new(), ok: false };
        let _ = w.sock.set_read_timeout(timeout);
        match w.startup(p) {
            Ok(()) => {
                let _ = w.sock.set_read_timeout(None);
                w.ok = true;
                Ok(w)
            }
            Err(e) => {
                let mut m = prefix.into_bytes();
                m.extend_from_slice(&e);
                Err(m)
            }
        }
    }

    /// The startup packet, authentication, and the server's settings.
    fn startup(&mut self, p: &Params) -> Result<(), Vec<u8>> {
        let user = p.user();
        let mut body = Vec::new();
        put_i32(&mut body, 196608);
        let dbname = p.get("dbname").filter(|d| !d.is_empty()).map(str::to_string).unwrap_or_else(|| user.clone());
        let mut pairs: Vec<(&str, String)> = vec![("user", user.clone()), ("database", dbname)];
        if let Some(o) = p.get("options").filter(|o| !o.is_empty()) {
            pairs.push(("options", o.to_string()));
        }
        if let Some(a) = p.get("application_name").or_else(|| p.get("fallback_application_name")).filter(|a| !a.is_empty()) {
            pairs.push(("application_name", a.to_string()));
        }
        if let Some(e) = p.get("client_encoding").filter(|e| !e.is_empty() && *e != "auto") {
            pairs.push(("client_encoding", e.to_string()));
        }
        for (k, v) in &pairs {
            put_cstr(&mut body, k.as_bytes());
            put_cstr(&mut body, v.as_bytes());
        }
        body.push(0);
        put_i32(&mut self.out, body.len() as i32 + 4);
        self.out.extend_from_slice(&body);
        self.send().map_err(|e| format!("could not send startup packet: {}\n", strerror(&e)).into_bytes())?;
        let password = p.get("password").filter(|s| !s.is_empty()).map(|s| s.as_bytes().to_vec());
        let mut scram: Option<sasl::ScramSha256> = None;
        loop {
            let (tag, body) = self.read_raw().map_err(|e| self.io_error_text(&e))?;
            match tag {
                b'R' => {
                    let mut at = 0;
                    match i32_at(&body, &mut at) {
                        0 => {}
                        3 => {
                            let Some(pw) = &password else { return Err(b"fe_sendauth: no password supplied\n".to_vec()) };
                            let mut b = Vec::new();
                            put_cstr(&mut b, pw);
                            message(&mut self.out, b'p', &b);
                            self.send().map_err(|e| self.io_error_text(&e))?;
                        }
                        5 => {
                            let Some(pw) = &password else { return Err(b"fe_sendauth: no password supplied\n".to_vec()) };
                            let salt = [body[4], body[5], body[6], body[7]];
                            let hash = md5_hash(user.as_bytes(), pw, salt);
                            let mut b = Vec::new();
                            put_cstr(&mut b, hash.as_bytes());
                            message(&mut self.out, b'p', &b);
                            self.send().map_err(|e| self.io_error_text(&e))?;
                        }
                        10 => {
                            let mut mechs = Vec::new();
                            while at < body.len() && body[at] != 0 {
                                mechs.push(cstr_at(&body, &mut at));
                            }
                            if !mechs.iter().any(|m| m == sasl::SCRAM_SHA_256.as_bytes()) {
                                return Err(b"none of the server's SASL authentication mechanisms are supported\n".to_vec());
                            }
                            let Some(pw) = &password else { return Err(b"fe_sendauth: no password supplied\n".to_vec()) };
                            let s = sasl::ScramSha256::new(pw, sasl::ChannelBinding::unsupported());
                            let mut b = Vec::new();
                            put_cstr(&mut b, sasl::SCRAM_SHA_256.as_bytes());
                            put_i32(&mut b, s.message().len() as i32);
                            b.extend_from_slice(s.message());
                            message(&mut self.out, b'p', &b);
                            self.send().map_err(|e| self.io_error_text(&e))?;
                            scram = Some(s);
                        }
                        11 => {
                            let s = scram.as_mut().ok_or_else(|| b"malformed SCRAM message\n".to_vec())?;
                            s.update(&body[4..]).map_err(|e| format!("malformed SCRAM message ({e})\n").into_bytes())?;
                            let m = s.message().to_vec();
                            message(&mut self.out, b'p', &m);
                            self.send().map_err(|e| self.io_error_text(&e))?;
                        }
                        12 => {
                            let s = scram.as_mut().ok_or_else(|| b"malformed SCRAM message\n".to_vec())?;
                            s.finish(&body[4..]).map_err(|_| b"invalid SCRAM server signature\n".to_vec())?;
                        }
                        n => return Err(format!("authentication method {n} not supported\n").into_bytes()),
                    }
                }
                b'E' => {
                    let f = ErrFields::parse(&body);
                    return Err(build_error_message(&f, None, true));
                }
                b'K' => {
                    let mut at = 0;
                    self.pid = i32_at(&body, &mut at);
                }
                b'S' => self.parameter_status(&body),
                b'N' => self.notice(&body),
                b'Z' => {
                    self.txn = body.first().copied().unwrap_or(b'I');
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    /// PQparameterStatus()
    pub fn parameter(&self, name: &str) -> Option<&[u8]> {
        self.params.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_slice())
    }

    fn parameter_status(&mut self, body: &[u8]) {
        let mut at = 0;
        let name = String::from_utf8_lossy(&cstr_at(body, &mut at)).into_owned();
        let value = cstr_at(body, &mut at);
        match self.params.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => self.params.push((name, value)),
        }
    }

    fn notice(&mut self, body: &[u8]) {
        let f = ErrFields::parse(body);
        self.notices.push(build_error_message(&f, None, false));
    }

    pub fn standard_strings(&self) -> bool {
        self.parameter("standard_conforming_strings") != Some(b"off")
    }

    fn send(&mut self) -> io::Result<()> {
        let r = self.sock.write_all(&self.out).and_then(|_| self.sock.flush());
        self.out.clear();
        r
    }

    /// Bytes until there are `n` unread.
    fn fill(&mut self, n: usize) -> io::Result<()> {
        if self.pos > 0 && self.pos == self.inbuf.len() {
            self.inbuf.clear();
            self.pos = 0;
        }
        let mut chunk = [0u8; 16384];
        while self.inbuf.len() - self.pos < n {
            let got = self.sock.read(&mut chunk)?;
            if got == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed"));
            }
            self.inbuf.extend_from_slice(&chunk[..got]);
        }
        Ok(())
    }

    /// The next message, whatever it is.
    fn read_raw(&mut self) -> io::Result<(u8, Vec<u8>)> {
        self.fill(5)?;
        let tag = self.inbuf[self.pos];
        let len = u32::from_be_bytes([self.inbuf[self.pos + 1], self.inbuf[self.pos + 2], self.inbuf[self.pos + 3], self.inbuf[self.pos + 4]]) as usize;
        if len < 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid message length"));
        }
        self.fill(1 + len)?;
        let body = self.inbuf[self.pos + 5..self.pos + 1 + len].to_vec();
        self.pos += 1 + len;
        if self.pos > 1 << 20 {
            self.inbuf.drain(..self.pos);
            self.pos = 0;
        }
        Ok((tag, body))
    }

    /// The next message that is not one the server may send at any time
    /// (notices, notifications, parameter changes: those are kept).
    fn read(&mut self) -> io::Result<(u8, Vec<u8>)> {
        loop {
            let (tag, body) = self.read_raw()?;
            match tag {
                b'N' => self.notice(&body),
                b'A' => {
                    let mut at = 0;
                    let pid = i32_at(&body, &mut at);
                    let channel = cstr_at(&body, &mut at);
                    let payload = cstr_at(&body, &mut at);
                    self.notifies.push_back(Notify { channel, pid, payload });
                }
                b'S' => self.parameter_status(&body),
                _ => return Ok((tag, body)),
            }
        }
    }

    fn io_error_text(&self, e: &io::Error) -> Vec<u8> {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            SERVER_CLOSED.to_vec()
        } else {
            format!("could not receive data from server: {}\n", strerror(e)).into_bytes()
        }
    }

    /// The connection broke: the error result libpq makes.
    fn broken(&mut self, e: &io::Error) -> PgResult {
        self.ok = false;
        self.error_message = self.io_error_text(e);
        PgResult::new(FATAL_ERROR)
    }

    fn not_connected(&mut self) -> PgResult {
        self.error_message = b"no connection to the server\n".to_vec();
        PgResult::new(FATAL_ERROR)
    }

    /// Send what is queued and read the results up to ReadyForQuery (or a
    /// COPY's start): the last one, as PQexec() keeps.
    fn run(&mut self, query: Option<&[u8]>) -> PgResult {
        self.error_message.clear();
        if !self.ok {
            self.out.clear();
            return self.not_connected();
        }
        if let Err(e) = self.send() {
            return self.broken(&e);
        }
        self.collect(query)
    }

    fn collect(&mut self, query: Option<&[u8]>) -> PgResult {
        let mut cur: Option<PgResult> = None;
        let mut last: Option<PgResult> = None;
        loop {
            let (tag, body) = match self.read() {
                Ok(m) => m,
                Err(e) => return self.broken(&e),
            };
            match tag {
                b'T' => {
                    let mut r = PgResult::new(TUPLES_OK);
                    let mut at = 0;
                    let n = i16_at(&body, &mut at);
                    for _ in 0..n.max(0) {
                        let name = cstr_at(&body, &mut at);
                        let table_oid = i32_at(&body, &mut at) as u32;
                        let _column = i16_at(&body, &mut at);
                        let type_oid = i32_at(&body, &mut at) as u32;
                        let size = i16_at(&body, &mut at);
                        let modifier = i32_at(&body, &mut at);
                        let _format = i16_at(&body, &mut at);
                        r.fields.push(Field { name, table_oid, type_oid, size, modifier });
                    }
                    cur = Some(r);
                }
                b'D' => {
                    let mut at = 0;
                    let n = i16_at(&body, &mut at);
                    let mut row = Vec::with_capacity(n.max(0) as usize);
                    for _ in 0..n.max(0) {
                        let len = i32_at(&body, &mut at);
                        if len < 0 {
                            row.push(None);
                        } else {
                            let end = (at + len as usize).min(body.len());
                            row.push(Some(body[at..end].to_vec()));
                            at = end;
                        }
                    }
                    cur.get_or_insert_with(|| PgResult::new(TUPLES_OK)).rows.push(row);
                }
                b'C' => {
                    let mut r = cur.take().unwrap_or_else(|| PgResult::new(COMMAND_OK));
                    let mut at = 0;
                    r.tag = cstr_at(&body, &mut at);
                    last = Some(r);
                }
                b's' => {
                    if let Some(r) = cur.take() {
                        last = Some(r);
                    }
                }
                b'I' => last = Some(PgResult::new(EMPTY_QUERY)),
                b'E' => {
                    let f = ErrFields::parse(&body);
                    let mut r = PgResult::new(FATAL_ERROR);
                    r.sqlstate = f.get(b'C').map(|c| String::from_utf8_lossy(c).into_owned());
                    self.error_message = build_error_message(&f, query, true);
                    cur = None;
                    last = Some(r);
                }
                b'G' => return PgResult::new(COPY_IN),
                b'H' => return PgResult::new(COPY_OUT),
                b'Z' => {
                    self.txn = body.first().copied().unwrap_or(b'I');
                    break;
                }
                _ => {}
            }
        }
        last.or(cur).unwrap_or_else(|| PgResult::new(EMPTY_QUERY))
    }

    /// PQexec()
    pub fn exec(&mut self, sql: &[u8]) -> PgResult {
        let mut b = Vec::new();
        put_cstr(&mut b, sql);
        message(&mut self.out, b'Q', &b);
        self.run(Some(sql))
    }

    fn parse_msg(&mut self, name: &[u8], sql: &[u8], types: &[u32]) {
        let mut b = Vec::new();
        put_cstr(&mut b, name);
        put_cstr(&mut b, sql);
        put_i16(&mut b, types.len() as i16);
        for t in types {
            put_i32(&mut b, *t as i32);
        }
        message(&mut self.out, b'P', &b);
    }

    fn bind_execute(&mut self, name: &[u8], params: &[Param]) {
        let mut b = Vec::new();
        put_cstr(&mut b, b"");
        put_cstr(&mut b, name);
        put_i16(&mut b, params.len() as i16);
        for p in params {
            put_i16(&mut b, p.as_ref().map_or(0, |(_, f)| *f));
        }
        put_i16(&mut b, params.len() as i16);
        for p in params {
            match p {
                None => put_i32(&mut b, -1),
                Some((v, _)) => {
                    put_i32(&mut b, v.len() as i32);
                    b.extend_from_slice(v);
                }
            }
        }
        put_i16(&mut b, 1);
        put_i16(&mut b, 0);
        message(&mut self.out, b'B', &b);
        message(&mut self.out, b'D', b"P\0");
        let mut e = Vec::new();
        put_cstr(&mut e, b"");
        put_i32(&mut e, 0);
        message(&mut self.out, b'E', &e);
        message(&mut self.out, b'S', &[]);
    }

    /// PQprepare()
    pub fn prepare(&mut self, name: &[u8], sql: &[u8], types: &[u32]) -> PgResult {
        self.parse_msg(name, sql, types);
        message(&mut self.out, b'S', &[]);
        let r = self.run(Some(sql));
        if r.status == EMPTY_QUERY { PgResult::new(COMMAND_OK) } else { r }
    }

    /// PQexecPrepared()
    pub fn exec_prepared(&mut self, name: &[u8], params: &[Param]) -> PgResult {
        self.bind_execute(name, params);
        self.run(None)
    }

    /// PQexecParams()
    pub fn exec_params(&mut self, sql: &[u8], types: &[u32], params: &[Param]) -> PgResult {
        self.parse_msg(b"", sql, types);
        self.bind_execute(b"", params);
        self.run(Some(sql))
    }

    /// PQclosePrepared()
    pub fn close_prepared(&mut self, name: &[u8]) -> PgResult {
        let mut b = vec![b'S'];
        put_cstr(&mut b, name);
        message(&mut self.out, b'C', &b);
        message(&mut self.out, b'S', &[]);
        let r = self.run(None);
        if r.status == EMPTY_QUERY { PgResult::new(COMMAND_OK) } else { r }
    }

    /// PQclosePortal()
    pub fn close_portal(&mut self, name: &[u8]) -> PgResult {
        let mut b = vec![b'P'];
        put_cstr(&mut b, name);
        message(&mut self.out, b'C', &b);
        message(&mut self.out, b'S', &[]);
        let r = self.run(None);
        if r.status == EMPTY_QUERY { PgResult::new(COMMAND_OK) } else { r }
    }

    /// PQputCopyData()
    pub fn put_copy_data(&mut self, data: &[u8]) -> bool {
        message(&mut self.out, b'd', data);
        if self.out.len() > 65536 {
            return self.send().is_ok();
        }
        true
    }

    /// PQputCopyEnd(), then PQgetResult() until there are none: the
    /// result, an error when there was one.
    pub fn put_copy_end(&mut self) -> PgResult {
        message(&mut self.out, b'c', &[]);
        self.run(None)
    }

    /// PQgetCopyData() (blocking).
    pub fn get_copy_data(&mut self) -> CopyOut {
        loop {
            match self.read() {
                Ok((b'd', body)) => return CopyOut::Row(body),
                Ok((b'c', _)) => {
                    let r = self.collect(None);
                    return if r.status == FATAL_ERROR { CopyOut::Failed } else { CopyOut::Done };
                }
                Ok((b'E', body)) => {
                    let f = ErrFields::parse(&body);
                    self.error_message = build_error_message(&f, None, true);
                    let _ = self.collect(None);
                    return CopyOut::Failed;
                }
                Ok(_) => {}
                Err(e) => {
                    self.broken(&e);
                    return CopyOut::Failed;
                }
            }
        }
    }

    /// PQconsumeInput(): read what has arrived (waiting at most `wait`),
    /// keeping its notifications. False when the connection broke.
    pub fn consume_input(&mut self, wait: Option<Duration>) -> bool {
        if !self.ok {
            self.error_message = b"no connection to the server\n".to_vec();
            return false;
        }
        let mut chunk = [0u8; 16384];
        let first = match wait {
            Some(d) => self.sock.set_read_timeout(Some(d.max(Duration::from_millis(1)))),
            None => self.sock.set_nonblocking(true),
        };
        if first.is_err() {
            return false;
        }
        let mut good = true;
        loop {
            match self.sock.read(&mut chunk) {
                Ok(0) => {
                    self.ok = false;
                    self.error_message = SERVER_CLOSED.to_vec();
                    good = false;
                    break;
                }
                Ok(n) => {
                    self.inbuf.extend_from_slice(&chunk[..n]);
                    if wait.is_some() {
                        let _ = self.sock.set_read_timeout(None);
                        let _ = self.sock.set_nonblocking(true);
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => break,
                Err(e) => {
                    self.broken(&e);
                    good = false;
                    break;
                }
            }
        }
        let _ = self.sock.set_nonblocking(false);
        let _ = self.sock.set_read_timeout(None);
        // Take the complete messages that came in.
        while self.inbuf.len() - self.pos >= 5 {
            let len = u32::from_be_bytes([self.inbuf[self.pos + 1], self.inbuf[self.pos + 2], self.inbuf[self.pos + 3], self.inbuf[self.pos + 4]]) as usize;
            if self.inbuf.len() - self.pos < 1 + len {
                break;
            }
            let tag = self.inbuf[self.pos];
            if !matches!(tag, b'N' | b'A' | b'S') {
                // Not something to keep while idle: drop it.
                self.pos += 1 + len;
                continue;
            }
            if self.read().is_err() {
                break;
            }
        }
        good
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        if self.ok {
            message(&mut self.out, b'X', &[]);
            let _ = self.send();
        }
    }
}

// ---- escaping (fe-exec.c) ----

/// The byte length of the UTF-8 character starting at `s[0]`, when the
/// bytes form one (pg_utf8_islegal()).
fn utf8_char(s: &[u8]) -> Option<usize> {
    let n = match s[0] {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    if s.len() < n || std::str::from_utf8(&s[..n]).is_err() {
        return None;
    }
    Some(n)
}

/// PQescapeStringConn(): `s` with quotes (and, without standard strings,
/// backslashes) doubled, up to a NUL; None for text that is not valid in
/// the client encoding (UTF-8).
pub fn escape_string(s: &[u8], std_strings: bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() + 2);
    let mut i = 0;
    while i < s.len() && s[i] != 0 {
        let c = s[i];
        if c < 0x80 {
            if c == b'\'' || (c == b'\\' && !std_strings) {
                out.push(c);
            }
            out.push(c);
            i += 1;
            continue;
        }
        let n = utf8_char(&s[i..])?;
        out.extend_from_slice(&s[i..i + n]);
        i += n;
    }
    Some(out)
}

/// PQescapeIdentifier(): `"s"` with inner quotes doubled; Err is the error
/// message for invalid text.
pub fn escape_identifier(s: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut out = vec![b'"'];
    let mut i = 0;
    while i < s.len() && s[i] != 0 {
        let c = s[i];
        if c < 0x80 {
            if c == b'"' {
                out.push(c);
            }
            out.push(c);
            i += 1;
            continue;
        }
        match utf8_char(&s[i..]) {
            Some(n) => {
                out.extend_from_slice(&s[i..i + n]);
                i += n;
            }
            None => return Err("invalid multibyte character\n"),
        }
    }
    out.push(b'"');
    Ok(out)
}

/// PQescapeByteaConn() in hex form (servers since 9.0).
pub fn escape_bytea(s: &[u8], std_strings: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2 + 3);
    if !std_strings {
        out.push(b'\\');
    }
    out.extend_from_slice(b"\\x");
    for b in s {
        out.extend_from_slice(format!("{b:02x}").as_bytes());
    }
    out
}

/// PQunescapeBytea(): hex (`\x...`) or escape format.
pub fn unescape_bytea(s: &[u8]) -> Vec<u8> {
    if let Some(hex) = s.strip_prefix(b"\\x") {
        let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        let mut out = Vec::with_capacity(hex.len() / 2);
        let mut i = 0;
        while i < hex.len() {
            match digit(hex[i]) {
                Some(hi) if i + 1 < hex.len() => {
                    if let Some(lo) = digit(hex[i + 1]) {
                        out.push(hi << 4 | lo);
                    }
                    i += 2;
                }
                _ => i += 1,
            }
        }
        return out;
    }
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' {
            if s.get(i + 1) == Some(&b'\\') {
                out.push(b'\\');
                i += 2;
                continue;
            }
            if i + 3 < s.len() &&s[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
                out.push(((s[i + 1] - b'0') << 6) | ((s[i + 2] - b'0') << 3) | (s[i + 3] - b'0'));
                i += 4;
                continue;
            }
            i += 1;
            continue;
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_under_the_error() {
        let mut m = Vec::new();
        report_error_position(&mut m, b"SELECT * FROM nope", 15);
        assert_eq!(String::from_utf8(m).expect("utf8"), "LINE 1: SELECT * FROM nope\n                      ^\n");
    }

    #[test]
    fn bytea_round_trip() {
        assert_eq!(unescape_bytea(b"\\x00ff41"), vec![0, 255, b'A']);
        assert_eq!(unescape_bytea(b"a\\\\b\\001"), b"a\\b\x01".to_vec());
        assert_eq!(escape_bytea(b"\x00A", true), b"\\x0041".to_vec());
    }

    #[test]
    fn escapes_strings() {
        assert_eq!(escape_string(b"it's\0x", true), Some(b"it''s".to_vec()));
        assert_eq!(escape_string(b"\xff", true), None);
        assert_eq!(escape_identifier(b"a\"b"), Ok(b"\"a\"\"b\"".to_vec()));
    }
}

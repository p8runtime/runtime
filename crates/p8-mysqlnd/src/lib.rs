//! A MySQL client: the wire protocol as PHP's mysqlnd speaks it, over std's
//! sockets. It connects (TCP or a Unix socket), authenticates with
//! mysql_native_password or caching_sha2_password, runs text-protocol
//! queries and server-side prepared statements (the binary protocol), and
//! reads their results one result set at a time: whole (buffered) or row by
//! row as the caller asks for them (unbuffered). Rows are decoded into PHP
//! values the way mysqlnd does it ([`text_values`], [`binary_values`]).
//!
//! Reference: $PHP_SRC_PATH/ext/mysqlnd (mysqlnd_wireprotocol.c,
//! mysqlnd_ps.c, mysqlnd_ps_codec.c, mysqlnd_auth.c, mysqlnd_connection.c,
//! mysqlnd_charset.c) and the MySQL client/server protocol documentation.

#![forbid(unsafe_code)]

use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::Duration;

// Capability flags (CLIENT_*).
const LONG_PASSWORD: u32 = 1;
/// UPDATE counts the rows it matched, not the ones it changed.
pub const FOUND_ROWS: u32 = 2;
const LONG_FLAG: u32 = 4;
const CONNECT_WITH_DB: u32 = 8;
/// The server may ask for a client file (LOAD DATA LOCAL INFILE, which
/// the client then refuses).
const LOCAL_FILES: u32 = 0x80;
/// Function names may be followed by spaces.
pub const IGNORE_SPACE: u32 = 0x100;
const PROTOCOL_41: u32 = 0x200;
const TRANSACTIONS: u32 = 0x2000;
const SECURE_CONNECTION: u32 = 0x8000;
pub const MULTI_STATEMENTS: u32 = 0x1_0000;
const MULTI_RESULTS: u32 = 0x2_0000;
const PS_MULTI_RESULTS: u32 = 0x4_0000;
const PLUGIN_AUTH: u32 = 0x8_0000;
const PLUGIN_AUTH_LENENC_CLIENT_DATA: u32 = 0x20_0000;

/// The flags a connection asks for whatever the caller passes (mysqlnd's
/// CLIENT_* defaults, MYSQLND_CAPABILITIES).
const BASE_FLAGS: u32 = LONG_PASSWORD
    | LONG_FLAG
    | LOCAL_FILES
    | PROTOCOL_41
    | TRANSACTIONS
    | SECURE_CONNECTION
    | MULTI_RESULTS
    | PS_MULTI_RESULTS
    | PLUGIN_AUTH
    | PLUGIN_AUTH_LENENC_CLIENT_DATA;

/// SERVER_STATUS_IN_TRANS: a transaction is open.
pub const STATUS_IN_TRANS: u16 = 0x01;
/// SERVER_MORE_RESULTS_EXISTS: another result follows this one.
const MORE_RESULTS: u16 = 0x08;
/// SERVER_STATUS_NO_BACKSLASH_ESCAPES: the NO_BACKSLASH_ESCAPES SQL mode.
pub const STATUS_NO_BACKSLASH_ESCAPES: u16 = 0x200;

/// What mysqlnd calls itself (mysqlnd_get_client_info()).
pub const CLIENT_INFO: &str = "mysqlnd 8.4.27";

/// utf8mb4_general_ci, the collation mysqlnd connects with (utf8mb4).
pub const DEFAULT_CHARSET: u16 = 45;

// Client error codes (CR_*), with the messages mysqlnd gives them.
pub const CR_CONNECTION_ERROR: u16 = 2002;
pub const CR_SERVER_GONE_ERROR: u16 = 2006;
pub const CR_COMMANDS_OUT_OF_SYNC: u16 = 2014;
pub const CR_CANT_READ_CHARSET: u16 = 2019;
pub const CR_LOAD_DATA_LOCAL_INFILE_REJECTED: u16 = 2068;
pub const CR_AUTH_PLUGIN_CANNOT_LOAD: u16 = 2059;
pub const CR_MALFORMED_PACKET: u16 = 2027;

/// A failure, as mysqli reports it: a code, its SQLSTATE and the message.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub code: u16,
    pub sqlstate: String,
    pub message: String,
}

impl Error {
    fn client(code: u16, message: impl Into<String>) -> Error {
        Error { code, sqlstate: "HY000".into(), message: message.into() }
    }

    fn gone() -> Error {
        Error::client(CR_SERVER_GONE_ERROR, "MySQL server has gone away")
    }

    fn malformed() -> Error {
        Error::client(CR_MALFORMED_PACKET, "Malformed packet")
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Where to connect: a TCP host and port, or a Unix socket.
pub enum Address<'a> {
    Tcp { host: &'a str, port: u16 },
    Socket(&'a str),
}

pub struct ConnectOptions<'a> {
    pub address: Address<'a>,
    pub user: &'a [u8],
    pub password: &'a [u8],
    pub database: Option<&'a [u8]>,
    /// CLIENT_* flags to add (mysqli_real_connect's $flags).
    pub flags: u32,
    pub timeout: Option<Duration>,
    /// The collation the handshake asks for (the connection's character
    /// set): [`DEFAULT_CHARSET`] unless the caller names another.
    pub charset: u16,
}

enum Stream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.read(buf),
            Stream::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.write(buf),
            Stream::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Tcp(s) => s.flush(),
            Stream::Unix(s) => s.flush(),
        }
    }
}

/// A column of a result set (COM_QUERY's column definition).
#[derive(Debug, Clone, Default)]
pub struct Column {
    pub catalog: Vec<u8>,
    pub db: Vec<u8>,
    pub table: Vec<u8>,
    pub org_table: Vec<u8>,
    pub name: Vec<u8>,
    pub org_name: Vec<u8>,
    pub charsetnr: u16,
    pub length: u32,
    pub kind: u8,
    pub flags: u16,
    pub decimals: u8,
}

/// A row: each value as the text protocol sends it, or None for NULL.
pub type Row = Vec<Option<Vec<u8>>>;

/// A result set, read in full.
#[derive(Debug, Default)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
}

/// What a statement gave: rows, or what it changed (an OK packet, whose
/// numbers the connection keeps).
pub enum Outcome {
    Rows(ResultSet),
    Done,
}

/// The start of a result: what the statement changed (an OK packet), or
/// the columns of a result set whose rows are still to be read with
/// [`Connection::read_row`].
pub enum Header {
    Done,
    Columns(Vec<Column>),
}

/// A statement prepared on the server (COM_STMT_PREPARE).
#[derive(Debug, Clone)]
pub struct Prepared {
    pub id: u32,
    /// How many `?` it has.
    pub params: usize,
    /// The columns of its result, as far as the server knows them before
    /// it runs (SHOW and CALL only know after).
    pub columns: Vec<Column>,
}

/// A value bound to a `?` of a prepared statement, by the type mysqlnd
/// sends it as (mysqlnd_stmt_execute_store_types()).
#[derive(Debug, Clone, PartialEq)]
pub enum Param<'a> {
    Null,
    /// MYSQL_TYPE_LONGLONG.
    Int(i64),
    /// MYSQL_TYPE_DOUBLE.
    Double(f64),
    /// MYSQL_TYPE_TINY (a PHP bool).
    Tiny(i8),
    /// MYSQL_TYPE_VAR_STRING.
    Bytes(&'a [u8]),
}

/// A column of a row as PHP gets it from mysqlnd.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Null,
    Int(i64),
    Double(f64),
    Bytes(Vec<u8>),
}

// Field types (MYSQL_TYPE_*).
pub const TYPE_DECIMAL: u8 = 0;
pub const TYPE_TINY: u8 = 1;
pub const TYPE_SHORT: u8 = 2;
pub const TYPE_LONG: u8 = 3;
pub const TYPE_FLOAT: u8 = 4;
pub const TYPE_DOUBLE: u8 = 5;
pub const TYPE_NULL: u8 = 6;
pub const TYPE_TIMESTAMP: u8 = 7;
pub const TYPE_LONGLONG: u8 = 8;
pub const TYPE_INT24: u8 = 9;
pub const TYPE_DATE: u8 = 10;
pub const TYPE_TIME: u8 = 11;
pub const TYPE_DATETIME: u8 = 12;
pub const TYPE_YEAR: u8 = 13;
pub const TYPE_NEWDATE: u8 = 14;
pub const TYPE_VARCHAR: u8 = 15;
pub const TYPE_BIT: u8 = 16;
pub const TYPE_VECTOR: u8 = 242;
pub const TYPE_JSON: u8 = 245;
pub const TYPE_NEWDECIMAL: u8 = 246;
pub const TYPE_ENUM: u8 = 247;
pub const TYPE_SET: u8 = 248;
pub const TYPE_TINY_BLOB: u8 = 249;
pub const TYPE_MEDIUM_BLOB: u8 = 250;
pub const TYPE_LONG_BLOB: u8 = 251;
pub const TYPE_BLOB: u8 = 252;
pub const TYPE_VAR_STRING: u8 = 253;
pub const TYPE_STRING: u8 = 254;
pub const TYPE_GEOMETRY: u8 = 255;

// Column flags.
pub const NOT_NULL_FLAG: u16 = 1;
pub const PRI_KEY_FLAG: u16 = 2;
pub const UNIQUE_KEY_FLAG: u16 = 4;
pub const MULTIPLE_KEY_FLAG: u16 = 8;
pub const BLOB_FLAG: u16 = 16;
pub const UNSIGNED_FLAG: u16 = 32;
pub const ZEROFILL_FLAG: u16 = 64;

/// An open connection, and what the server last reported.
pub struct Connection {
    stream: Stream,
    seq: u8,
    pub protocol_version: u8,
    pub server_version: String,
    pub thread_id: u32,
    pub host_info: String,
    pub charset: u16,
    pub status: u16,
    pub affected_rows: u64,
    pub insert_id: u64,
    pub warnings: u16,
    pub info: Vec<u8>,
    /// Another result of the last statement is waiting to be read.
    pending: bool,
    /// The rows of the current result are still to be read: Some(true)
    /// when they are binary-protocol rows (of a prepared statement).
    reading: Option<bool>,
    /// The results of the last command come in the binary protocol.
    binary: bool,
}

/// Reads a packet's fields in order.
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Cursor<'a> {
        Cursor { data, at: 0 }
    }

    fn rest(&self) -> &'a [u8] {
        &self.data[self.at.min(self.data.len())..]
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len()).ok_or_else(Error::malformed)?;
        let b = &self.data[self.at..end];
        self.at = end;
        Ok(b)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A length-encoded integer (None: 0xFB, the NULL marker in a row).
    fn lenenc(&mut self) -> Result<Option<u64>> {
        Ok(Some(match self.u8()? {
            0xFB => return Ok(None),
            0xFC => self.u16()? as u64,
            0xFD => {
                let b = self.bytes(3)?;
                u32::from_le_bytes([b[0], b[1], b[2], 0]) as u64
            }
            0xFE => {
                let b = self.bytes(8)?;
                u64::from_le_bytes(b.try_into().expect("8 bytes"))
            }
            n => n as u64,
        }))
    }

    fn lenenc_int(&mut self) -> Result<u64> {
        self.lenenc()?.ok_or_else(Error::malformed)
    }

    fn lenenc_bytes(&mut self) -> Result<Option<&'a [u8]>> {
        match self.lenenc()? {
            Some(n) => Ok(Some(self.bytes(usize::try_from(n).map_err(|_| Error::malformed())?)?)),
            None => Ok(None),
        }
    }

    /// A NUL-terminated string (to the end when there is no NUL).
    fn nul_str(&mut self) -> &'a [u8] {
        let rest = self.rest();
        let n = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
        self.at += (n + 1).min(rest.len());
        &rest[..n]
    }
}

fn put_lenenc(out: &mut Vec<u8>, n: u64) {
    match n {
        0..=250 => out.push(n as u8),
        251..=0xFFFF => {
            out.push(0xFC);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        0x1_0000..=0xFF_FFFF => {
            out.push(0xFD);
            out.extend_from_slice(&(n as u32).to_le_bytes()[..3]);
        }
        _ => {
            out.push(0xFE);
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

/// mysql_native_password: SHA1(password) XOR SHA1(scramble + SHA1(SHA1(password))).
fn native_password(password: &[u8], scramble: &[u8]) -> Vec<u8> {
    if password.is_empty() {
        return Vec::new();
    }
    let stage1 = Sha1::digest(password);
    let stage2 = Sha1::digest(stage1);
    let mut h = Sha1::new();
    h.update(&scramble[..scramble.len().min(20)]);
    h.update(stage2);
    let mix = h.finalize();
    stage1.iter().zip(mix.iter()).map(|(a, b)| a ^ b).collect()
}

/// caching_sha2_password's fast path: SHA256(password) XOR
/// SHA256(SHA256(SHA256(password)) + scramble).
fn caching_sha2_password(password: &[u8], scramble: &[u8]) -> Vec<u8> {
    if password.is_empty() {
        return Vec::new();
    }
    let stage1 = Sha256::digest(password);
    let stage2 = Sha256::digest(stage1);
    let mut h = Sha256::new();
    h.update(stage2);
    h.update(&scramble[..scramble.len().min(20)]);
    let mix = h.finalize();
    stage1.iter().zip(mix.iter()).map(|(a, b)| a ^ b).collect()
}

fn auth_response(plugin: &[u8], password: &[u8], scramble: &[u8]) -> Result<Vec<u8>> {
    match plugin {
        b"mysql_native_password" => Ok(native_password(password, scramble)),
        b"caching_sha2_password" => Ok(caching_sha2_password(password, scramble)),
        b"mysql_clear_password" => {
            let mut p = password.to_vec();
            p.push(0);
            Ok(p)
        }
        other => Err(Error::client(
            CR_AUTH_PLUGIN_CANNOT_LOAD,
            format!("The server requested authentication method unknown to the client [{}]", String::from_utf8_lossy(other)),
        )),
    }
}

/// The server's error packet (after its 0xFF).
fn server_error(packet: &[u8]) -> Error {
    let mut c = Cursor::new(packet);
    let _ = c.u8();
    let code = c.u16().unwrap_or(0);
    let rest = c.rest();
    let (sqlstate, message) = match rest.first() {
        Some(b'#') if rest.len() >= 6 => (String::from_utf8_lossy(&rest[1..6]).into_owned(), &rest[6..]),
        _ => ("HY000".to_string(), rest),
    };
    Error { code, sqlstate, message: String::from_utf8_lossy(message).into_owned() }
}

impl Connection {
    pub fn connect(options: &ConnectOptions) -> Result<Connection> {
        let (stream, host_info) = match options.address {
            Address::Tcp { host, port } => {
                let stream = Self::tcp(host, port, options.timeout)?;
                let _ = stream.set_nodelay(true);
                (Stream::Tcp(stream), format!("{host} via TCP/IP"))
            }
            Address::Socket(path) => {
                let stream = UnixStream::connect(path).map_err(|e| Error::client(CR_CONNECTION_ERROR, io_message(&e)))?;
                (Stream::Unix(stream), "Localhost via UNIX socket".to_string())
            }
        };
        let mut conn = Connection {
            stream,
            seq: 0,
            protocol_version: 10,
            server_version: String::new(),
            thread_id: 0,
            host_info,
            charset: options.charset,
            status: 0,
            affected_rows: 0,
            insert_id: 0,
            warnings: 0,
            info: Vec::new(),
            pending: false,
            reading: None,
            binary: false,
        };
        // A failed connect reports SQLSTATE HY000, whatever the server said.
        conn.handshake(options).map_err(|e| Error { sqlstate: "HY000".into(), ..e })?;
        Ok(conn)
    }

    fn tcp(host: &str, port: u16, timeout: Option<Duration>) -> Result<TcpStream> {
        use std::net::ToSocketAddrs;
        let addrs = (host, port).to_socket_addrs().map_err(|_| Error::client(CR_CONNECTION_ERROR, format!("php_network_getaddresses: getaddrinfo for {host} failed: Name or service not known")))?;
        let mut last = None;
        for addr in addrs {
            let r = match timeout {
                Some(t) => TcpStream::connect_timeout(&addr, t),
                None => TcpStream::connect(addr),
            };
            match r {
                Ok(s) => return Ok(s),
                Err(e) => last = Some(e),
            }
        }
        Err(Error::client(CR_CONNECTION_ERROR, last.map_or_else(|| "Connection refused".to_string(), |e| io_message(&e))))
    }

    /// One packet's payload (joining the 16 MB pieces a large one comes in).
    fn read_packet(&mut self) -> Result<Vec<u8>> {
        let mut payload = Vec::new();
        loop {
            let mut header = [0u8; 4];
            self.stream.read_exact(&mut header).map_err(|_| Error::gone())?;
            let len = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
            self.seq = header[3].wrapping_add(1);
            let start = payload.len();
            payload.resize(start + len, 0);
            self.stream.read_exact(&mut payload[start..]).map_err(|_| Error::gone())?;
            if len < 0xFF_FFFF {
                return Ok(payload);
            }
        }
    }

    fn write_packet(&mut self, payload: &[u8]) -> Result<()> {
        let mut out = Vec::with_capacity(payload.len() + 4);
        let mut chunks = payload.chunks(0xFF_FFFF).peekable();
        if payload.is_empty() {
            out.extend_from_slice(&[0, 0, 0, self.seq]);
            self.seq = self.seq.wrapping_add(1);
        }
        while let Some(chunk) = chunks.next() {
            out.extend_from_slice(&(chunk.len() as u32).to_le_bytes()[..3]);
            out.push(self.seq);
            self.seq = self.seq.wrapping_add(1);
            out.extend_from_slice(chunk);
            // A payload of exactly n * 16 MB ends with an empty packet.
            if chunks.peek().is_none() && chunk.len() == 0xFF_FFFF {
                out.extend_from_slice(&[0, 0, 0, self.seq]);
                self.seq = self.seq.wrapping_add(1);
            }
        }
        self.stream.write_all(&out).and_then(|_| self.stream.flush()).map_err(|_| Error::gone())
    }

    fn handshake(&mut self, options: &ConnectOptions) -> Result<()> {
        let greeting = self.read_packet()?;
        if greeting.first() == Some(&0xFF) {
            return Err(server_error(&greeting));
        }
        let mut c = Cursor::new(&greeting);
        self.protocol_version = c.u8()?;
        self.server_version = String::from_utf8_lossy(c.nul_str()).into_owned();
        self.thread_id = c.u32()?;
        let mut scramble = c.bytes(8)?.to_vec();
        c.u8()?;
        let mut caps = c.u16()? as u32;
        let mut plugin: Vec<u8> = b"mysql_native_password".to_vec();
        if !c.rest().is_empty() {
            c.u8()?; // the server's charset
            self.status = c.u16()?;
            caps |= (c.u16()? as u32) << 16;
            let data_len = c.u8()? as usize;
            c.bytes(10)?;
            if caps & SECURE_CONNECTION != 0 {
                let n = data_len.saturating_sub(8).max(13);
                let part = c.bytes(n.min(c.rest().len()))?;
                // The second part ends with a NUL that is not scramble.
                scramble.extend_from_slice(part.strip_suffix(&[0]).unwrap_or(part));
            }
            if caps & PLUGIN_AUTH != 0 {
                plugin = c.nul_str().to_vec();
            }
        }
        let mut flags = (BASE_FLAGS | options.flags) & caps | PROTOCOL_41;
        if options.database.is_some_and(|d| !d.is_empty()) {
            flags |= CONNECT_WITH_DB & caps;
        }
        let response = auth_response(&plugin, options.password, &scramble)?;
        let mut out = Vec::new();
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&0x0100_0000u32.to_le_bytes());
        out.push(self.charset as u8);
        out.extend_from_slice(&[0; 23]);
        out.extend_from_slice(options.user);
        out.push(0);
        if flags & PLUGIN_AUTH_LENENC_CLIENT_DATA != 0 {
            put_lenenc(&mut out, response.len() as u64);
        } else {
            out.push(response.len() as u8);
        }
        out.extend_from_slice(&response);
        if flags & CONNECT_WITH_DB != 0 {
            out.extend_from_slice(options.database.unwrap_or_default());
            out.push(0);
        }
        if flags & PLUGIN_AUTH != 0 {
            out.extend_from_slice(&plugin);
            out.push(0);
        }
        self.write_packet(&out)?;
        self.authenticate(plugin, options.password)
    }

    /// The server's answers to the handshake response, until OK or ERR.
    fn authenticate(&mut self, mut plugin: Vec<u8>, password: &[u8]) -> Result<()> {
        loop {
            let packet = self.read_packet()?;
            match packet.first() {
                Some(0x00) => {
                    self.read_ok(&packet)?;
                    return Ok(());
                }
                Some(0xFF) => return Err(server_error(&packet)),
                // An auth switch request: another plugin, a new scramble.
                Some(0xFE) => {
                    let mut c = Cursor::new(&packet[1..]);
                    plugin = c.nul_str().to_vec();
                    let data = c.rest();
                    let scramble = data.strip_suffix(&[0]).unwrap_or(data);
                    let response = auth_response(&plugin, password, scramble)?;
                    self.write_packet(&response)?;
                }
                // More data for the plugin: caching_sha2_password's verdict.
                Some(0x01) if plugin == b"caching_sha2_password" => match packet.get(1) {
                    Some(3) => {} // fast auth succeeded; the OK follows
                    Some(4) => {
                        return Err(Error::client(
                            CR_AUTH_PLUGIN_CANNOT_LOAD,
                            "caching_sha2_password full authentication needs a TLS connection or the server's RSA key, which p8 does not support yet",
                        ));
                    }
                    _ => return Err(Error::malformed()),
                },
                _ => return Err(Error::malformed()),
            }
        }
    }

    /// An OK packet: what the statement changed.
    fn read_ok(&mut self, packet: &[u8]) -> Result<()> {
        let mut c = Cursor::new(packet);
        c.u8()?;
        self.affected_rows = c.lenenc_int()?;
        self.insert_id = c.lenenc_int()?;
        self.status = c.u16()?;
        self.warnings = c.u16()?;
        // The message, if any, is length-encoded (mysqlnd reads it so).
        self.info = if c.rest().is_empty() { Vec::new() } else { c.lenenc_bytes()?.unwrap_or_default().to_vec() };
        self.pending = self.status & MORE_RESULTS != 0;
        Ok(())
    }

    /// An EOF packet's warnings and status.
    fn read_eof(&mut self, packet: &[u8]) {
        let mut c = Cursor::new(packet);
        let _ = c.u8();
        if let (Ok(w), Ok(s)) = (c.u16(), c.u16()) {
            self.warnings = w;
            self.status = s;
        }
    }

    fn command(&mut self, command: u8, arg: &[u8]) -> Result<()> {
        if self.pending || self.reading.is_some() {
            return Err(Error::client(CR_COMMANDS_OUT_OF_SYNC, "Commands out of sync; you can't run this command now"));
        }
        self.seq = 0;
        let mut out = Vec::with_capacity(arg.len() + 1);
        out.push(command);
        out.extend_from_slice(arg);
        self.write_packet(&out)
    }

    /// COM_QUERY: run `sql` and read its first result.
    pub fn query(&mut self, sql: &[u8]) -> Result<Outcome> {
        let header = self.query_start(sql)?;
        self.store(header)
    }

    /// COM_QUERY: run `sql` and read the start of its first result; the
    /// rows of a result set are left for [`Connection::read_row`].
    pub fn query_start(&mut self, sql: &[u8]) -> Result<Header> {
        self.command(0x03, sql)?;
        self.binary = false;
        self.read_header()
    }

    /// A result whose header was read, with its rows.
    fn store(&mut self, header: Header) -> Result<Outcome> {
        match header {
            Header::Done => Ok(Outcome::Done),
            Header::Columns(columns) => {
                let mut rows = Vec::new();
                while let Some(packet) = self.read_row()? {
                    rows.push(text_row(&packet, columns.len())?);
                }
                self.affected_rows = rows.len() as u64;
                Ok(Outcome::Rows(ResultSet { columns, rows }))
            }
        }
    }

    /// Another result of the last statement waits (more_results()).
    pub fn more_results(&self) -> bool {
        self.pending
    }

    /// The rows of the current result are still being read (an unbuffered
    /// result): nothing else can run until they are.
    pub fn reading_rows(&self) -> bool {
        self.reading.is_some()
    }

    /// The next result of the last statement, if there is one.
    pub fn next_result(&mut self) -> Option<Result<Outcome>> {
        let header = self.next_header()?;
        Some(header.and_then(|h| self.store(h)))
    }

    /// The start of the next result of the last statement, if there is one.
    pub fn next_header(&mut self) -> Option<Result<Header>> {
        if !self.pending || self.reading.is_some() {
            return None;
        }
        self.pending = false;
        Some(self.read_header())
    }

    fn read_header(&mut self) -> Result<Header> {
        let packet = self.read_packet()?;
        match packet.first() {
            Some(0x00) => {
                self.read_ok(&packet)?;
                Ok(Header::Done)
            }
            Some(0xFF) => {
                self.pending = false;
                Err(server_error(&packet))
            }
            Some(0xFB) => {
                // LOAD DATA LOCAL INFILE: the server asks for a file. None
                // is sent (an empty packet), and the server's answer to
                // that is read so the connection stays in step.
                self.write_packet(&[])?;
                let answer = self.read_packet()?;
                if answer.first() == Some(&0x00) {
                    self.read_ok(&answer)?;
                }
                self.pending = false;
                Err(Error::client(
                    CR_LOAD_DATA_LOCAL_INFILE_REJECTED,
                    "LOAD DATA LOCAL INFILE is forbidden, check related settings like mysqli.allow_local_infile|mysqli.local_infile_directory or PDO::MYSQL_ATTR_LOCAL_INFILE|PDO::MYSQL_ATTR_LOCAL_INFILE_DIRECTORY",
                ))
            }
            _ => {
                // A result set: what the last statement changed is reset
                // (UPSERT_STATUS_RESET), its affected rows unknown (-1).
                self.affected_rows = u64::MAX;
                self.insert_id = 0;
                self.warnings = 0;
                self.info.clear();
                let count = Cursor::new(&packet).lenenc_int()? as usize;
                let columns = self.columns(count)?;
                self.reading = Some(self.binary);
                Ok(Header::Columns(columns))
            }
        }
    }

    /// `count` column definitions and the EOF packet after them.
    fn columns(&mut self, count: usize) -> Result<Vec<Column>> {
        let mut columns = Vec::with_capacity(count);
        for _ in 0..count {
            let packet = self.read_packet()?;
            columns.push(Self::column(&packet)?);
        }
        let eof = self.read_packet()?;
        self.read_eof(&eof);
        Ok(columns)
    }

    /// The next row of the current result, as the server sent it (decode
    /// it with [`text_row`], [`text_values`] or [`binary_values`]); None at
    /// its end.
    pub fn read_row(&mut self) -> Result<Option<Vec<u8>>> {
        if self.reading.is_none() {
            return Ok(None);
        }
        let packet = match self.read_packet() {
            Ok(p) => p,
            Err(e) => {
                self.reading = None;
                return Err(e);
            }
        };
        match packet.first() {
            Some(0xFE) if packet.len() < 9 => {
                self.read_eof(&packet);
                self.reading = None;
                self.pending = self.status & MORE_RESULTS != 0;
                Ok(None)
            }
            Some(0xFF) => {
                self.reading = None;
                self.pending = false;
                Err(server_error(&packet))
            }
            _ => Ok(Some(packet)),
        }
    }

    /// Read and drop the rest of the current result's rows.
    pub fn skip_rows(&mut self) -> Result<()> {
        while self.read_row()?.is_some() {}
        Ok(())
    }

    /// COM_STMT_PREPARE: prepare `sql` on the server.
    pub fn prepare(&mut self, sql: &[u8]) -> Result<Prepared> {
        self.command(0x16, sql)?;
        let packet = self.read_packet()?;
        match packet.first() {
            Some(0x00) => {}
            Some(0xFF) => return Err(server_error(&packet)),
            _ => return Err(Error::malformed()),
        }
        let mut c = Cursor::new(&packet[1..]);
        let id = c.u32()?;
        let ncolumns = c.u16()? as usize;
        let params = c.u16()? as usize;
        if c.rest().len() >= 3 {
            c.u8()?;
            self.warnings = c.u16()?;
        }
        if params > 0 {
            self.columns(params)?;
        }
        let columns = if ncolumns > 0 { self.columns(ncolumns)? } else { Vec::new() };
        Ok(Prepared { id, params, columns })
    }

    /// COM_STMT_EXECUTE: run prepared statement `id` with `params` and read
    /// the start of its first result (its rows are binary-protocol rows).
    pub fn execute(&mut self, id: u32, params: &[Param]) -> Result<Header> {
        let mut out = Vec::new();
        out.extend_from_slice(&id.to_le_bytes());
        // No cursor; one iteration.
        out.push(0);
        out.extend_from_slice(&1u32.to_le_bytes());
        if !params.is_empty() {
            let mut nulls = vec![0u8; params.len().div_ceil(8)];
            for (i, p) in params.iter().enumerate() {
                if *p == Param::Null {
                    nulls[i / 8] |= 1 << (i % 8);
                }
            }
            out.extend_from_slice(&nulls);
            // The types follow.
            out.push(1);
            for p in params {
                let kind = match p {
                    Param::Null => TYPE_NULL,
                    Param::Int(_) => TYPE_LONGLONG,
                    Param::Double(_) => TYPE_DOUBLE,
                    Param::Tiny(_) => TYPE_TINY,
                    Param::Bytes(_) => TYPE_VAR_STRING,
                };
                out.extend_from_slice(&[kind, 0]);
            }
            for p in params {
                match p {
                    Param::Null => {}
                    Param::Int(i) => out.extend_from_slice(&i.to_le_bytes()),
                    Param::Double(d) => out.extend_from_slice(&d.to_le_bytes()),
                    Param::Tiny(t) => out.extend_from_slice(&t.to_le_bytes()),
                    Param::Bytes(b) => {
                        put_lenenc(&mut out, b.len() as u64);
                        out.extend_from_slice(b);
                    }
                }
            }
        }
        self.command(0x17, &out)?;
        self.binary = true;
        self.read_header()
    }

    /// COM_STMT_CLOSE: the server forgets statement `id` (no answer comes).
    /// Nothing is sent while another result is being read.
    pub fn close_statement(&mut self, id: u32) {
        let _ = self.command(0x19, &id.to_le_bytes());
    }

    /// COM_STATISTICS: the server's status line (mysqlnd_stat()).
    pub fn stat(&mut self) -> Result<Vec<u8>> {
        self.command(0x09, b"")?;
        let packet = self.read_packet()?;
        match packet.first() {
            Some(0xFF) => Err(server_error(&packet)),
            _ => Ok(packet),
        }
    }

    fn column(packet: &[u8]) -> Result<Column> {
        let mut c = Cursor::new(packet);
        let mut s = || c.lenenc_bytes().map(|b| b.unwrap_or_default().to_vec());
        let (catalog, db, table, org_table, name, org_name) = (s()?, s()?, s()?, s()?, s()?, s()?);
        c.lenenc_int()?;
        let charsetnr = c.u16()?;
        let length = c.u32()?;
        let kind = c.u8()?;
        let mut flags = c.u16()?;
        let decimals = c.u8()?;
        // NUM_FLAG, which mysqlnd sets for the numeric types (as libmysql
        // does): up to INT24 (TIMESTAMP only with length 14 or 8), and YEAR.
        if (kind <= 9 && (kind != 7 || length == 14 || length == 8)) || kind == 13 {
            flags |= 0x8000;
        }
        Ok(Column { catalog, db, table, org_table, name, org_name, charsetnr, length, kind, flags, decimals })
    }

    /// COM_INIT_DB: make `db` the default database.
    pub fn select_db(&mut self, db: &[u8]) -> Result<()> {
        self.command(0x02, db)?;
        let packet = self.read_packet()?;
        match packet.first() {
            Some(0x00) => self.read_ok(&packet),
            Some(0xFF) => Err(server_error(&packet)),
            _ => Err(Error::malformed()),
        }
    }

    /// COM_PING.
    pub fn ping(&mut self) -> Result<()> {
        self.command(0x0E, b"")?;
        let packet = self.read_packet()?;
        match packet.first() {
            Some(0x00) => self.read_ok(&packet),
            Some(0xFF) => Err(server_error(&packet)),
            _ => Err(Error::malformed()),
        }
    }

    /// COM_QUIT; the connection is closed either way.
    pub fn close(mut self) {
        if !self.pending {
            let _ = self.command(0x01, b"");
        }
    }

    /// Change the connection's character set (SET NAMES).
    pub fn set_charset(&mut self, name: &str) -> Result<()> {
        let Some(cs) = charset(name) else {
            return Err(Error::client(2019, "Invalid character set was provided"));
        };
        match self.query(format!("SET NAMES {}", cs.name).as_bytes())? {
            Outcome::Done | Outcome::Rows(_) => {}
        }
        self.charset = cs.nr;
        Ok(())
    }
}

/// A text-protocol row: each of its `n` values as the server sent it, or
/// None for NULL.
pub fn text_row(packet: &[u8], n: usize) -> Result<Row> {
    let mut c = Cursor::new(packet);
    let mut row = Vec::with_capacity(n);
    for _ in 0..n {
        row.push(c.lenenc_bytes()?.map(<[u8]>::to_vec));
    }
    Ok(row)
}

/// atoll()/strtoull(): the leading integer of `s`, wrapping as C does.
fn c_integer(s: &[u8]) -> u64 {
    let s = s.trim_ascii_start();
    let (neg, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits.iter().take_while(|c| c.is_ascii_digit()).fold(0u64, |n, &c| n.wrapping_mul(10).wrapping_add(u64::from(c - b'0')));
    if neg { n.wrapping_neg() } else { n }
}

/// The integer types mysqlnd reads into a PHP int (php_type IS_LONG), with
/// how many bytes the binary protocol gives them.
fn int_bytes(kind: u8) -> Option<usize> {
    match kind {
        TYPE_TINY => Some(1),
        TYPE_SHORT | TYPE_YEAR => Some(2),
        TYPE_INT24 | TYPE_LONG => Some(4),
        TYPE_LONGLONG => Some(8),
        _ => None,
    }
}

/// ps_fetch_from_1_to_8_bytes(): an integer of `bytes` (little-endian; a
/// BIT is big-endian), signed or not, zero-filled to the column's width.
fn fixed_int(column: &Column, bytes: &[u8]) -> FieldValue {
    if column.flags & UNSIGNED_FLAG != 0 {
        let u = if column.kind == TYPE_BIT {
            bytes.iter().fold(0u64, |n, &b| (n << 8) | u64::from(b))
        } else {
            bytes.iter().rev().fold(0u64, |n, &b| (n << 8) | u64::from(b))
        };
        if column.flags & ZEROFILL_FLAG != 0 {
            return FieldValue::Bytes(format!("{u:0width$}", width = column.length as usize).into_bytes());
        }
        return match i64::try_from(u) {
            Ok(i) => FieldValue::Int(i),
            Err(_) => FieldValue::Bytes(u.to_string().into_bytes()),
        };
    }
    // Signed: 1 to 4 bytes or 8, sign-extended from the top one.
    let n = bytes.len();
    if !matches!(n, 1..=4 | 8) {
        return FieldValue::Int(0);
    }
    let mut b = if bytes[n - 1] & 0x80 != 0 { [0xFF; 8] } else { [0; 8] };
    b[..n].copy_from_slice(bytes);
    FieldValue::Int(i64::from_le_bytes(b))
}

/// A text-protocol row decoded as mysqlnd does it
/// (php_mysqlnd_rowp_read_text_protocol()): with `native` (the
/// MYSQLND_OPT_INT_AND_FLOAT_NATIVE option), integer columns are ints
/// (unless zero-filled, or unsigned and past PHP_INT_MAX) and FLOAT and
/// DOUBLE floats; a BIT is an int with `native`, its decimal digits without.
pub fn text_values(packet: &[u8], columns: &[Column], native: bool) -> Result<Vec<FieldValue>> {
    let row = text_row(packet, columns.len())?;
    Ok(row
        .into_iter()
        .zip(columns)
        .map(|(v, col)| {
            let Some(v) = v else { return FieldValue::Null };
            if col.kind == TYPE_BIT {
                if v.is_empty() || v.len() > 8 {
                    return FieldValue::Bytes(v);
                }
                return match fixed_int(col, &v) {
                    FieldValue::Int(i) if !native => FieldValue::Bytes((i as u64).to_string().into_bytes()),
                    other => other,
                };
            }
            if native && col.flags & ZEROFILL_FLAG == 0 {
                if let Some(n) = int_bytes(col.kind) {
                    let u = c_integer(&v);
                    if n < 8 || col.flags & UNSIGNED_FLAG == 0 || i64::try_from(u).is_ok() {
                        return FieldValue::Int(u as i64);
                    }
                    return FieldValue::Bytes(v);
                }
                if col.kind == TYPE_FLOAT || col.kind == TYPE_DOUBLE {
                    // zend_strtod() of the server's text.
                    return FieldValue::Double(String::from_utf8_lossy(&v).trim().parse().unwrap_or(0.0));
                }
            }
            FieldValue::Bytes(v)
        })
        .collect())
}

/// mysql_float_to_double(): a FLOAT as the double PHP shows, through its
/// decimal text: `decimals` digits after the point, or six significant
/// digits when the column has no fixed decimals (31 and up).
fn float_to_double(f: f32, decimals: u8) -> f64 {
    let text = if decimals >= 31 { format!("{:.5e}", f64::from(f)) } else { format!("{:.*}", usize::from(decimals), f64::from(f)) };
    text.parse().unwrap_or(0.0)
}

/// A binary-protocol date, time or datetime (ps_fetch_date(),
/// ps_fetch_time(), ps_fetch_datetime()): `to` is what follows its length.
fn temporal(column: &Column, to: &[u8]) -> Result<Vec<u8>> {
    let min = if column.kind == TYPE_TIME { 8 } else { 4 };
    if !to.is_empty() && to.len() < min {
        return Err(Error::malformed());
    }
    let at = |i: usize| u32::from(to.get(i).copied().unwrap_or(0));
    let u32_at = |i: usize| to.get(i..i + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    // Fractional seconds to the column's decimals.
    let fraction = |micros: u32| match u32::from(column.decimals) {
        d @ 1..=6 => format!(".{:0w$}", micros / 10u32.pow(6 - d), w = d as usize),
        _ => String::new(),
    };
    let date = || if to.is_empty() { (0, 0, 0) } else { (at(0) | at(1) << 8, at(2), at(3)) };
    Ok(match column.kind {
        TYPE_DATE => {
            let (y, m, d) = date();
            format!("{y:04}-{m:02}-{d:02}")
        }
        TYPE_TIME => {
            let neg = to.first().is_some_and(|&b| b != 0);
            // Days count as hours.
            let h = at(5) + u32_at(1) * 24;
            let micros = if to.len() >= 12 { u32_at(8) } else { 0 };
            format!("{}{h:02}:{:02}:{:02}{}", if neg { "-" } else { "" }, at(6), at(7), fraction(micros))
        }
        _ => {
            let (y, m, d) = date();
            let micros = if to.len() >= 11 { u32_at(7) } else { 0 };
            format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}{}", at(4), at(5), at(6), fraction(micros))
        }
    }
    .into_bytes())
}

/// A binary-protocol row decoded as mysqlnd does it
/// (php_mysqlnd_rowp_read_binary_protocol() and the ps_fetch_* functions):
/// integers are ints (unsigned past PHP_INT_MAX and zero-filled ones are
/// strings), FLOAT and DOUBLE floats, dates and times their text.
pub fn binary_values(packet: &[u8], columns: &[Column]) -> Result<Vec<FieldValue>> {
    let mut c = Cursor::new(packet);
    c.u8()?;
    let nulls = c.bytes((columns.len() + 9) / 8)?;
    let mut out = Vec::with_capacity(columns.len());
    for (i, col) in columns.iter().enumerate() {
        // The first two bits of the NULL bitmap are reserved.
        let bit = i + 2;
        if nulls[bit / 8] & (1 << (bit % 8)) != 0 {
            out.push(FieldValue::Null);
            continue;
        }
        let v = match col.kind {
            TYPE_NULL => FieldValue::Null,
            TYPE_FLOAT => {
                let b = c.bytes(4)?;
                FieldValue::Double(float_to_double(f32::from_le_bytes([b[0], b[1], b[2], b[3]]), col.decimals))
            }
            TYPE_DOUBLE => FieldValue::Double(f64::from_le_bytes(c.bytes(8)?.try_into().expect("8 bytes"))),
            TYPE_DATE | TYPE_TIME | TYPE_DATETIME | TYPE_TIMESTAMP => {
                let n = usize::try_from(c.lenenc_int()?).map_err(|_| Error::malformed())?;
                FieldValue::Bytes(temporal(col, c.bytes(n)?)?)
            }
            TYPE_BIT => {
                let b = c.lenenc_bytes()?.unwrap_or_default();
                if b.is_empty() || b.len() > 8 { FieldValue::Bytes(b.to_vec()) } else { fixed_int(col, b) }
            }
            kind => match int_bytes(kind) {
                Some(n) => fixed_int(col, c.bytes(n)?),
                None => FieldValue::Bytes(c.lenenc_bytes()?.unwrap_or_default().to_vec()),
            },
        };
        out.push(v);
    }
    Ok(out)
}

/// The OS error text mysqlnd shows (without Rust's " (os error N)").
fn io_message(e: &io::Error) -> String {
    let s = e.to_string();
    match s.find(" (os error") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

/// A character set and collation mysqlnd knows: the collation's number,
/// the set's name, the most bytes a character takes, and how its
/// multi-byte characters are told apart (for escaping).
#[derive(Clone, Copy, Debug)]
pub struct Charset {
    pub nr: u16,
    pub name: &'static str,
    pub collation: &'static str,
    pub max_len: u8,
    pub comment: &'static str,
    pub mb: Mb,
    /// The lowest byte that may start a multi-byte character (0x100: none).
    pub lowest_mb: u16,
}

/// mysqlnd's multi-byte character checks (check_mb_* and mysqlnd_mbcharlen_*).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mb {
    Null,
    Big5,
    Cp932,
    EucKr,
    EucJpMs,
    Gb2312,
    Gbk,
    Sjis,
    Ucs2,
    Ujis,
    Utf16,
    Utf32,
    Utf8,
    Utf8Mb3,
    Gb18030,
}

mod charsets;
pub use charsets::CHARSETS;

/// The character set `name` (any case; utf8mb3 is utf8), by its default
/// collation (the first of the name, as mysqlnd_find_charset_name finds it).
pub fn charset(name: &str) -> Option<Charset> {
    let lower = name.to_ascii_lowercase();
    let lower = if lower == "utf8mb3" { "utf8".to_string() } else { lower };
    CHARSETS.iter().find(|c| c.name == lower).copied()
}

pub fn charset_by_nr(nr: u16) -> Option<Charset> {
    CHARSETS.iter().find(|c| c.nr == nr).copied()
}

fn utf8_len(s: &[u8], max: usize) -> usize {
    let Some(&c) = s.first() else { return 0 };
    let tail = |i: usize| s.get(i).is_some_and(|&b| (b ^ 0x80) < 0x40);
    match c {
        0x00..=0x7F => 1,
        0x80..=0xC1 => 0,
        0xC2..=0xDF => if tail(1) { 2 } else { 0 },
        0xE0..=0xEF => if tail(1) && tail(2) && (c >= 0xE1 || s[1] >= 0xA0) { 3 } else { 0 },
        0xF0..=0xF4 if max == 4 && tail(1) && tail(2) && tail(3) && (c >= 0xF1 || s[1] >= 0x90) && (c <= 0xF3 || s[1] <= 0x8F) => 4,
        _ => 0,
    }
}

impl Mb {
    /// check_mb_*: the length of the valid multi-byte character at the
    /// start of `s`, or 0.
    fn valid(self, s: &[u8]) -> usize {
        let at = |i: usize| s.get(i).copied();
        let two = |head: fn(u8) -> bool, tail: fn(u8) -> bool| match (at(0), at(1)) {
            (Some(a), Some(b)) if head(a) && tail(b) => 2,
            _ => 0,
        };
        match self {
            Mb::Null => 0,
            Mb::Utf8 => match utf8_len(s, 4) { n if n > 1 => n, _ => 0 },
            Mb::Utf8Mb3 => match utf8_len(s, 3) { n if n > 1 => n, _ => 0 },
            Mb::Big5 => two(|c| (0xA1..=0xF9).contains(&c), |c| (0x40..=0x7E).contains(&c) || (0xA1..=0xFE).contains(&c)),
            Mb::Cp932 | Mb::Sjis => two(|c| (0x81..=0x9F).contains(&c) || (0xE0..=0xFC).contains(&c), |c| (0x40..=0x7E).contains(&c) || (0x80..=0xFC).contains(&c)),
            Mb::Gb2312 => two(|c| (0xA1..=0xF7).contains(&c), |c| (0xA1..=0xFE).contains(&c)),
            Mb::Gbk => two(|c| (0x81..=0xFE).contains(&c), |c| (0x40..=0x7E).contains(&c) || (0x80..=0xFE).contains(&c)),
            Mb::EucKr => match (at(0), at(1)) {
                (Some(a), Some(b)) if a >= 0x80 && (0xA1..=0xFE).contains(&b) => 2,
                _ => 0,
            },
            Mb::EucJpMs | Mb::Ujis => {
                let valid = |c: Option<u8>| c.is_some_and(|c| (0xA1..=0xFE).contains(&c));
                let kata = |c: Option<u8>| c.is_some_and(|c| (0xA1..=0xDF).contains(&c));
                match at(0) {
                    Some(c) if c < 0x80 => 0,
                    c if valid(c) && valid(at(1)) => 2,
                    Some(0x8E) if kata(at(1)) => 2,
                    Some(0x8F) if valid(at(1)) && valid(at(2)) => 3,
                    _ => 0,
                }
            }
            Mb::Ucs2 => 2,
            Mb::Utf32 => 4,
            Mb::Utf16 => {
                let high = |c: u8| c & 0xFC == 0xD8;
                let low = |c: u8| c & 0xFC == 0xDC;
                match at(0) {
                    _ if s.len() < 2 => 0,
                    Some(c) if high(c) => if s.len() >= 4 && low(s[2]) { 4 } else { 0 },
                    Some(c) if low(c) => 0,
                    _ => 2,
                }
            }
            Mb::Gb18030 => {
                let odd = |c: u8| (0x81..=0xFE).contains(&c);
                let even2 = |c: u8| (0x40..=0x7E).contains(&c) || (0x80..=0xFE).contains(&c);
                let even4 = |c: u8| (0x30..=0x39).contains(&c);
                if s.len() <= 1 || !odd(s[0]) {
                    0
                } else if even2(s[1]) {
                    2
                } else if s.len() > 3 && even4(s[1]) && odd(s[2]) && even4(s[3]) {
                    4
                } else {
                    0
                }
            }
        }
    }

    /// mysqlnd_mbcharlen_*: how long a character starting with byte `c` is.
    fn char_len(self, c: u8) -> usize {
        match self {
            Mb::Null => 1,
            Mb::Utf8 | Mb::Utf8Mb3 => match c {
                0x00..=0x7F => 1,
                0x80..=0xC1 => 0,
                0xC2..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 if self == Mb::Utf8 => 4,
                _ => 0,
            },
            Mb::Big5 => if (0xA1..=0xF9).contains(&c) { 2 } else { 1 },
            Mb::Cp932 | Mb::Sjis => if (0x81..=0x9F).contains(&c) || (0xE0..=0xFC).contains(&c) { 2 } else { 1 },
            Mb::EucKr => if (0xA1..=0xFE).contains(&c) { 2 } else { 1 },
            Mb::EucJpMs | Mb::Ujis => match c {
                0xA1..=0xFE | 0x8E => 2,
                0x8F => 3,
                _ => 1,
            },
            Mb::Gb2312 => if (0xA1..=0xF7).contains(&c) { 2 } else { 1 },
            Mb::Gbk => if (0x81..=0xFE).contains(&c) { 2 } else { 1 },
            Mb::Ucs2 => 2,
            Mb::Utf32 => 4,
            Mb::Utf16 => if c & 0xFC == 0xD8 { 4 } else { 2 },
            Mb::Gb18030 => usize::from(!(0x81..=0xFE).contains(&c)),
        }
    }
}

/// mysql_real_escape_string() in the connection's character set, for a
/// connection without NO_BACKSLASH_ESCAPES (mysqlnd_cset_escape_slashes):
/// a valid multi-byte character is copied as it is (its trailing byte may
/// be a \ or a '), the lone start of one gets a backslash; NUL, \n, \r,
/// \, ', " and Ctrl-Z get a backslash.
pub fn escape(cs: &Charset, s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + s.len() / 8);
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        let mut esc = None;
        if u16::from(c) >= cs.lowest_mb {
            let len = cs.mb.valid(&s[i..]);
            if len > 0 {
                out.extend_from_slice(&s[i..i + len]);
                i += len;
                continue;
            }
            if cs.mb.char_len(c) > 1 {
                esc = Some(c);
            }
        }
        let esc = esc.or(match c {
            0 => Some(b'0'),
            b'\n' => Some(b'n'),
            b'\r' => Some(b'r'),
            b'\\' | b'\'' | b'"' => Some(c),
            0x1A => Some(b'Z'),
            _ => None,
        });
        match esc {
            Some(e) => out.extend_from_slice(&[b'\\', e]),
            None => out.push(c),
        }
        i += 1;
    }
    out
}

/// mysql_real_escape_string() for a connection in NO_BACKSLASH_ESCAPES
/// mode (mysqlnd_cset_escape_quotes): a ' is doubled, nothing else changes
/// (a valid multi-byte character is copied whole).
pub fn escape_quotes(cs: &Charset, s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + s.len() / 8);
    let mut i = 0;
    while i < s.len() {
        let len = if cs.max_len > 1 { cs.mb.valid(&s[i..]) } else { 0 };
        if len > 0 {
            out.extend_from_slice(&s[i..i + len]);
            i += len;
            continue;
        }
        if s[i] == b'\'' {
            out.push(b'\'');
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
    fn scrambles() {
        // A known pair: password "secret" with a fixed 20-byte scramble.
        let scramble = b"abcdefghijklmnopqrst";
        let native = native_password(b"secret", scramble);
        assert_eq!(native.len(), 20);
        // XOR with stage1 undoes the mix: the server's check.
        let stage1 = Sha1::digest(b"secret");
        let mut h = Sha1::new();
        h.update(scramble);
        h.update(Sha1::digest(stage1));
        let mix = h.finalize();
        let back: Vec<u8> = native.iter().zip(mix.iter()).map(|(a, b)| a ^ b).collect();
        assert_eq!(back, stage1.to_vec());
        assert!(native_password(b"", scramble).is_empty());
        assert_eq!(caching_sha2_password(b"secret", scramble).len(), 32);
    }

    #[test]
    fn lenenc_round_trip() {
        for n in [0u64, 250, 251, 0xFFFF, 0x1_0000, 0xFF_FFFF, 0x100_0000, u64::MAX] {
            let mut out = Vec::new();
            put_lenenc(&mut out, n);
            assert_eq!(Cursor::new(&out).lenenc_int().unwrap(), n);
        }
    }

    /// A server on a local port that speaks just enough of the protocol:
    /// a greeting (mysql_native_password), checks the client's scramble,
    /// then answers COM_QUERY "rows" with a result set, "bad" with an
    /// error, anything else with an OK; COM_QUIT ends it.
    fn fake_server(password: &'static [u8]) -> u16 {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let send = |s: &mut TcpStream, seq: u8, payload: &[u8]| {
                let mut p = (payload.len() as u32).to_le_bytes()[..3].to_vec();
                p.push(seq);
                p.extend_from_slice(payload);
                s.write_all(&p).unwrap();
            };
            let recv = |s: &mut TcpStream| {
                let mut h = [0u8; 4];
                s.read_exact(&mut h).ok()?;
                let mut p = vec![0; u32::from_le_bytes([h[0], h[1], h[2], 0]) as usize];
                s.read_exact(&mut p).ok()?;
                Some(p)
            };
            let scramble = b"0123456789abcdefghij";
            let mut g = vec![10];
            g.extend_from_slice(b"8.4.0-fake\0");
            g.extend_from_slice(&7u32.to_le_bytes());
            g.extend_from_slice(&scramble[..8]);
            g.push(0);
            let caps = PROTOCOL_41 | SECURE_CONNECTION | PLUGIN_AUTH | CONNECT_WITH_DB | PLUGIN_AUTH_LENENC_CLIENT_DATA;
            g.extend_from_slice(&(caps as u16).to_le_bytes());
            g.push(45);
            g.extend_from_slice(&2u16.to_le_bytes());
            g.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
            g.push(21);
            g.extend_from_slice(&[0; 10]);
            g.extend_from_slice(&scramble[8..]);
            g.push(0);
            g.extend_from_slice(b"mysql_native_password\0");
            send(&mut s, 0, &g);
            let response = recv(&mut s).unwrap();
            // user "u\0", then the length-encoded scramble.
            let at = 32 + response[32..].iter().position(|&c| c == 0).unwrap() + 1;
            let n = response[at] as usize;
            if response[at + 1..at + 1 + n] == native_password(password, scramble)[..] {
                send(&mut s, 2, &[0, 0, 0, 2, 0, 0, 0]);
            } else {
                send(&mut s, 2, b"\xff\x15\x04#28000Access denied");
                return;
            }
            while let Some(cmd) = recv(&mut s) {
                match (cmd[0], &cmd[1..]) {
                    (0x01, _) => return,
                    (0x03, b"rows") => {
                        send(&mut s, 1, &[2]);
                        let col = |name: &[u8], kind: u8| {
                            let mut c = Vec::new();
                            for part in [&b"def"[..], b"db", b"t", b"t", name, name] {
                                put_lenenc(&mut c, part.len() as u64);
                                c.extend_from_slice(part);
                            }
                            c.push(0x0c);
                            c.extend_from_slice(&63u16.to_le_bytes());
                            c.extend_from_slice(&11u32.to_le_bytes());
                            c.push(kind);
                            c.extend_from_slice(&[0, 0, 0, 0, 0]);
                            c
                        };
                        send(&mut s, 2, &col(b"id", 3));
                        send(&mut s, 3, &col(b"name", 253));
                        send(&mut s, 4, &[0xFE, 0, 0, 2, 0]);
                        send(&mut s, 5, b"\x011\x03abc");
                        send(&mut s, 6, b"\x012\xfb");
                        send(&mut s, 7, &[0xFE, 0, 0, 2, 0]);
                    }
                    (0x03, b"bad") => send(&mut s, 1, b"\xff\x28\x04#42000You have an error"),
                    _ => send(&mut s, 1, b"\x00\x03\x05\x02\x00\x00\x00\x05Done!"),
                }
            }
        });
        port
    }

    fn connect(port: u16, password: &[u8]) -> Result<Connection> {
        Connection::connect(&ConnectOptions {
            address: Address::Tcp { host: "127.0.0.1", port },
            user: b"u",
            password,
            database: Some(b"db"),
            flags: 0,
            timeout: None,
            charset: DEFAULT_CHARSET,
        })
    }

    #[test]
    fn talks_to_a_server() {
        let port = fake_server(b"pw");
        let mut c = connect(port, b"pw").unwrap();
        assert_eq!((c.server_version.as_str(), c.thread_id, c.protocol_version), ("8.4.0-fake", 7, 10));
        let Outcome::Rows(set) = c.query(b"rows").unwrap() else { panic!("rows") };
        let names: Vec<&[u8]> = set.columns.iter().map(|c| &c.name[..]).collect();
        assert_eq!(names, [&b"id"[..], b"name"]);
        // LONG gets NUM_FLAG; VAR_STRING does not.
        assert_eq!((set.columns[0].flags, set.columns[1].flags), (0x8000, 0));
        assert_eq!(set.rows, vec![vec![Some(b"1".to_vec()), Some(b"abc".to_vec())], vec![Some(b"2".to_vec()), None]]);
        let e = c.query(b"bad").err().unwrap();
        assert_eq!((e.code, e.sqlstate.as_str(), e.message.as_str()), (1064, "42000", "You have an error"));
        assert!(matches!(c.query(b"update").unwrap(), Outcome::Done));
        assert_eq!((c.affected_rows, c.insert_id, &c.info[..]), (3, 5, &b"Done!"[..]));
        c.close();
    }

    #[test]
    fn a_wrong_password_is_refused() {
        let port = fake_server(b"pw");
        let e = connect(port, b"nope").err().unwrap();
        // A failed connect reports HY000, as mysqlnd does.
        assert_eq!((e.code, e.sqlstate.as_str(), e.message.as_str()), (1045, "HY000", "Access denied"));
    }

    fn col(kind: u8, flags: u16, length: u32, decimals: u8) -> Column {
        Column { kind, flags, length, decimals, ..Column::default() }
    }

    #[test]
    fn binary_rows_as_php_values() {
        let cols = [
            col(TYPE_LONGLONG, UNSIGNED_FLAG, 20, 0),
            col(TYPE_TINY, 0, 4, 0),
            col(TYPE_FLOAT, 0, 12, 31),
            col(TYPE_DATETIME, 0, 23, 3),
            col(TYPE_TIME, 0, 10, 0),
            col(TYPE_VAR_STRING, 0, 10, 0),
            col(TYPE_LONG, ZEROFILL_FLAG | UNSIGNED_FLAG, 5, 0),
        ];
        // The sixth column is NULL: bit 5 + 2 of the bitmap.
        let mut p = vec![0, 0b1000_0000, 0];
        p.extend_from_slice(&u64::MAX.to_le_bytes());
        p.push(0xFB);
        p.extend_from_slice(&1.1f32.to_le_bytes());
        // 2024-01-02 03:04:05.678000
        p.extend_from_slice(&[11, 0xE8, 0x07, 1, 2, 3, 4, 5, 0x70, 0x58, 0x0A, 0]);
        // -34 days 22:59:59
        p.extend_from_slice(&[8, 1, 34, 0, 0, 0, 22, 59, 59]);
        p.extend_from_slice(&42u32.to_le_bytes());
        let b = |s: &str| FieldValue::Bytes(s.as_bytes().to_vec());
        assert_eq!(
            binary_values(&p, &cols).unwrap(),
            vec![b("18446744073709551615"), FieldValue::Int(-5), FieldValue::Double(1.1), b("2024-01-02 03:04:05.678"), b("-838:59:59"), FieldValue::Null, b("00042")]
        );
        // A FLOAT with fixed decimals rounds to them.
        assert_eq!(float_to_double(123.4567, 4), 123.4567);
    }

    #[test]
    fn text_rows_as_php_values() {
        let cols = [
            col(TYPE_LONG, 0, 11, 0),
            col(TYPE_LONGLONG, UNSIGNED_FLAG, 20, 0),
            col(TYPE_DOUBLE, 0, 22, 31),
            col(TYPE_BIT, UNSIGNED_FLAG, 10, 0),
            col(TYPE_YEAR, UNSIGNED_FLAG | ZEROFILL_FLAG, 4, 0),
            col(TYPE_NEWDECIMAL, 0, 12, 3),
        ];
        let mut p = Vec::new();
        for v in [&b"-7"[..], b"18446744073709551615", b"2.5e-5", &[0x02, 0xAA], b"2024", b"3.142"] {
            put_lenenc(&mut p, v.len() as u64);
            p.extend_from_slice(v);
        }
        let b = |s: &str| FieldValue::Bytes(s.as_bytes().to_vec());
        assert_eq!(
            text_values(&p, &cols, true).unwrap(),
            vec![FieldValue::Int(-7), b("18446744073709551615"), FieldValue::Double(2.5e-5), FieldValue::Int(682), b("2024"), b("3.142")]
        );
        assert_eq!(text_values(&p, &cols, false).unwrap(), vec![b("-7"), b("18446744073709551615"), b("2.5e-5"), b("682"), b("2024"), b("3.142")]);
    }

    #[test]
    fn escapes() {
        let utf8 = charset("utf8mb4").expect("utf8mb4");
        assert_eq!(escape(&utf8, b"a'b\"c\\d\0e\nf\rg\x1ah"), b"a\\'b\\\"c\\\\d\\0e\\nf\\rg\\Zh".to_vec());
        // A BIG-5 character whose second byte is a backslash is copied whole.
        let big5 = charset("big5").expect("big5");
        assert_eq!(escape(&big5, b"\xa5\x5c'"), b"\xa5\x5c\\'".to_vec());
        assert_eq!(escape(&utf8, b"\xa5\x5c'"), b"\xa5\\\\\\'".to_vec());
        // NO_BACKSLASH_ESCAPES: only the quote is doubled.
        assert_eq!(escape_quotes(&utf8, b"a'b\\c\n"), b"a''b\\c\n".to_vec());
    }
}

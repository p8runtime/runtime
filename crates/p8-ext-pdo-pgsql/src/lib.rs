#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's pdo_pgsql driver: the `pgsql:` DSN, `Pdo\Pgsql`, and PostgreSQL's
//! type handling, over the PostgreSQL protocol spoken from Rust on a
//! blocking socket ([`wire`]): no libpq, no C. What is observable follows
//! php-src's ext/pdo_pgsql: DSNs (libpq connection strings), error codes,
//! SQLSTATEs and libpq's error messages, `?` and `:name` rewritten to `$1`,
//! statements prepared on the server at their first execute (or run with
//! their values as one PQexecParams() under PDO::PGSQL_ATTR_DISABLE_PREPARES,
//! or with the values quoted in under PDO::ATTR_EMULATE_PREPARES), `bool`
//! columns as bool, integers as int, floats as float, `bytea` as streams.
//!
//! Not implemented: TLS (an `sslmode` that requires it fails to connect;
//! `prefer` and `allow` connect without it), scrollable cursors, the
//! `postgresql://` URI form of the DSN, `service` files and `.pgpass`.
//! Large object streams (`lobOpen()`, an oid column fetched as
//! PDO::PARAM_LOB) hold the object's bytes in memory: what is written to
//! one reaches the server when the stream closes, not at each fwrite().
//!
//! Reference: $PHP_SRC_PATH/ext/pdo_pgsql (pgsql_driver.c, pgsql_statement.c, pdo_pgsql.c)

mod conninfo;
mod wire;

use p8_ext_pdo::driver::{BindKey, ColumnMeta, ConnectError, Connection, DbValue, Driver, DriverError, DriverMethod, ExecInfo, Options, Placeholders, Scanner, Statement};
use p8_ext_pdo::state::{Dbh, Where, handle_error, record};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{ClassSpec, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::stream::{Resource, Stream, Target};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use wire::{COMMAND_OK, COPY_IN, COPY_OUT, CopyOut, FATAL_ERROR, Param, PgResult, TUPLES_OK, Wire};

pub const EXTENSION: Extension = Extension { name: "pdo_pgsql", register };

fn register(registry: &mut Registry) {
    p8_ext_pdo::register_driver(&PGSQL);
    registry.add_classes(CLASSES);
    registry.add_module("pdo_pgsql");
}

const ATTR_TIMEOUT: i64 = 2;
const ATTR_SERVER_VERSION: i64 = 4;
const ATTR_CLIENT_VERSION: i64 = 5;
const ATTR_SERVER_INFO: i64 = 6;
const ATTR_CONNECTION_STATUS: i64 = 7;
const ATTR_EMULATE_PREPARES: i64 = 20;
const ATTR_DISABLE_PREPARES: i64 = 1000;
const ATTR_RESULT_MEMORY_SIZE: i64 = 1001;

const FETCH_ASSOC: i64 = 2;
const FETCH_NUM: i64 = 3;
const FETCH_BOTH: i64 = 4;

/// The libpq version PHP would report (PQlibVersion()).
const LIBPQ_VERSION: &str = "17.0";

/// SQLSTATE of a failed connection (PHP_PDO_PGSQL_CONNECTION_FAILURE_SQLSTATE).
const CONNECTION_FAILURE: &str = "08006";

// pg_type.h
const BOOLOID: u32 = 16;
const BYTEAOID: u32 = 17;
const INT8OID: u32 = 20;
const INT2OID: u32 = 21;
const INT4OID: u32 = 23;
const TEXTOID: u32 = 25;
const OIDOID: u32 = 26;
const FLOAT4OID: u32 = 700;
const FLOAT8OID: u32 = 701;
const VARCHAROID: u32 = 1043;
const DATEOID: u32 = 1082;
const TIMESTAMPOID: u32 = 1114;

struct PgsqlDriver;
static PGSQL: PgsqlDriver = PgsqlDriver;

/// _pdo_pgsql_trim_message(): the message without its trailing newlines.
fn trim_message(m: &[u8]) -> String {
    let mut end = m.len();
    if end >= 3 && m[end - 1] == b'.' && (m[end - 2] == b'\n' || m[end - 2] == b'\r') {
        end -= 1;
    }
    while end > 0 && (m[end - 1] == b'\n' || m[end - 1] == b'\r') {
        end -= 1;
    }
    String::from_utf8_lossy(&m[..end]).into_owned()
}

/// _pdo_pgsql_error(): `status` is the driver code, the SQLSTATE that of the
/// result (HY000 when there is none), the message PQerrorMessage().
fn pg_error(w: &Wire, status: i64, sqlstate: Option<&str>) -> DriverError {
    let state = match sqlstate {
        Some(s) if s.len() < 6 => s,
        _ => "HY000",
    };
    DriverError::new(state, status, trim_message(&w.error_message))
}

/// _pdo_pgsql_escape_credentials(): addcslashes($s, "\\'").
fn escape_credentials(s: &[u8]) -> String {
    let mut out = String::new();
    for &b in s {
        if b == b'\\' || b == b'\'' {
            out.push('\\');
        }
        out.push(b as char);
    }
    out
}

impl Driver for PgsqlDriver {
    fn name(&self) -> &'static str {
        "pgsql"
    }

    fn class_name(&self) -> Option<&'static str> {
        Some("Pdo\\Pgsql")
    }

    fn connect(&self, dsn: &str, user: Option<&[u8]>, pass: Option<&[u8]>, options: &Options, _cwd: &std::path::Path) -> Result<Box<dyn Connection>, ConnectError> {
        // libpq wants spaces between the parameters; PDO's DSNs use semicolons.
        let mut conn = dsn.replace(';', " ");
        let timeout = options.long(ATTR_TIMEOUT, 30);
        let user = if conn.contains("user=") { None } else { user.map(escape_credentials) };
        let pass = if conn.contains("password=") { None } else { pass.map(escape_credentials) };
        conn.push_str(&format!(" connect_timeout={timeout}"));
        if let Some(u) = user {
            conn.push_str(&format!(" user='{u}'"));
        }
        if let Some(p) = pass {
            conn.push_str(&format!(" password='{p}'"));
        }
        let fail = |message: String| {
            let code = FATAL_ERROR;
            ConnectError { message: format!("SQLSTATE[{CONNECTION_FAILURE}] [{code}] {message}"), code, info: Some(DriverError::new(CONNECTION_FAILURE, code, message)) }
        };
        let params = conninfo::parse(&conn).map_err(|e| fail(trim_message(e.as_bytes())))?;
        let wire = Wire::connect(&params).map_err(|e| fail(trim_message(&e)))?;
        Ok(Box::new(PgConn { wire: Rc::new(RefCell::new(wire)), emulate: false, disable_prepares: false, counter: Rc::new(Cell::new(0)), notice_callback: None, lobs: Vec::new() }))
    }

    fn pdo_method(&self, name: &str) -> Option<DriverMethod> {
        Some(match name {
            "pgsqlCopyFromArray" => pdo_copy_from_array,
            "pgsqlCopyFromFile" => pdo_copy_from_file,
            "pgsqlCopyToArray" => pdo_copy_to_array,
            "pgsqlCopyToFile" => pdo_copy_to_file,
            "pgsqlLOBCreate" => pdo_lob_create,
            "pgsqlLOBOpen" => pdo_lob_open,
            "pgsqlLOBUnlink" => pdo_lob_unlink,
            "pgsqlGetNotify" => pdo_get_notify,
            "pgsqlGetPid" => pdo_get_pid,
            _ => return None,
        })
    }
}

/// pdo_pgsql_db_handle
struct PgConn {
    wire: Rc<RefCell<Wire>>,
    emulate: bool,
    disable_prepares: bool,
    /// stmt_counter: names the server-side statements.
    counter: Rc<Cell<u32>>,
    notice_callback: Option<Value>,
    /// The large object streams lobOpen() made (lob_streams): closed when
    /// the transaction ends.
    lobs: Vec<std::rc::Weak<Object>>,
}

impl PgConn {
    fn exec_status(&mut self, sql: &[u8]) -> Result<PgResult, DriverError> {
        let mut w = self.wire.borrow_mut();
        let r = w.exec(sql);
        if r.ok() { Ok(r) } else { Err(pg_error(&w, r.status, r.sqlstate.as_deref())) }
    }

    /// pdo_pgsql_close_lob_streams(): every large object stream still open
    /// is closed (its resource then of type Unknown); `flush` writes what
    /// was written to them back first.
    fn close_lobs(&mut self, flush: bool) {
        for weak in std::mem::take(&mut self.lobs) {
            let Some(o) = weak.upgrade() else { continue };
            let Some(r) = p8_runtime::stream::resource(&o) else { continue };
            if r.closed.get() {
                continue;
            }
            r.closed.set(true);
            let mut s = r.stream.borrow_mut();
            if !flush {
                s.on_close = None;
            }
            s.close();
        }
    }

    /// The statement options' (or the connection's) choice: emulate, and
    /// run without a server-side prepare.
    fn modes(&self, options: &Options) -> (bool, bool) {
        if options.entries.is_empty() {
            return (self.emulate, self.disable_prepares);
        }
        (options.long(ATTR_EMULATE_PREPARES, i64::from(self.emulate)) == 1, options.long(ATTR_DISABLE_PREPARES, i64::from(self.disable_prepares)) == 1)
    }
}

/// PDO::ATTR_CURSOR => PDO::CURSOR_SCROLL
fn scroll(options: &Options) -> bool {
    options.long(10, 0) == 1
}

/// pdo_get_bool_param()
fn bool_param(ctx: &mut dyn Ctx, value: &Value) -> Result<bool, Throw> {
    if let Some(b) = value.as_bool() {
        Ok(b)
    } else if let Some(i) = value.as_int() {
        Ok(i != 0)
    } else {
        Err(ctx.throw("TypeError", format!("Attribute value must be of type bool for selected attribute, {} given", p8_runtime::args::value_name(value))))
    }
}

impl Connection for PgConn {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    fn placeholders(&self) -> Placeholders {
        Placeholders::Dollar
    }

    fn placeholders_for(&mut self, options: &Options) -> Placeholders {
        if scroll(options) || self.modes(options).0 { Placeholders::None } else { Placeholders::Dollar }
    }

    fn scrollable(&self) -> bool {
        true
    }

    fn emulated_shell(&mut self, options: &Options) -> Option<Box<dyn Statement>> {
        if !scroll(options) {
            return None;
        }
        self.counter.set(self.counter.get().wrapping_add(1));
        let name = format!("pdo_crsr_{:08x}", self.counter.get()).into_bytes();
        Some(Box::new(PgStmt::new(&self.wire, Vec::new(), Mode::Cursor { name, declared: false })))
    }

    fn scanner(&self) -> Scanner {
        Scanner::Pgsql
    }

    fn prepare(&mut self, sql: &str) -> Result<Box<dyn Statement>, DriverError> {
        self.prepare_with(sql.as_bytes(), &Options { entries: &[] })
    }

    fn prepare_with(&mut self, sql: &[u8], options: &Options) -> Result<Box<dyn Statement>, DriverError> {
        let mode = if self.modes(options).1 {
            Mode::ExecOnly
        } else {
            self.counter.set(self.counter.get().wrapping_add(1));
            Mode::Named(format!("pdo_stmt_{:08x}", self.counter.get()).into_bytes())
        };
        Ok(Box::new(PgStmt::new(&self.wire, sql.to_vec(), mode)))
    }

    fn prepare_emulated(&mut self, sql: &[u8]) -> Result<Box<dyn Statement>, DriverError> {
        Ok(Box::new(PgStmt::new(&self.wire, sql.to_vec(), Mode::Plain)))
    }

    fn emulated_bool(&mut self, b: bool) -> Option<Vec<u8>> {
        Some(if b { b"t".to_vec() } else { b"f".to_vec() })
    }

    fn exec(&mut self, sql: &[u8]) -> Result<i64, DriverError> {
        let r = self.exec_status(sql)?;
        Ok(if r.status == COMMAND_OK { r.cmd_tuples() } else { 0 })
    }

    fn quote(&mut self, s: &[u8], param_type: i64) -> Option<Vec<u8>> {
        let std = self.wire.borrow().standard_strings();
        let body = if param_type == 3 { wire::escape_bytea(s, std) } else { wire::escape_string(s, std)? };
        let mut out = Vec::with_capacity(body.len() + 2);
        out.push(b'\'');
        out.extend_from_slice(&body);
        out.push(b'\'');
        Some(out)
    }

    fn last_insert_id(&mut self, name: Option<&[u8]>) -> Result<Vec<u8>, DriverError> {
        let mut w = self.wire.borrow_mut();
        let r = match name {
            None => w.exec(b"SELECT LASTVAL()"),
            Some(n) => w.exec_params(b"SELECT CURRVAL($1)", &[], &[Some((n.to_vec(), 0))]),
        };
        if r.status == TUPLES_OK
            && let Some(Some(v)) = r.rows.first().and_then(|row| row.first())
        {
            return Ok(v.clone());
        }
        Err(pg_error(&w, r.status, r.sqlstate.as_deref()))
    }

    fn begin(&mut self) -> Result<(), DriverError> {
        self.exec_status(b"BEGIN").map(|_| ())
    }

    fn commit(&mut self) -> Result<(), DriverError> {
        // What was written to the streams reaches the server first.
        self.close_lobs(true);
        self.exec_status(b"COMMIT").map(|_| ())
    }

    fn rollback(&mut self) -> Result<(), DriverError> {
        let r = self.exec_status(b"ROLLBACK").map(|_| ());
        if r.is_ok() {
            self.close_lobs(false);
        }
        r
    }

    fn in_transaction(&mut self) -> Option<bool> {
        let w = self.wire.borrow();
        Some(w.ok && w.txn != b'I')
    }

    fn set_attribute(&mut self, ctx: &mut dyn Ctx, attr: i64, value: &Value) -> Result<bool, Throw> {
        match attr {
            ATTR_EMULATE_PREPARES => self.emulate = bool_param(ctx, value)?,
            ATTR_DISABLE_PREPARES => self.disable_prepares = bool_param(ctx, value)?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn get_attribute(&mut self, attr: i64) -> Option<Value> {
        let w = self.wire.borrow();
        let param = |name: &str| String::from_utf8_lossy(w.parameter(name).unwrap_or_default()).into_owned();
        Some(match attr {
            ATTR_EMULATE_PREPARES => Value::bool(self.emulate),
            ATTR_DISABLE_PREPARES => Value::bool(self.disable_prepares),
            ATTR_CLIENT_VERSION => Value::str(LIBPQ_VERSION),
            ATTR_SERVER_VERSION => Value::str(param("server_version").as_str()),
            ATTR_CONNECTION_STATUS => Value::str(if w.ok { "Connection OK; waiting to send." } else { "Bad connection." }),
            ATTR_SERVER_INFO => Value::str(
                format!(
                    "PID: {}; Client Encoding: {}; Is Superuser: {}; Session Authorization: {}; Date Style: {}",
                    w.pid,
                    param("client_encoding"),
                    param("is_superuser"),
                    param("session_authorization"),
                    param("DateStyle")
                )
                .as_str(),
            ),
            _ => return None,
        })
    }

    fn lob_param(&mut self, path: &str) -> Option<DbValue> {
        let oid = path.strip_prefix("pgsql-lob:")?;
        Some(DbValue::Text(oid.as_bytes().to_vec()))
    }

    fn take_notices(&mut self) -> Option<(Value, Vec<Vec<u8>>)> {
        let notices = std::mem::take(&mut self.wire.borrow_mut().notices);
        match &self.notice_callback {
            Some(cb) if !notices.is_empty() => Some((cb.clone(), notices)),
            _ => None,
        }
    }
}

// ---- statements ----

/// How a statement runs (pdo_pgsql_stmt's stmt_name, or none).
enum Mode {
    /// Prepared on the server under this name at the first execute.
    Named(Vec<u8>),
    /// PQexecParams(): PDO::PGSQL_ATTR_DISABLE_PREPARES.
    ExecOnly,
    /// PQexec() of a query with its values quoted in (emulated prepares).
    Plain,
    /// A scrollable cursor: DECLARE ... SCROLL CURSOR WITH HOLD, read with FETCH.
    Cursor { name: Vec<u8>, declared: bool },
}

/// pdo_pgsql_stmt
struct PgStmt {
    wire: Rc<RefCell<Wire>>,
    query: Vec<u8>,
    mode: Mode,
    prepared: bool,
    result: Option<PgResult>,
    row: usize,
}

impl PgStmt {
    fn new(wire: &Rc<RefCell<Wire>>, query: Vec<u8>, mode: Mode) -> PgStmt {
        PgStmt { wire: Rc::clone(wire), query, mode, prepared: false, result: None, row: 0 }
    }
}

impl Drop for PgStmt {
    fn drop(&mut self) {
        let Ok(mut w) = self.wire.try_borrow_mut() else { return };
        if !w.ok {
            return;
        }
        match &self.mode {
            Mode::Named(name) if self.prepared => {
                w.close_prepared(name);
            }
            // PQclosePortal(): closing a cursor that is gone is no error,
            // and never breaks the transaction around it.
            Mode::Cursor { name, declared: true } => {
                w.close_portal(name);
            }
            _ => {}
        }
    }
}

/// ZEND_ATOL(): the leading integer of `s` (strtol), saturating.
fn atol(s: &[u8]) -> i64 {
    let t = String::from_utf8_lossy(s);
    let t = t.trim_start();
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut n: i64 = 0;
    for c in digits.bytes().take_while(u8::is_ascii_digit) {
        let d = i64::from(c - b'0');
        n = match n.checked_mul(10).and_then(|n| if neg { n.checked_sub(d) } else { n.checked_add(d) }) {
            Some(n) => n,
            None => return if neg { i64::MIN } else { i64::MAX },
        };
    }
    n
}

/// pgsql_stmt_get_col(): a column's text as PHP gets it, by its type.
fn column_value(type_oid: u32, v: &[u8]) -> DbValue {
    match type_oid {
        BOOLOID => DbValue::Bool(v.first() == Some(&b't')),
        INT2OID | INT4OID | INT8OID => DbValue::Int(atol(v)),
        FLOAT4OID | FLOAT8OID => {
            let prefix = |word: &[u8]| word.starts_with(v);
            DbValue::Float(if prefix(b"Infinity") {
                f64::INFINITY
            } else if prefix(b"-Infinity") {
                f64::NEG_INFINITY
            } else if prefix(b"NaN") {
                f64::NAN
            } else {
                String::from_utf8_lossy(v).trim().parse().unwrap_or(0.0)
            })
        }
        OIDOID => DbValue::Int(String::from_utf8_lossy(v).trim().parse::<u32>().map_or(0, i64::from)),
        BYTEAOID => DbValue::Lob(wire::unescape_bytea(v)),
        _ => DbValue::Text(v.to_vec()),
    }
}

impl Statement for PgStmt {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    fn execute(&mut self, binds: &[(BindKey, DbValue)]) -> Result<ExecInfo, DriverError> {
        self.execute_dollar(binds, true)
    }

    fn execute_dollar(&mut self, binds: &[(BindKey, DbValue)], mapped: bool) -> Result<ExecInfo, DriverError> {
        self.result = None;
        self.row = 0;
        let mut params: Vec<Param> = vec![None; binds.len()];
        for (key, value) in binds {
            let BindKey::Pos(p) = key else { continue };
            if !mapped {
                // No bound_param_map: PHP never fills param_values.
                continue;
            }
            let v = match value {
                DbValue::Null => None,
                DbValue::Bool(b) => Some((if *b { b"t".to_vec() } else { b"f".to_vec() }, 0)),
                DbValue::Int(i) => Some((i.to_string().into_bytes(), 0)),
                DbValue::Float(f) => Some((Value::float(*f).to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default(), 0)),
                DbValue::Text(t) => Some((t.clone(), 0)),
                DbValue::Blob(b) | DbValue::Lob(b) => Some((b.clone(), 1)),
            };
            if *p >= params.len() {
                params.resize(p + 1, None);
            }
            params[*p] = v;
        }
        params.truncate(binds.len());
        let types = if mapped { vec![0u32; params.len()] } else { Vec::new() };
        let mut w = self.wire.borrow_mut();
        let r = match &mut self.mode {
            Mode::Named(name) => {
                if !self.prepared {
                    let mut p = w.prepare(name, &self.query, &types);
                    if p.sqlstate.as_deref() == Some("42P05") {
                        // The name is taken (a pooler kept an old session's statement): drop it and retry once.
                        w.close_prepared(name);
                        p = w.prepare(name, &self.query, &types);
                    }
                    if !p.ok() {
                        return Err(pg_error(&w, p.status, p.sqlstate.as_deref()));
                    }
                    self.prepared = true;
                }
                w.exec_prepared(name, &params)
            }
            Mode::ExecOnly => w.exec_params(&self.query, &types, &params),
            Mode::Plain => w.exec(&self.query),
            Mode::Cursor { name, declared } => {
                if *declared && w.close_portal(name).status == COMMAND_OK {
                    *declared = false;
                }
                let mut q = b"DECLARE ".to_vec();
                q.extend_from_slice(name);
                q.extend_from_slice(b" SCROLL CURSOR WITH HOLD FOR ");
                q.extend_from_slice(&self.query);
                let d = w.exec(&q);
                if !d.ok() {
                    return Err(pg_error(&w, d.status, d.sqlstate.as_deref()));
                }
                *declared = true;
                let mut f = b"FETCH FORWARD 0 FROM ".to_vec();
                f.extend_from_slice(name);
                w.exec(&f)
            }
        };
        if !r.ok() {
            return Err(pg_error(&w, r.status, r.sqlstate.as_deref()));
        }
        let column_count = r.fields.len();
        let row_count = if r.status == COMMAND_OK { r.cmd_tuples() } else { r.rows.len() as i64 };
        self.result = Some(r);
        Ok(ExecInfo { column_count, row_count: Some(row_count) })
    }

    fn set_query(&mut self, sql: &[u8]) {
        self.query = sql.to_vec();
    }

    fn fetch_scroll(&mut self, ori: i64, offset: i64) -> Result<Option<Vec<DbValue>>, DriverError> {
        let Mode::Cursor { name, .. } = &self.mode else { return self.fetch() };
        let how = match ori {
            0 => "NEXT".to_string(),
            1 => "BACKWARD".to_string(),
            2 => "FIRST".to_string(),
            3 => "LAST".to_string(),
            4 => format!("ABSOLUTE {offset}"),
            5 => format!("RELATIVE {offset}"),
            _ => return Ok(None),
        };
        let mut q = format!("FETCH {how} FROM ").into_bytes();
        q.extend_from_slice(name);
        let mut w = self.wire.borrow_mut();
        let r = w.exec(&q);
        if !r.ok() {
            return Err(pg_error(&w, r.status, r.sqlstate.as_deref()));
        }
        drop(w);
        self.result = Some(r);
        self.row = 0;
        self.fetch()
    }

    fn fetch(&mut self) -> Result<Option<Vec<DbValue>>, DriverError> {
        let Some(r) = &self.result else { return Ok(None) };
        let Some(row) = r.rows.get(self.row) else { return Ok(None) };
        self.row += 1;
        Ok(Some(row.iter().zip(&r.fields).map(|(v, f)| v.as_deref().map_or(DbValue::Null, |v| column_value(f.type_oid, v))).collect()))
    }

    fn column_count(&self) -> usize {
        self.result.as_ref().map_or(0, |r| r.fields.len())
    }

    fn column_name(&self, i: usize) -> Vec<u8> {
        self.result.as_ref().and_then(|r| r.fields.get(i)).map(|f| f.name.clone()).unwrap_or_default()
    }

    fn column_meta(&mut self, i: usize) -> Option<ColumnMeta> {
        let f = self.result.as_ref()?.fields.get(i)?.clone();
        let mut w = self.wire.borrow_mut();
        let mut entries: Vec<(String, Value)> = vec![("pgsql:oid".into(), Value::int(i64::from(f.type_oid))), ("pgsql:table_oid".into(), Value::int(i64::from(f.table_oid)))];
        let table = w.exec(format!("SELECT RELNAME FROM PG_CLASS WHERE OID={}", f.table_oid as i32).as_bytes());
        if table.status == TUPLES_OK
            && let Some(Some(name)) = table.rows.first().and_then(|r| r.first())
        {
            entries.push(("table".into(), Value::str(PhpStr::new(name.clone()))));
        }
        let label = match f.type_oid {
            BOOLOID => Some("bool"),
            BYTEAOID => Some("bytea"),
            INT8OID => Some("int8"),
            INT2OID => Some("int2"),
            INT4OID => Some("int4"),
            FLOAT4OID => Some("float4"),
            FLOAT8OID => Some("float8"),
            TEXTOID => Some("text"),
            VARCHAROID => Some("varchar"),
            DATEOID => Some("date"),
            TIMESTAMPOID => Some("timestamp"),
            _ => None,
        };
        match label {
            Some(l) => entries.push(("native_type".into(), Value::str(l))),
            None => {
                let t = w.exec(format!("SELECT TYPNAME FROM PG_TYPE WHERE OID={}", f.type_oid).as_bytes());
                if t.status == TUPLES_OK
                    && t.rows.len() == 1
                    && let Some(Some(name)) = t.rows[0].first()
                {
                    entries.push(("native_type".into(), Value::str(PhpStr::new(name.clone()))));
                }
            }
        }
        let pdo_type = match f.type_oid {
            BOOLOID => 5,
            INT2OID | INT4OID | INT8OID => 1,
            OIDOID | BYTEAOID => 3,
            _ => 2,
        };
        entries.push(("pdo_type".into(), Value::int(pdo_type)));
        Some(ColumnMeta { ordered: Some(entries), len: Some(i64::from(f.size)), precision: Some(i64::from(f.modifier)), ..ColumnMeta::default() })
    }

    fn lob_column(&self, col: usize) -> Option<Vec<u8>> {
        let r = self.result.as_ref()?;
        if r.fields.get(col)?.type_oid != OIDOID {
            return None;
        }
        let v = r.rows.get(self.row.checked_sub(1)?)?.get(col)?.as_deref()?;
        lob_read(&self.wire, String::from_utf8_lossy(v).trim().parse().ok()?)
    }

    fn get_attribute(&mut self, attr: i64) -> Option<Value> {
        match attr {
            ATTR_RESULT_MEMORY_SIZE => Some(self.result.as_ref().map_or(Value::NULL, |r| Value::int(r.memory_size()))),
            _ => None,
        }
    }
}

// ---- Pdo\Pgsql and PDO::pgsql*() ----

type R = Result<Value, Throw>;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

fn type_error(ctx: &mut dyn Ctx, func: &str, i: usize, name: &str, want: &str, v: &Value) -> Throw {
    ctx.throw("TypeError", format!("{func}(): Argument #{} (${name}) must be of type {want}, {} given", i + 1, p8_runtime::args::value_name(v)))
}

/// A `string` parameter (zpp "s"); `default` when it was not passed.
fn str_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value], i: usize, name: &str, default: Option<&[u8]>) -> Result<Vec<u8>, Throw> {
    if i >= args.len()
        && let Some(d) = default
    {
        return Ok(d.to_vec());
    }
    let v = arg(args, i);
    if let Some(s) = v.as_str() {
        Ok(s.as_bytes().to_vec())
    } else if v.is_number() || v.is_bool() {
        Ok(v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default())
    } else if v.is_null() && i < args.len() {
        ctx.deprecated(&format!("{func}(): Passing null to parameter #{} (${name}) of type string is deprecated", i + 1));
        Ok(Vec::new())
    } else {
        Err(type_error(ctx, func, i, name, "string", &v))
    }
}

/// A `?string` parameter (zpp "s!").
fn opt_str_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value], i: usize, name: &str) -> Result<Option<Vec<u8>>, Throw> {
    match arg(args, i) {
        v if v.is_null() => Ok(None),
        _ => str_arg(ctx, func, args, i, name, None).map(Some),
    }
}

fn int_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value], i: usize, name: &str, default: i64) -> Result<i64, Throw> {
    if i >= args.len() {
        return Ok(default);
    }
    let v = arg(args, i);
    if let Some(n) = v.as_int() {
        Ok(n)
    } else if let Some(b) = v.as_bool() {
        Ok(i64::from(b))
    } else if let Some(f) = v.as_float()
        && f.fract() == 0.0
        && f.is_finite()
    {
        Ok(f as i64)
    } else if v.is_str() && matches!(p8_runtime::numeric::numeric_string(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()), Some(p8_runtime::numeric::Number::Int(_))) {
        Ok(v.to_int())
    } else {
        Err(type_error(ctx, func, i, name, "int", &v))
    }
}

fn no_args(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<(), Throw> {
    if !args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", format!("{func}() expects exactly 0 arguments, {} given", args.len())));
    }
    Ok(())
}

/// The connection behind `this` (PDO_CONSTRUCT_CHECK), and its driver state.
fn conn_of(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<Rc<Dbh>, Throw> {
    let dbh = p8_ext_pdo::dbh_of(ctx, this)?;
    let is_pg = dbh.conn.borrow_mut().as_any().is::<PgConn>();
    if !is_pg {
        return Err(ctx.throw("Error", format!("Call to undefined method {}", String::from_utf8_lossy(this.class.shown()))));
    }
    Ok(dbh)
}

fn with_pg<T>(dbh: &Dbh, f: impl FnOnce(&mut PgConn) -> T) -> T {
    let mut c = dbh.conn.borrow_mut();
    f(c.as_any().downcast_mut::<PgConn>().expect("a pgsql connection"))
}

/// pdo_pgsql_error() on the connection, then PDO_HANDLE_DBH_ERR(): false.
fn fail(ctx: &mut dyn Ctx, func: &str, dbh: &Dbh, e: DriverError) -> R {
    record(dbh, &Where::Dbh, &e);
    handle_error(ctx, func, dbh, Where::Dbh)?;
    Ok(Value::FALSE)
}

/// The error of the connection's last command (pdo_pgsql_error(dbh, status, sqlstate)).
fn last_error(dbh: &Dbh, status: i64, sqlstate: Option<&str>) -> DriverError {
    with_pg(dbh, |pg| pg_error(&pg.wire.borrow(), status, sqlstate))
}

/// The COPY command pdo_pgsql sends (in pre-9.0 syntax).
fn copy_query(table: &[u8], fields: Option<&[u8]>, dir: &str, sep: &[u8], null_as: &[u8]) -> Vec<u8> {
    let cut = |s: &[u8]| s.iter().position(|&b| b == 0).map_or(s.to_vec(), |n| s[..n].to_vec());
    let mut q = b"COPY ".to_vec();
    q.extend_from_slice(&cut(table));
    if let Some(f) = fields {
        q.extend_from_slice(b" (");
        q.extend_from_slice(&cut(f));
        q.push(b')');
    }
    q.extend_from_slice(format!(" {dir} STDIN WITH DELIMITER E'").as_bytes());
    q.push(sep.first().copied().unwrap_or(b'\t'));
    q.extend_from_slice(b"' NULL AS E'");
    q.extend_from_slice(&cut(if null_as.is_empty() { b"\\\\N" } else { null_as }));
    q.push(b'\'');
    q
}

/// The arguments the copy methods share after the table (and file).
struct CopyOptions {
    separator: Vec<u8>,
    null_as: Vec<u8>,
    fields: Option<Vec<u8>>,
}

impl CopyOptions {
    fn read(ctx: &mut dyn Ctx, func: &str, args: &[Value], from: usize) -> Result<CopyOptions, Throw> {
        let separator = str_arg(ctx, func, args, from, "separator", Some(b"\t"))?;
        let null_as = str_arg(ctx, func, args, from + 1, "nullAs", Some(b"\\\\N"))?;
        let fields = opt_str_arg(ctx, func, args, from + 2, "fields")?;
        Ok(CopyOptions { separator, null_as, fields })
    }

    /// The COPY command for `table` in direction `dir`.
    fn query(&self, table: &[u8], dir: &str) -> Vec<u8> {
        copy_query(table, self.fields.as_deref(), dir, &self.separator, &self.null_as)
    }
}

/// COPY ... FROM STDIN with these lines.
fn copy_in(ctx: &mut dyn Ctx, func: &str, dbh: &Dbh, query: &[u8], lines: &[Vec<u8>]) -> R {
    let outcome = with_pg(dbh, |pg| {
        let mut w = pg.wire.borrow_mut();
        let r = w.exec(query);
        if r.status != COPY_IN {
            return Err(pg_error(&w, FATAL_ERROR, r.sqlstate.as_deref()));
        }
        for l in lines {
            if !w.put_copy_data(l) {
                return Err(pg_error(&w, FATAL_ERROR, None));
            }
        }
        let end = w.put_copy_end();
        if end.status != COMMAND_OK {
            return Err(pg_error(&w, FATAL_ERROR, end.sqlstate.as_deref()));
        }
        Ok(())
    });
    p8_ext_pdo::dispatch_notices(ctx, dbh)?;
    match outcome {
        Ok(()) => Ok(Value::TRUE),
        Err(e) => fail(ctx, func, dbh, e),
    }
}

/// COPY ... TO STDIN: the rows, or the error.
fn copy_out(dbh: &Dbh, query: &[u8]) -> Result<Vec<Vec<u8>>, DriverError> {
    with_pg(dbh, |pg| {
        let mut w = pg.wire.borrow_mut();
        let r = w.exec(query);
        if r.status != COPY_OUT {
            return Err(pg_error(&w, FATAL_ERROR, r.sqlstate.as_deref()));
        }
        let mut rows = Vec::new();
        loop {
            match w.get_copy_data() {
                CopyOut::Row(r) => rows.push(r),
                CopyOut::Done => return Ok(rows),
                CopyOut::Failed => return Err(pg_error(&w, FATAL_ERROR, None)),
            }
        }
    })
}

fn copy_from_array(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let table = str_arg(ctx, func, args, 0, "tableName", None)?;
    let rows = match arg(args, 1).into_array() {
        Ok(a) => a,
        Err(v) => return Err(type_error(ctx, func, 1, "rows", "array", &v)),
    };
    let opts = CopyOptions::read(ctx, func, args, 2)?;
    if rows.is_empty() {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #2 ($rows) must not be empty")));
    }
    let dbh = conn_of(ctx, this)?;
    dbh.clear_error();
    let mut lines = Vec::with_capacity(rows.len());
    for (_, v) in rows.iter() {
        let v = v.deref();
        let mut line = if v.is_array() {
            ctx.warn("Array to string conversion");
            b"Array".to_vec()
        } else if let Some(o) = v.as_object() {
            if o.class.method(b"__tostring").is_none() {
                return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(o.class.shown()))));
            }
            let callable = Value::array(PhpArray::from_list(vec![Value::object(Rc::clone(o)), Value::str("__toString")]));
            ctx.call(&callable, &mut [])?.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()
        } else {
            v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()
        };
        if line.last() != Some(&b'\n') {
            line.push(b'\n');
        }
        lines.push(line);
    }
    let q = opts.query(&table, "FROM");
    copy_in(ctx, func, &dbh, &q, &lines)
}

fn copy_from_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let table = str_arg(ctx, func, args, 0, "tableName", None)?;
    let file = str_arg(ctx, func, args, 1, "filename", None)?;
    let opts = CopyOptions::read(ctx, func, args, 2)?;
    let dbh = conn_of(ctx, this)?;
    dbh.clear_error();
    let name = String::from_utf8_lossy(&file).into_owned();
    let data = match std::fs::read(p8_runtime::cwd::resolve(ctx, &name)) {
        Ok(d) => d,
        Err(_) => {
            // php_stream_open_wrapper_ex() without REPORT_ERRORS: no warning.
            let msg = "Unable to open the file";
            return fail(ctx, func, &dbh, DriverError::new("HY000", FATAL_ERROR, msg));
        }
    };
    let lines: Vec<Vec<u8>> = data.split_inclusive(|&b| b == b'\n').map(<[u8]>::to_vec).collect();
    let q = opts.query(&table, "FROM");
    copy_in(ctx, func, &dbh, &q, &lines)
}

fn copy_to_array(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let table = str_arg(ctx, func, args, 0, "tableName", None)?;
    let opts = CopyOptions::read(ctx, func, args, 1)?;
    let dbh = conn_of(ctx, this)?;
    dbh.clear_error();
    let q = opts.query(&table, "TO");
    let r = copy_out(&dbh, &q);
    p8_ext_pdo::dispatch_notices(ctx, &dbh)?;
    match r {
        Ok(rows) => Ok(Value::array(PhpArray::from_list(rows.into_iter().map(|r| Value::str(PhpStr::new(r))).collect()))),
        Err(e) => fail(ctx, func, &dbh, e),
    }
}

fn copy_to_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let table = str_arg(ctx, func, args, 0, "tableName", None)?;
    let file = str_arg(ctx, func, args, 1, "filename", None)?;
    let opts = CopyOptions::read(ctx, func, args, 2)?;
    let dbh = conn_of(ctx, this)?;
    dbh.clear_error();
    let name = String::from_utf8_lossy(&file).into_owned();
    let mut out = match std::fs::File::create(p8_runtime::cwd::resolve(ctx, &name)) {
        Ok(f) => f,
        Err(_) => {
            return fail(ctx, func, &dbh, DriverError::new("HY000", FATAL_ERROR, "Unable to open the file for writing"));
        }
    };
    let q = opts.query(&table, "TO");
    let r = copy_out(&dbh, &q);
    match r {
        Ok(rows) => {
            use std::io::Write;
            for row in rows {
                if out.write_all(&row).is_err() {
                    return fail(ctx, func, &dbh, DriverError::new("HY000", FATAL_ERROR, "Unable to write to file"));
                }
            }
            Ok(Value::TRUE)
        }
        Err(e) => fail(ctx, func, &dbh, e),
    }
}

/// An oid argument as strtoul() reads it; None when it is not a number.
fn oid_arg(s: &[u8]) -> Option<u32> {
    let t = String::from_utf8_lossy(s);
    let digits: String = t.trim_start().chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    Some(digits.parse::<u64>().map_or(u32::MAX, |n| n.min(u64::from(u32::MAX)) as u32))
}

/// A large object call run as SQL (libpq's lo_* functions call the same
/// server functions); the first column of its row.
fn lo_call(dbh: &Dbh, sql: String) -> Result<Option<Vec<u8>>, DriverError> {
    with_pg(dbh, |pg| {
        let mut w = pg.wire.borrow_mut();
        let r = w.exec(sql.as_bytes());
        if r.status != TUPLES_OK {
            return Err(pg_error(&w, FATAL_ERROR, None));
        }
        Ok(r.rows.first().and_then(|row| row.first().cloned()).flatten())
    })
}

fn lob_create(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    no_args(ctx, func, args)?;
    let dbh = conn_of(ctx, this)?;
    dbh.clear_error();
    // INV_READ | INV_WRITE
    match lo_call(&dbh, "SELECT pg_catalog.lo_creat(393216)".into()) {
        Ok(Some(oid)) if oid != b"0" => Ok(Value::str(PhpStr::new(oid))),
        Ok(_) => {
            let e = last_error(&dbh, FATAL_ERROR, None);
            fail(ctx, func, &dbh, e)
        }
        Err(e) => fail(ctx, func, &dbh, e),
    }
}

fn lob_open(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let oid = str_arg(ctx, func, args, 0, "oid", None)?;
    let mode = str_arg(ctx, func, args, 1, "mode", Some(b"rb"))?;
    let Some(oid) = oid_arg(&oid) else { return Ok(Value::FALSE) };
    let dbh = conn_of(ctx, this)?;
    dbh.clear_error();
    let write = mode.iter().any(|&c| c == b'+' || c == b'w');
    // The object's bytes as a stream; what is written goes back to the
    // object when the stream closes.
    match lo_call(&dbh, format!("SELECT pg_catalog.lo_get({oid})")) {
        Ok(Some(bytes)) => {
            let mut stream = Stream::new(Target::Memory(wire::unescape_bytea(&bytes)), &lob_path(oid), if write { "r+b" } else { "rb" });
            if write {
                let wire = with_pg(&dbh, |pg| Rc::clone(&pg.wire));
                stream.on_close = Some(Box::new(move |data: &[u8]| {
                    if let Ok(mut w) = wire.try_borrow_mut() {
                        let sql = format!("SELECT pg_catalog.lo_put({oid}, 0, $1)");
                        w.exec_params(sql.as_bytes(), &[], &[Some((data.to_vec(), 1))]);
                    }
                }));
            }
            let v = ctx.new_resource(Resource { kind: "stream", closed: Cell::new(false), stream: RefCell::new(stream) });
            if let Some(o) = v.as_object() {
                with_pg(&dbh, |pg| pg.lobs.push(Rc::downgrade(o)));
            }
            Ok(v)
        }
        Ok(None) => Ok(Value::FALSE),
        Err(e) => fail(ctx, func, &dbh, e),
    }
}

/// The path of a large object's stream, by which a bound PDO::PARAM_LOB
/// is known to be one.
fn lob_path(oid: u32) -> String {
    format!("pgsql-lob:{oid}")
}

/// The contents of large object `oid` (lo_open() and lo_read() to the end).
fn lob_read(wire: &RefCell<Wire>, oid: u32) -> Option<Vec<u8>> {
    let mut w = wire.try_borrow_mut().ok()?;
    let r = w.exec(format!("SELECT pg_catalog.lo_get({oid})").as_bytes());
    if r.status != TUPLES_OK {
        return None;
    }
    r.rows.first()?.first()?.as_deref().map(wire::unescape_bytea)
}

fn lob_unlink(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let oid = str_arg(ctx, func, args, 0, "oid", None)?;
    let Some(oid) = oid_arg(&oid) else { return Ok(Value::FALSE) };
    let dbh = conn_of(ctx, this)?;
    dbh.clear_error();
    match lo_call(&dbh, format!("SELECT pg_catalog.lo_unlink({oid})")) {
        Ok(Some(r)) if r == b"1" => Ok(Value::TRUE),
        Ok(_) => {
            let e = last_error(&dbh, FATAL_ERROR, None);
            fail(ctx, func, &dbh, e)
        }
        Err(e) => fail(ctx, func, &dbh, e),
    }
}

fn get_notify(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let mut mode = int_arg(ctx, func, args, 0, "fetchMode", 0)?;
    let mut timeout = int_arg(ctx, func, args, 1, "timeoutMilliseconds", 0)?;
    let dbh = conn_of(ctx, this)?;
    if mode == 0 {
        mode = dbh.default_fetch.get();
    }
    if ![FETCH_BOTH, FETCH_ASSOC, FETCH_NUM].contains(&mode) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($fetchMode) must be one of PDO::FETCH_BOTH, PDO::FETCH_ASSOC, or PDO::FETCH_NUM")));
    }
    if timeout < 0 {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #2 ($timeoutMilliseconds) must be greater than or equal to 0")));
    }
    if timeout > i64::from(i32::MAX) {
        ctx.warn(&format!("{func}(): Timeout was shrunk to {}", i32::MAX));
        timeout = i64::from(i32::MAX);
    }
    let got = with_pg(&dbh, |pg| {
        let mut w = pg.wire.borrow_mut();
        if !w.consume_input(None) {
            return Err(pg_error(&w, FATAL_ERROR, None));
        }
        let mut n = w.notifies.pop_front();
        if n.is_none() && timeout > 0 {
            if !w.consume_input(Some(Duration::from_millis(timeout as u64))) {
                return Err(pg_error(&w, FATAL_ERROR, None));
            }
            n = w.notifies.pop_front();
        }
        Ok(n)
    });
    p8_ext_pdo::dispatch_notices(ctx, &dbh)?;
    let n = match got {
        Ok(Some(n)) => n,
        Ok(None) => return Ok(Value::FALSE),
        Err(e) => return fail(ctx, func, &dbh, e),
    };
    let mut a = PhpArray::new();
    let text = |b: &[u8]| Value::str(PhpStr::new(b.iter().take_while(|&&c| c != 0).copied().collect::<Vec<u8>>()));
    if mode == FETCH_NUM || mode == FETCH_BOTH {
        a.insert(Key::Int(0), text(&n.channel));
        a.insert(Key::Int(1), Value::int(i64::from(n.pid)));
        if !n.payload.is_empty() {
            a.insert(Key::Int(2), text(&n.payload));
        }
    }
    if mode == FETCH_ASSOC || mode == FETCH_BOTH {
        a.insert(Key::from_string(PhpStr::from("message")), text(&n.channel));
        a.insert(Key::from_string(PhpStr::from("pid")), Value::int(i64::from(n.pid)));
        if !n.payload.is_empty() {
            a.insert(Key::from_string(PhpStr::from("payload")), text(&n.payload));
        }
    }
    Ok(Value::array(a))
}

fn get_pid(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    no_args(ctx, func, args)?;
    let dbh = conn_of(ctx, this)?;
    Ok(Value::int(i64::from(with_pg(&dbh, |pg| pg.wire.borrow().pid))))
}

fn escape_identifier(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "Pdo\\Pgsql::escapeIdentifier";
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", format!("{func}() expects exactly 1 argument, 0 given")));
    }
    let input = str_arg(ctx, func, args, 0, "input", None)?;
    let dbh = conn_of(ctx, this)?;
    dbh.clear_error();
    match wire::escape_identifier(&input) {
        Ok(s) => Ok(Value::str(PhpStr::new(s))),
        Err(msg) => {
            with_pg(&dbh, |pg| pg.wire.borrow_mut().error_message = msg.as_bytes().to_vec());
            let e = last_error(&dbh, FATAL_ERROR, None);
            fail(ctx, func, &dbh, e)?;
            Ok(Value::NULL)
        }
    }
}

fn set_notice_callback(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "Pdo\\Pgsql::setNoticeCallback";
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", format!("{func}() expects exactly 1 argument, 0 given")));
    }
    let cb = arg(args, 0);
    if !cb.is_null() && !ctx.is_callable(&cb) {
        let why = p8_runtime::args::callable_error(ctx, &cb);
        return Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($callback) must be a valid callback or null, {why}")));
    }
    let dbh = conn_of(ctx, this)?;
    with_pg(&dbh, |pg| pg.notice_callback = if cb.is_null() { None } else { Some(cb) });
    Ok(Value::NULL)
}

macro_rules! methods {
    ($($pdo:ident, $own:ident => $f:ident, $pdo_name:literal, $own_name:literal;)*) => {
        $(
            fn $pdo(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
                $f(ctx, this, args, $pdo_name)
            }
            fn $own(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
                $f(ctx, this, args, $own_name)
            }
        )*
    };
}

methods! {
    pdo_copy_from_array, own_copy_from_array => copy_from_array, "PDO::pgsqlCopyFromArray", "Pdo\\Pgsql::copyFromArray";
    pdo_copy_from_file, own_copy_from_file => copy_from_file, "PDO::pgsqlCopyFromFile", "Pdo\\Pgsql::copyFromFile";
    pdo_copy_to_array, own_copy_to_array => copy_to_array, "PDO::pgsqlCopyToArray", "Pdo\\Pgsql::copyToArray";
    pdo_copy_to_file, own_copy_to_file => copy_to_file, "PDO::pgsqlCopyToFile", "Pdo\\Pgsql::copyToFile";
    pdo_lob_create, own_lob_create => lob_create, "PDO::pgsqlLOBCreate", "Pdo\\Pgsql::lobCreate";
    pdo_lob_open, own_lob_open => lob_open, "PDO::pgsqlLOBOpen", "Pdo\\Pgsql::lobOpen";
    pdo_lob_unlink, own_lob_unlink => lob_unlink, "PDO::pgsqlLOBUnlink", "Pdo\\Pgsql::lobUnlink";
    pdo_get_notify, own_get_notify => get_notify, "PDO::pgsqlGetNotify", "Pdo\\Pgsql::getNotify";
    pdo_get_pid, own_get_pid => get_pid, "PDO::pgsqlGetPid", "Pdo\\Pgsql::getPid";
}

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "Pdo\\Pgsql",
    parent: Some("PDO"),
    interfaces: &[],
    flags: CLASS_UNCLONEABLE,
    properties: &[],
    methods: &[
        ("escapeIdentifier", Some(Native::Method(escape_identifier))),
        ("copyFromArray", Some(Native::Method(own_copy_from_array))),
        ("copyFromFile", Some(Native::Method(own_copy_from_file))),
        ("copyToArray", Some(Native::Method(own_copy_to_array))),
        ("copyToFile", Some(Native::Method(own_copy_to_file))),
        ("lobCreate", Some(Native::Method(own_lob_create))),
        ("lobOpen", Some(Native::Method(own_lob_open))),
        ("lobUnlink", Some(Native::Method(own_lob_unlink))),
        ("getNotify", Some(Native::Method(own_get_notify))),
        ("getPid", Some(Native::Method(own_get_pid))),
        ("setNoticeCallback", Some(Native::Method(set_notice_callback))),
    ],
    constants: p8_ext_pdo::consts::PGSQL_CONSTANTS,
    debug_info: None,
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_as_strtol_reads_them() {
        assert_eq!(atol(b"42"), 42);
        assert_eq!(atol(b"-7x"), -7);
        assert_eq!(atol(b"99999999999999999999"), i64::MAX);
    }

    #[test]
    fn columns_by_type() {
        assert_eq!(column_value(BOOLOID, b"t"), DbValue::Bool(true));
        assert_eq!(column_value(INT8OID, b"123"), DbValue::Int(123));
        assert_eq!(column_value(FLOAT8OID, b"1.5"), DbValue::Float(1.5));
        assert_eq!(column_value(FLOAT8OID, b"-Infinity"), DbValue::Float(f64::NEG_INFINITY));
        assert_eq!(column_value(BYTEAOID, b"\\x4142"), DbValue::Lob(b"AB".to_vec()));
        assert_eq!(column_value(1700, b"1.50"), DbValue::Text(b"1.50".to_vec()));
    }

    #[test]
    fn messages_lose_trailing_newlines() {
        assert_eq!(trim_message(b"ERROR:  boom\nLINE 1: x\n        ^\n"), "ERROR:  boom\nLINE 1: x\n        ^");
    }

    #[test]
    fn copy_commands() {
        assert_eq!(copy_query(b"t", Some(b"a, b"), "FROM", b";", b""), b"COPY t (a, b) FROM STDIN WITH DELIMITER E';' NULL AS E'\\\\N'".to_vec());
    }
}

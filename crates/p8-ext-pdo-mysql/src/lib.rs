#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's pdo_mysql driver: the `mysql:` DSN, `Pdo\Mysql` and the
//! PDO::MYSQL_ATTR_* attributes, over p8-mysqlnd (the MySQL protocol spoken
//! from Rust, no libmysqlclient, no C). What is observable follows php-src's
//! ext/pdo_mysql built on mysqlnd: emulated prepares by default (values
//! quoted into the query, run in the text protocol), server-side prepares
//! (`?` and `:name` rewritten to `?`, run in the binary protocol) when
//! PDO::ATTR_EMULATE_PREPARES is off, ints and floats as PHP ints and floats
//! in both unless PDO::ATTR_STRINGIFY_FETCHES, buffered or unbuffered result
//! sets, multiple statements and result sets, MySQL's SQLSTATEs, codes and
//! messages.
//!
//! Not implemented: TLS (the PDO::MYSQL_ATTR_SSL_* options are accepted
//! and ignored: the connection is not encrypted), compression
//! (PDO::MYSQL_ATTR_COMPRESS is ignored), LOAD DATA LOCAL INFILE (refused
//! with error 2068 even when PDO::MYSQL_ATTR_LOCAL_INFILE allows it),
//! PDO::MYSQL_ATTR_SERVER_PUBLIC_KEY (caching_sha2_password works over its
//! fast path only), persistent connections.
//!
//! Reference: $PHP_SRC_PATH/ext/pdo_mysql (mysql_driver.c,
//! mysql_statement.c, pdo_mysql.c)

use p8_ext_pdo::driver::{BindKey, ColumnMeta, ConnectError, Connection, DbValue, Driver, DriverError, ExecInfo, Options, Placeholders, Scanner, Statement};
use p8_mysqlnd::{Address, Column, ConnectOptions, FieldValue, Header, Param, Prepared};
use p8_runtime::ext::{ClassSpec, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::value::Value;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

pub const EXTENSION: Extension = Extension { name: "pdo_mysql", register };

fn register(registry: &mut Registry) {
    p8_ext_pdo::register_driver(&MYSQL);
    registry.add_classes(CLASSES);
    registry.add_module("pdo_mysql");
}

const ATTR_AUTOCOMMIT: i64 = 0;
const ATTR_TIMEOUT: i64 = 2;
const ATTR_SERVER_VERSION: i64 = 4;
const ATTR_CLIENT_VERSION: i64 = 5;
const ATTR_SERVER_INFO: i64 = 6;
const ATTR_CONNECTION_STATUS: i64 = 7;
const ATTR_FETCH_TABLE_NAMES: i64 = 14;
const ATTR_STRINGIFY_FETCHES: i64 = 17;
const ATTR_EMULATE_PREPARES: i64 = 20;
const ATTR_DEFAULT_STR_PARAM: i64 = 21;

const MYSQL_ATTR_USE_BUFFERED_QUERY: i64 = 1000;
const MYSQL_ATTR_LOCAL_INFILE: i64 = 1001;
const MYSQL_ATTR_INIT_COMMAND: i64 = 1002;
const MYSQL_ATTR_DIRECT_QUERY: i64 = 1004;
const MYSQL_ATTR_FOUND_ROWS: i64 = 1005;
const MYSQL_ATTR_IGNORE_SPACE: i64 = 1006;
const MYSQL_ATTR_MULTI_STATEMENTS: i64 = 1013;
const MYSQL_ATTR_LOCAL_INFILE_DIRECTORY: i64 = 1015;

const PARAM_INT: i64 = 1;
const PARAM_STR: i64 = 2;
const PARAM_LOB: i64 = 3;
const PARAM_STR_NATL: i64 = 0x4000_0000;
const PARAM_STR_CHAR: i64 = 0x2000_0000;

/// ER_UNSUPPORTED_PS: the statement cannot be prepared on the server.
const ER_UNSUPPORTED_PS: u16 = 1295;

/// Where `host=localhost` connects (pdo_mysql.default_socket).
const DEFAULT_SOCKET: &str = "/tmp/mysql.sock";

struct MysqlDriver;
static MYSQL: MysqlDriver = MysqlDriver;

/// php_pdo_parse_data_source(): the values of `names` in a DSN of
/// `name=value;` pairs (`;;` is a semicolon in a value).
fn parse_data_source(dsn: &[u8], names: &[&str]) -> Vec<Option<Vec<u8>>> {
    let mut out = vec![None; names.len()];
    let mut i = 0;
    let mut optstart = 0;
    while i < dsn.len() && dsn[i] != 0 {
        if dsn[i] != b'=' {
            i += 1;
            continue;
        }
        let name = &dsn[optstart..i];
        i += 1;
        let mut value = Vec::new();
        while i < dsn.len() && dsn[i] != 0 {
            if dsn[i] == b';' {
                if dsn.get(i + 1) == Some(&b';') {
                    value.push(b';');
                    i += 2;
                    continue;
                }
                break;
            }
            value.push(dsn[i]);
            i += 1;
        }
        // Past the semicolon (or NUL) that ended the value.
        i += 1;
        if let Some(j) = names.iter().position(|n| n.as_bytes() == name) {
            out[j] = Some(value);
        }
        while i < dsn.len() && dsn[i].is_ascii_whitespace() {
            i += 1;
        }
        optstart = i;
    }
    out
}

/// What pdo_mysql's connection and statements share.
struct Shared {
    conn: RefCell<p8_mysqlnd::Connection>,
    /// PDO::MYSQL_ATTR_USE_BUFFERED_QUERY.
    buffered: Cell<bool>,
    /// MYSQLND_OPT_INT_AND_FLOAT_NATIVE: not PDO::ATTR_STRINGIFY_FETCHES.
    native: Cell<bool>,
    /// PDO::ATTR_FETCH_TABLE_NAMES.
    table_names: Cell<bool>,
}

/// _pdo_mysql_error(): a mysqlnd error as PDO reports it. Commands out of
/// sync (2014) are explained by what is still unread.
fn my_error(conn: &p8_mysqlnd::Connection, e: &p8_mysqlnd::Error) -> DriverError {
    let message = match e.code {
        p8_mysqlnd::CR_COMMANDS_OUT_OF_SYNC if conn.more_results() => {
            "Cannot execute queries while there are pending result sets. Consider unsetting the previous PDOStatement or calling PDOStatement::closeCursor()".to_string()
        }
        p8_mysqlnd::CR_COMMANDS_OUT_OF_SYNC => "Cannot execute queries while other unbuffered queries are active.  Consider using PDOStatement::fetchAll().  Alternatively, if your code is only ever going to run against mysql, you may enable query buffering by setting the PDO::MYSQL_ATTR_USE_BUFFERED_QUERY attribute.".to_string(),
        _ => e.message.clone(),
    };
    DriverError::new(&e.sqlstate, i64::from(e.code), message)
}

/// A failed connect: PDO throws "SQLSTATE[...] [code] message"
/// (pdo_throw_exception()), its code the driver's.
fn connect_error(e: &p8_mysqlnd::Error) -> ConnectError {
    ConnectError {
        message: format!("SQLSTATE[{}] [{}] {}", e.sqlstate, e.code, e.message),
        code: i64::from(e.code),
        info: Some(DriverError::new(&e.sqlstate, i64::from(e.code), e.message.clone())),
    }
}

/// pdo_attr_strval(): a driver option as a string.
fn option_str(options: &Options, attr: i64) -> Option<Vec<u8>> {
    options.entries.iter().find(|(k, _)| *k == attr).map(|(_, v)| v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default())
}

impl Driver for MysqlDriver {
    fn name(&self) -> &'static str {
        "mysql"
    }

    fn class_name(&self) -> Option<&'static str> {
        Some("Pdo\\Mysql")
    }

    /// pdo_mysql_handle_factory()
    fn connect(&self, dsn: &str, user: Option<&[u8]>, pass: Option<&[u8]>, options: &Options) -> Result<Box<dyn Connection>, ConnectError> {
        let vars = parse_data_source(dsn.as_bytes(), &["charset", "dbname", "host", "port", "unix_socket", "user", "password"]);
        let text = |i: usize| vars[i].as_ref().map(|v| String::from_utf8_lossy(v).into_owned());
        let mut flags = 0;
        if options.long(MYSQL_ATTR_MULTI_STATEMENTS, 1) != 0 {
            flags |= p8_mysqlnd::MULTI_STATEMENTS;
        }
        if options.long(MYSQL_ATTR_FOUND_ROWS, 0) != 0 {
            flags |= p8_mysqlnd::FOUND_ROWS;
        }
        if options.long(MYSQL_ATTR_IGNORE_SPACE, 0) != 0 {
            flags |= p8_mysqlnd::IGNORE_SPACE;
        }
        let charset = match text(0) {
            Some(name) => match p8_mysqlnd::charset(&name) {
                Some(cs) => cs.nr,
                None => {
                    let e = p8_mysqlnd::Error { code: p8_mysqlnd::CR_CANT_READ_CHARSET, sqlstate: "HY000".into(), message: "Unknown character set".into() };
                    return Err(connect_error(&e));
                }
            },
            None => p8_mysqlnd::DEFAULT_CHARSET,
        };
        let host = text(2).unwrap_or_else(|| "localhost".into());
        // atoi(): the leading digits; 0 is the default port.
        let port = text(3).map_or(3306, |p| p.trim_start().bytes().take_while(u8::is_ascii_digit).fold(0u32, |n, d| n.wrapping_mul(10).wrapping_add(u32::from(d - b'0'))));
        let port = u16::try_from(port).ok().filter(|&p| p != 0).unwrap_or(3306);
        let socket = text(4);
        let address = if host == "localhost" { Address::Socket(socket.as_deref().unwrap_or(DEFAULT_SOCKET)) } else { Address::Tcp { host: &host, port } };
        let user = user.map(<[u8]>::to_vec).or_else(|| vars[5].clone()).unwrap_or_default();
        let pass = pass.map(<[u8]>::to_vec).or_else(|| vars[6].clone()).unwrap_or_default();
        let timeout = options.long(ATTR_TIMEOUT, 30);
        let conn = p8_mysqlnd::Connection::connect(&ConnectOptions {
            address,
            user: &user,
            password: &pass,
            database: vars[1].as_deref().filter(|d| !d.is_empty()),
            flags,
            timeout: u64::try_from(timeout).ok().filter(|&t| t > 0).map(Duration::from_secs),
            charset,
        })
        .map_err(|e| connect_error(&e))?;
        let shared = Rc::new(Shared {
            conn: RefCell::new(conn),
            buffered: Cell::new(options.long(MYSQL_ATTR_USE_BUFFERED_QUERY, 1) != 0),
            native: Cell::new(options.long(ATTR_STRINGIFY_FETCHES, 0) == 0),
            table_names: Cell::new(false),
        });
        let emulate = options.long(ATTR_EMULATE_PREPARES, options.long(MYSQL_ATTR_DIRECT_QUERY, 1)) != 0;
        let autocommit = options.long(ATTR_AUTOCOMMIT, 1) != 0;
        let my = MyConn {
            shared,
            emulate,
            national: options.long(ATTR_DEFAULT_STR_PARAM, 0) == PARAM_STR_NATL,
            autocommit,
            local_infile: options.long(MYSQL_ATTR_LOCAL_INFILE, 0) != 0,
            local_infile_directory: option_str(options, MYSQL_ATTR_LOCAL_INFILE_DIRECTORY).filter(|d| !d.is_empty()),
            einfo: RefCell::new(None),
            refused: None,
        };
        // MYSQL_INIT_COMMAND runs as part of connecting: its error is the
        // connection's.
        if let Some(init) = option_str(options, MYSQL_ATTR_INIT_COMMAND) {
            let mut c = my.shared.conn.borrow_mut();
            c.query(&init).map_err(|e| connect_error(&e))?;
            // Each statement of it is read to its end.
            while let Some(next) = c.next_result() {
                next.map_err(|e| connect_error(&e))?;
            }
        }
        if !autocommit {
            let _ = my.shared.conn.borrow_mut().query(b"SET AUTOCOMMIT=0");
        }
        Ok(Box::new(my))
    }
}

/// pdo_mysql_db_handle
struct MyConn {
    shared: Rc<Shared>,
    emulate: bool,
    /// assume_national_character_set_strings: PDO::ATTR_DEFAULT_STR_PARAM
    /// is PDO::PARAM_STR_NATL.
    national: bool,
    autocommit: bool,
    local_infile: bool,
    local_infile_directory: Option<Vec<u8>>,
    /// The connection's last error (H->einfo), which errorInfo() shows
    /// after an error PDO raised itself.
    einfo: RefCell<Option<(i64, String)>>,
    /// Why the last attribute was refused.
    refused: Option<DriverError>,
}

impl MyConn {
    /// _pdo_mysql_error() on the connection: the error is also kept as the
    /// connection's last (H->einfo).
    fn error(&self, e: &p8_mysqlnd::Error) -> DriverError {
        let d = my_error(&self.shared.conn.borrow(), e);
        *self.einfo.borrow_mut() = Some((d.code, d.message.clone()));
        d
    }

    /// mysql_handle_doer(): run `sql`, the rows it changed. A result set
    /// is left unread (as PHP leaves it: the connection is out of sync until
    /// it closes); the results of further statements are read and dropped.
    fn doer(&mut self, sql: &[u8]) -> Result<i64, DriverError> {
        let r = (|| {
            let mut c = self.shared.conn.borrow_mut();
            if let Header::Columns(_) = c.query_start(sql)? {
                return Ok(0);
            }
            let affected = c.affected_rows as i64;
            while let Some(next) = c.next_header() {
                if let Header::Columns(_) = next? {
                    c.skip_rows()?;
                }
            }
            Ok(affected)
        })();
        r.map_err(|e| self.error(&e))
    }

    fn simple(&mut self, sql: &[u8]) -> Result<(), DriverError> {
        let r = self.shared.conn.borrow_mut().query(sql).map(|_| ());
        r.map_err(|e| self.error(&e))
    }

    fn statement(&self, kind: Kind) -> MyStmt {
        MyStmt { shared: Rc::clone(&self.shared), kind, columns: None, binary: false, rows: VecDeque::new(), streaming: false, done: false }
    }
}

/// pdo_get_bool_param()
fn bool_param(ctx: &mut dyn Ctx, value: &Value) -> Result<bool, Throw> {
    match value {
        Value::Bool(b) => Ok(*b),
        Value::Int(i) => Ok(*i != 0),
        other => Err(ctx.throw("TypeError", format!("Attribute value must be of type bool for selected attribute, {} given", p8_runtime::args::value_name(other)))),
    }
}

/// pdo_get_long_param()
fn long_param(ctx: &mut dyn Ctx, value: &Value) -> Result<i64, Throw> {
    match value {
        Value::Int(i) => Ok(*i),
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Str(s) => match p8_runtime::numeric::numeric_string(s.as_bytes()) {
            Some(p8_runtime::numeric::Number::Int(i)) => Ok(i),
            _ => Err(ctx.throw("TypeError", format!("Attribute value must be of type int for selected attribute, {} given", p8_runtime::args::value_name(value)))),
        },
        other => Err(ctx.throw("TypeError", format!("Attribute value must be of type int for selected attribute, {} given", p8_runtime::args::value_name(other)))),
    }
}

impl Connection for MyConn {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    fn placeholders(&self) -> Placeholders {
        if self.emulate { Placeholders::None } else { Placeholders::Positional }
    }

    fn scanner(&self) -> Scanner {
        Scanner::Mysql
    }

    fn prepare(&mut self, sql: &str) -> Result<Box<dyn Statement>, DriverError> {
        match self.prepare_native(sql.as_bytes(), &Options { entries: &[] })? {
            Some(s) => Ok(s),
            None => Ok(Box::new(self.statement(Kind::Text(sql.as_bytes().to_vec())))),
        }
    }

    /// mysql_handle_preparer(): a statement the server cannot prepare
    /// (1295) falls back to emulation.
    fn prepare_native(&mut self, sql: &[u8], _: &Options) -> Result<Option<Box<dyn Statement>>, DriverError> {
        let r = self.shared.conn.borrow_mut().prepare(sql);
        match r {
            Ok(p) => Ok(Some(Box::new(self.statement(Kind::Prepared(p))))),
            Err(e) if e.code == ER_UNSUPPORTED_PS => Ok(None),
            Err(e) => Err(self.error(&e)),
        }
    }

    fn prepare_emulated(&mut self, sql: &[u8]) -> Result<Box<dyn Statement>, DriverError> {
        Ok(Box::new(self.statement(Kind::Text(sql.to_vec()))))
    }

    fn exec(&mut self, sql: &[u8]) -> Result<i64, DriverError> {
        self.doer(sql)
    }

    /// mysql_handle_quoter(): `_binary'...'` for a LOB, `N'...'` for a
    /// national string, escaped in the connection's character set.
    fn quote(&mut self, s: &[u8], param_type: i64) -> Option<Vec<u8>> {
        let c = self.shared.conn.borrow();
        let cs = p8_mysqlnd::charset_by_nr(c.charset).or_else(|| p8_mysqlnd::charset("utf8mb4")).expect("a known charset");
        let body = if c.status & p8_mysqlnd::STATUS_NO_BACKSLASH_ESCAPES != 0 { p8_mysqlnd::escape_quotes(&cs, s) } else { p8_mysqlnd::escape(&cs, s) };
        let binary = param_type & PARAM_LOB == PARAM_LOB;
        let national = !binary && (self.national || param_type & PARAM_STR_NATL == PARAM_STR_NATL) && param_type & PARAM_STR_CHAR != PARAM_STR_CHAR;
        let mut out = Vec::with_capacity(body.len() + 9);
        if binary {
            out.extend_from_slice(b"_binary");
        } else if national {
            out.push(b'N');
        }
        out.push(b'\'');
        out.extend_from_slice(&body);
        out.push(b'\'');
        Some(out)
    }

    fn last_insert_id(&mut self, _: Option<&[u8]>) -> Result<Vec<u8>, DriverError> {
        Ok(self.shared.conn.borrow().insert_id.to_string().into_bytes())
    }

    fn begin(&mut self) -> Result<(), DriverError> {
        self.doer(b"START TRANSACTION").map(|_| ())
    }

    fn commit(&mut self) -> Result<(), DriverError> {
        self.simple(b"COMMIT")
    }

    fn rollback(&mut self) -> Result<(), DriverError> {
        self.simple(b"ROLLBACK")
    }

    /// pdo_mysql_in_transaction(): the server's status says.
    fn in_transaction(&mut self) -> Option<bool> {
        Some(self.shared.conn.borrow().status & p8_mysqlnd::STATUS_IN_TRANS != 0)
    }

    fn set_attribute(&mut self, ctx: &mut dyn Ctx, attr: i64, value: &Value) -> Result<bool, Throw> {
        self.refused = None;
        match attr {
            ATTR_AUTOCOMMIT => {
                let b = bool_param(ctx, value)?;
                if b != self.autocommit {
                    self.autocommit = b;
                    let sql: &[u8] = if b { b"SET AUTOCOMMIT=1" } else { b"SET AUTOCOMMIT=0" };
                    if let Err(e) = self.simple(sql) {
                        self.refused = Some(e);
                        return Ok(false);
                    }
                }
            }
            ATTR_DEFAULT_STR_PARAM => self.national = long_param(ctx, value)? == PARAM_STR_NATL,
            MYSQL_ATTR_USE_BUFFERED_QUERY => self.shared.buffered.set(bool_param(ctx, value)?),
            MYSQL_ATTR_DIRECT_QUERY | ATTR_EMULATE_PREPARES => self.emulate = bool_param(ctx, value)?,
            ATTR_FETCH_TABLE_NAMES => self.shared.table_names.set(bool_param(ctx, value)?),
            ATTR_STRINGIFY_FETCHES => self.shared.native.set(!bool_param(ctx, value)?),
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn attribute_error(&mut self) -> Option<DriverError> {
        self.refused.take()
    }

    fn error_info(&self) -> Option<(i64, String)> {
        self.einfo.borrow().clone()
    }

    fn get_attribute(&mut self, attr: i64) -> Option<Value> {
        let c = self.shared.conn.borrow();
        Some(match attr {
            ATTR_CLIENT_VERSION => Value::str(p8_mysqlnd::CLIENT_INFO),
            ATTR_SERVER_VERSION => Value::str(c.server_version.as_str()),
            ATTR_CONNECTION_STATUS => Value::str(c.host_info.as_str()),
            ATTR_AUTOCOMMIT => Value::Bool(self.autocommit),
            ATTR_DEFAULT_STR_PARAM => Value::Int(if self.national { PARAM_STR_NATL } else { PARAM_STR_CHAR }),
            MYSQL_ATTR_USE_BUFFERED_QUERY => Value::Bool(self.shared.buffered.get()),
            ATTR_EMULATE_PREPARES | MYSQL_ATTR_DIRECT_QUERY => Value::Bool(self.emulate),
            MYSQL_ATTR_LOCAL_INFILE => Value::Bool(self.local_infile),
            MYSQL_ATTR_LOCAL_INFILE_DIRECTORY => self.local_infile_directory.as_ref().map_or(Value::Null, |d| Value::Str(p8_runtime::string::PhpStr::new(d.clone()))),
            ATTR_FETCH_TABLE_NAMES => Value::Bool(self.shared.table_names.get()),
            _ => return None,
        })
    }

    fn get_attribute_checked(&mut self, attr: i64) -> Result<Option<Value>, DriverError> {
        if attr != ATTR_SERVER_INFO {
            return Ok(self.get_attribute(attr));
        }
        // mysqlnd_stat(): COM_STATISTICS.
        let r = self.shared.conn.borrow_mut().stat();
        match r {
            Ok(s) => Ok(Some(Value::Str(p8_runtime::string::PhpStr::new(s)))),
            Err(e) => Err(self.error(&e)),
        }
    }
}

// ---- statements ----

/// How a statement runs.
enum Kind {
    /// COM_QUERY of a query with its values quoted in (emulated prepares,
    /// and what the server cannot prepare).
    Text(Vec<u8>),
    /// Prepared on the server; run with COM_STMT_EXECUTE.
    Prepared(Prepared),
}

/// pdo_mysql_stmt
struct MyStmt {
    shared: Rc<Shared>,
    kind: Kind,
    /// The columns of the current result set (S->result); None for a
    /// statement that returned no rows.
    columns: Option<Vec<Column>>,
    /// The current result's rows are binary-protocol rows.
    binary: bool,
    /// The rows read and not yet fetched (a buffered result).
    rows: VecDeque<Vec<u8>>,
    /// The rows are read from the connection as they are fetched (an
    /// unbuffered result still being read).
    streaming: bool,
    /// The results after the current one were dealt with (S->done).
    done: bool,
}

impl MyStmt {
    fn error(&self, e: &p8_mysqlnd::Error) -> DriverError {
        my_error(&self.shared.conn.borrow(), e)
    }

    /// pdo_mysql_free_result(): what is left of the current result goes
    /// (an unbuffered one's rows are read and dropped).
    fn free_result(&mut self) {
        if self.streaming {
            self.streaming = false;
            let _ = self.shared.conn.borrow_mut().skip_rows();
        }
        self.rows.clear();
        self.columns = None;
    }

    /// The results after the current one, read and dropped.
    fn drain(&mut self) -> Result<(), p8_mysqlnd::Error> {
        let mut c = self.shared.conn.borrow_mut();
        while let Some(next) = c.next_header() {
            if let Header::Columns(_) = next? {
                c.skip_rows()?;
            }
        }
        Ok(())
    }

    /// pdo_mysql_fill_stmt_from_result() and
    /// pdo_mysql_stmt_after_execute_prepared(): take in a result whose
    /// header was read; its rows are read now (buffered) or as they are
    /// fetched.
    fn take(&mut self, header: Header) -> Result<ExecInfo, DriverError> {
        let columns = match header {
            Header::Done => {
                let affected = self.shared.conn.borrow().affected_rows as i64;
                return Ok(ExecInfo { column_count: 0, row_count: Some(affected) });
            }
            Header::Columns(c) => c,
        };
        let count = columns.len();
        self.binary = matches!(self.kind, Kind::Prepared(_));
        self.columns = Some(columns);
        if !self.shared.buffered.get() {
            self.streaming = true;
            // mysql_num_rows() of an unbuffered result is 0; a prepared
            // statement's affected rows are unknown (-1, kept as it was).
            return Ok(ExecInfo { column_count: count, row_count: if self.binary { None } else { Some(0) } });
        }
        let mut rows = VecDeque::new();
        loop {
            let r = self.shared.conn.borrow_mut().read_row();
            match r {
                Ok(Some(row)) => rows.push_back(row),
                Ok(None) => break,
                Err(e) => {
                    self.columns = None;
                    return Err(self.error(&e));
                }
            }
        }
        let n = rows.len() as i64;
        self.rows = rows;
        Ok(ExecInfo { column_count: count, row_count: Some(n) })
    }

    fn decode(&self, packet: &[u8]) -> Result<Vec<DbValue>, DriverError> {
        let columns = self.columns.as_deref().unwrap_or_default();
        let values = if self.binary { p8_mysqlnd::binary_values(packet, columns) } else { p8_mysqlnd::text_values(packet, columns, self.shared.native.get()) };
        let values = values.map_err(|e| self.error(&e))?;
        Ok(values
            .into_iter()
            .map(|v| match v {
                FieldValue::Null => DbValue::Null,
                FieldValue::Int(i) => DbValue::Int(i),
                FieldValue::Double(d) => DbValue::Float(d),
                FieldValue::Bytes(b) => DbValue::Text(b),
            })
            .collect())
    }
}

impl Drop for MyStmt {
    /// pdo_mysql_stmt_dtor(): the unread rows and results go, and the
    /// server forgets a prepared statement. Results still pending on the
    /// connection are read and dropped unless this statement dealt with
    /// its own (S->done); closing a prepared statement drops them anyway
    /// (mysqlnd's close_on_server), whoever left them.
    fn drop(&mut self) {
        if self.shared.conn.try_borrow_mut().is_err() {
            return;
        }
        self.free_result();
        let prepared = matches!(self.kind, Kind::Prepared(_));
        if !self.done || prepared {
            let _ = self.drain();
        }
        if let Kind::Prepared(p) = &self.kind {
            self.shared.conn.borrow_mut().close_statement(p.id);
        }
    }
}

/// A bound value as mysqlnd sends it (pdo_mysql_stmt_param_hook()).
fn param(v: &DbValue) -> Param<'_> {
    match v {
        DbValue::Null => Param::Null,
        DbValue::Int(i) => Param::Int(*i),
        DbValue::Float(f) => Param::Double(*f),
        DbValue::Bool(b) => Param::Tiny(i8::from(*b)),
        DbValue::Text(t) | DbValue::Blob(t) | DbValue::Lob(t) => Param::Bytes(t),
    }
}

/// type_to_name_native()
fn native_type(kind: u8) -> Option<&'static str> {
    use p8_mysqlnd::*;
    Some(match kind {
        TYPE_STRING => "STRING",
        TYPE_VAR_STRING => "VAR_STRING",
        TYPE_TINY => "TINY",
        TYPE_BIT => "BIT",
        TYPE_SHORT => "SHORT",
        TYPE_LONG => "LONG",
        TYPE_LONGLONG => "LONGLONG",
        TYPE_INT24 => "INT24",
        TYPE_FLOAT => "FLOAT",
        TYPE_DOUBLE => "DOUBLE",
        TYPE_DECIMAL => "DECIMAL",
        TYPE_NEWDECIMAL => "NEWDECIMAL",
        TYPE_GEOMETRY => "GEOMETRY",
        TYPE_TIMESTAMP => "TIMESTAMP",
        TYPE_YEAR => "YEAR",
        TYPE_SET => "SET",
        TYPE_ENUM => "ENUM",
        TYPE_DATE => "DATE",
        TYPE_NEWDATE => "NEWDATE",
        TYPE_VECTOR => "VECTOR",
        TYPE_JSON => "JSON",
        TYPE_TIME => "TIME",
        TYPE_DATETIME => "DATETIME",
        TYPE_TINY_BLOB => "TINY_BLOB",
        TYPE_MEDIUM_BLOB => "MEDIUM_BLOB",
        TYPE_LONG_BLOB => "LONG_BLOB",
        TYPE_BLOB => "BLOB",
        TYPE_NULL => "NULL",
        _ => return None,
    })
}

impl Statement for MyStmt {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    /// pdo_mysql_stmt_execute()
    fn execute(&mut self, binds: &[(BindKey, DbValue)]) -> Result<ExecInfo, DriverError> {
        self.free_result();
        self.done = false;
        let r = match &self.kind {
            Kind::Text(sql) => {
                let mut c = self.shared.conn.borrow_mut();
                c.query_start(sql).map_err(|e| my_error(&c, &e))
            }
            Kind::Prepared(p) => {
                let mut params: Vec<Option<Param>> = vec![None; p.params];
                for (key, v) in binds {
                    if let BindKey::Pos(i) = key
                        && let Some(slot) = params.get_mut(*i)
                    {
                        *slot = Some(param(v));
                    }
                }
                // A `?` left without a value (two binds went to one).
                let Some(params) = params.into_iter().collect::<Option<Vec<_>>>() else {
                    return Err(DriverError::new("HY000", 2031, "No data supplied for parameters in prepared statement"));
                };
                let mut c = self.shared.conn.borrow_mut();
                c.execute(p.id, &params).map_err(|e| my_error(&c, &e))
            }
        };
        self.take(r?)
    }

    fn fetch(&mut self) -> Result<Option<Vec<DbValue>>, DriverError> {
        if self.columns.is_none() {
            return Ok(None);
        }
        let packet = if self.streaming {
            let r = self.shared.conn.borrow_mut().read_row();
            match r {
                Ok(Some(p)) => p,
                Ok(None) => {
                    self.streaming = false;
                    return Ok(None);
                }
                Err(e) => {
                    self.streaming = false;
                    return Err(self.error(&e));
                }
            }
        } else {
            match self.rows.pop_front() {
                Some(p) => p,
                None => return Ok(None),
            }
        };
        self.decode(&packet).map(Some)
    }

    fn column_count(&self) -> usize {
        self.columns.as_ref().map_or(0, Vec::len)
    }

    /// pdo_mysql_stmt_describe(): `table.name` with PDO::ATTR_FETCH_TABLE_NAMES.
    fn column_name(&self, i: usize) -> Vec<u8> {
        let Some(col) = self.columns.as_ref().and_then(|c| c.get(i)) else { return Vec::new() };
        if self.shared.table_names.get() {
            let mut n = col.table.clone();
            n.push(b'.');
            n.extend_from_slice(&col.name);
            return n;
        }
        col.name.clone()
    }

    /// pdo_mysql_stmt_col_meta()
    fn column_meta(&mut self, i: usize) -> Option<ColumnMeta> {
        let col = self.columns.as_ref()?.get(i)?;
        let mut flags = Vec::new();
        for (bit, name) in [
            (p8_mysqlnd::NOT_NULL_FLAG, "not_null"),
            (p8_mysqlnd::PRI_KEY_FLAG, "primary_key"),
            (p8_mysqlnd::MULTIPLE_KEY_FLAG, "multiple_key"),
            (p8_mysqlnd::UNIQUE_KEY_FLAG, "unique_key"),
            (p8_mysqlnd::BLOB_FLAG, "blob"),
        ] {
            if col.flags & bit != 0 {
                flags.push(Value::str(name));
            }
        }
        let mut entries = Vec::new();
        if let Some(t) = native_type(col.kind) {
            entries.push(("native_type".to_string(), Value::str(t)));
        }
        let pdo_type = match col.kind {
            p8_mysqlnd::TYPE_BIT | p8_mysqlnd::TYPE_YEAR | p8_mysqlnd::TYPE_TINY | p8_mysqlnd::TYPE_SHORT | p8_mysqlnd::TYPE_INT24 | p8_mysqlnd::TYPE_LONG | p8_mysqlnd::TYPE_LONGLONG => PARAM_INT,
            _ => PARAM_STR,
        };
        entries.push(("pdo_type".to_string(), Value::Int(pdo_type)));
        entries.push(("flags".to_string(), Value::Array(Rc::new(p8_runtime::array::PhpArray::from_list(flags)))));
        entries.push(("table".to_string(), Value::Str(p8_runtime::string::PhpStr::new(col.table.clone()))));
        Some(ColumnMeta { ordered: Some(entries), len: Some(i64::from(col.length)), precision: Some(i64::from(col.decimals)), ..ColumnMeta::default() })
    }

    fn meta_index_error(&self) -> bool {
        false
    }

    /// pdo_mysql_stmt_cursor_closer(): the rest of this result and the
    /// results after it are read and dropped.
    fn close_cursor(&mut self) {
        let _ = self.close_cursor_checked();
    }

    fn close_cursor_checked(&mut self) -> Result<(), DriverError> {
        self.done = true;
        self.free_result();
        self.drain().map_err(|e| self.error(&e))
    }

    fn param_count(&self) -> Option<usize> {
        match &self.kind {
            Kind::Prepared(p) => Some(p.params),
            Kind::Text(_) => None,
        }
    }

    /// pdo_mysql_stmt_next_rowset()
    fn next_rowset(&mut self) -> Option<Result<Option<ExecInfo>, DriverError>> {
        self.free_result();
        let next = self.shared.conn.borrow_mut().next_header();
        Some(match next {
            None => {
                self.done = true;
                Ok(None)
            }
            Some(Err(e)) => {
                self.done = true;
                Err(self.error(&e))
            }
            Some(Ok(header)) => self.take(header).map(Some),
        })
    }
}

// ---- Pdo\Mysql ----

type R = Result<Value, Throw>;

/// Pdo\Mysql::getWarningCount(): the warnings of the last statement.
fn get_warning_count(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    if !args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", format!("Pdo\\Mysql::getWarningCount() expects exactly 0 arguments, {} given", args.len())));
    }
    let dbh = p8_ext_pdo::dbh_of(ctx, this)?;
    let mut c = dbh.conn.borrow_mut();
    let warnings = c.as_any().downcast_mut::<MyConn>().map_or(0, |my| my.shared.conn.borrow().warnings);
    Ok(Value::Int(i64::from(warnings)))
}

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "Pdo\\Mysql",
    parent: Some("PDO"),
    interfaces: &[],
    flags: CLASS_UNCLONEABLE,
    properties: &[],
    methods: &[("getWarningCount", Some(Native::Method(get_warning_count)))],
    constants: p8_ext_pdo::consts::MYSQL_CONSTANTS,
    debug_info: None,
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_source_pairs() {
        let v = parse_data_source(b"host=db; port=3307;dbname=a;;b;charset=utf8mb4", &["charset", "dbname", "host", "port", "unix_socket"]);
        assert_eq!(v[0].as_deref(), Some(&b"utf8mb4"[..]));
        assert_eq!(v[1].as_deref(), Some(&b"a;b"[..]));
        assert_eq!(v[2].as_deref(), Some(&b"db"[..]));
        assert_eq!(v[3].as_deref(), Some(&b"3307"[..]));
        assert_eq!(v[4], None);
    }

    #[test]
    fn native_type_names() {
        assert_eq!(native_type(p8_mysqlnd::TYPE_NEWDECIMAL), Some("NEWDECIMAL"));
        assert_eq!(native_type(p8_mysqlnd::TYPE_VAR_STRING), Some("VAR_STRING"));
        assert_eq!(native_type(19), None);
    }
}

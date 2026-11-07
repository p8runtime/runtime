#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's pdo_sqlite driver: the `sqlite:` DSN, `Pdo\Sqlite`, and SQLite's
//! type handling, over Turso's Rust rewrite of SQLite (`turso_core`, behind
//! [`engine`]): no C library. What is observable follows php-src's
//! ext/pdo_sqlite: DSNs, error codes and SQLSTATEs, integers as int, reals
//! as float, text and blobs as strings.
//!
//! Not implemented: user-defined SQL functions, aggregates and collations
//! (`createFunction` and friends check their arguments and succeed, but a
//! query that calls the function fails: turso_core's only hook for them is
//! a C ABI that needs `unsafe`), `loadExtension`, `openBlob`, URI filenames
//! other than `file::memory:`.
//!
//! Reference: $PHP_SRC_PATH/ext/pdo_sqlite (sqlite_driver.c, sqlite_statement.c)

mod engine;

use engine::{Engine, EngineError, Prepared, Val};
use p8_ext_pdo::driver::{BindKey, ColumnMeta, ConnectError, Connection, DbValue, Driver, DriverError, ExecInfo, Options, Placeholders, Scanner, Statement};
use p8_runtime::ext::{ClassSpec, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::value::Value;
use std::any::Any;
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "pdo_sqlite", register };

fn register(registry: &mut Registry) {
    p8_ext_pdo::register_driver(&SQLITE);
    registry.add_classes(CLASSES);
    registry.add_module("pdo_sqlite");
}

const PDO_ATTR_TIMEOUT: i64 = 2;
const SQLITE_ATTR_OPEN_FLAGS: i64 = 1000;
const SQLITE_ATTR_READONLY_STATEMENT: i64 = 1001;
const SQLITE_ATTR_EXTENDED_RESULT_CODES: i64 = 1002;
const OPEN_READONLY: i64 = 1;
const OPEN_READWRITE: i64 = 2;
const OPEN_CREATE: i64 = 4;
const ATTR_CLIENT_VERSION: i64 = 5;
const ATTR_SERVER_VERSION: i64 = 4;

struct SqliteDriver;
static SQLITE: SqliteDriver = SqliteDriver;

/// pdo_sqlite_error(): SQLSTATE from SQLite's result code.
fn sqlstate_of(code: i64) -> &'static str {
    match code {
        12 => "42S02",
        engine::SQLITE_INTERRUPT => "01002",
        22 => "HYC00",
        engine::SQLITE_TOOBIG => "22001",
        engine::SQLITE_CONSTRAINT => "23000",
        _ => "HY000",
    }
}

fn driver_error(e: EngineError) -> DriverError {
    DriverError { sqlstate: sqlstate_of(e.code).to_string(), code: e.code, message: e.message }
}

impl Driver for SqliteDriver {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn class_name(&self) -> Option<&'static str> {
        Some("Pdo\\Sqlite")
    }

    fn connect(&self, dsn: &str, _user: Option<&[u8]>, _pass: Option<&[u8]>, options: &Options) -> Result<Box<dyn Connection>, ConnectError> {
        let flags = options.long(SQLITE_ATTR_OPEN_FLAGS, OPEN_READWRITE | OPEN_CREATE);
        let path = filename(dsn);
        let engine = Engine::open(&path, flags & OPEN_READONLY != 0 && flags & OPEN_READWRITE == 0, flags & OPEN_CREATE != 0).map_err(|e| {
            let _ = e;
            // Turso words open failures by what its I/O layer hit; SQLite says this.
            let message = "unable to open database file".to_string();
            let code = engine::SQLITE_CANTOPEN;
            let state = sqlstate_of(code);
            ConnectError { message: format!("SQLSTATE[{state}] [{code}] {message}"), code, info: Some(DriverError::new(state, code, message)) }
        })?;
        let timeout = options.long(PDO_ATTR_TIMEOUT, 60);
        engine.busy_timeout_ms((timeout.max(0) as u64) * 1000);
        Ok(Box::new(SqliteConn { engine: Rc::new(engine), functions: Vec::new() }))
    }
}

/// The file a DSN names: `:memory:`, an empty name (a private database,
/// kept in memory here), a `file:` URI of memory, or a path.
fn filename(dsn: &str) -> String {
    if dsn.is_empty() || dsn == ":memory:" {
        return ":memory:".into();
    }
    if let Some(rest) = dsn.strip_prefix("file:") {
        let name = rest.split('?').next().unwrap_or("");
        if name == ":memory:" || name.is_empty() {
            return ":memory:".into();
        }
        return name.to_string();
    }
    dsn.to_string()
}

struct SqliteConn {
    engine: Rc<Engine>,
    /// Names given to createFunction() and its kin.
    functions: Vec<String>,
}

impl Connection for SqliteConn {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    fn placeholders(&self) -> Placeholders {
        Placeholders::Native
    }

    fn scanner(&self) -> Scanner {
        Scanner::Sqlite
    }

    fn prepare(&mut self, sql: &str) -> Result<Box<dyn Statement>, DriverError> {
        let p = self.engine.prepare(sql).map_err(driver_error)?;
        Ok(Box::new(SqliteStmt { p, pre_fetched: None, done: false, executed: false, last_row: None }))
    }

    fn exec(&mut self, sql: &[u8]) -> Result<i64, DriverError> {
        // sqlite3_exec() stops at a NUL.
        let end = sql.iter().position(|&b| b == 0).unwrap_or(sql.len());
        self.engine.exec(&String::from_utf8_lossy(&sql[..end])).map_err(driver_error)
    }

    fn quote(&mut self, s: &[u8], _param_type: i64) -> Option<Vec<u8>> {
        // sqlite3_snprintf("'%q'"): a C string, so it ends at a NUL.
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        let mut out = Vec::with_capacity(end + 2);
        out.push(b'\'');
        for &b in &s[..end] {
            if b == b'\'' {
                out.push(b'\'');
            }
            out.push(b);
        }
        out.push(b'\'');
        Some(out)
    }

    fn last_insert_id(&mut self, _name: Option<&[u8]>) -> Result<Vec<u8>, DriverError> {
        Ok(self.engine.last_insert_rowid().to_string().into_bytes())
    }

    fn begin(&mut self) -> Result<(), DriverError> {
        self.engine.exec("BEGIN").map(|_| ()).map_err(driver_error)
    }

    fn commit(&mut self) -> Result<(), DriverError> {
        self.engine.exec("COMMIT").map(|_| ()).map_err(driver_error)
    }

    fn rollback(&mut self) -> Result<(), DriverError> {
        self.engine.exec("ROLLBACK").map(|_| ()).map_err(driver_error)
    }

    fn in_transaction(&mut self) -> Option<bool> {
        Some(self.engine.in_transaction())
    }

    fn set_attribute(&mut self, ctx: &mut dyn Ctx, attr: i64, value: &Value) -> Result<bool, Throw> {
        match attr {
            PDO_ATTR_TIMEOUT => {
                let secs = int_param(ctx, value)?;
                self.engine.busy_timeout_ms((secs.max(0) as u64) * 1000);
                Ok(true)
            }
            SQLITE_ATTR_EXTENDED_RESULT_CODES => {
                int_param(ctx, value)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn get_attribute(&mut self, attr: i64) -> Option<Value> {
        match attr {
            ATTR_CLIENT_VERSION | ATTR_SERVER_VERSION => {
                let mut p = self.engine.prepare("SELECT sqlite_version()").ok()?;
                match p.step().ok()?? .into_iter().next()? {
                    Val::Text(t) => Some(Value::str(p8_runtime::string::PhpStr::new(t))),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn call_method(&mut self, ctx: &mut dyn Ctx, name: &str, args: &mut [Value]) -> Option<Result<Value, Throw>> {
        Some(match name {
            "sqliteCreateFunction" | "createFunction" => create_function(ctx, self, name, args),
            "sqliteCreateAggregate" | "createAggregate" => create_aggregate(ctx, self, name, args),
            "sqliteCreateCollation" | "createCollation" => create_collation(ctx, self, name, args),
            _ => return None,
        })
    }
}

fn int_param(ctx: &mut dyn Ctx, value: &Value) -> Result<i64, Throw> {
    if let Some(i) = value.as_int() {
        Ok(i)
    } else if let Some(b) = value.as_bool() {
        Ok(i64::from(b))
    } else if let Some(s) = value.as_str() {
        match p8_runtime::numeric::numeric_string(s.as_bytes()) {
            Some(p8_runtime::numeric::Number::Int(i)) => Ok(i),
            _ => Err(ctx.throw("TypeError", format!("Attribute value must be of type int for selected attribute, {} given", p8_runtime::args::value_name(value)))),
        }
    } else {
        Err(ctx.throw("TypeError", format!("Attribute value must be of type int for selected attribute, {} given", p8_runtime::args::value_name(value))))
    }
}

// ---- user-defined functions (checked, not run: see the crate docs) ----

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

fn name_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<String, Throw> {
    let v = arg(args, 0);
    match v.as_str() {
        Some(s) => Ok(String::from_utf8_lossy(s.as_bytes()).into_owned()),
        None => Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($function_name) must be of type string, {} given", p8_runtime::args::value_name(&v)))),
    }
}

fn callable_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value], i: usize, name: &str) -> Result<(), Throw> {
    let v = arg(args, i);
    if !ctx.is_callable(&v) {
        let why = p8_runtime::args::callable_error(ctx, &v);
        return Err(ctx.throw("TypeError", format!("{func}(): Argument #{} (${name}) must be a valid callback, {why}", i + 1)));
    }
    Ok(())
}

fn create_function(ctx: &mut dyn Ctx, conn: &mut SqliteConn, func: &str, args: &mut [Value]) -> Result<Value, Throw> {
    let full = format!("PDO::{func}");
    let name = name_arg(ctx, &full, args)?;
    callable_arg(ctx, &full, args, 1, "callback")?;
    let n = arg(args, 2);
    if let Some(n) = n.as_int()
        && n < -1
    {
        ctx.warn(&format!("{full}(): Argument #3 ($num_args) must be greater than or equal to -1"));
        return Ok(Value::FALSE);
    }
    conn.functions.push(name);
    Ok(Value::TRUE)
}

fn create_aggregate(ctx: &mut dyn Ctx, conn: &mut SqliteConn, func: &str, args: &mut [Value]) -> Result<Value, Throw> {
    let full = format!("PDO::{func}");
    let name = name_arg(ctx, &full, args)?;
    callable_arg(ctx, &full, args, 1, "step")?;
    callable_arg(ctx, &full, args, 2, "finalize")?;
    conn.functions.push(name);
    Ok(Value::TRUE)
}

fn create_collation(ctx: &mut dyn Ctx, conn: &mut SqliteConn, func: &str, args: &mut [Value]) -> Result<Value, Throw> {
    let full = format!("PDO::{func}");
    let name = name_arg(ctx, &full, args)?;
    callable_arg(ctx, &full, args, 1, "callback")?;
    conn.functions.push(name);
    Ok(Value::TRUE)
}

// ---- statements ----

struct SqliteStmt {
    p: Prepared,
    /// The first row, read by execute() as sqlite3_step() does.
    pre_fetched: Option<Vec<Val>>,
    done: bool,
    executed: bool,
    /// The row column_meta() describes.
    last_row: Option<Vec<Val>>,
}

fn to_val(v: &DbValue) -> Val {
    match v {
        DbValue::Null => Val::Null,
        DbValue::Int(i) => Val::Int(*i),
        DbValue::Float(f) => Val::Float(*f),
        DbValue::Text(t) => Val::Text(t.clone()),
        DbValue::Blob(b) | DbValue::Lob(b) => Val::Blob(b.clone()),
        DbValue::Bool(b) => Val::Int(i64::from(*b)),
    }
}

fn from_val(v: Val) -> DbValue {
    match v {
        Val::Null => DbValue::Null,
        Val::Int(i) => DbValue::Int(i),
        Val::Float(f) => DbValue::Float(f),
        Val::Text(t) => DbValue::Text(t),
        Val::Blob(b) => DbValue::Blob(b),
    }
}

impl Statement for SqliteStmt {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    fn execute(&mut self, binds: &[(BindKey, DbValue)]) -> Result<ExecInfo, DriverError> {
        if self.executed && !self.done {
            self.p.reset();
        }
        self.done = false;
        self.pre_fetched = None;
        self.last_row = None;
        for (key, value) in binds {
            let index = match key {
                BindKey::Pos(i) => i + 1,
                BindKey::Name(n) => self.p.param_index(n).unwrap_or(0),
            };
            self.p.bind(index, to_val(value)).map_err(driver_error)?;
        }
        self.executed = true;
        match self.p.step() {
            Ok(Some(row)) => {
                let n = row.len();
                self.pre_fetched = Some(row.clone());
                self.last_row = Some(row);
                Ok(ExecInfo { column_count: n, row_count: None })
            }
            Ok(None) => {
                let changes = self.p.changes();
                self.p.reset();
                self.done = true;
                Ok(ExecInfo { column_count: self.p.column_count(), row_count: Some(changes) })
            }
            Err(e) => {
                self.p.reset();
                self.done = true;
                Err(driver_error(e))
            }
        }
    }

    fn fetch(&mut self) -> Result<Option<Vec<DbValue>>, DriverError> {
        if let Some(row) = self.pre_fetched.take() {
            return Ok(Some(row.into_iter().map(from_val).collect()));
        }
        if self.done {
            return Ok(None);
        }
        match self.p.step() {
            Ok(Some(row)) => {
                self.last_row = Some(row.clone());
                Ok(Some(row.into_iter().map(from_val).collect()))
            }
            Ok(None) => {
                self.done = true;
                self.last_row = None;
                self.p.reset();
                Ok(None)
            }
            Err(e) => {
                self.p.reset();
                self.done = true;
                Err(driver_error(e))
            }
        }
    }

    fn column_count(&self) -> usize {
        self.p.column_count()
    }

    fn column_name(&self, i: usize) -> Vec<u8> {
        self.p.column_name(i).into_bytes()
    }

    fn column_meta(&mut self, i: usize) -> Option<ColumnMeta> {
        if !self.executed || i >= self.p.column_count() {
            return None;
        }
        let mut m = ColumnMeta::default();
        match self.last_row.as_ref().and_then(|r| r.get(i)) {
            None | Some(Val::Null) => {
                m.native_type = Some("null".into());
                m.pdo_type = 0;
            }
            Some(Val::Float(_)) => {
                m.native_type = Some("double".into());
                m.pdo_type = 2;
            }
            Some(Val::Blob(_)) => {
                m.flags.push("blob".into());
                m.native_type = Some("string".into());
                m.pdo_type = 2;
            }
            Some(Val::Text(_)) => {
                m.native_type = Some("string".into());
                m.pdo_type = 2;
            }
            Some(Val::Int(_)) => {
                m.native_type = Some("integer".into());
                m.pdo_type = 1;
            }
        }
        if let Some(d) = self.p.column_decltype(i) {
            m.extra.push(("sqlite:decl_type".into(), d));
        }
        if let Some(t) = self.p.column_table(i) {
            m.extra.push(("table".into(), t));
        }
        Some(m)
    }

    fn close_cursor(&mut self) {
        self.p.reset();
        self.pre_fetched = None;
        self.done = true;
    }

    fn get_attribute(&mut self, attr: i64) -> Option<Value> {
        match attr {
            SQLITE_ATTR_READONLY_STATEMENT => Some(Value::bool(self.p.readonly())),
            _ => None,
        }
    }
}

// ---- Pdo\Sqlite ----

fn sqlite_create_function(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    p8_ext_pdo::call_driver_method(ctx, this, "createFunction", args)
}

fn sqlite_create_aggregate(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    p8_ext_pdo::call_driver_method(ctx, this, "createAggregate", args)
}

fn sqlite_create_collation(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    p8_ext_pdo::call_driver_method(ctx, this, "createCollation", args)
}

fn sqlite_load_extension(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(ctx.throw("PDOException", "SQLSTATE[HY000]: General error: Extension loading is not supported by p8's pdo_sqlite".into()))
}

fn sqlite_open_blob(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(ctx.throw("PDOException", "SQLSTATE[HY000]: General error: Pdo\\Sqlite::openBlob() is not supported by p8's pdo_sqlite".into()))
}

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "Pdo\\Sqlite",
    parent: Some("PDO"),
    interfaces: &[],
    flags: CLASS_UNCLONEABLE,
    properties: &[],
    methods: &[
        ("createAggregate", Some(Native::Method(sqlite_create_aggregate))),
        ("createCollation", Some(Native::Method(sqlite_create_collation))),
        ("createFunction", Some(Native::Method(sqlite_create_function))),
        ("loadExtension", Some(Native::Method(sqlite_load_extension))),
        ("openBlob", Some(Native::Method(sqlite_open_blob))),
    ],
    constants: p8_ext_pdo::consts::SQLITE_CONSTANTS,
    debug_info: None,
}];

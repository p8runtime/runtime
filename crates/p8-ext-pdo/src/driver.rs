//! What a PDO driver is: the traits `pdo_sqlite` implements and a later
//! `pdo_mysql` (over p8-mysqlnd) will. The core owns everything PHP-visible
//! (objects, error modes, fetch modes, bound variables); a driver owns the
//! connection and speaks in plain Rust values.
//!
//! Reference: $PHP_SRC_PATH/ext/pdo/php_pdo_driver.h (pdo_dbh_methods,
//! pdo_stmt_methods)

use p8_runtime::ext::Ctx;
use p8_runtime::ext::Throw;
use p8_runtime::value::Value;
use std::any::Any;

/// A value in a row, or bound to a parameter, as the database holds it.
#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Int(i64),
    Float(f64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}

/// An error the driver reports: PDO's errorInfo is `[sqlstate, code, message]`.
#[derive(Debug, Clone)]
pub struct DriverError {
    /// Five characters ("HY000").
    pub sqlstate: String,
    /// The driver's own error code.
    pub code: i64,
    pub message: String,
}

impl DriverError {
    pub fn new(sqlstate: &str, code: i64, message: impl Into<String>) -> DriverError {
        DriverError { sqlstate: sqlstate.to_string(), code, message: message.into() }
    }
}

/// What a connection attempt can fail with: PDO throws a PDOException, with
/// this exact message (the driver words it as PHP does).
#[derive(Debug, Clone)]
pub struct ConnectError {
    pub message: String,
    pub code: i64,
    /// The sqlstate and driver error for `errorInfo`, when there are some.
    pub info: Option<DriverError>,
}

/// The placeholder syntaxes a driver takes natively (pdo_dbh.c's
/// `supports_placeholders`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholders {
    /// None: the core puts quoted values into the query (emulated prepares).
    None,
    /// `?` and `:name` both: the driver binds them itself, by position or
    /// by name (pdo_sqlite).
    Native,
}

/// Which lexer finds placeholders: the default one, or SQLite's (which also
/// skips backtick and bracket quoted identifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scanner {
    Default,
    Sqlite,
}

/// A column as `getColumnMeta()` describes it (the driver's part).
#[derive(Debug, Clone, Default)]
pub struct ColumnMeta {
    pub native_type: Option<String>,
    pub pdo_type: i64,
    pub flags: Vec<String>,
    /// Further entries, in order (`sqlite:decl_type`, `table`).
    pub extra: Vec<(String, String)>,
}

/// One driver (the `sqlite` of `sqlite:...`).
pub trait Driver: Send + Sync {
    /// The DSN prefix: `sqlite`.
    fn name(&self) -> &'static str;
    /// The class `PDO::connect()` makes for it (`Pdo\Sqlite`).
    fn class_name(&self) -> Option<&'static str> {
        None
    }
    /// Open a connection. `dsn` is what follows the first colon, `options`
    /// the constructor's driver options by their integer key.
    fn connect(&self, dsn: &str, user: Option<&[u8]>, pass: Option<&[u8]>, options: &Options) -> Result<Box<dyn Connection>, ConnectError>;
}

/// The constructor's options array, by integer key.
pub struct Options<'a> {
    pub entries: &'a [(i64, Value)],
}

impl Options<'_> {
    /// pdo_attr_lval(): the option as an integer, or `default`.
    pub fn long(&self, attr: i64, default: i64) -> i64 {
        self.entries.iter().find(|(k, _)| *k == attr).map_or(default, |(_, v)| v.to_int())
    }
}

/// A live connection.
pub trait Connection: Any {
    fn as_any(&mut self) -> &mut dyn Any;

    fn placeholders(&self) -> Placeholders {
        Placeholders::None
    }

    fn scanner(&self) -> Scanner {
        Scanner::Default
    }

    /// Prepare `sql` (placeholders already rewritten for `placeholders()`).
    fn prepare(&mut self, sql: &str) -> Result<Box<dyn Statement>, DriverError>;

    /// Run statements that return no rows; the rows affected.
    fn exec(&mut self, sql: &[u8]) -> Result<i64, DriverError>;

    /// `PDO::quote()`; None when it cannot.
    fn quote(&mut self, s: &[u8], param_type: i64) -> Option<Vec<u8>>;

    fn last_insert_id(&mut self, name: Option<&[u8]>) -> Result<Vec<u8>, DriverError>;

    fn begin(&mut self) -> Result<(), DriverError>;
    fn commit(&mut self) -> Result<(), DriverError>;
    fn rollback(&mut self) -> Result<(), DriverError>;

    /// Whether a transaction is open, when the connection knows (None: the
    /// core tracks it from begin/commit/rollback).
    fn in_transaction(&mut self) -> Option<bool> {
        None
    }

    /// `PDO::setAttribute()` for an attribute the core does not handle.
    /// `Ok(false)` is "not supported / refused" (PDO then reports the
    /// connection's error state); Err is a PHP exception (a TypeError).
    fn set_attribute(&mut self, ctx: &mut dyn Ctx, attr: i64, value: &Value) -> Result<bool, Throw> {
        let _ = (ctx, attr, value);
        Ok(false)
    }

    /// `PDO::getAttribute()` for an attribute the core does not handle:
    /// None: not supported.
    fn get_attribute(&mut self, attr: i64) -> Option<Value> {
        let _ = attr;
        None
    }

    /// The driver-specific methods (`sqliteCreateFunction`): `name` in the
    /// case PHP lists it. None: this driver has no such method.
    fn call_method(&mut self, ctx: &mut dyn Ctx, name: &str, args: &mut [Value]) -> Option<Result<Value, Throw>> {
        let _ = (ctx, name, args);
        None
    }

    /// The most recent error the connection holds (`fetch_err`): extra
    /// elements of `errorInfo` after the sqlstate.
    fn error_info(&self) -> Option<(i64, String)> {
        None
    }
}

/// A prepared statement.
pub trait Statement: Any {
    fn as_any(&mut self) -> &mut dyn Any;

    /// Run it with the bound values.
    fn execute(&mut self, binds: &[(BindKey, DbValue)]) -> Result<ExecInfo, DriverError>;

    /// The next row, None at the end.
    fn fetch(&mut self) -> Result<Option<Vec<DbValue>>, DriverError>;

    fn column_count(&self) -> usize;

    fn column_name(&self, i: usize) -> Vec<u8>;

    /// None: the driver has no meta data.
    fn column_meta(&mut self, i: usize) -> Option<ColumnMeta> {
        let _ = i;
        None
    }

    /// Stop reading the result so the statement can run again.
    fn close_cursor(&mut self) {}

    fn get_attribute(&mut self, attr: i64) -> Option<Value> {
        let _ = attr;
        None
    }
}

/// What a value is bound to: a 0-based position, or a name with its colon
/// (`:id`) as the script wrote it.
#[derive(Debug, Clone, PartialEq)]
pub enum BindKey {
    Pos(usize),
    Name(String),
}

/// What running a statement tells the core.
#[derive(Debug, Clone, Copy)]
pub struct ExecInfo {
    pub column_count: usize,
    /// Rows changed (a statement that returns no rows); None keeps the old count.
    pub row_count: Option<i64>,
}

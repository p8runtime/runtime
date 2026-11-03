//! What a PDO driver is: the traits `pdo_sqlite`, `pdo_pgsql` and
//! `pdo_mysql` (over p8-mysqlnd) implement. The core owns everything PHP-visible
//! (objects, error modes, fetch modes, bound variables); a driver owns the
//! connection and speaks in plain Rust values.
//!
//! Reference: $PHP_SRC_PATH/ext/pdo/php_pdo_driver.h (pdo_dbh_methods,
//! pdo_stmt_methods)

use p8_runtime::ext::Ctx;
use p8_runtime::ext::Throw;
use p8_runtime::object::Object;
use p8_runtime::value::Value;
use std::any::Any;
use std::rc::Rc;

/// A value in a row, or bound to a parameter, as the database holds it.
#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Int(i64),
    Float(f64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
    /// A boolean (pdo_pgsql's `bool` columns; bound for PDO::PARAM_BOOL).
    Bool(bool),
    /// A large object the script gets as a stream (pdo_pgsql's `bytea`).
    Lob(Vec<u8>),
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
    /// `$1`, `$2`: the core rewrites `?` and `:name` into them
    /// (`named_rewrite_template = "$%d"`, pdo_pgsql) and binds by position.
    Dollar,
    /// `?` only (PDO_PLACEHOLDER_POSITIONAL, pdo_mysql): the core rewrites
    /// `:name` into `?` and binds by position, each value as its PHP type.
    Positional,
}

/// Which lexer finds placeholders: the default one, or SQLite's (which also
/// skips backtick and bracket quoted identifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scanner {
    Default,
    Sqlite,
    /// pgsql_sql_parser.re: E'' strings, dollar quoting, `??` for `?`.
    Pgsql,
    /// mysql_sql_parser.re: backslash escapes in strings, `` quoting,
    /// `#` and `-- ` comments.
    Mysql,
}

/// A column as `getColumnMeta()` describes it (the driver's part).
#[derive(Debug, Clone, Default)]
pub struct ColumnMeta {
    pub native_type: Option<String>,
    pub pdo_type: i64,
    pub flags: Vec<String>,
    /// Further entries, in order (`sqlite:decl_type`, `table`).
    pub extra: Vec<(String, String)>,
    /// The whole driver part, in the driver's order (replaces the fields
    /// above; no `flags` is added).
    pub ordered: Option<Vec<(String, Value)>>,
    /// `len` and `precision` (-1 and 0 when None).
    pub len: Option<i64>,
    pub precision: Option<i64>,
}

/// A driver method reached through PDO itself (`PDO::pgsqlGetPid()`).
pub type DriverMethod = fn(&mut dyn Ctx, &Rc<Object>, &mut [Value]) -> Result<Value, Throw>;

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
    /// A driver method PHP lists on PDO (`pgsqlGetPid`), by name.
    fn pdo_method(&self, name: &str) -> Option<DriverMethod> {
        let _ = name;
        None
    }
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

    /// The placeholders one statement takes, given `PDO::prepare()`'s
    /// options (pdo_pgsql emulates per statement).
    fn placeholders_for(&mut self, options: &Options) -> Placeholders {
        let _ = options;
        self.placeholders()
    }

    /// Prepare `sql` (placeholders already rewritten for `placeholders()`).
    fn prepare(&mut self, sql: &str) -> Result<Box<dyn Statement>, DriverError>;

    /// Prepare `sql` with `PDO::prepare()`'s options.
    fn prepare_with(&mut self, sql: &[u8], options: &Options) -> Result<Box<dyn Statement>, DriverError> {
        let _ = options;
        self.prepare(&String::from_utf8_lossy(sql))
    }

    /// Prepare `sql` for Placeholders::Positional (`:name` already
    /// rewritten to `?`). Ok(None): the server cannot prepare it, and the
    /// statement is emulated instead (pdo_mysql's fallback on error 1295).
    fn prepare_native(&mut self, sql: &[u8], options: &Options) -> Result<Option<Box<dyn Statement>>, DriverError> {
        self.prepare_with(sql, options).map(Some)
    }

    /// A statement for an emulated prepare: `sql` has the values in it.
    fn prepare_emulated(&mut self, sql: &[u8]) -> Result<Box<dyn Statement>, DriverError> {
        self.prepare(&String::from_utf8_lossy(sql))
    }

    /// Whether PDO::ATTR_CURSOR => PDO::CURSOR_SCROLL is supported.
    fn scrollable(&self) -> bool {
        false
    }

    /// For an emulated prepare: a statement made at PDO::prepare() time,
    /// kept across executes and given each one's query with
    /// [`Statement::set_query`] (pdo_pgsql's scroll cursors). None: a new
    /// statement per execute.
    fn emulated_shell(&mut self, options: &Options) -> Option<Box<dyn Statement>> {
        let _ = options;
        None
    }

    /// The string a PDO::PARAM_BOOL parameter of an emulated prepare
    /// becomes when it is bound (as PDO::PARAM_STR), when the driver words
    /// booleans itself (pdo_pgsql's EVT_NORMALIZE: `t`, `f`).
    fn emulated_bool(&mut self, b: bool) -> Option<Vec<u8>> {
        let _ = b;
        None
    }

    /// What a stream bound as PDO::PARAM_LOB is sent as, when it is one of
    /// the driver's own (pdo_pgsql's large objects go as their oid); `path`
    /// is the stream's.
    fn lob_param(&mut self, path: &str) -> Option<DbValue> {
        let _ = path;
        None
    }

    /// Messages the server sent that a script callback should see, and the
    /// callback (pdo_pgsql's notices).
    fn take_notices(&mut self) -> Option<(Value, Vec<Vec<u8>>)> {
        None
    }

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

    /// Why the last [`Connection::set_attribute`] that returned `Ok(false)`
    /// failed, when the server refused it (pdo_mysql's autocommit).
    fn attribute_error(&mut self) -> Option<DriverError> {
        None
    }

    /// `PDO::getAttribute()` for an attribute the core does not handle:
    /// None: not supported.
    fn get_attribute(&mut self, attr: i64) -> Option<Value> {
        let _ = attr;
        None
    }

    /// `PDO::getAttribute()` for an attribute that asks the server, which
    /// can fail (the driver's -1: PDO reports the connection's error).
    fn get_attribute_checked(&mut self, attr: i64) -> Result<Option<Value>, DriverError> {
        Ok(self.get_attribute(attr))
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

    /// Run a Placeholders::Dollar statement; `mapped` is false when the
    /// query had no `?`/`:name` to rewrite (pdo_pgsql then sends as many
    /// untyped NULLs as values were bound, as PHP does).
    fn execute_dollar(&mut self, binds: &[(BindKey, DbValue)], mapped: bool) -> Result<ExecInfo, DriverError> {
        let _ = mapped;
        self.execute(binds)
    }

    /// The next row, None at the end.
    fn fetch(&mut self) -> Result<Option<Vec<DbValue>>, DriverError>;

    /// The row PDO::FETCH_ORI_* `ori` (with `offset`) points to; drivers
    /// without scrollable cursors read the next one.
    fn fetch_scroll(&mut self, ori: i64, offset: i64) -> Result<Option<Vec<DbValue>>, DriverError> {
        let _ = (ori, offset);
        self.fetch()
    }

    /// Take the query of the next execute (see [`Connection::emulated_shell`]).
    fn set_query(&mut self, sql: &[u8]) {
        let _ = sql;
    }

    fn column_count(&self) -> usize;

    fn column_name(&self, i: usize) -> Vec<u8>;

    /// None: the driver has no meta data.
    fn column_meta(&mut self, i: usize) -> Option<ColumnMeta> {
        let _ = i;
        None
    }

    /// Whether `getColumnMeta()` of a column past the last is SQLSTATE
    /// 07009 (pdo_sqlite, pdo_pgsql); false: a quiet `false` (pdo_mysql).
    fn meta_index_error(&self) -> bool {
        true
    }

    /// Stop reading the result so the statement can run again.
    fn close_cursor(&mut self) {}

    /// `PDOStatement::closeCursor()`: [`Statement::close_cursor`], with the
    /// error reading what was left gave (pdo_mysql's later result sets).
    fn close_cursor_checked(&mut self) -> Result<(), DriverError> {
        self.close_cursor();
        Ok(())
    }

    /// How many parameters a Placeholders::Positional statement takes
    /// (binding past them is HY093); None when the driver does not say.
    fn param_count(&self) -> Option<usize> {
        None
    }

    /// Move to the next result set (`PDOStatement::nextRowset()`): Ok(None)
    /// when there is none. None: the driver has no multiple rowsets.
    fn next_rowset(&mut self) -> Option<Result<Option<ExecInfo>, DriverError>> {
        None
    }

    /// The bytes a column of the current row stands for when it is fetched
    /// as PDO::PARAM_LOB (pdo_pgsql: the large object an oid names).
    fn lob_column(&self, col: usize) -> Option<Vec<u8>> {
        let _ = col;
        None
    }

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

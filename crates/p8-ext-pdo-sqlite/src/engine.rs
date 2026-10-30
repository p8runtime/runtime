//! The only module that names turso_core. A gap in Turso is fixed here, so
//! the PHP-facing driver never sees it.

use std::num::NonZero;
use std::sync::Arc;
use turso_core::{Connection, Database, LimboError, OpenFlags, OpenOptions, PlatformIO, SqliteDialect, StepResult, Value, IO};

/// SQLite's primary result codes, the ones PDO tells apart.
pub const SQLITE_ERROR: i64 = 1;
pub const SQLITE_BUSY: i64 = 5;
pub const SQLITE_READONLY: i64 = 8;
pub const SQLITE_INTERRUPT: i64 = 9;
pub const SQLITE_CANTOPEN: i64 = 14;
pub const SQLITE_TOOBIG: i64 = 18;
pub const SQLITE_CONSTRAINT: i64 = 19;
pub const SQLITE_RANGE: i64 = 25;

/// A value as SQLite stores it.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Null,
    Int(i64),
    Float(f64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct EngineError {
    pub code: i64,
    pub message: String,
}

fn map_error(e: &LimboError) -> EngineError {
    let message = e.to_string();
    let code = match e {
        LimboError::Constraint(_) | LimboError::ForeignKeyConstraint(_) | LimboError::Raise(..) => SQLITE_CONSTRAINT,
        LimboError::Busy | LimboError::TableLocked => SQLITE_BUSY,
        LimboError::ReadOnly => SQLITE_READONLY,
        LimboError::Interrupt => SQLITE_INTERRUPT,
        LimboError::TooBig => SQLITE_TOOBIG,
        _ => SQLITE_ERROR,
    };
    // Turso words parse errors "Parse error: ..." where SQLite says the bare message.
    let message = message.strip_prefix("Parse error: ").map(str::to_string).unwrap_or(message);
    EngineError { code, message }
}

pub struct Engine {
    _db: Arc<Database>,
    conn: Arc<Connection>,
}

impl Engine {
    /// Open `path` (":memory:" for a private memory database).
    pub fn open(path: &str, readonly: bool, create: bool) -> Result<Engine, EngineError> {
        let io: Arc<dyn IO> = if path == ":memory:" { Arc::new(turso_core::MemoryIO::new()) } else { Arc::new(PlatformIO::new().map_err(|e| map_error(&e))?) };
        let mut flags = OpenFlags::None;
        if create {
            flags |= OpenFlags::Create;
        }
        if readonly {
            flags |= OpenFlags::ReadOnly;
        }
        let db = Database::open(io, path, OpenOptions::new(Arc::new(SqliteDialect)).flags(flags)).map_err(|e| map_error(&e))?;
        let conn = db.connect().map_err(|e| map_error(&e))?;
        Ok(Engine { _db: db, conn })
    }

    /// Run SQL that returns no rows (one or more statements); the rows changed.
    pub fn exec(&self, sql: &str) -> Result<i64, EngineError> {
        self.conn.prepare_execute_batch(sql).map_err(|e| map_error(&e))?;
        Ok(self.conn.changes())
    }

    pub fn prepare(&self, sql: &str) -> Result<Prepared, EngineError> {
        let stmt = self.conn.prepare(sql).map_err(|e| map_error(&e))?;
        Ok(Prepared { stmt, conn: Arc::clone(&self.conn) })
    }

    pub fn last_insert_rowid(&self) -> i64 {
        self.conn.last_insert_rowid()
    }

    pub fn in_transaction(&self) -> bool {
        !self.conn.get_auto_commit()
    }

    pub fn busy_timeout_ms(&self, ms: u64) {
        self.conn.set_busy_timeout(std::time::Duration::from_millis(ms));
    }
}

pub struct Prepared {
    stmt: turso_core::Statement,
    conn: Arc<Connection>,
}

fn to_turso(v: Val) -> Value {
    match v {
        Val::Null => Value::Null,
        Val::Int(i) => Value::from_i64(i),
        // SQLite stores a NaN as NULL.
        Val::Float(f) => match turso_core::NonNan::new(f) {
            Some(n) => Value::Numeric(turso_core::Numeric::Float(n)),
            None => Value::Null,
        },
        // Turso's text is UTF-8 (a Rust str); anything else is kept as a blob.
        Val::Text(b) => match String::from_utf8(b) {
            Ok(s) => Value::from_text(s),
            Err(e) => Value::from_blob(e.into_bytes().into_iter().collect()),
        },
        Val::Blob(b) => Value::from_blob(b.into_iter().collect()),
    }
}

fn from_turso(v: &Value) -> Val {
    match v {
        Value::Null => Val::Null,
        Value::Numeric(turso_core::Numeric::Integer(i)) => Val::Int(*i),
        Value::Numeric(turso_core::Numeric::Float(f)) => Val::Float(f64::from(*f)),
        Value::Text(t) => Val::Text(t.as_str().as_bytes().to_vec()),
        Value::Blob(b) => Val::Blob(b.to_vec()),
    }
}

impl Prepared {
    pub fn param_count(&self) -> usize {
        self.stmt.parameters_count()
    }

    /// The 1-based index of a named parameter (`:id`).
    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.stmt.parameter_index(name).map(NonZero::get)
    }

    /// Bind parameter `index` (1-based).
    pub fn bind(&mut self, index: usize, v: Val) -> Result<(), EngineError> {
        if index == 0 || index > self.param_count() {
            return Err(EngineError { code: SQLITE_RANGE, message: "column index out of range".into() });
        }
        let index = NonZero::new(index).expect("checked");
        self.stmt.bind_at(index, to_turso(v)).map_err(|e| map_error(&e))
    }

    pub fn reset(&mut self) {
        let _ = self.stmt.reset();
    }

    /// The next row; None when the statement is done.
    pub fn step(&mut self) -> Result<Option<Vec<Val>>, EngineError> {
        loop {
            match self.stmt.step().map_err(|e| map_error(&e))? {
                StepResult::Row => {
                    let row = self.stmt.row().expect("a row after Row");
                    return Ok(Some(row.get_values().map(from_turso).collect()));
                }
                StepResult::Done => return Ok(None),
                StepResult::IO | StepResult::Yield | StepResult::Sleep { .. } => {
                    self.stmt._io().step().map_err(|e| map_error(&e))?;
                }
                StepResult::Interrupt => return Err(EngineError { code: SQLITE_INTERRUPT, message: "interrupted".into() }),
                StepResult::Busy => return Err(EngineError { code: SQLITE_BUSY, message: "database is locked".into() }),
            }
        }
    }

    pub fn column_count(&self) -> usize {
        self.stmt.num_columns()
    }

    pub fn column_name(&self, i: usize) -> String {
        self.stmt.get_column_name(i).into_owned()
    }

    pub fn column_decltype(&self, i: usize) -> Option<String> {
        self.stmt.get_column_decltype(i)
    }

    pub fn column_table(&self, i: usize) -> Option<String> {
        self.stmt.get_column_table_name(i).map(|c| c.into_owned())
    }

    /// Rows changed by the last statement run on the connection.
    pub fn changes(&self) -> i64 {
        self.conn.changes()
    }

    /// Whether the statement leaves the database alone.
    pub fn readonly(&self) -> bool {
        self.stmt.get_program().sql.trim_start().get(..6).is_some_and(|s| s.eq_ignore_ascii_case("select"))
    }
}

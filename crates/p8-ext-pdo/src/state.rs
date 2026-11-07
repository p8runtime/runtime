//! The state behind a PDO object (pdo_dbh_t), shared by the statements it
//! makes, and PDO's error reporting (pdo_handle_error, pdo_raise_impl_error).

use crate::driver::{Connection, Driver};
use crate::sqlstate;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const ERRMODE_SILENT: i64 = 0;
pub const ERRMODE_WARNING: i64 = 1;
pub const ERRMODE_EXCEPTION: i64 = 2;

pub const CASE_NATURAL: i64 = 0;
pub const CASE_UPPER: i64 = 1;
pub const CASE_LOWER: i64 = 2;

pub const NULL_NATURAL: i64 = 0;
pub const NULL_EMPTY_STRING: i64 = 1;
pub const NULL_TO_STRING: i64 = 2;

pub const FETCH_USE_DEFAULT: i64 = 0;
pub const FETCH_LAZY: i64 = 1;
pub const FETCH_ASSOC: i64 = 2;
pub const FETCH_NUM: i64 = 3;
pub const FETCH_BOTH: i64 = 4;
pub const FETCH_OBJ: i64 = 5;
pub const FETCH_BOUND: i64 = 6;
pub const FETCH_COLUMN: i64 = 7;
pub const FETCH_CLASS: i64 = 8;
pub const FETCH_INTO: i64 = 9;
pub const FETCH_FUNC: i64 = 10;
pub const FETCH_NAMED: i64 = 11;
pub const FETCH_KEY_PAIR: i64 = 12;
/// One past the last plain mode (PDO_FETCH__MAX).
pub const FETCH_MAX: i64 = 13;
pub const FETCH_GROUP: i64 = 0x10000;
pub const FETCH_UNIQUE: i64 = 0x30000;
pub const FETCH_CLASSTYPE: i64 = 0x40000;
pub const FETCH_SERIALIZE: i64 = 0x80000;
pub const FETCH_PROPS_LATE: i64 = 0x100000;
pub const FETCH_FLAGS: i64 = 0xFFFF0000;

pub const PARAM_NULL: i64 = 0;
pub const PARAM_INT: i64 = 1;
pub const PARAM_STR: i64 = 2;
pub const PARAM_LOB: i64 = 3;
pub const PARAM_STMT: i64 = 4;
pub const PARAM_BOOL: i64 = 5;

pub const ATTR_AUTOCOMMIT: i64 = 0;
pub const ATTR_PREFETCH: i64 = 1;
pub const ATTR_TIMEOUT: i64 = 2;
pub const ATTR_ERRMODE: i64 = 3;
pub const ATTR_SERVER_VERSION: i64 = 4;
pub const ATTR_CLIENT_VERSION: i64 = 5;
pub const ATTR_CASE: i64 = 8;
pub const ATTR_ORACLE_NULLS: i64 = 11;
pub const ATTR_PERSISTENT: i64 = 12;
pub const ATTR_STATEMENT_CLASS: i64 = 13;
pub const ATTR_DRIVER_NAME: i64 = 16;
pub const ATTR_STRINGIFY_FETCHES: i64 = 17;
pub const ATTR_DEFAULT_FETCH_MODE: i64 = 19;
pub const ATTR_EMULATE_PREPARES: i64 = 20;
pub const ATTR_DEFAULT_STR_PARAM: i64 = 21;

/// The error state of a connection or statement: the SQLSTATE (empty: none
/// yet, PHP's null) and the driver's own error, if it has one.
#[derive(Debug, Clone, Default)]
pub struct ErrState {
    pub sqlstate: String,
    pub info: Option<(i64, String)>,
}

impl ErrState {
    pub fn clear(&mut self) {
        self.sqlstate = "00000".into();
        self.info = None;
    }
}

/// ATTR_STATEMENT_CLASS: the class statements are made of, and its
/// constructor arguments.
#[derive(Clone)]
pub struct StmtClass {
    pub name: PhpStr,
    pub ctor_args: Option<Rc<PhpArray>>,
}

/// A database handle (pdo_dbh_t).
pub struct Dbh {
    pub driver: &'static dyn Driver,
    pub conn: RefCell<Box<dyn Connection>>,
    pub error_mode: Cell<i64>,
    pub desired_case: Cell<i64>,
    pub oracle_nulls: Cell<i64>,
    pub stringify: Cell<bool>,
    pub emulate: Cell<bool>,
    pub default_fetch: Cell<i64>,
    pub default_str_param: Cell<i64>,
    pub autocommit: Cell<bool>,
    pub persistent: Cell<bool>,
    pub in_txn: Cell<bool>,
    pub error: RefCell<ErrState>,
    pub stmt_class: RefCell<Option<StmtClass>>,
    /// A `query()` that failed after preparing: errorCode()/errorInfo() are its.
    pub query_error: RefCell<Option<ErrState>>,
}

impl Dbh {
    pub fn clear_error(&self) {
        self.error.borrow_mut().clear();
        *self.query_error.borrow_mut() = None;
    }

    pub fn in_transaction(&self) -> bool {
        match self.conn.borrow_mut().in_transaction() {
            Some(b) => b,
            None => self.in_txn.get(),
        }
    }
}

/// A PDO object's native data: empty until its constructor runs.
#[derive(Default)]
pub struct PdoData {
    pub dbh: RefCell<Option<Rc<Dbh>>>,
}

pub fn pdo_data(o: &Object) -> &PdoData {
    o.native::<PdoData>().expect("PDO state")
}

/// The connection of `this`, or the Error PHP throws for a PDO whose
/// constructor did not run (PDO_CONSTRUCT_CHECK).
pub fn dbh_of(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<Rc<Dbh>, Throw> {
    match pdo_data(this).dbh.borrow().as_ref() {
        Some(d) => Ok(Rc::clone(d)),
        None => Err(ctx.throw("Error", format!("{} object is uninitialized", String::from_utf8_lossy(this.class.shown())))),
    }
}

/// Where an error is recorded: the statement's state, or the connection's.
pub enum Where<'a> {
    Dbh,
    Stmt(&'a RefCell<ErrState>),
}

fn description(state: &str) -> &'static str {
    sqlstate::describe(state).unwrap_or("<<Unknown error>>")
}

pub fn exception(ctx: &mut dyn Ctx, message: String, code: Value, info: Option<PhpArray>) -> Throw {
    let mut t = ctx.throw("PDOException", message);
    if let Throw::Error { props, .. } = &mut t {
        props.push(("code", code));
        if let Some(info) = info {
            props.push(("errorInfo", Value::array(info)));
        }
    }
    t
}

fn info_array(items: Vec<Value>) -> PhpArray {
    PhpArray::from_list(items)
}

/// pdo_raise_impl_error(): record `sqlstate` and report it as PDO itself
/// does (not a driver error). `func` is "PDOStatement::execute".
pub fn impl_error(ctx: &mut dyn Ctx, func: &str, dbh: &Dbh, at: Where, sqlstate: &str, supp: Option<&str>) -> Result<(), Throw> {
    match at {
        Where::Dbh => dbh.error.borrow_mut().sqlstate = sqlstate.to_string(),
        Where::Stmt(e) => e.borrow_mut().sqlstate = sqlstate.to_string(),
    }
    let message = match supp {
        Some(s) => format!("SQLSTATE[{sqlstate}]: {}: {s}", description(sqlstate)),
        None => format!("SQLSTATE[{sqlstate}]: {}", description(sqlstate)),
    };
    if dbh.error_mode.get() != ERRMODE_EXCEPTION {
        ctx.warn(&format!("{func}(): {message}"));
        return Ok(());
    }
    let info = info_array(vec![Value::str(sqlstate), Value::int(0)]);
    Err(exception(ctx, message, Value::str(sqlstate), Some(info)))
}

/// pdo_handle_error(): report the error the driver left in the state.
pub fn handle_error(ctx: &mut dyn Ctx, func: &str, dbh: &Dbh, at: Where) -> Result<(), Throw> {
    if dbh.error_mode.get() == ERRMODE_SILENT {
        return Ok(());
    }
    let (sqlstate, native) = match at {
        Where::Dbh => {
            let e = dbh.error.borrow();
            (e.sqlstate.clone(), e.info.clone())
        }
        Where::Stmt(e) => {
            let e = e.borrow();
            (e.sqlstate.clone(), e.info.clone())
        }
    };
    // PDO_HANDLE_STMT_ERR(): nothing to report when no error is recorded.
    if sqlstate == "00000" {
        return Ok(());
    }
    let mut items = vec![Value::str(sqlstate.as_str())];
    let message = match &native {
        Some((code, supp)) => {
            items.push(Value::int(*code));
            items.push(Value::str(supp.as_str()));
            if *code != 0 {
                format!("SQLSTATE[{sqlstate}]: {}: {code} {supp}", description(&sqlstate))
            } else {
                format!("SQLSTATE[{sqlstate}]: {}: {supp}", description(&sqlstate))
            }
        }
        None => format!("SQLSTATE[{sqlstate}]: {}", description(&sqlstate)),
    };
    if dbh.error_mode.get() == ERRMODE_WARNING {
        ctx.warn(&format!("{func}(): {message}"));
        return Ok(());
    }
    Err(exception(ctx, message, Value::str(sqlstate.as_str()), Some(info_array(items))))
}

/// Record a driver error in `at`.
pub fn record(dbh: &Dbh, at: &Where, e: &crate::driver::DriverError) {
    let st = ErrState { sqlstate: e.sqlstate.clone(), info: Some((e.code, e.message.clone())) };
    match at {
        Where::Dbh => *dbh.error.borrow_mut() = st,
        Where::Stmt(c) => *c.borrow_mut() = st,
    }
}

pub fn str_key(s: &[u8]) -> Key {
    Key::from_string(PhpStr::from(s))
}

/// Hand the driver's pending notices to the script's callback (pdo_pgsql's
/// notice processor), once the connection is free again.
pub fn dispatch_notices(ctx: &mut dyn Ctx, dbh: &Dbh) -> Result<(), Throw> {
    let pending = dbh.conn.borrow_mut().take_notices();
    if let Some((callback, messages)) = pending {
        for m in messages {
            ctx.call(&callback, &mut [Value::str(PhpStr::new(m))])?;
        }
    }
    Ok(())
}

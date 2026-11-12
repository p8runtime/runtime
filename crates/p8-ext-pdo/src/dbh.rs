//! The PDO class: connecting, preparing, transactions, attributes.
//!
//! Reference: $PHP_SRC_PATH/ext/pdo/pdo_dbh.c

use crate::driver::{ConnectError, Driver, Options};
use crate::state::*;
use crate::stmt::{self, R, arg, value_name};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Mutex;

static DRIVERS: Mutex<Vec<&'static dyn Driver>> = Mutex::new(Vec::new());

/// Make `driver` known to PDO (`sqlite:` DSNs, `PDO::getAvailableDrivers()`).
pub fn register_driver(driver: &'static dyn Driver) {
    let mut d = DRIVERS.lock().expect("driver list");
    if !d.iter().any(|x| x.name() == driver.name()) {
        d.push(driver);
    }
}

fn drivers() -> Vec<&'static dyn Driver> {
    DRIVERS.lock().expect("driver list").clone()
}

fn find_driver(name: &[u8]) -> Option<&'static dyn Driver> {
    drivers().into_iter().find(|d| d.name().as_bytes() == name)
}

pub fn pdo_drivers(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::array(PhpArray::from_list(drivers().iter().map(|d| Value::str(d.name())).collect())))
}

pub fn m_get_available_drivers(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    pdo_drivers(ctx, args)
}

fn pdo_exception(ctx: &mut dyn Ctx, message: String) -> Throw {
    exception(ctx, message, Value::int(0), None)
}

/// The text of a string argument; a TypeError for anything else.
fn str_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value], i: usize, name: &str) -> Result<PhpStr, Throw> {
    match arg(args, i).into_str() {
        Ok(s) => Ok(s),
        Err(v) if v.is_number() || v.is_bool() => Ok(arg(args, i).to_php_string().unwrap_or_else(PhpStr::empty)),
        Err(other) => Err(ctx.throw("TypeError", format!("{func}(): Argument #{} (${name}) must be of type string, {} given", i + 1, value_name(&other)))),
    }
}

fn opt_str(ctx: &mut dyn Ctx, func: &str, args: &[Value], i: usize, name: &str) -> Result<Option<PhpStr>, Throw> {
    match arg(args, i) {
        v if v.is_null() => Ok(None),
        _ => str_arg(ctx, func, args, i, name).map(Some),
    }
}

fn long_param(ctx: &mut dyn Ctx, value: &Value) -> Result<i64, Throw> {
    if let Some(i) = value.as_int() {
        Ok(i)
    } else if let Some(b) = value.as_bool() {
        Ok(i64::from(b))
    } else if let Some(s) = value.as_str() {
        match p8_runtime::numeric::numeric_string(s.as_bytes()) {
            Some(p8_runtime::numeric::Number::Int(i)) => Ok(i),
            _ => Err(ctx.throw("TypeError", format!("Attribute value must be of type int for selected attribute, {} given", value_name(value)))),
        }
    } else {
        Err(ctx.throw("TypeError", format!("Attribute value must be of type int for selected attribute, {} given", value_name(value))))
    }
}

fn bool_param(ctx: &mut dyn Ctx, value: &Value) -> Result<bool, Throw> {
    if let Some(b) = value.as_bool() {
        Ok(b)
    } else if let Some(i) = value.as_int() {
        Ok(i != 0)
    } else {
        Err(ctx.throw("TypeError", format!("Attribute value must be of type bool for selected attribute, {} given", value_name(value))))
    }
}

/// A new Dbh around a connection.
fn new_dbh(driver: &'static dyn Driver, conn: Box<dyn crate::driver::Connection>, options: &Options) -> Rc<Dbh> {
    Rc::new(Dbh {
        driver,
        conn: RefCell::new(conn),
        error_mode: Cell::new(options.long(ATTR_ERRMODE, ERRMODE_EXCEPTION)),
        desired_case: Cell::new(CASE_NATURAL),
        oracle_nulls: Cell::new(NULL_NATURAL),
        stringify: Cell::new(false),
        emulate: Cell::new(false),
        default_fetch: Cell::new(FETCH_BOTH),
        default_str_param: Cell::new(PARAM_STR),
        autocommit: Cell::new(options.long(ATTR_AUTOCOMMIT, 1) != 0),
        persistent: Cell::new(false),
        in_txn: Cell::new(false),
        error: RefCell::new(ErrState::default()),
        stmt_class: RefCell::new(None),
        query_error: RefCell::new(None),
    })
}

fn find_class(ctx: &mut dyn Ctx, name: &str) -> Option<Rc<p8_runtime::object::Class>> {
    ctx.class(name.as_bytes())
}

/// php_pdo_internal_construct_driver(): `this` is the PDO object (its
/// native state is filled in).
fn construct(ctx: &mut dyn Ctx, this: &Rc<Object>, func: &str, args: &[Value]) -> Result<(), Throw> {
    let dsn = str_arg(ctx, func, args, 0, "dsn")?;
    let user = opt_str(ctx, func, args, 1, "username")?;
    let pass = opt_str(ctx, func, args, 2, "password")?;
    let options = match arg(args, 3).into_array() {
        Ok(a) => Some(a),
        Err(v) if v.is_null() => None,
        Err(other) => {
            return Err(ctx.throw("TypeError", format!("{func}(): Argument #4 ($options) must be of type ?array, {} given", value_name(&other))));
        }
    };
    let bytes = dsn.as_bytes();
    let Some(colon) = bytes.iter().position(|&b| b == b':') else {
        return Err(exception(ctx, format!("{func}(): Argument #1 ($dsn) must be a valid data source name"), Value::int(0), None));
    };
    let Some(driver) = find_driver(&bytes[..colon]) else {
        return Err(pdo_exception(ctx, "could not find driver".into()));
    };
    // A driver-specific subclass (Pdo\Sqlite) only connects to its own driver.
    for d in drivers() {
        let Some(cn) = d.class_name() else { continue };
        let lower = cn.to_ascii_lowercase();
        if this.class.lower != b"pdo" && this.class.is_a(lower.as_bytes()) && d.name() != driver.name() {
            let mine = if func.ends_with("connect") { "connect" } else { "__construct" };
            let ours = driver.class_name();
            let msg = match ours {
                Some(o) => format!("{}::{mine}() cannot be used for connecting to the \"{}\" driver, either call {o}::{mine}() or PDO::{mine}() instead", String::from_utf8_lossy(this.class.shown()), driver.name()),
                None => format!("{}::{mine}() cannot be used for connecting to the \"{}\" driver, call PDO::{mine}() instead", String::from_utf8_lossy(this.class.shown()), driver.name()),
            };
            return Err(pdo_exception(ctx, msg));
        }
    }
    let entries: Vec<(i64, Value)> = options
        .as_ref()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| match k {
                    Key::Int(i) => Some((i, v.deref())),
                    Key::Str(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let opts = Options { entries: &entries };
    let dsn_rest = String::from_utf8_lossy(&bytes[colon + 1..]).into_owned();
    let conn = match driver.connect(&dsn_rest, user.as_ref().map(|u| u.as_bytes()), pass.as_ref().map(|p| p.as_bytes()), &opts, ctx.cwd()) {
        Ok(c) => c,
        Err(ConnectError { message, code, info }) => {
            let info = info.map(|e| PhpArray::from_list(vec![Value::str(e.sqlstate.as_str()), Value::int(e.code), Value::str(e.message.as_str())]));
            return Err(exception(ctx, message, Value::int(code), info));
        }
    };
    let dbh = new_dbh(driver, conn, &opts);
    let final_mode = dbh.error_mode.get();
    dbh.persistent.set(entries.iter().any(|(k, v)| *k == ATTR_PERSISTENT && v.to_bool()));
    // Warnings while the options are set are exceptions (EH_THROW).
    dbh.error_mode.set(ERRMODE_EXCEPTION);
    *pdo_data(this).dbh.borrow_mut() = Some(Rc::clone(&dbh));
    let mut requested = final_mode;
    for (attr, value) in &entries {
        if *attr == ATTR_ERRMODE
            && let Ok(m) = long_param(ctx, value)
            && (ERRMODE_SILENT..=ERRMODE_EXCEPTION).contains(&m)
        {
            requested = m;
        }
        attribute_set(ctx, func, &dbh, *attr, value)?;
    }
    dbh.error_mode.set(requested);
    Ok(())
}

pub fn m_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", "PDO::__construct() expects at least 1 argument, 0 given".into()));
    }
    construct(ctx, this, "PDO::__construct", args)?;
    Ok(Value::NULL)
}

/// PDO::connect(): a new PDO (the driver's own class for PDO::connect()).
pub fn m_connect(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", "PDO::connect() expects at least 1 argument, 0 given".into()));
    }
    let called = ctx.native_class().or_else(|| ctx.called_class()).or_else(|| find_class(ctx, "PDO")).expect("PDO");
    let dsn = str_arg(ctx, "PDO::connect", args, 0, "dsn")?;
    let colon = dsn.as_bytes().iter().position(|&b| b == b':');
    let class = match colon.and_then(|c| find_driver(&dsn.as_bytes()[..c])) {
        Some(d) if called.lower == b"pdo" => d.class_name().and_then(|n| find_class(ctx, n)).unwrap_or(called),
        _ => called,
    };
    let obj = ctx.new_instance(&class, &mut [], false)?;
    construct(ctx, &obj, "PDO::connect", args)?;
    Ok(Value::object(obj))
}

/// pdo_dbh_attribute_set(): Ok(false) when the attribute was not set.
pub fn attribute_set(ctx: &mut dyn Ctx, func: &str, dbh: &Rc<Dbh>, attr: i64, value: &Value) -> Result<bool, Throw> {
    match attr {
        ATTR_ERRMODE => {
            let l = long_param(ctx, value)?;
            if !(ERRMODE_SILENT..=ERRMODE_EXCEPTION).contains(&l) {
                return Err(ctx.throw("ValueError", "Error mode must be one of the PDO::ERRMODE_* constants".into()));
            }
            dbh.error_mode.set(l);
            return Ok(true);
        }
        ATTR_CASE => {
            let l = long_param(ctx, value)?;
            if ![CASE_NATURAL, CASE_UPPER, CASE_LOWER].contains(&l) {
                return Err(ctx.throw("ValueError", "Case folding mode must be one of the PDO::CASE_* constants".into()));
            }
            dbh.desired_case.set(l);
            return Ok(true);
        }
        ATTR_ORACLE_NULLS => {
            let l = long_param(ctx, value)?;
            dbh.oracle_nulls.set(l);
            return Ok(true);
        }
        ATTR_DEFAULT_FETCH_MODE => {
            let l = match value.as_array() {
                Some(a) => {
                    if let Some(m) = a.get_int(0).and_then(Value::as_int)
                        && (m == FETCH_INTO || m == FETCH_CLASS)
                    {
                        return Err(ctx.throw("ValueError", "PDO::FETCH_INTO and PDO::FETCH_CLASS cannot be set as the default fetch mode".into()));
                    }
                    i64::from(!a.is_empty())
                }
                None => long_param(ctx, value)?,
            };
            if l == FETCH_USE_DEFAULT {
                return Err(ctx.throw("ValueError", "Fetch mode must be a bitmask of PDO::FETCH_* constants".into()));
            }
            dbh.default_fetch.set(l);
            return Ok(true);
        }
        ATTR_STRINGIFY_FETCHES => {
            let b = bool_param(ctx, value)?;
            dbh.stringify.set(b);
            // The driver hears of it too (pdo_mysql decodes rows by it);
            // what it answers does not matter.
            let _ = dbh.conn.borrow_mut().set_attribute(ctx, attr, value)?;
            return Ok(true);
        }
        ATTR_STATEMENT_CLASS => return statement_class_set(ctx, func, dbh, value),
        _ => {}
    }
    dbh.clear_error();
    let r = dbh.conn.borrow_mut().set_attribute(ctx, attr, value)?;
    if r {
        return Ok(true);
    }
    let refused = dbh.conn.borrow_mut().attribute_error();
    if let Some(e) = refused {
        record(dbh, &Where::Dbh, &e);
    }
    // The driver refused: its error state, or "driver does not support".
    // A driver that refuses with no error of its own is silent (PDO_HANDLE_DBH_ERR
    // reports nothing in state 00000).
    handle_error(ctx, func, dbh, Where::Dbh)?;
    Ok(false)
}

fn statement_class_set(ctx: &mut dyn Ctx, _func: &str, dbh: &Rc<Dbh>, value: &Value) -> Result<bool, Throw> {
    let Some(a) = value.as_array() else {
        return Err(ctx.throw("TypeError", format!("PDO::ATTR_STATEMENT_CLASS value must be of type array, {} given", value_name(value))));
    };
    let Some(item) = a.get_int(0).map(Value::deref) else {
        return Err(ctx.throw("ValueError", "PDO::ATTR_STATEMENT_CLASS value must be an array with the format array(classname, constructor_args)".into()));
    };
    let class = match item.as_str() {
        Some(s) => ctx.class_load(s.as_bytes())?,
        None => None,
    };
    let Some(class) = class else {
        return Err(ctx.throw("TypeError", "PDO::ATTR_STATEMENT_CLASS class must be a valid class".into()));
    };
    if !class.is_a(b"pdostatement") {
        return Err(ctx.throw("TypeError", "PDO::ATTR_STATEMENT_CLASS class must be derived from PDOStatement".into()));
    }
    if let Some(m) = class.method(b"__construct")
        && m.visibility == p8_runtime::object::Visibility::Public
    {
        return Err(ctx.throw("TypeError", "User-supplied statement class cannot have a public constructor".into()));
    }
    let ctor_args = match a.get_int(1).map(Value::deref).map(Value::into_array) {
        None => None,
        Some(Ok(c)) => Some(c),
        Some(Err(other)) => {
            return Err(ctx.throw("TypeError", format!("PDO::ATTR_STATEMENT_CLASS constructor_args must be of type ?array, {} given", value_name(&other))));
        }
    };
    *dbh.stmt_class.borrow_mut() = Some(StmtClass { name: class.name.clone(), ctor_args });
    Ok(true)
}

pub fn m_set_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    let attr = arg(args, 0).to_int();
    let value = arg(args, 1);
    dbh.clear_error();
    Ok(Value::bool(attribute_set(ctx, "PDO::setAttribute", &dbh, attr, &value)?))
}

pub fn m_get_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    let attr = arg(args, 0).to_int();
    dbh.clear_error();
    match attr {
        ATTR_PERSISTENT => return Ok(Value::bool(dbh.persistent.get())),
        ATTR_CASE => return Ok(Value::int(dbh.desired_case.get())),
        ATTR_ORACLE_NULLS => return Ok(Value::int(dbh.oracle_nulls.get())),
        ATTR_ERRMODE => return Ok(Value::int(dbh.error_mode.get())),
        ATTR_DRIVER_NAME => return Ok(Value::str(dbh.driver.name())),
        ATTR_STATEMENT_CLASS => {
            let c = dbh.stmt_class.borrow().clone();
            let mut items = vec![Value::str(c.as_ref().map_or_else(|| PhpStr::from("PDOStatement"), |c| c.name.clone()))];
            if let Some(args) = c.and_then(|c| c.ctor_args) {
                items.push(Value::array(args));
            }
            return Ok(Value::array(PhpArray::from_list(items)));
        }
        ATTR_DEFAULT_FETCH_MODE => return Ok(Value::int(dbh.default_fetch.get())),
        ATTR_STRINGIFY_FETCHES => return Ok(Value::bool(dbh.stringify.get())),
        _ => {}
    }
    let r = dbh.conn.borrow_mut().get_attribute_checked(attr);
    match r {
        Ok(Some(v)) => Ok(v),
        Err(e) => {
            record(&dbh, &Where::Dbh, &e);
            handle_error(ctx, "PDO::getAttribute", &dbh, Where::Dbh)?;
            Ok(Value::FALSE)
        }
        Ok(None) => {
            impl_error(ctx, "PDO::getAttribute", &dbh, Where::Dbh, "IM001", Some("driver does not support that attribute"))?;
            Ok(Value::FALSE)
        }
    }
}

// ---- statements ----

/// pdo_stmt_instantiate(): an object of the statement class, not yet prepared.
fn instantiate(ctx: &mut dyn Ctx, class: Option<StmtClass>) -> Result<(Rc<Object>, Option<StmtClass>), Throw> {
    let cls = match &class {
        Some(c) => ctx.class_load(c.name.as_bytes())?.ok_or_else(|| ctx.throw("Error", "Cannot instantiate user-supplied statement class".into()))?,
        None => ctx.class(b"PDOStatement").expect("PDOStatement"),
    };
    if class.as_ref().is_some_and(|c| c.ctor_args.is_some()) && cls.method(b"__construct").is_none() {
        return Err(ctx.throw("Error", "User-supplied statement does not accept constructor arguments".into()));
    }
    let o = ctx.new_object(&cls);
    Ok((o, class))
}

/// pdo_stmt_construct(): the queryString property, then the class's constructor.
fn finish_stmt(ctx: &mut dyn Ctx, obj: &Rc<Object>, query: &PhpStr, class: &Option<StmtClass>) -> Result<(), Throw> {
    if let Some(i) = obj.class.slot(b"querystring", None).or_else(|| obj.class.slot(b"queryString", None)) {
        obj.slots()[i].set(Value::str(query.clone()));
    }
    if obj.class.method(b"__construct").is_some() {
        stmt::call_ctor(ctx, obj, &class.as_ref().and_then(|c| c.ctor_args.clone()))?;
    }
    Ok(())
}

pub fn m_prepare(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDO::prepare";
    let dbh = dbh_of(ctx, this)?;
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", "PDO::prepare() expects at least 1 argument, 0 given".into()));
    }
    let query = str_arg(ctx, func, args, 0, "query")?;
    let options = match arg(args, 1).into_array() {
        Ok(a) => Some(a),
        Err(v) if v.is_null() => None,
        Err(other) => return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($options) must be of type array, {} given", value_name(&other)))),
    };
    if query.is_empty() {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($query) must not be empty")));
    }
    dbh.clear_error();
    let mut class = dbh.stmt_class.borrow().clone();
    if let Some(v) = options.as_ref().and_then(|o| o.get_int(ATTR_STATEMENT_CLASS)).map(Value::deref) {
        let Some(a) = v.as_array() else {
            return Err(ctx.throw("TypeError", format!("PDO::ATTR_STATEMENT_CLASS value must be of type array, {} given", value_name(&v))));
        };
        let Some(item) = a.get_int(0).map(Value::deref) else {
            return Err(ctx.throw("ValueError", "PDO::ATTR_STATEMENT_CLASS value must be an array with the format array(classname, constructor_args)".into()));
        };
        let pce = match item.as_str() {
            Some(s) => ctx.class_load(s.as_bytes())?,
            None => None,
        };
        let Some(pce) = pce else {
            return Err(ctx.throw("TypeError", "PDO::ATTR_STATEMENT_CLASS class must be a valid class".into()));
        };
        if !pce.is_a(b"pdostatement") {
            return Err(ctx.throw("TypeError", "PDO::ATTR_STATEMENT_CLASS class must be derived from PDOStatement".into()));
        }
        if let Some(m) = pce.method(b"__construct")
            && m.visibility == p8_runtime::object::Visibility::Public
        {
            return Err(ctx.throw("TypeError", "User-supplied statement class cannot have a public constructor".into()));
        }
        let ctor_args = match a.get_int(1).map(Value::deref).map(Value::into_array) {
            None => None,
            Some(Ok(c)) => Some(c),
            Some(Err(other)) => return Err(ctx.throw("TypeError", format!("PDO::ATTR_STATEMENT_CLASS constructor_args must be of type ?array, {} given", value_name(&other)))),
        };
        class = Some(StmtClass { name: pce.name.clone(), ctor_args });
    }
    let (obj, class) = instantiate(ctx, class)?;
    // PDO::ATTR_CURSOR: only forward-only cursors.
    if let Some(c) = options.as_ref().and_then(|o| o.get_int(10)).map(Value::deref)
        && c.to_int() != 0
        && !dbh.conn.borrow().scrollable()
    {
        dbh.error.borrow_mut().sqlstate = "HY000".into();
        dbh.error.borrow_mut().info = Some((1, "SQL logic error".into()));
        handle_error(ctx, func, &dbh, Where::Dbh)?;
        return Ok(Value::FALSE);
    }
    let entries: Vec<(i64, Value)> = options
        .as_ref()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| match k {
                    Key::Int(i) => Some((i, v.deref())),
                    Key::Str(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    match stmt::prepare_driver(ctx, func, &dbh, &query, &Options { entries: &entries }) {
        Ok(prepared) => {
            *stmt::data(&obj).s.borrow_mut() = Some(stmt::new_state(&dbh, query.clone(), prepared));
            finish_stmt(ctx, &obj, &query, &class)?;
            Ok(Value::object(obj))
        }
        Err(e) => {
            prepare_failed(ctx, func, &dbh, e)?;
            Ok(Value::FALSE)
        }
    }
}

/// Report a failed prepare on the connection.
fn prepare_failed(ctx: &mut dyn Ctx, func: &str, dbh: &Rc<Dbh>, e: stmt::PrepareError) -> Result<(), Throw> {
    match e {
        stmt::PrepareError::Driver(e) => {
            record(dbh, &Where::Dbh, &e);
            handle_error(ctx, func, dbh, Where::Dbh)
        }
        // pdo_parse_params() raised it on the statement; the preparer then
        // copies the SQLSTATE to the connection, which PDO reports too.
        stmt::PrepareError::Impl(state, msg) => {
            impl_error(ctx, func, dbh, Where::Dbh, state, Some(msg))?;
            handle_error(ctx, func, dbh, Where::Dbh)
        }
    }
}

pub fn m_query(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDO::query";
    let dbh = dbh_of(ctx, this)?;
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", "PDO::query() expects at least 1 argument, 0 given".into()));
    }
    let query = str_arg(ctx, func, args, 0, "query")?;
    let mode = match arg(args, 1) {
        v if v.is_null() => None,
        v if v.is_int() => v.as_int(),
        other => return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($fetchMode) must be of type ?int, {} given", value_name(&other)))),
    };
    if query.is_empty() {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($query) must not be empty")));
    }
    dbh.clear_error();
    let class = dbh.stmt_class.borrow().clone();
    let (obj, class) = instantiate(ctx, class)?;
    match stmt::prepare_driver(ctx, func, &dbh, &query, &Options { entries: &[] }) {
        Ok(prepared) => {
            let mut st = stmt::new_state(&dbh, query.clone(), prepared);
            st.active_query = Some(query.clone());
            *stmt::data(&obj).s.borrow_mut() = Some(st);
        }
        Err(e) => {
            prepare_failed(ctx, func, &dbh, e)?;
            return Ok(Value::FALSE);
        }
    }
    stmt::data(&obj).err.borrow_mut().clear();
    if let Some(mode) = mode {
        let rest: Vec<Value> = args[2..].iter().map(Value::deref).collect();
        stmt::setup_fetch_mode(ctx, &obj, func, mode, 2, "fetchMode", &rest)?;
    }
    stmt::data(&obj).err.borrow_mut().clear();
    let ran = stmt::run(ctx, func, &obj);
    if let Ok(true) = ran {
        finish_stmt(ctx, &obj, &query, &class)?;
        return Ok(Value::object(obj));
    }
    // Something broke: the statement's error is the connection's until the
    // next call (also when it was thrown).
    *dbh.query_error.borrow_mut() = Some(stmt::data(&obj).err.borrow().clone());
    ran?;
    Ok(Value::FALSE)
}

pub fn m_exec(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDO::exec";
    let dbh = dbh_of(ctx, this)?;
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", "PDO::exec() expects exactly 1 argument, 0 given".into()));
    }
    let sql = str_arg(ctx, func, args, 0, "statement")?;
    if sql.is_empty() {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($statement) must not be empty")));
    }
    dbh.clear_error();
    let r = dbh.conn.borrow_mut().exec(sql.as_bytes());
    dispatch_notices(ctx, &dbh)?;
    match r {
        Ok(n) => Ok(Value::int(n)),
        Err(e) => {
            record(&dbh, &Where::Dbh, &e);
            handle_error(ctx, func, &dbh, Where::Dbh)?;
            Ok(Value::FALSE)
        }
    }
}

pub fn m_last_insert_id(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    let name = opt_str(ctx, "PDO::lastInsertId", args, 0, "name")?;
    dbh.clear_error();
    let r = dbh.conn.borrow_mut().last_insert_id(name.as_ref().map(|n| n.as_bytes()));
    match r {
        Ok(id) => Ok(Value::str(PhpStr::new(id))),
        Err(e) => {
            record(&dbh, &Where::Dbh, &e);
            handle_error(ctx, "PDO::lastInsertId", &dbh, Where::Dbh)?;
            Ok(Value::FALSE)
        }
    }
}

pub fn m_quote(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", "PDO::quote() expects at least 1 argument, 0 given".into()));
    }
    let s = str_arg(ctx, "PDO::quote", args, 0, "string")?;
    let ty = if args.len() > 1 { arg(args, 1).to_int() } else { PARAM_STR };
    dbh.clear_error();
    let r = dbh.conn.borrow_mut().quote(s.as_bytes(), ty);
    match r {
        Some(q) => Ok(Value::str(PhpStr::new(q))),
        None => {
            handle_error(ctx, "PDO::quote", &dbh, Where::Dbh)?;
            Ok(Value::FALSE)
        }
    }
}

pub fn m_begin(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    if dbh.in_transaction() {
        return Err(pdo_exception(ctx, "There is already an active transaction".into()));
    }
    let r = dbh.conn.borrow_mut().begin();
    match r {
        Ok(()) => {
            dbh.in_txn.set(true);
            Ok(Value::TRUE)
        }
        Err(e) => {
            record(&dbh, &Where::Dbh, &e);
            handle_error(ctx, "PDO::beginTransaction", &dbh, Where::Dbh)?;
            Ok(Value::FALSE)
        }
    }
}

pub fn m_commit(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    if !dbh.in_transaction() {
        return Err(pdo_exception(ctx, "There is no active transaction".into()));
    }
    let r = dbh.conn.borrow_mut().commit();
    match r {
        Ok(()) => {
            dbh.in_txn.set(false);
            Ok(Value::TRUE)
        }
        Err(e) => {
            record(&dbh, &Where::Dbh, &e);
            handle_error(ctx, "PDO::commit", &dbh, Where::Dbh)?;
            Ok(Value::FALSE)
        }
    }
}

pub fn m_rollback(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    if !dbh.in_transaction() {
        return Err(pdo_exception(ctx, "There is no active transaction".into()));
    }
    let r = dbh.conn.borrow_mut().rollback();
    match r {
        Ok(()) => {
            dbh.in_txn.set(false);
            Ok(Value::TRUE)
        }
        Err(e) => {
            record(&dbh, &Where::Dbh, &e);
            handle_error(ctx, "PDO::rollBack", &dbh, Where::Dbh)?;
            Ok(Value::FALSE)
        }
    }
}

pub fn m_in_transaction(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    Ok(Value::bool(dbh.in_transaction()))
}

pub fn m_error_code(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    if let Some(q) = dbh.query_error.borrow().as_ref() {
        return Ok(Value::str(q.sqlstate.as_str()));
    }
    let e = dbh.error.borrow();
    Ok(if e.sqlstate.is_empty() { Value::NULL } else { Value::str(e.sqlstate.as_str()) })
}

pub fn m_error_info(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    let e = match dbh.query_error.borrow().as_ref() {
        Some(q) => q.clone(),
        None => dbh.error.borrow().clone(),
    };
    let mut items = vec![Value::str(e.sqlstate.as_str())];
    // An error PDO raised itself has no driver part: the driver adds the
    // last one it holds (fetch_err), if it keeps one.
    let info = if e.info.is_some() { e.info.clone() } else { dbh.conn.borrow().error_info() };
    if e.sqlstate != "00000"
        && let Some((code, msg)) = &info
    {
        items.push(Value::int(*code));
        items.push(Value::str(msg.as_str()));
    }
    while items.len() < 3 {
        items.push(Value::NULL);
    }
    Ok(Value::array(PhpArray::from_list(items)))
}

/// A driver-specific method PHP lists on PDO (`sqliteCreateFunction`).
fn driver_method(ctx: &mut dyn Ctx, this: &Rc<Object>, name: &str, args: &mut [Value]) -> R {
    let dbh = dbh_of(ctx, this)?;
    if let Some(f) = dbh.driver.pdo_method(name) {
        return f(ctx, this, args);
    }
    let r = dbh.conn.borrow_mut().call_method(ctx, name, args);
    match r {
        Some(r) => r,
        None => Err(ctx.throw("Error", format!("Call to undefined method {}::{name}()", String::from_utf8_lossy(this.class.shown())))),
    }
}

macro_rules! pgsql_methods {
    ($($f:ident => $name:literal),*) => {
        $(pub fn $f(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
            driver_method(ctx, this, $name, args)
        })*
    };
}

pgsql_methods!(
    m_pgsql_copy_from_array => "pgsqlCopyFromArray",
    m_pgsql_copy_from_file => "pgsqlCopyFromFile",
    m_pgsql_copy_to_array => "pgsqlCopyToArray",
    m_pgsql_copy_to_file => "pgsqlCopyToFile",
    m_pgsql_lob_create => "pgsqlLOBCreate",
    m_pgsql_lob_open => "pgsqlLOBOpen",
    m_pgsql_lob_unlink => "pgsqlLOBUnlink",
    m_pgsql_get_notify => "pgsqlGetNotify",
    m_pgsql_get_pid => "pgsqlGetPid"
);

pub fn m_sqlite_create_function(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    driver_method(ctx, this, "sqliteCreateFunction", args)
}

pub fn m_sqlite_create_aggregate(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    driver_method(ctx, this, "sqliteCreateAggregate", args)
}

pub fn m_sqlite_create_collation(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    driver_method(ctx, this, "sqliteCreateCollation", args)
}

/// The driver's methods for `Pdo\Sqlite`-style subclasses (pub for them).
pub fn call_driver_method(ctx: &mut dyn Ctx, this: &Rc<Object>, name: &str, args: &mut [Value]) -> R {
    driver_method(ctx, this, name, args)
}

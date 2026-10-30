//! PDOStatement: binding, execute, and the fetch machinery.
//!
//! Reference: $PHP_SRC_PATH/ext/pdo/pdo_stmt.c

use crate::driver::{BindKey, DbValue, ExecInfo, Placeholders, Statement};
use crate::parser;
use crate::state::*;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::{Class, Object};
use p8_runtime::stream::{Resource, Stream, Target};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub type R = Result<Value, Throw>;

#[derive(Clone, Debug, PartialEq)]
pub enum PKey {
    Pos(i64),
    Name(String),
}

/// A bound parameter or column (pdo_bound_param_data).
#[derive(Clone)]
pub struct Bound {
    pub key: PKey,
    pub value: Value,
    pub ty: i64,
    pub max_len: i64,
    /// A bound column's index, once known.
    pub colno: Option<usize>,
}

/// What the fetch modes that take arguments hold (pdo_stmt_t.fetch).
#[derive(Clone, Default)]
pub struct Fetch {
    pub column: i64,
    pub class: Option<PhpStr>,
    pub ctor_args: Option<Rc<PhpArray>>,
    pub into: Option<Value>,
    pub func: Option<Value>,
}

pub struct StmtState {
    pub dbh: Rc<Dbh>,
    pub query: PhpStr,
    pub active_query: Option<PhpStr>,
    pub driver: Option<Box<dyn Statement>>,
    pub scan: Option<parser::Scan>,
    pub params: Vec<Bound>,
    pub cols: Vec<Bound>,
    pub default_fetch: i64,
    pub fetch: Fetch,
    pub columns: Option<Vec<PhpStr>>,
    pub column_count: usize,
    pub row: Vec<DbValue>,
    pub executed: bool,
    pub row_count: i64,
    pub in_param_event: bool,
    /// Placeholders::Dollar: how `?`/`:name` became `$N` (None when the
    /// query had none: values go by their own position).
    pub dollar: Option<Option<parser::DollarMap>>,
    /// fetch()'s orientation and offset for the next row (FETCH_ORI_NEXT: 0).
    pub ori: (i64, i64),
    /// An emulated statement whose driver statement lives across executes.
    pub shell: bool,
}

#[derive(Default)]
pub struct StmtData {
    pub err: RefCell<ErrState>,
    pub s: RefCell<Option<StmtState>>,
}

pub fn data(o: &Object) -> &StmtData {
    o.native::<StmtData>().expect("PDOStatement state")
}

/// Run `f` on the statement's state; the Error PHP throws for a
/// PDOStatement that was never prepared.
pub fn with<T>(ctx: &mut dyn Ctx, this: &Rc<Object>, f: impl FnOnce(&mut StmtState) -> T) -> Result<T, Throw> {
    let d = data(this);
    let mut s = d.s.borrow_mut();
    match s.as_mut() {
        Some(st) => Ok(f(st)),
        None => Err(ctx.throw("Error", format!("{} object is uninitialized", String::from_utf8_lossy(this.class.shown())))),
    }
}

pub fn dbh(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<Rc<Dbh>, Throw> {
    with(ctx, this, |st| Rc::clone(&st.dbh))
}

fn clear_err(this: &Object) {
    data(this).err.borrow_mut().clear();
}

/// PDO_HANDLE_STMT_ERR()
fn handle_err(ctx: &mut dyn Ctx, func: &str, this: &Rc<Object>) -> Result<(), Throw> {
    let d = dbh(ctx, this)?;
    handle_error(ctx, func, &d, Where::Stmt(&data(this).err))
}

fn impl_err(ctx: &mut dyn Ctx, func: &str, this: &Rc<Object>, state: &str, supp: Option<&str>) -> Result<(), Throw> {
    let d = dbh(ctx, this)?;
    impl_error(ctx, func, &d, Where::Stmt(&data(this).err), state, supp)
}

pub fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

pub fn value_name(v: &Value) -> String {
    p8_runtime::args::value_name(v).into_owned()
}

fn db_to_value(v: &DbValue) -> Value {
    match v {
        DbValue::Null => Value::Null,
        DbValue::Int(i) => Value::Int(*i),
        DbValue::Float(f) => Value::Float(*f),
        DbValue::Text(b) | DbValue::Blob(b) | DbValue::Lob(b) => Value::Str(PhpStr::new(b.clone())),
        DbValue::Bool(b) => Value::Bool(*b),
    }
}

/// A new memory stream holding `bytes`: a LOB column as PDO::PARAM_LOB gives it.
fn lob_stream(ctx: &mut dyn Ctx, bytes: Vec<u8>) -> Value {
    let stream = Stream::new(Target::Memory(bytes), "php://memory", "rb");
    ctx.new_resource(Resource { kind: "stream", closed: Cell::new(false), stream: RefCell::new(stream) })
}

/// fetch_value(): column `col` of the current row, as PDO hands it to the script.
fn column_value(ctx: &mut dyn Ctx, st: &StmtState, col: usize, ty: Option<i64>) -> Value {
    let dbh = &st.dbh;
    // A large object is a stream, unless the script asks for a string (or
    // another scalar, which is what the string converts to).
    if let Some(DbValue::Lob(bytes)) = st.row.get(col) {
        let ty = if dbh.stringify.get() { Some(PARAM_STR) } else { ty };
        let s = || Value::Str(PhpStr::new(bytes.clone()));
        return match ty {
            Some(PARAM_STR) => s(),
            Some(PARAM_INT) => Value::Int(s().to_int()),
            Some(PARAM_BOOL) => Value::Bool(true),
            Some(PARAM_NULL) => Value::Null,
            _ => lob_stream(ctx, bytes.clone()),
        };
    }
    if ty == Some(PARAM_LOB)
        && let Some(bytes) = st.driver.as_ref().and_then(|d| d.lob_column(col))
    {
        return if dbh.stringify.get() { Value::Str(PhpStr::new(bytes)) } else { lob_stream(ctx, bytes) };
    }
    let mut v = st.row.get(col).map_or(Value::Null, db_to_value);
    if let Value::Str(s) = &v
        && s.is_empty()
        && dbh.oracle_nulls.get() == NULL_EMPTY_STRING
    {
        v = Value::Null;
    }
    let ty = if dbh.stringify.get() { Some(PARAM_STR) } else { ty };
    if let Some(t) = ty
        && !matches!(v, Value::Null)
    {
        v = match t {
            PARAM_INT => Value::Int(v.to_int()),
            PARAM_BOOL => Value::Bool(v.to_bool()),
            PARAM_STR => match v {
                Value::Bool(false) => Value::str("0"),
                other => Value::Str(other.to_php_string().unwrap_or_else(PhpStr::empty)),
            },
            PARAM_NULL => Value::Null,
            PARAM_LOB => match v {
                Value::Str(s) => lob_stream(ctx, s.as_bytes().to_vec()),
                other => other,
            },
            _ => v,
        };
    }
    if matches!(v, Value::Null) && dbh.oracle_nulls.get() == NULL_TO_STRING {
        v = Value::str("");
    }
    v
}

// ---- columns ----

fn fold_case(name: &[u8], case: i64) -> Vec<u8> {
    match case {
        CASE_LOWER => name.to_ascii_lowercase(),
        CASE_UPPER => name.to_ascii_uppercase(),
        _ => name.to_vec(),
    }
}

/// pdo_stmt_describe_columns()
fn describe(st: &mut StmtState) {
    let Some(driver) = st.driver.as_ref() else { return };
    let case = st.dbh.desired_case.get();
    let names: Vec<PhpStr> = (0..st.column_count).map(|i| PhpStr::new(fold_case(&driver.column_name(i), case))).collect();
    for b in st.cols.iter_mut() {
        if let PKey::Name(n) = &b.key
            && let Some(i) = names.iter().position(|c| c.as_bytes() == n.as_bytes())
        {
            b.colno = Some(i);
        }
    }
    st.columns = Some(names);
}

fn column_key(name: &PhpStr) -> Key {
    Key::from_string(name.clone())
}

// ---- running ----

/// One statement's rows are read through here: the next row into `st.row`.
/// Ok(false) at the end; the error is left in the statement.
fn next_row(this: &Object, st: &mut StmtState) -> bool {
    if !st.executed {
        return false;
    }
    let Some(driver) = st.driver.as_mut() else { return false };
    let (ori, offset) = std::mem::take(&mut st.ori);
    match driver.fetch_scroll(ori, offset) {
        Ok(Some(row)) => {
            st.row = row;
            if st.columns.is_none() {
                describe(st);
            }
            true
        }
        Ok(None) => false,
        Err(e) => {
            *data(this).err.borrow_mut() = ErrState { sqlstate: e.sqlstate, info: Some((e.code, e.message)) };
            false
        }
    }
}

/// do_fetch_common(): the next row, and the bound columns updated.
fn fetch_common(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<bool, Throw> {
    let more = with(ctx, this, |st| next_row(this, st))?;
    if !more {
        return Ok(false);
    }
    // Update the variables bound with bindColumn().
    let updates: Vec<(Rc<std::cell::RefCell<Value>>, usize, i64)> = with(ctx, this, |st| {
        st.cols
            .iter()
            .filter_map(|b| match (&b.value, b.colno) {
                (Value::Ref(cell), Some(c)) => Some((Rc::clone(cell), c, b.ty)),
                _ => None,
            })
            .collect()
    })?;
    for (cell, col, ty) in updates {
        let value = {
            let d = data(this).s.borrow();
            let st = d.as_ref().expect("state");
            column_value(ctx, st, col, Some(ty))
        };
        *cell.borrow_mut() = value;
    }
    Ok(true)
}

/// The text of a value as a bound parameter or a class name, running
/// __toString for an object; None (with the exception pending) when it has none.
fn to_text(ctx: &mut dyn Ctx, v: &Value) -> Result<PhpStr, Throw> {
    match v.deref() {
        Value::Array(_) => {
            ctx.warn("Array to string conversion");
            Ok(PhpStr::from("Array"))
        }
        Value::Object(o) => {
            if o.class.method(b"__tostring").is_some() {
                let callable = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(Rc::clone(&o)), Value::str("__toString")])));
                let r = ctx.call(&callable, &mut [])?;
                return Ok(r.to_php_string().unwrap_or_else(PhpStr::empty));
            }
            Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(o.class.shown()))))
        }
        other => Ok(other.to_php_string().unwrap_or_else(PhpStr::empty)),
    }
}

fn bind_value_of(ctx: &mut dyn Ctx, b: &Bound) -> Result<Option<DbValue>, Throw> {
    let v = b.value.deref();
    Ok(Some(match b.ty & 0xFFFF {
        PARAM_STMT => return Ok(None),
        PARAM_NULL => DbValue::Null,
        PARAM_INT | PARAM_BOOL => match v {
            Value::Null => DbValue::Null,
            Value::Bool(b) => DbValue::Bool(b),
            other => DbValue::Int(other.to_int()),
        },
        PARAM_LOB => {
            if let Some(r) = p8_runtime::stream::resource_of(&v) {
                let bytes = p8_runtime::stream::resource(&r).map(|res| res.stream.borrow_mut().read_all()).unwrap_or_default();
                DbValue::Blob(bytes)
            } else if matches!(v, Value::Null) {
                DbValue::Null
            } else {
                DbValue::Blob(to_text(ctx, &v)?.as_bytes().to_vec())
            }
        }
        _ => match v {
            Value::Null => DbValue::Null,
            other => DbValue::Text(to_text(ctx, &other)?.as_bytes().to_vec()),
        },
    }))
}

/// pgsql_stmt_param_hook(EXEC_PRE): what a `$N` parameter is sent as. A
/// boolean is `t`/`f`, a LOB is its bytes (binary), anything else its text.
fn dollar_value_of(ctx: &mut dyn Ctx, d: &Dbh, b: &Bound) -> Result<DbValue, Throw> {
    let v = b.value.deref();
    let ty = b.ty & 0xFFFF;
    if ty == PARAM_LOB
        && let Some(r) = p8_runtime::stream::resource_of(&v)
    {
        let Some(res) = p8_runtime::stream::resource(&r) else { return Ok(DbValue::Blob(Vec::new())) };
        let path = res.stream.borrow().path.clone();
        if let Some(own) = d.conn.borrow_mut().lob_param(&path) {
            return Ok(own);
        }
        let bytes = res.stream.borrow_mut().read_all();
        return Ok(DbValue::Blob(bytes));
    }
    Ok(match v {
        _ if ty == PARAM_NULL => DbValue::Null,
        Value::Null => DbValue::Null,
        Value::Bool(x) => DbValue::Bool(x),
        other if ty == PARAM_LOB => DbValue::Blob(to_text(ctx, &other)?.as_bytes().to_vec()),
        other => DbValue::Text(to_text(ctx, &other)?.as_bytes().to_vec()),
    })
}

/// The `$N` position (from 0) a bound parameter goes to; Err is the
/// message of the HY093 pdo_pgsql raises (EVT_NORMALIZE, EVT_ALLOC).
fn dollar_position(map: &Option<parser::DollarMap>, key: &PKey) -> Result<usize, String> {
    match (key, map) {
        (PKey::Pos(i), None) => Ok(*i as usize),
        (PKey::Pos(i), Some(m)) => {
            if (*i as usize) < m.count {
                Ok(*i as usize)
            } else {
                Err("parameter was not defined".into())
            }
        }
        (PKey::Name(n), Some(m)) => m.names.iter().find(|(x, _)| x == n).map(|(_, p)| *p).ok_or_else(|| n.clone()),
        (PKey::Name(n), None) => Err(n.clone()),
    }
}

/// The statement's executer: bind and run.
pub fn run(ctx: &mut dyn Ctx, func: &str, this: &Rc<Object>) -> Result<bool, Throw> {
    let d = dbh(ctx, this)?;
    let (params, native, query, dollar) = with(ctx, this, |st| (st.params.clone(), st.driver.is_some() && st.scan.is_none(), st.query.clone(), st.dollar.clone()))?;
    if !native {
        // Emulated prepares: quote the values into the query, and prepare that.
        let r = run_emulated(ctx, func, this, &d, &params, &query);
        crate::state::dispatch_notices(ctx, &d)?;
        return r;
    }
    if let Some(map) = dollar {
        let mut binds = Vec::with_capacity(params.len());
        for b in &params {
            let pos = match dollar_position(&map, &b.key) {
                Ok(p) => p,
                Err(msg) => {
                    *data(this).err.borrow_mut() = ErrState { sqlstate: "HY093".into(), info: Some((0, msg)) };
                    handle_err(ctx, func, this)?;
                    return Ok(false);
                }
            };
            binds.push((BindKey::Pos(pos), dollar_value_of(ctx, &d, b)?));
        }
        let mapped = map.is_some();
        let result = with(ctx, this, |st| st.driver.as_mut().expect("prepared").execute_dollar(&binds, mapped))?;
        crate::state::dispatch_notices(ctx, &d)?;
        return finish_run(ctx, func, this, result);
    }
    let mut binds = Vec::with_capacity(params.len());
    for b in &params {
        let Some(v) = bind_value_of(ctx, b)? else {
            // PDO::PARAM_STMT is refused by the driver.
            let state = data(this);
            state.err.borrow_mut().sqlstate = "HY000".into();
            handle_err(ctx, func, this)?;
            return Ok(false);
        };
        let key = match &b.key {
            PKey::Pos(i) => BindKey::Pos(*i as usize),
            PKey::Name(n) => BindKey::Name(n.clone()),
        };
        binds.push((key, v));
    }
    let result = with(ctx, this, |st| st.driver.as_mut().expect("prepared").execute(&binds))?;
    finish_run(ctx, func, this, result)
}

fn finish_run(ctx: &mut dyn Ctx, func: &str, this: &Rc<Object>, result: Result<ExecInfo, crate::driver::DriverError>) -> Result<bool, Throw> {
    match result {
        Ok(info) => {
            with(ctx, this, |st| {
                if st.column_count != info.column_count || st.columns.is_none() {
                    st.column_count = info.column_count;
                    st.columns = None;
                    describe(st);
                }
                if let Some(n) = info.row_count {
                    st.row_count = n;
                }
                st.executed = true;
            })?;
            Ok(true)
        }
        Err(e) => {
            *data(this).err.borrow_mut() = ErrState { sqlstate: e.sqlstate, info: Some((e.code, e.message)) };
            handle_err(ctx, func, this)?;
            Ok(false)
        }
    }
}

/// pdo_parse_params() for a driver without native placeholders: the query
/// with the bound values quoted into it.
fn run_emulated(ctx: &mut dyn Ctx, func: &str, this: &Rc<Object>, d: &Rc<Dbh>, params: &[Bound], query: &PhpStr) -> Result<bool, Throw> {
    let scan = parser::scan(query.as_bytes(), d.conn.borrow().scanner());
    if scan.escapes_in_quote {
        ctx.deprecated(&format!("{func}(): Escaping question marks inside dollar quoted strings is not required anymore and is deprecated"));
    }
    if scan.named && scan.positional {
        impl_err(ctx, func, this, "HY093", Some("mixed named and positional parameters"))?;
        return Ok(false);
    }
    let nbinds = scan.binds().count();
    if nbinds != params.len() && !params.is_empty() && !(scan.named && nbinds > params.len()) {
        impl_err(ctx, func, this, "HY093", Some("number of bound variables does not match number of tokens"))?;
        return Ok(false);
    }
    let sql: PhpStr = if scan.placeholders.is_empty() {
        query.clone()
    } else {
        let mut quoted: Vec<Vec<u8>> = Vec::new();
        for p in &scan.placeholders {
            // With nothing bound, PHP leaves the placeholders as they are.
            if p.escaped || params.is_empty() {
                quoted.push(if p.escaped { b"?".to_vec() } else { query.as_bytes()[p.pos..p.pos + p.len].to_vec() });
                continue;
            }
            let found = if scan.positional {
                params.iter().find(|b| b.key == PKey::Pos(p.bindno as i64))
            } else {
                let name = String::from_utf8_lossy(&query.as_bytes()[p.pos..p.pos + p.len]).into_owned();
                params.iter().find(|b| b.key == PKey::Name(name.clone()))
            };
            let Some(b) = found else {
                impl_err(ctx, func, this, "HY093", Some("parameter was not defined"))?;
                return Ok(false);
            };
            let value = b.value.deref();
            let lob = if b.ty & 0xFFFF == PARAM_LOB { p8_runtime::stream::resource_of(&value) } else { None };
            let ty = if matches!(value, Value::Null) { PARAM_NULL } else { b.ty & 0xFFFF };
            let q = match ty {
                _ if lob.is_some() => {
                    let bytes = lob.as_ref().and_then(|r| p8_runtime::stream::resource(r)).map(|res| res.stream.borrow_mut().read_all()).unwrap_or_default();
                    d.conn.borrow_mut().quote(&bytes, PARAM_LOB)
                }
                PARAM_BOOL => Some(if value.to_bool() { b"1".to_vec() } else { b"0".to_vec() }),
                PARAM_INT => Some(value.to_int().to_string().into_bytes()),
                PARAM_NULL => Some(b"NULL".to_vec()),
                _ => {
                    let text = to_text(ctx, &value)?;
                    d.conn.borrow_mut().quote(text.as_bytes(), ty)
                }
            };
            let Some(q) = q else {
                // The quoter refused (invalid text): the statement takes the
                // connection's error code, and nothing is reported.
                data(this).err.borrow_mut().sqlstate = d.error.borrow().sqlstate.clone();
                return Ok(false);
            };
            quoted.push(q);
        }
        let mut i = 0;
        PhpStr::new(parser::rewrite(query.as_bytes(), &scan.placeholders, |_| {
            i += 1;
            quoted[i - 1].clone()
        }))
    };
    let shell = with(ctx, this, |st| st.shell)?;
    let prepared = if shell {
        with(ctx, this, |st| {
            let mut s = st.driver.take().expect("emulated shell");
            s.set_query(sql.as_bytes());
            Ok(s)
        })?
    } else {
        d.conn.borrow_mut().prepare_emulated(sql.as_bytes())
    };
    match prepared {
        Ok(stmt) => {
            with(ctx, this, |st| {
                st.driver = Some(stmt);
                st.active_query = Some(sql.clone());
            })?;
            let r = with(ctx, this, |st| st.driver.as_mut().expect("prepared").execute(&[]))?;
            finish_run(ctx, func, this, r)
        }
        Err(e) => {
            *data(this).err.borrow_mut() = ErrState { sqlstate: e.sqlstate, info: Some((e.code, e.message)) };
            handle_err(ctx, func, this)?;
            Ok(false)
        }
    }
}

/// Prepare a statement object's driver side: PDO::prepare() and query().
pub struct Prepared {
    driver: Option<Box<dyn Statement>>,
    native: bool,
    dollar: Option<Option<parser::DollarMap>>,
}

/// Why preparing failed: the driver's error, or PDO's own (an SQLSTATE and
/// its message, pdo_raise_impl_error()).
pub enum PrepareError {
    Driver(crate::driver::DriverError),
    Impl(&'static str, &'static str),
}

/// Prepare a statement object's driver side: PDO::prepare() and query().
pub fn prepare_driver(ctx: &mut dyn Ctx, func: &str, d: &Rc<Dbh>, sql: &PhpStr, options: &crate::driver::Options) -> Result<Prepared, PrepareError> {
    let mode = d.conn.borrow_mut().placeholders_for(options);
    match mode {
        Placeholders::None => Ok(Prepared { driver: d.conn.borrow_mut().emulated_shell(options), native: false, dollar: None }),
        Placeholders::Native => {
            let stmt = d.conn.borrow_mut().prepare(&String::from_utf8_lossy(sql.as_bytes())).map_err(PrepareError::Driver)?;
            Ok(Prepared { driver: Some(stmt), native: true, dollar: None })
        }
        Placeholders::Dollar => {
            let scan = parser::scan(sql.as_bytes(), d.conn.borrow().scanner());
            if scan.escapes_in_quote {
                ctx.deprecated(&format!("{func}(): Escaping question marks inside dollar quoted strings is not required anymore and is deprecated"));
            }
            if scan.named && scan.positional {
                return Err(PrepareError::Impl("HY093", "mixed named and positional parameters"));
            }
            let (rewritten, map) = parser::dollar_rewrite(sql.as_bytes(), &scan);
            let stmt = d.conn.borrow_mut().prepare_with(&rewritten, options).map_err(PrepareError::Driver)?;
            Ok(Prepared { driver: Some(stmt), native: true, dollar: Some(map) })
        }
    }
}

pub fn new_state(d: &Rc<Dbh>, query: PhpStr, prepared: Prepared) -> StmtState {
    let Prepared { driver, native, dollar } = prepared;
    StmtState {
        ori: (0, 0),
        shell: !native && driver.is_some(),
        dbh: Rc::clone(d),
        query,
        active_query: None,
        driver,
        dollar,
        scan: if native { None } else { Some(parser::Scan::default()) },
        params: Vec::new(),
        cols: Vec::new(),
        default_fetch: d.default_fetch.get(),
        fetch: Fetch::default(),
        columns: None,
        column_count: 0,
        row: Vec::new(),
        executed: false,
        row_count: 0,
        in_param_event: false,
    }
}

/// pdo_stmt_invalidate_result()
fn invalidate(st: &mut StmtState) {
    if let Some(d) = st.driver.as_mut() {
        d.close_cursor();
    }
    st.executed = false;
}

// ---- binding ----

fn param_key(ctx: &mut dyn Ctx, func: &str, arg: &Value, is_param: bool) -> Result<PKey, Throw> {
    match arg {
        Value::Str(s) => {
            if s.is_empty() {
                return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($param) must not be empty")));
            }
            let _ = is_param;
            Ok(PKey::Name(String::from_utf8_lossy(s.as_bytes()).into_owned()))
        }
        Value::Int(i) if *i > 0 => Ok(PKey::Pos(*i - 1)),
        Value::Int(_) => Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($param) must be greater than or equal to 1"))),
        other => Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($param) must be of type string|int, {} given", value_name(other)))),
    }
}

/// really_register_bound_param()
fn register(ctx: &mut dyn Ctx, func: &str, this: &Rc<Object>, mut b: Bound, is_param: bool) -> Result<bool, Throw> {
    // Convert the variable the way PDO does when it is bound.
    {
        let cell = if let Value::Ref(c) = &b.value { Some(Rc::clone(c)) } else { None };
        let cur = b.value.deref();
        let converted = match (b.ty & 0xFFFF, &cur) {
            (PARAM_STR, v) if b.max_len <= 0 && !matches!(v, Value::Null) => Some(Value::Str(to_text(ctx, v)?)),
            (PARAM_INT, Value::Bool(x)) => Some(Value::Int(i64::from(*x))),
            (PARAM_BOOL, Value::Int(i)) => Some(Value::Bool(*i != 0)),
            _ => None,
        };
        if let Some(c) = converted {
            match cell {
                Some(cell) => *cell.borrow_mut() = c,
                None => b.value = c,
            }
        }
    }
    if is_param {
        if let PKey::Name(n) = &mut b.key
            && !n.starts_with(':')
        {
            n.insert(0, ':');
        }
        // pdo_pgsql's EVT_NORMALIZE for an emulated prepare: a boolean is
        // bound as the string 't' or 'f' (the variable itself is let go).
        if b.ty & 0xFFFF == PARAM_BOOL
            && b.ty & 0x8000_0000 == 0
            && with(ctx, this, |st| st.scan.is_some())?
        {
            let truth = b.value.deref().to_bool();
            let d = dbh(ctx, this)?;
            let text = d.conn.borrow_mut().emulated_bool(truth);
            if let Some(text) = text {
                b.value = Value::Str(PhpStr::new(text));
                b.ty = PARAM_STR;
            }
        }
        // pdo_pgsql's EVT_NORMALIZE/EVT_ALLOC: the name or number must be one
        // the query has.
        if let Some(map) = with(ctx, this, |st| st.dollar.clone())?
            && let Err(msg) = dollar_position(&map, &b.key)
        {
            *data(this).err.borrow_mut() = ErrState { sqlstate: "HY093".into(), info: Some((0, msg)) };
            handle_err(ctx, func, this)?;
            return Ok(false);
        }
    } else if let PKey::Name(n) = &b.key {
        let found = with(ctx, this, |st| st.columns.as_ref().and_then(|c| c.iter().position(|x| x.as_bytes() == n.as_bytes())))?;
        match found {
            Some(i) => b.colno = Some(i),
            None => {
                let has_cols = with(ctx, this, |st| st.columns.is_some())?;
                if has_cols {
                    let msg = format!("Did not find column name '{n}' in the defined columns; it will not be bound");
                    impl_err(ctx, func, this, "HY000", Some(&msg))?;
                    return Ok(false);
                }
            }
        }
    } else if let PKey::Pos(i) = &b.key {
        b.colno = Some(*i as usize);
    }
    with(ctx, this, |st| {
        let list = if is_param { &mut st.params } else { &mut st.cols };
        if let Some(slot) = list.iter_mut().find(|x| x.key == b.key) {
            *slot = b;
        } else {
            list.push(b);
        }
    })?;
    Ok(true)
}

fn bind_common(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str, is_param: bool, by_ref: bool) -> R {
    if args.len() < 2 {
        return Err(ctx.throw("ArgumentCountError", format!("{func}() expects at least 2 arguments, {} given", args.len())));
    }
    let key = param_key(ctx, func, &arg(args, 0), is_param)?;
    // bindParam / bindColumn keep the variable's reference; bindValue the value.
    let value = if by_ref { args[1].clone() } else { arg(args, 1) };
    let ty = match arg(args, 2) {
        Value::Null => PARAM_STR,
        v => v.to_int(),
    };
    let max_len = arg(args, 3).to_int();
    let b = Bound { key, value, ty, max_len, colno: None };
    Ok(Value::Bool(register(ctx, func, this, b, is_param)?))
}

pub fn m_bind_value(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    bind_common(ctx, this, args, "PDOStatement::bindValue", true, false)
}

pub fn m_bind_param(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    bind_common(ctx, this, args, "PDOStatement::bindParam", true, true)
}

pub fn m_bind_column(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    bind_common(ctx, this, args, "PDOStatement::bindColumn", false, true)
}

pub fn m_execute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDOStatement::execute";
    let input = match arg(args, 0) {
        Value::Null => None,
        Value::Array(a) => Some(a),
        other => {
            return Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($params) must be of type ?array, {} given", value_name(&other))));
        }
    };
    if with(ctx, this, |st| st.in_param_event)? {
        return Err(ctx.throw("Error", "Cannot modify a PDOStatement while parameter hooks are running".into()));
    }
    with(ctx, this, |st| {
        if st.executed {
            invalidate(st);
        }
    })?;
    clear_err(this);
    if let Some(input) = input {
        with(ctx, this, |st| st.params.clear())?;
        for (k, v) in input.iter() {
            let key = match k {
                Key::Int(i) => PKey::Pos(i),
                Key::Str(s) => PKey::Name(String::from_utf8_lossy(s.as_bytes()).into_owned()),
            };
            let b = Bound { key, value: v.deref(), ty: PARAM_STR, max_len: 0, colno: None };
            if !register(ctx, func, this, b, true)? {
                return Ok(Value::Bool(false));
            }
        }
    }
    Ok(Value::Bool(run(ctx, func, this)?))
}

// ---- fetching ----

fn verify_mode(ctx: &mut dyn Ctx, this: &Rc<Object>, func: &str, mode: i64, arg_num: u32, arg_name: &str, fetch_all: bool) -> Result<(), Throw> {
    let mut flags = mode & FETCH_FLAGS;
    let mut m = mode & !FETCH_FLAGS;
    let verr = |ctx: &mut dyn Ctx, msg: &str| ctx.throw("ValueError", format!("{func}(): Argument #{arg_num} (${arg_name}) {msg}"));
    if !(0..=FETCH_MAX).contains(&m) {
        return Err(verr(ctx, "must be a bitmask of PDO::FETCH_* constants"));
    }
    if m == FETCH_USE_DEFAULT {
        let d = with(ctx, this, |st| st.default_fetch)?;
        flags = d & FETCH_FLAGS;
        m = d & !FETCH_FLAGS;
    }
    match m {
        FETCH_FUNC => {
            if !fetch_all {
                return Err(ctx.throw("ValueError", "Can only use PDO::FETCH_FUNC in PDOStatement::fetchAll()".into()));
            }
            Ok(())
        }
        FETCH_CLASS => {
            if flags & FETCH_SERIALIZE != 0 {
                ctx.deprecated(&format!("{func}(): The PDO::FETCH_SERIALIZE mode is deprecated"));
            }
            Ok(())
        }
        _ => {
            if m == FETCH_LAZY && fetch_all {
                return Err(verr(ctx, "cannot be PDO::FETCH_LAZY in PDOStatement::fetchAll()"));
            }
            if flags & FETCH_SERIALIZE == FETCH_SERIALIZE {
                return Err(verr(ctx, "must use PDO::FETCH_SERIALIZE with PDO::FETCH_CLASS"));
            }
            if flags & FETCH_CLASSTYPE == FETCH_CLASSTYPE {
                return Err(verr(ctx, "must use PDO::FETCH_CLASSTYPE with PDO::FETCH_CLASS"));
            }
            if m >= FETCH_MAX {
                return Err(verr(ctx, "must be a bitmask of PDO::FETCH_* constants"));
            }
            if flags & FETCH_SERIALIZE != 0 {
                ctx.deprecated(&format!("{func}(): The PDO::FETCH_SERIALIZE mode is deprecated"));
            }
            Ok(())
        }
    }
}

/// zend_update_property_ex(): a declared property, or a dynamic one (which
/// a class that does not allow them is told off for).
fn set_prop(ctx: &mut dyn Ctx, obj: &Rc<Object>, class: &Rc<Class>, name: &[u8], value: Value, scoped: bool) -> Result<(), Throw> {
    let scope = if scoped { Some(class.lower.as_slice()) } else { None };
    match class.slot(name, scope) {
        Some(i) => {
            let vis = class.properties[i].visibility;
            if !scoped && vis != p8_runtime::object::Visibility::Public {
                return Err(ctx.throw("Error", format!("Cannot access {} property {}::${}", vis.word(), String::from_utf8_lossy(class.shown()), String::from_utf8_lossy(name))));
            }
            obj.slots.borrow_mut()[i] = value
        }
        None => {
            let exists = obj.dynamic.borrow().contains_key(&str_key(name));
            if !exists && class.flags & p8_runtime::object::CLASS_ALLOW_DYNAMIC == 0 && class.lower != b"stdclass" {
                ctx.deprecated(&format!("Creation of dynamic property {}::${} is deprecated", String::from_utf8_lossy(class.shown()), String::from_utf8_lossy(name)));
            }
            obj.dynamic.borrow_mut().insert(str_key(name), value);
        }
    }
    Ok(())
}

fn lookup_class(ctx: &mut dyn Ctx, name: &PhpStr) -> Result<Option<Rc<Class>>, Throw> {
    ctx.class_load(name.as_bytes())
}

/// The class's constructor, whatever its visibility (zend_call_function
/// on the constructor, as PDO does).
pub fn call_ctor(ctx: &mut dyn Ctx, obj: &Rc<Object>, args: &Option<Rc<PhpArray>>) -> Result<(), Throw> {
    // Each argument is a reference, so a by-reference parameter takes it.
    let mut a: Vec<Value> = args.as_ref().map(|a| a.iter().map(|(_, v)| Value::Ref(Rc::new(RefCell::new(v.deref())))).collect()).unwrap_or_default();
    let callee = p8_runtime::reflect::Callee::Method(Rc::clone(&obj.class), PhpStr::from("__construct"));
    ctx.invoke(&callee, Some(Rc::clone(obj)), &mut a)?;
    Ok(())
}

/// do_fetch(): one row in `how` (Value::Null result is `false`: None).
/// With `all`, the row is added to the result of fetchAll() as the group
/// and key modes ask.
fn do_fetch(ctx: &mut dyn Ctx, func: &str, this: &Rc<Object>, how: i64, all: Option<&mut PhpArray>) -> Result<Option<Value>, Throw> {
    let (mut how, st_default, fetch_cfg) = with(ctx, this, |st| (how, st.default_fetch, st.fetch.clone()))?;
    if how == FETCH_USE_DEFAULT {
        how = st_default;
    }
    let mut flags = how & FETCH_FLAGS;
    let mut how = how & !FETCH_FLAGS;
    if !fetch_common(ctx, this)? {
        return Ok(None);
    }
    if how == FETCH_BOUND {
        return Ok(Some(Value::Bool(true)));
    }
    let column_count = with(ctx, this, |st| st.column_count)?;
    let colno = if flags & FETCH_GROUP != 0 && fetch_cfg.column == -1 { 1 } else { fetch_cfg.column };
    if how == FETCH_LAZY {
        return Ok(Some(lazy_row(ctx, this)?));
    }
    let names: Vec<PhpStr> = with(ctx, this, |st| st.columns.clone().unwrap_or_default())?;
    let has_all = all.is_some();
    let mut i = 0usize;
    let mut result = Value::Bool(false);
    let mut obj_class: Option<Rc<Class>> = None;
    let mut cfg = fetch_cfg.clone();
    let mut array = PhpArray::new();
    let mut ret_obj: Option<Rc<Object>> = None;
    let fetch_col = |ctx: &mut dyn Ctx, this: &Rc<Object>, col: usize| -> Result<Value, Throw> {
        let d = data(this).s.borrow();
        let st = d.as_ref().expect("state");
        let v = column_value(ctx, st, col, None);
        Ok(v)
    };
    match how {
        FETCH_USE_DEFAULT | FETCH_ASSOC | FETCH_BOTH | FETCH_NUM | FETCH_NAMED => {}
        FETCH_KEY_PAIR => {
            if column_count != 2 {
                impl_err(ctx, func, this, "HY000", Some("PDO::FETCH_KEY_PAIR fetch mode requires the result set to contain exactly 2 columns."))?;
                return Ok(None);
            }
        }
        FETCH_COLUMN => {
            if colno < 0 {
                return Err(ctx.throw("ValueError", "Column index must be greater than or equal to 0".into()));
            }
            if colno as usize >= column_count {
                return Err(ctx.throw("ValueError", "Invalid column index".into()));
            }
            let col = if flags == FETCH_GROUP && fetch_cfg.column == -1 {
                1
            } else if flags == FETCH_GROUP && colno != 0 {
                0
            } else {
                colno as usize
            };
            result = fetch_col(ctx, this, col)?;
            if !has_all {
                return Ok(Some(result));
            }
        }
        FETCH_OBJ => {
            let class = ctx.class(b"stdClass").expect("stdClass");
            ret_obj = Some(ctx.new_object(&class));
            obj_class = Some(class);
        }
        FETCH_CLASS => {
            if flags & FETCH_CLASSTYPE != 0 {
                let v = fetch_col(ctx, this, i)?;
                i += 1;
                if !matches!(v, Value::Null) {
                    let name = to_text(ctx, &v)?;
                    cfg.class = Some(name);
                }
            }
            let class = match &cfg.class {
                Some(name) => match lookup_class(ctx, name)? {
                    Some(c) => c,
                    None => ctx.class(b"stdClass").expect("stdClass"),
                },
                None => {
                    impl_err(ctx, func, this, "HY000", Some("No fetch class specified"))?;
                    return Ok(None);
                }
            };
            if flags & FETCH_SERIALIZE == 0 {
                let o = ctx.new_instance(&class, &mut [], false)?;
                if class.method(b"__construct").is_none() && cfg.ctor_args.is_some() {
                    return Err(ctx.throw("Error", "User-supplied statement does not accept constructor arguments".into()));
                }
                if class.method(b"__construct").is_some() && flags & FETCH_PROPS_LATE != 0 {
                    call_ctor(ctx, &o, &cfg.ctor_args)?;
                }
                ret_obj = Some(o);
            }
            obj_class = Some(class);
        }
        FETCH_INTO => {
            let Some(into) = cfg.into.clone() else {
                impl_err(ctx, func, this, "HY000", Some("No fetch-into object specified."))?;
                return Ok(None);
            };
            if let Value::Object(o) = &into {
                if o.class.lower == b"stdclass" {
                    how = FETCH_OBJ;
                }
                obj_class = Some(Rc::clone(&o.class));
                ret_obj = Some(Rc::clone(o));
            }
        }
        FETCH_FUNC if cfg.func.is_none() => {
            impl_err(ctx, func, this, "HY000", Some("No fetch function specified"))?;
            return Ok(None);
        }
        _ => {}
    }
    let mut grp = Value::Null;
    if has_all && how != FETCH_KEY_PAIR {
        let v = if flags == FETCH_GROUP && how == FETCH_COLUMN && fetch_cfg.column > 0 { fetch_col(ctx, this, colno as usize)? } else { fetch_col(ctx, this, i)? };
        grp = Value::Str(to_text(ctx, &v)?);
        if how == FETCH_COLUMN {
            i = column_count;
        } else {
            i += 1;
        }
    }
    let mut func_args: Vec<Value> = Vec::new();
    let mut idx = 0usize;
    while i < column_count {
        let val = fetch_col(ctx, this, i)?;
        let name = names.get(i).cloned().unwrap_or_default();
        match how {
            FETCH_ASSOC => {
                array.insert(column_key(&name), val);
            }
            FETCH_KEY_PAIR => {
                let tmp = fetch_col(ctx, this, i + 1)?;
                let k = match val {
                    Value::Int(n) => Key::Int(n),
                    other => Key::from_string(to_text(ctx, &other)?),
                };
                match all {
                    Some(all) => {
                        all.insert(k, tmp);
                        return Ok(Some(Value::Bool(true)));
                    }
                    None => {
                        array.insert(k, tmp);
                        return Ok(Some(Value::Array(Rc::new(array))));
                    }
                }
            }
            FETCH_USE_DEFAULT | FETCH_BOTH => {
                array.insert(column_key(&name), val.clone());
                if !array.contains_key(&Key::Int(i as i64)) {
                    array.insert(Key::Int(i as i64), val);
                }
            }
            FETCH_NAMED => {
                let k = column_key(&name);
                match array.get(&k).cloned() {
                    Some(Value::Array(existing)) if existing.is_list() && named_is_group(&array, &k) => {
                        let mut a = (*existing).clone();
                        let _ = a.push(val);
                        array.insert(k, Value::Array(Rc::new(a)));
                    }
                    Some(cur) => {
                        let mut a = PhpArray::new();
                        let _ = a.push(cur);
                        let _ = a.push(val);
                        array.insert(k, Value::Array(Rc::new(a)));
                        mark_named(&mut array, &name);
                    }
                    None => {
                        array.insert(k, val);
                    }
                }
            }
            FETCH_NUM => {
                let _ = array.push(val);
            }
            FETCH_OBJ | FETCH_INTO => {
                if let (Some(o), Some(c)) = (&ret_obj, &obj_class) {
                    set_prop(ctx, o, c, name.as_bytes(), val, false)?;
                }
            }
            FETCH_CLASS => {
                if flags & FETCH_SERIALIZE == 0 || idx != 0 {
                    if let (Some(o), Some(c)) = (&ret_obj, &obj_class) {
                        set_prop(ctx, o, c, name.as_bytes(), val, true)?;
                    }
                } else {
                    // PDO::FETCH_SERIALIZE: the first column is the object's
                    // serialized form, handed to Serializable::unserialize().
                    let class = obj_class.clone().expect("class");
                    if class.method(b"unserialize").is_none() || !class.is_a(b"serializable") {
                        impl_err(ctx, func, this, "HY000", Some("cannot unserialize class"))?;
                        return Ok(None);
                    }
                    let o = ctx.new_instance(&class, &mut [], false)?;
                    let text = match val {
                        Value::Str(s) => Value::Str(s),
                        _ => Value::str(""),
                    };
                    let callee = p8_runtime::reflect::Callee::Method(Rc::clone(&class), PhpStr::from("unserialize"));
                    ctx.invoke(&callee, Some(Rc::clone(&o)), &mut [text])?;
                    ret_obj = Some(o);
                }
            }
            FETCH_FUNC => func_args.push(val),
            _ => {
                return Err(ctx.throw("ValueError", "Fetch mode must be a bitmask of PDO::FETCH_* constants".into()));
            }
        }
        i += 1;
        idx += 1;
    }
    match how {
        FETCH_USE_DEFAULT | FETCH_ASSOC | FETCH_BOTH | FETCH_NUM | FETCH_NAMED => {
            result = Value::Array(Rc::new(array));
        }
        FETCH_OBJ | FETCH_INTO => {
            result = Value::Object(ret_obj.clone().expect("object"));
        }
        FETCH_CLASS => {
            let o = ret_obj.clone().expect("object");
            let class = obj_class.clone().expect("class");
            if class.method(b"__construct").is_some() && flags & (FETCH_PROPS_LATE | FETCH_SERIALIZE) == 0 {
                call_ctor(ctx, &o, &cfg.ctor_args)?;
            }
            result = Value::Object(o);
        }
        FETCH_FUNC => {
            let f = cfg.func.clone().expect("function");
            result = ctx.call(&f, &mut func_args)?;
        }
        _ => {}
    }
    flags &= !0;
    if let Some(all) = all {
        let key = match &grp {
            Value::Str(s) => Key::from_string(s.clone()),
            _ => Key::Int(0),
        };
        if (flags & FETCH_UNIQUE) == FETCH_UNIQUE {
            all.insert(key, result);
        } else {
            let mut g = match all.get(&key) {
                Some(Value::Array(a)) => (**a).clone(),
                _ => PhpArray::new(),
            };
            let _ = g.push(result);
            all.insert(key, Value::Array(Rc::new(g)));
        }
        return Ok(Some(Value::Bool(true)));
    }
    Ok(Some(result))
}

// FETCH_NAMED: a name seen twice holds an array of its values; the values
// of one name that are arrays themselves must not be mistaken for it.
fn named_is_group(array: &PhpArray, key: &Key) -> bool {
    array.get(&named_marker(key)).is_some()
}

fn named_marker(key: &Key) -> Key {
    let mut b = vec![0u8, b'n'];
    match key {
        Key::Int(i) => b.extend_from_slice(i.to_string().as_bytes()),
        Key::Str(s) => b.extend_from_slice(s.as_bytes()),
    }
    Key::Str(PhpStr::new(b))
}

fn mark_named(array: &mut PhpArray, name: &PhpStr) {
    array.insert(named_marker(&column_key(name)), Value::Bool(true));
}

/// The PDORow of the current row (PDO::FETCH_LAZY).
fn lazy_row(ctx: &mut dyn Ctx, this: &Rc<Object>) -> R {
    let class = ctx.class(b"PDORow").expect("PDORow");
    let o = ctx.new_object(&class);
    let (names, query) = with(ctx, this, |st| (st.columns.clone().unwrap_or_default(), st.query.clone()))?;
    let _ = set_prop(ctx, &o, &class, b"queryString", Value::Str(query), true);
    for (i, n) in names.iter().enumerate() {
        let v = {
            let d = data(this).s.borrow();
            column_value(ctx, d.as_ref().expect("state"), i, None)
        };
        if n.as_bytes() != b"queryString" {
            o.dynamic.borrow_mut().insert(column_key(n), v);
        }
    }
    Ok(Value::Object(o))
}

/// Remove the markers FETCH_NAMED left in a row.
fn strip_named(v: Value) -> Value {
    match v {
        Value::Array(a) => {
            let mut out = PhpArray::new();
            for (k, val) in a.iter() {
                if let Key::Str(s) = &k
                    && s.as_bytes().starts_with(&[0, b'n'])
                {
                    continue;
                }
                out.insert(k, val.clone());
            }
            Value::Array(Rc::new(out))
        }
        other => other,
    }
}

pub fn m_fetch(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDOStatement::fetch";
    let how = if args.is_empty() { FETCH_USE_DEFAULT } else { arg(args, 0).to_int() };
    let ori = arg(args, 1).to_int();
    let offset = arg(args, 2).to_int();
    clear_err(this);
    with(ctx, this, |st| st.ori = (ori, offset))?;
    verify_mode(ctx, this, func, how, 1, "mode", false)?;
    match do_fetch(ctx, func, this, how, None)? {
        Some(v) => Ok(strip_named(v)),
        None => {
            handle_err(ctx, func, this)?;
            Ok(Value::Bool(false))
        }
    }
}

pub fn m_fetch_object(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDOStatement::fetchObject";
    let class = match arg(args, 0) {
        Value::Null => None,
        Value::Str(s) => match lookup_class(ctx, &s)? {
            Some(c) => Some(c),
            None => {
                return Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($class) must be a valid class name or null, {} given", String::from_utf8_lossy(s.as_bytes()))));
            }
        },
        other => return Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($class) must be of type ?string, {} given", value_name(&other)))),
    };
    let ctor = match arg(args, 1) {
        Value::Array(a) if !a.is_empty() => Some(a),
        _ => None,
    };
    clear_err(this);
    let old = with(ctx, this, |st| st.fetch.clone())?;
    with(ctx, this, |st| {
        st.fetch.ctor_args = ctor;
        st.fetch.class = Some(class.as_ref().map_or_else(|| PhpStr::from("stdClass"), |c| c.name.clone()));
    })?;
    let r = do_fetch(ctx, func, this, FETCH_CLASS, None);
    with(ctx, this, |st| st.fetch = old)?;
    match r? {
        Some(v) => Ok(v),
        None => {
            handle_err(ctx, func, this)?;
            Ok(Value::Bool(false))
        }
    }
}

pub fn m_fetch_column(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDOStatement::fetchColumn";
    let col = arg(args, 0).to_int();
    clear_err(this);
    if !fetch_common(ctx, this)? {
        handle_err(ctx, func, this)?;
        return Ok(Value::Bool(false));
    }
    if col < 0 {
        return Err(ctx.throw("ValueError", "Column index must be greater than or equal to 0".into()));
    }
    let count = with(ctx, this, |st| st.column_count)?;
    if col as usize >= count {
        return Err(ctx.throw("ValueError", "Invalid column index".into()));
    }
    let d = data(this).s.borrow();
    Ok(column_value(ctx, d.as_ref().expect("state"), col as usize, None))
}

pub fn m_fetch_all(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDOStatement::fetchAll";
    let mut how = if args.is_empty() || matches!(arg(args, 0), Value::Null) { FETCH_USE_DEFAULT } else { arg(args, 0).to_int() };
    let nargs = args.len();
    verify_mode(ctx, this, func, how, 1, "mode", true)?;
    let old = with(ctx, this, |st| st.fetch.clone())?;
    let restore = |ctx: &mut dyn Ctx, this: &Rc<Object>, old: Fetch| {
        let _ = with(ctx, this, |st| st.fetch = old);
    };
    let arg2 = if nargs > 1 { Some(arg(args, 1)) } else { None };
    let ctor = match arg(args, 2) {
        Value::Array(a) if !a.is_empty() => Some(a),
        _ => None,
    };
    let base = how & !FETCH_FLAGS;
    let bad = match base {
        FETCH_CLASS => {
            let name = match &arg2 {
                Some(Value::Str(s)) => s.clone(),
                Some(other) => {
                    return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($args) must be of type string, {} given", value_name(other))));
                }
                None => PhpStr::from("stdClass"),
            };
            if lookup_class(ctx, &name)?.is_none() {
                return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($args) must be a valid class")));
            }
            with(ctx, this, |st| {
                st.fetch.class = Some(name);
                st.fetch.ctor_args = ctor;
            })?;
            None
        }
        FETCH_FUNC => {
            if nargs != 2 {
                return Err(ctx.throw("ArgumentCountError", format!("{func}() expects exactly 2 argument for PDO::FETCH_FUNC, {nargs} given")));
            }
            match arg2 {
                Some(Value::Null) | None => return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 must be a callable, null given"))),
                Some(f) => {
                    if !ctx.is_callable(&f) {
                        let why = p8_runtime::args::callable_error(ctx, &f);
                        return Err(ctx.throw("TypeError", why));
                    }
                    with(ctx, this, |st| st.fetch.func = Some(f))?;
                }
            }
            None
        }
        FETCH_COLUMN => {
            if nargs > 2 {
                return Err(ctx.throw("ArgumentCountError", format!("{func}() expects at most 2 argument for the fetch mode provided, {nargs} given")));
            }
            match arg2 {
                Some(Value::Int(n)) => {
                    if n < 0 {
                        return Err(ctx.throw("ValueError", format!("{func}(): Argument #2 ($args) must be greater than or equal to 0")));
                    }
                    with(ctx, this, |st| st.fetch.column = n)?;
                }
                Some(other) => return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($args) must be of type int, {} given", value_name(&other)))),
                None => {
                    let c = if how & FETCH_GROUP != 0 { -1 } else { 0 };
                    with(ctx, this, |st| st.fetch.column = c)?;
                }
            }
            None
        }
        _ => {
            if nargs > 1 {
                Some(format!("{func}() expects exactly 1 argument for the fetch mode provided, {nargs} given"))
            } else {
                None
            }
        }
    };
    if let Some(msg) = bad {
        restore(ctx, this, old);
        return Err(ctx.throw("ArgumentCountError", msg));
    }
    let default = with(ctx, this, |st| st.default_fetch)?;
    if (how & !FETCH_FLAGS) == FETCH_USE_DEFAULT {
        how |= default;
    }
    clear_err(this);
    let grouped = how & FETCH_GROUP != 0 || (how & !FETCH_FLAGS) == FETCH_KEY_PAIR;
    let mut all = PhpArray::new();
    let mut list = PhpArray::new();
    let mut failed = false;
    let mut error = None;
    loop {
        let r = if grouped { do_fetch(ctx, func, this, how, Some(&mut all)) } else { do_fetch(ctx, func, this, how, None) };
        match r {
            Ok(Some(v)) => {
                if !grouped {
                    let _ = list.push(strip_named(v));
                }
            }
            Ok(None) => {
                let empty = if grouped { all.is_empty() } else { list.is_empty() };
                failed = empty && with(ctx, this, |st| st.executed)? && data(this).err.borrow().sqlstate != "00000";
                break;
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    restore(ctx, this, old);
    if let Some(e) = error {
        return Err(e);
    }
    if failed {
        handle_err(ctx, func, this)?;
    }
    Ok(Value::Array(Rc::new(if grouped { all } else { list })))
}

pub fn m_row_count(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Int(with(ctx, this, |st| st.row_count)?))
}

pub fn m_column_count(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Int(with(ctx, this, |st| st.column_count as i64)?))
}

pub fn m_error_code(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    with(ctx, this, |_| ())?;
    let e = data(this).err.borrow();
    Ok(if e.sqlstate.is_empty() { Value::Null } else { Value::str(e.sqlstate.as_str()) })
}

pub fn m_error_info(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    with(ctx, this, |_| ())?;
    let e = data(this).err.borrow();
    let mut items = vec![Value::str(e.sqlstate.as_str())];
    if e.sqlstate != "00000"
        && let Some((code, msg)) = &e.info
    {
        items.push(Value::Int(*code));
        items.push(Value::str(msg.as_str()));
    }
    while items.len() < 3 {
        items.push(Value::Null);
    }
    Ok(Value::Array(Rc::new(PhpArray::from_list(items))))
}

pub fn m_close_cursor(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    if with(ctx, this, |st| st.in_param_event)? {
        return Err(ctx.throw("Error", "Cannot modify a PDOStatement while parameter hooks are running".into()));
    }
    clear_err(this);
    with(ctx, this, invalidate)?;
    Ok(Value::Bool(true))
}

pub fn m_next_rowset(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    impl_err(ctx, "PDOStatement::nextRowset", this, "IM001", Some("driver does not support multiple rowsets"))?;
    Ok(Value::Bool(false))
}

pub fn m_set_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    with(ctx, this, |_| ())?;
    let _ = args;
    impl_err(ctx, "PDOStatement::setAttribute", this, "IM001", Some("This driver doesn't support setting attributes"))?;
    Ok(Value::Bool(false))
}

pub fn m_get_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let attr = arg(args, 0).to_int();
    clear_err(this);
    let r = with(ctx, this, |st| st.driver.as_mut().and_then(|d| d.get_attribute(attr)))?;
    if let Some(v) = r {
        return Ok(v);
    }
    if attr == ATTR_EMULATE_PREPARES {
        let emulated = with(ctx, this, |st| st.scan.is_some())?;
        return Ok(Value::Bool(emulated));
    }
    impl_err(ctx, "PDOStatement::getAttribute", this, "IM001", Some("driver doesn't support getting that attribute"))?;
    Ok(Value::Bool(false))
}

pub fn m_get_column_meta(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDOStatement::getColumnMeta";
    let col = arg(args, 0).to_int();
    if col < 0 {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($column) must be greater than or equal to 0")));
    }
    clear_err(this);
    let (ok, meta, name, count) = with(ctx, this, |st| {
        let executed = st.executed;
        let count = st.column_count;
        let name = st.columns.as_ref().and_then(|c| c.get(col as usize).cloned());
        let meta = if executed && (col as usize) < count { st.driver.as_mut().and_then(|d| d.column_meta(col as usize)) } else { None };
        (executed, meta, name, count)
    })?;
    let _ = count;
    let Some(meta) = meta else {
        if !ok {
            handle_err(ctx, func, this)?;
        } else {
            impl_err(ctx, func, this, "07009", Some("invalid column index"))?;
        }
        return Ok(Value::Bool(false));
    };
    let Some(name) = name else {
        impl_err(ctx, func, this, "07009", Some("invalid column index"))?;
        return Ok(Value::Bool(false));
    };
    let mut a = PhpArray::new();
    if let Some(entries) = &meta.ordered {
        for (k, v) in entries {
            a.insert(str_key(k.as_bytes()), v.clone());
        }
    } else {
        if let Some(t) = &meta.native_type {
            a.insert(str_key(b"native_type"), Value::str(t.as_str()));
        }
        a.insert(str_key(b"pdo_type"), Value::Int(meta.pdo_type));
        for (k, v) in &meta.extra {
            a.insert(str_key(k.as_bytes()), Value::str(v.as_str()));
        }
        let flags: Vec<Value> = meta.flags.iter().map(|f| Value::str(f.as_str())).collect();
        a.insert(str_key(b"flags"), Value::Array(Rc::new(PhpArray::from_list(flags))));
    }
    a.insert(str_key(b"name"), Value::Str(name));
    a.insert(str_key(b"len"), Value::Int(meta.len.unwrap_or(-1)));
    a.insert(str_key(b"precision"), Value::Int(meta.precision.unwrap_or(0)));
    Ok(Value::Array(Rc::new(a)))
}

/// pdo_stmt_setup_fetch_mode(): `args` are the arguments after the mode.
pub fn setup_fetch_mode(ctx: &mut dyn Ctx, this: &Rc<Object>, func: &str, mode: i64, mode_arg: u32, mode_name: &str, args: &[Value]) -> Result<(), Throw> {
    let flags = mode & FETCH_FLAGS;
    let fetch_type = mode & !FETCH_FLAGS;
    let n = args.len() as u32;
    let total = mode_arg + n;
    let arg1 = mode_arg + 1;
    verify_mode(ctx, this, func, mode, mode_arg, mode_name, false)?;
    let count_err = |ctx: &mut dyn Ctx, kind: &str, expect: u32| ctx.throw("ArgumentCountError", format!("{func}() expects {kind} {expect} arguments for the fetch mode provided, {total} given"));
    let mut fetch_column = 0;
    let mut fetch_class: Option<PhpStr> = None;
    let mut fetch_ctor: Option<Rc<PhpArray>> = None;
    let mut fetch_into: Option<Value> = None;
    match fetch_type {
        FETCH_USE_DEFAULT | FETCH_LAZY | FETCH_ASSOC | FETCH_NUM | FETCH_BOTH | FETCH_OBJ | FETCH_BOUND | FETCH_NAMED | FETCH_KEY_PAIR => {
            if n != 0 {
                return Err(count_err(ctx, "exactly", mode_arg));
            }
        }
        FETCH_COLUMN => {
            if n != 1 {
                return Err(count_err(ctx, "exactly", arg1));
            }
            match arg(args, 0) {
                Value::Int(c) => {
                    if c < 0 {
                        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{arg1} ($args) must be greater than or equal to 0")));
                    }
                    fetch_column = c;
                }
                other => return Err(ctx.throw("TypeError", format!("{func}(): Argument #{arg1} ($args) must be of type int, {} given", value_name(&other)))),
            }
        }
        FETCH_CLASS => {
            if flags & FETCH_CLASSTYPE == FETCH_CLASSTYPE {
                if n != 0 {
                    return Err(count_err(ctx, "exactly", mode_arg));
                }
            } else {
                if n == 0 {
                    return Err(count_err(ctx, "at least", arg1));
                }
                if n > 2 {
                    return Err(count_err(ctx, "at most", mode_arg + 2));
                }
                let Value::Str(name) = arg(args, 0) else {
                    return Err(ctx.throw("TypeError", format!("{func}(): Argument #{arg1} ($args) must be of type string, {} given", value_name(&arg(args, 0)))));
                };
                let Some(class) = lookup_class(ctx, &name)? else {
                    return Err(ctx.throw("TypeError", format!("{func}(): Argument #{arg1} ($args) must be a valid class")));
                };
                if n == 2 {
                    match arg(args, 1) {
                        Value::Null => {}
                        Value::Array(a) => {
                            if !a.is_empty() {
                                if class.method(b"__construct").is_none() {
                                    return Err(ctx.throw("Error", "User-supplied statement does not accept constructor arguments".into()));
                                }
                                fetch_ctor = Some(a);
                            }
                        }
                        other => {
                            return Err(ctx.throw("TypeError", format!("{func}(): Argument #{} ($args) must be of type ?array, {} given", mode_arg + 2, value_name(&other))));
                        }
                    }
                }
                fetch_class = Some(class.name.clone());
            }
        }
        FETCH_INTO => {
            if total != arg1 {
                return Err(count_err(ctx, "exactly", arg1));
            }
            match arg(args, 0) {
                v @ Value::Object(_) => fetch_into = Some(v),
                other => return Err(ctx.throw("TypeError", format!("{func}(): Argument #{arg1} ($args) must be of type object, {} given", value_name(&other)))),
            }
        }
        _ => return Err(ctx.throw("ValueError", format!("{func}(): Argument #{mode_arg} (${mode_name}) must be one of the PDO::FETCH_* constants"))),
    }
    with(ctx, this, |st| {
        match fetch_type {
            FETCH_COLUMN => st.fetch.column = fetch_column,
            FETCH_CLASS => {
                st.fetch.class = fetch_class;
                st.fetch.ctor_args = fetch_ctor;
            }
            FETCH_INTO => st.fetch.into = fetch_into,
            _ => {}
        }
        st.default_fetch = if fetch_type == FETCH_USE_DEFAULT { st.dbh.default_fetch.get() } else { mode };
    })?;
    Ok(())
}

pub fn m_set_fetch_mode(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let func = "PDOStatement::setFetchMode";
    if args.is_empty() {
        return Err(ctx.throw("ArgumentCountError", format!("{func}() expects at least 1 argument, 0 given")));
    }
    let mode = arg(args, 0).to_int();
    let rest: Vec<Value> = args[1..].iter().map(Value::deref).collect();
    setup_fetch_mode(ctx, this, func, mode, 1, "mode", &rest)?;
    Ok(Value::Bool(true))
}

pub fn m_debug_dump_params(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let (query, active, params) = with(ctx, this, |st| (st.query.clone(), st.active_query.clone(), st.params.clone()))?;
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(format!("SQL: [{}] ", query.len()).as_bytes());
    out.extend_from_slice(query.as_bytes());
    out.push(b'\n');
    if let Some(a) = &active
        && a.as_bytes() != query.as_bytes()
    {
        out.extend_from_slice(format!("Sent SQL: [{}] ", a.len()).as_bytes());
        out.extend_from_slice(a.as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(format!("Params:  {}\n", params.len()).as_bytes());
    for p in &params {
        match &p.key {
            PKey::Name(n) => {
                out.extend_from_slice(format!("Key: Name: [{}] {}\n", n.len(), n).as_bytes());
                out.extend_from_slice(format!("paramno={}\nname=[{}] \"{}\"\nis_param=1\nparam_type={}\n", -1, n.len(), n, p.ty).as_bytes());
            }
            PKey::Pos(i) => {
                out.extend_from_slice(format!("Key: Position #{i}:\n").as_bytes());
                out.extend_from_slice(format!("paramno={i}\nname=[0] \"\"\nis_param=1\nparam_type={}\n", p.ty).as_bytes());
            }
        }
    }
    ctx.echo(&out);
    Ok(Value::Null)
}

// ---- iteration ----

#[derive(Default)]
pub struct IterData {
    stmt: RefCell<Option<Rc<Object>>>,
    ahead: RefCell<Option<Value>>,
    key: Cell<i64>,
}

fn iter_data(o: &Object) -> &IterData {
    o.native::<IterData>().expect("PDOStatement iterator state")
}

pub fn make_iter() -> Rc<dyn std::any::Any> {
    Rc::new(IterData::default())
}

fn iter_step(ctx: &mut dyn Ctx, it: &Rc<Object>) -> Result<(), Throw> {
    let d = iter_data(it);
    let stmt = d.stmt.borrow().clone().expect("statement");
    match do_fetch(ctx, "PDOStatement::getIterator", &stmt, FETCH_USE_DEFAULT, None)? {
        Some(v) => {
            *d.ahead.borrow_mut() = Some(strip_named(v));
            d.key.set(d.key.get() + 1);
        }
        None => {
            handle_err(ctx, "PDOStatement::getIterator", &stmt)?;
            *d.ahead.borrow_mut() = None;
            d.key.set(-1);
        }
    }
    Ok(())
}

pub fn m_get_iterator(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    with(ctx, this, |_| ())?;
    let class = ctx.class(b"PDOStatementIterator").expect("PDOStatementIterator");
    let it = ctx.new_object(&class);
    {
        let d = iter_data(&it);
        *d.stmt.borrow_mut() = Some(Rc::clone(this));
        d.key.set(-1);
    }
    iter_step(ctx, &it)?;
    Ok(Value::Object(it))
}

pub fn it_current(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(iter_data(this).ahead.borrow().clone().unwrap_or(Value::Null))
}

pub fn it_key(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let d = iter_data(this);
    Ok(if d.ahead.borrow().is_none() && d.key.get() < 0 { Value::Null } else { Value::Int(d.key.get()) })
}

pub fn it_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    iter_step(ctx, this)?;
    Ok(Value::Null)
}

pub fn it_valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Bool(iter_data(this).ahead.borrow().is_some()))
}

pub fn it_rewind(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Null)
}

// ---- PDORow (PDO::FETCH_LAZY) as an array ----

pub fn row_offset_get(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let d = this.dynamic.borrow();
    Ok(match arg(args, 0) {
        Value::Int(i) => d.values().nth(i.max(0) as usize).map_or(Value::Null, Value::deref),
        other => d.get(&Key::from_string(other.to_php_string().unwrap_or_else(PhpStr::empty))).map_or(Value::Null, Value::deref),
    })
}

pub fn row_offset_exists(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let d = this.dynamic.borrow();
    Ok(Value::Bool(match arg(args, 0) {
        Value::Int(i) => i >= 0 && (i as usize) < d.len(),
        other => d.contains_key(&Key::from_string(other.to_php_string().unwrap_or_else(PhpStr::empty))),
    }))
}

pub fn row_offset_set(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Cannot write to PDORow offset".into()))
}

pub fn row_offset_unset(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Cannot unset PDORow offset".into()))
}

#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's mysqli extension over p8-mysqlnd: the mysqli, mysqli_result and
//! mysqli_sql_exception classes and the procedural functions, which share
//! one implementation (a function's link is the method's `$this`; only the
//! name in messages differs). Results are buffered (MYSQLI_STORE_RESULT),
//! values are strings as the text protocol sends them.
//!
//! Not yet: prepared statements (mysqli_stmt), multi_query, TLS,
//! caching_sha2_password's full authentication, persistent links.
//!
//! Reference: $PHP_SRC_PATH/ext/mysqli (mysqli_api.c, mysqli_nonapi.c,
//! mysqli_prop.c, mysqli_result_iterator.c, mysqli_exception.c)

mod constants;

use p8_mysqlnd::{Address, ConnectOptions, Connection, Outcome, ResultSet};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_FINAL, Native, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "mysqli", register };

type R = Result<Value, Throw>;

const CLIENT_INFO: &str = "mysqlnd 8.4.27";
const CLIENT_VERSION: i64 = 80427;

const REPORT_ERROR: i64 = 1;
const REPORT_STRICT: i64 = 2;
const ASSOC: i64 = 1;
const NUM: i64 = 2;
const BOTH: i64 = 3;

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_data("mysqli", || Rc::new(Link::default()));
    registry.add_data("mysqli_result", || Rc::new(ResultData::default()));
    for (name, value) in constants::CONSTANTS {
        registry.add_constant(name.as_bytes(), *value);
    }
    for (name, default) in [
        ("mysqli.allow_local_infile", Some(&b"0"[..])),
        ("mysqli.allow_persistent", Some(b"1")),
        ("mysqli.default_host", None),
        ("mysqli.default_port", Some(b"3306")),
        ("mysqli.default_pw", None),
        ("mysqli.default_socket", None),
        ("mysqli.default_user", None),
        ("mysqli.local_infile_directory", None),
        ("mysqli.max_links", Some(b"-1")),
        ("mysqli.max_persistent", Some(b"-1")),
        ("mysqli.rollback_on_cached_plink", Some(b"0")),
    ] {
        registry.add_ini(name, default);
    }
    // fetch_object() runs the class's constructor; getIterator() makes one.
    registry.mark_reenters(&["mysqli_fetch_object"]);
    registry.add_module("mysqli");
}

// ---- state ----

/// A link's connection and the last error it reported.
#[derive(Default)]
pub struct Link {
    conn: RefCell<Option<Connection>>,
    /// mysqli_init() made it and nothing connected it yet.
    state: Cell<LinkState>,
    errno: Cell<u16>,
    sqlstate: RefCell<String>,
    error: RefCell<String>,
    /// mysqli_affected_rows(): -1 after an error.
    affected: Cell<i64>,
    insert_id: Cell<u64>,
    field_count: Cell<u32>,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum LinkState {
    #[default]
    Init,
    Connected,
    Closed,
}

#[derive(Default)]
pub struct ResultData {
    set: RefCell<ResultSet>,
    /// The next row fetch() gives, and the next field fetch_field() gives.
    row: Cell<usize>,
    field: Cell<usize>,
    lengths: RefCell<Option<Vec<i64>>>,
    freed: Cell<bool>,
}

fn link(o: &Object) -> &Link {
    o.native::<Link>().expect("mysqli state")
}

fn result(o: &Object) -> &ResultData {
    o.native::<ResultData>().expect("mysqli_result state")
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

/// An optional string argument: None when missing or null.
fn opt_str(args: &[Value], i: usize) -> Option<PhpStr> {
    match arg(args, i) {
        Value::Null => None,
        v => Some(v.to_php_string().unwrap_or_else(PhpStr::empty)),
    }
}

fn str_arg(args: &[Value], i: usize) -> PhpStr {
    arg(args, i).to_php_string().unwrap_or_else(PhpStr::empty)
}

fn report_mode(ctx: &mut dyn Ctx) -> i64 {
    match ctx.state("mysqli.report_mode") {
        Value::Null => REPORT_ERROR | REPORT_STRICT,
        v => v.to_int(),
    }
}

/// php_mysqli_throw_sql_exception(): a mysqli_sql_exception in strict mode,
/// else a warning "(sqlstate/errno): message".
fn sql_error(ctx: &mut dyn Ctx, func: &str, e: &p8_mysqlnd::Error) -> Result<(), Throw> {
    if report_mode(ctx) & REPORT_STRICT == 0 {
        ctx.warn(&format!("{func}(): ({}/{}): {}", e.sqlstate, e.code, e.message));
        return Ok(());
    }
    let mut t = ctx.throw("mysqli_sql_exception", e.message.clone());
    if let Throw::Error { props, .. } = &mut t {
        props.push(("code", Value::Int(e.code as i64)));
        props.push(("sqlstate", Value::str(e.sqlstate.as_str())));
    }
    Err(t)
}

/// MYSQLI_REPORT_MYSQL_ERROR: a statement's error, reported when the mode asks.
fn report(ctx: &mut dyn Ctx, func: &str, e: &p8_mysqlnd::Error) -> Result<(), Throw> {
    if report_mode(ctx) & REPORT_ERROR != 0 { sql_error(ctx, func, e) } else { Ok(()) }
}

/// The link's connection, or the Error PHP throws for a closed or
/// unconnected one.
fn connected<'a>(ctx: &mut dyn Ctx, this: &'a Object) -> Result<&'a Link, Throw> {
    let l = link(this);
    match l.state.get() {
        LinkState::Connected => Ok(l),
        LinkState::Closed => Err(ctx.throw("Error", "mysqli object is already closed".into())),
        LinkState::Init => Err(ctx.throw("Error", "mysqli object is not fully initialized".into())),
    }
}

impl Link {
    fn set_error(&self, e: Option<&p8_mysqlnd::Error>) {
        match e {
            Some(e) => {
                self.errno.set(e.code);
                *self.sqlstate.borrow_mut() = e.sqlstate.clone();
                *self.error.borrow_mut() = e.message.clone();
            }
            None => {
                self.errno.set(0);
                *self.sqlstate.borrow_mut() = "00000".into();
                self.error.borrow_mut().clear();
            }
        }
    }
}

// ---- properties ----

/// The values mysqli's properties have for a link (mysqli_prop.c): none
/// until it is connected; once closed, reading one throws.
fn link_props(o: &Object) -> Vec<(&'static str, Value)> {
    let l = link(o);
    let mut v = vec![
        ("client_info", Value::str(CLIENT_INFO)),
        ("client_version", Value::Int(CLIENT_VERSION)),
    ];
    let connect = CONNECT_ERROR.with(|c| c.borrow().clone());
    v.push(("connect_errno", Value::Int(connect.0)));
    v.push(("connect_error", connect.1.map_or(Value::Null, |s| Value::str(s.as_str()))));
    if l.state.get() != LinkState::Connected {
        v.push(("errno", Value::Int(l.errno.get() as i64)));
        v.push(("error", Value::str(l.error.borrow().as_str())));
        return v;
    }
    let conn = l.conn.borrow();
    let c = conn.as_ref().expect("connected");
    let mut list = PhpArray::new();
    if l.errno.get() != 0 {
        let mut e = PhpArray::new();
        e.insert(Key::from_string(PhpStr::from("errno")), Value::Int(l.errno.get() as i64));
        e.insert(Key::from_string(PhpStr::from("sqlstate")), Value::str(l.sqlstate.borrow().as_str()));
        e.insert(Key::from_string(PhpStr::from("error")), Value::str(l.error.borrow().as_str()));
        let _ = list.push(Value::Array(Rc::new(e)));
    }
    v.extend([
        ("affected_rows", Value::Int(l.affected.get())),
        ("errno", Value::Int(l.errno.get() as i64)),
        ("error", Value::str(l.error.borrow().as_str())),
        ("error_list", Value::Array(Rc::new(list))),
        ("field_count", Value::Int(l.field_count.get() as i64)),
        ("host_info", Value::str(c.host_info.as_str())),
        ("info", if c.info.is_empty() { Value::Null } else { Value::Str(PhpStr::new(c.info.clone())) }),
        ("insert_id", Value::Int(l.insert_id.get() as i64)),
        ("server_info", Value::str(c.server_version.as_str())),
        ("server_version", Value::Int(server_version(&c.server_version))),
        ("sqlstate", Value::str(l.sqlstate.borrow().as_str())),
        ("protocol_version", Value::Int(c.protocol_version as i64)),
        ("thread_id", Value::Int(c.thread_id as i64)),
        ("warning_count", Value::Int(c.warnings as i64)),
    ]);
    v
}

thread_local! {
    /// The last connect attempt's error (MyG(error_no), MyG(error_msg)).
    static CONNECT_ERROR: RefCell<(i64, Option<String>)> = const { RefCell::new((0, None)) };
}

fn set_connect_error(e: Option<&p8_mysqlnd::Error>) {
    CONNECT_ERROR.with(|c| *c.borrow_mut() = e.map_or((0, None), |e| (e.code as i64, Some(e.message.clone()))));
}

/// Write the link's property values into its object, after each operation.
fn sync(o: &Object) {
    for (name, value) in link_props(o) {
        o.class.write(o, name.as_bytes(), b"mysqli", value);
    }
}

/// mysqlnd's server version number: major * 10000 + minor * 100 + patch
/// (after MariaDB's "5.5.5-" prefix).
fn server_version(v: &str) -> i64 {
    let v = v.strip_prefix("5.5.5-").unwrap_or(v);
    let mut parts = v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse::<i64>().unwrap_or(0));
    let (a, b, c) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    a * 10000 + b * 100 + c
}

/// mysqli_result's properties: current_field, field_count, lengths,
/// num_rows, type (always MYSQLI_STORE_RESULT here).
fn result_props(o: &Object) -> [(&'static str, Value); 5] {
    let r = result(o);
    let set = r.set.borrow();
    let lengths = match &*r.lengths.borrow() {
        Some(l) => Value::Array(Rc::new(PhpArray::from_list(l.iter().map(|&n| Value::Int(n)).collect()))),
        None => Value::Null,
    };
    [
        ("current_field", Value::Int(r.field.get() as i64)),
        ("field_count", Value::Int(set.columns.len() as i64)),
        ("lengths", lengths),
        ("num_rows", Value::Int(set.rows.len() as i64)),
        ("type", Value::Int(0)),
    ]
}

fn sync_result(o: &Object) {
    for (name, value) in result_props(o) {
        o.class.write(o, name.as_bytes(), b"mysqli_result", value);
    }
}

fn result_debug(o: &Object) -> PhpArray {
    let mut a = PhpArray::new();
    for (name, value) in result_props(o) {
        a.insert(Key::from_string(PhpStr::from(name)), value);
    }
    a
}

// ---- connecting ----

/// mysqli_real_connect()'s work, for a link: its arguments are host, user,
/// password, database, port, socket, flags (each optional).
fn real_connect(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &[Value], func: &str) -> Result<bool, Throw> {
    let l = link(this);
    if let Some(old) = l.conn.borrow_mut().take() {
        old.close();
    }
    let ini = |ctx: &mut dyn Ctx, name: &[u8]| ctx.ini_get(name).and_then(|v| v.to_php_string()).filter(|s| !s.is_empty());
    let host = match opt_str(args, 0) {
        Some(h) => Some(h),
        None => ini(ctx, b"mysqli.default_host"),
    };
    let user = opt_str(args, 1).or_else(|| ini(ctx, b"mysqli.default_user")).unwrap_or_else(PhpStr::empty);
    let password = opt_str(args, 2).or_else(|| ini(ctx, b"mysqli.default_pw")).unwrap_or_else(PhpStr::empty);
    let database = opt_str(args, 3);
    let port = match arg(args, 4) {
        Value::Null => ini(ctx, b"mysqli.default_port").map_or(3306, |p| String::from_utf8_lossy(p.as_bytes()).trim().parse().unwrap_or(3306)),
        v => v.to_int(),
    };
    let socket = opt_str(args, 5).or_else(|| ini(ctx, b"mysqli.default_socket"));
    let flags = arg(args, 6).to_int();
    let host = host.map(|h| String::from_utf8_lossy(h.as_bytes()).into_owned()).unwrap_or_default();
    // "p:" asks for a persistent link: p8 opens a plain one.
    let host = host.strip_prefix("p:").unwrap_or(&host).to_string();
    let socket = socket.map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned());
    // As mysqlnd: no host, or "localhost", is the Unix socket.
    let address = if host.is_empty() || host == "localhost" {
        Address::Socket(socket.as_deref().unwrap_or("/tmp/mysql.sock"))
    } else {
        Address::Tcp { host: &host, port: u16::try_from(port).unwrap_or(3306) }
    };
    let options = ConnectOptions {
        address,
        user: user.as_bytes(),
        password: password.as_bytes(),
        database: database.as_ref().map(|d| d.as_bytes()),
        flags: flags as u32,
        timeout: None,
    };
    match Connection::connect(&options) {
        Ok(conn) => {
            *l.conn.borrow_mut() = Some(conn);
            l.state.set(LinkState::Connected);
            l.set_error(None);
            l.affected.set(0);
            l.insert_id.set(0);
            set_connect_error(None);
            sync(this);
            Ok(true)
        }
        Err(e) => {
            l.set_error(Some(&e));
            set_connect_error(Some(&e));
            sync(this);
            sql_error(ctx, func, &e)?;
            Ok(false)
        }
    }
}

fn new_link(ctx: &mut dyn Ctx) -> Rc<Object> {
    let class = ctx.class(b"mysqli").expect("mysqli");
    let o = ctx.new_object(&class);
    sync(&o);
    o
}

fn mysqli_init(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::Object(new_link(ctx)))
}

fn mysqli_connect(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = new_link(ctx);
    Ok(if real_connect(ctx, &o, args, "mysqli_connect")? { Value::Object(o) } else { Value::Bool(false) })
}

fn mysqli_real_connect(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(o) = arg(args, 0) else { unreachable!("checked against the stub") };
    Ok(Value::Bool(real_connect(ctx, &o, &args[1..], "mysqli_real_connect")?))
}

fn m_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    sync(this);
    // new mysqli() with no arguments only initializes (mysqli_init()).
    if !args.is_empty() {
        real_connect(ctx, this, args, "mysqli::__construct")?;
    }
    Ok(Value::Null)
}

fn m_real_connect(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    Ok(Value::Bool(real_connect(ctx, this, args, "mysqli::real_connect")?))
}

fn mysqli_connect_errno(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::Int(CONNECT_ERROR.with(|c| c.borrow().0)))
}

fn mysqli_connect_error(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(CONNECT_ERROR.with(|c| c.borrow().1.clone()).map_or(Value::Null, |s| Value::str(s.as_str())))
}

fn mysqli_report(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    *ctx.state("mysqli.report_mode") = Value::Int(arg(args, 0).to_int());
    Ok(Value::Bool(true))
}

// ---- operations on a link (function and method) ----

/// Define `mysqli_$name($link, ...)` and the method `mysqli::$name(...)`
/// over one operation `fn(ctx, link, args, func_name)`.
/// (Only the function, for what the object has as a property instead.)
macro_rules! link_op {
    ($op:ident, $func:ident, $method:ident, $fname:literal, $mname:literal) => {
        link_op!($op, $func, $fname);
        fn $method(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
            $op(ctx, this, args, $mname)
        }
    };
    ($op:ident, $func:ident, $fname:literal) => {
        fn $func(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
            let Value::Object(o) = arg(args, 0) else { unreachable!("checked against the stub") };
            $op(ctx, &o, &mut args[1..], $fname)
        }
    };
}

link_op!(query, mysqli_query, m_query, "mysqli_query", "mysqli::query");
link_op!(select_db, mysqli_select_db, m_select_db, "mysqli_select_db", "mysqli::select_db");
link_op!(set_charset, mysqli_set_charset, m_set_charset, "mysqli_set_charset", "mysqli::set_charset");
link_op!(character_set_name, mysqli_character_set_name, m_character_set_name, "mysqli_character_set_name", "mysqli::character_set_name");
link_op!(get_charset, mysqli_get_charset, m_get_charset, "mysqli_get_charset", "mysqli::get_charset");
link_op!(escape, mysqli_real_escape_string, m_real_escape_string, "mysqli_real_escape_string", "mysqli::real_escape_string");
link_op!(errno, mysqli_errno, "mysqli_errno");
link_op!(error, mysqli_error, "mysqli_error");
link_op!(sqlstate, mysqli_sqlstate, "mysqli_sqlstate");
link_op!(close, mysqli_close, m_close, "mysqli_close", "mysqli::close");
link_op!(ping, mysqli_ping, m_ping, "mysqli_ping", "mysqli::ping");
link_op!(insert_id, mysqli_insert_id, "mysqli_insert_id");
link_op!(affected_rows, mysqli_affected_rows, "mysqli_affected_rows");
link_op!(server_info, mysqli_get_server_info, m_get_server_info, "mysqli_get_server_info", "mysqli::get_server_info");
link_op!(server_version_op, mysqli_get_server_version, "mysqli_get_server_version");
link_op!(host_info, mysqli_get_host_info, "mysqli_get_host_info");
link_op!(proto_info, mysqli_get_proto_info, "mysqli_get_proto_info");
link_op!(thread_id, mysqli_thread_id, "mysqli_thread_id");
link_op!(more_results, mysqli_more_results, m_more_results, "mysqli_more_results", "mysqli::more_results");
link_op!(next_result, mysqli_next_result, m_next_result, "mysqli_next_result", "mysqli::next_result");
link_op!(field_count, mysqli_field_count, "mysqli_field_count");
link_op!(warning_count, mysqli_warning_count, "mysqli_warning_count");
link_op!(info, mysqli_info, "mysqli_info");
link_op!(autocommit, mysqli_autocommit, m_autocommit, "mysqli_autocommit", "mysqli::autocommit");
link_op!(commit, mysqli_commit, m_commit, "mysqli_commit", "mysqli::commit");
link_op!(rollback, mysqli_rollback, m_rollback, "mysqli_rollback", "mysqli::rollback");
link_op!(begin_transaction, mysqli_begin_transaction, m_begin_transaction, "mysqli_begin_transaction", "mysqli::begin_transaction");

/// What a statement gave, as mysqli_query() returns it.
fn outcome_value(ctx: &mut dyn Ctx, this: &Rc<Object>, outcome: Outcome) -> Value {
    let l = link(this);
    let conn = l.conn.borrow();
    let c = conn.as_ref().expect("connected");
    match outcome {
        Outcome::Done => {
            l.affected.set(c.affected_rows as i64);
            l.insert_id.set(c.insert_id);
            l.field_count.set(0);
            drop(conn);
            sync(this);
            Value::Bool(true)
        }
        Outcome::Rows(set) => {
            l.affected.set(set.rows.len() as i64);
            l.insert_id.set(0);
            l.field_count.set(set.columns.len() as u32);
            drop(conn);
            sync(this);
            Value::Object(new_result(ctx, set))
        }
    }
}

fn new_result(ctx: &mut dyn Ctx, set: ResultSet) -> Rc<Object> {
    let class = ctx.class(b"mysqli_result").expect("mysqli_result");
    let o = ctx.new_object(&class);
    *result(&o).set.borrow_mut() = set;
    sync_result(&o);
    o
}

/// Run a statement on the link: Ok(outcome) or the error, recorded.
fn run(this: &Rc<Object>, sql: &[u8]) -> Result<Outcome, p8_mysqlnd::Error> {
    let l = link(this);
    let r = l.conn.borrow_mut().as_mut().expect("connected").query(sql);
    match &r {
        Ok(_) => l.set_error(None),
        Err(e) => {
            l.set_error(Some(e));
            l.affected.set(-1);
        }
    }
    r
}

fn query(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    connected(ctx, this)?;
    let sql = str_arg(args, 0);
    if sql.is_empty() {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{} ($query) cannot be empty", if func.contains("::") { 1 } else { 2 })));
    }
    let mode = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    if mode & !(16 | 32 | 1) != 0 && mode != 0 {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{} ($result_mode) must be either MYSQLI_USE_RESULT, or MYSQLI_STORE_RESULT with MYSQLI_ASYNC as an optional bitmask flag", if func.contains("::") { 2 } else { 3 })));
    }
    match run(this, sql.as_bytes()) {
        Ok(outcome) => Ok(outcome_value(ctx, this, outcome)),
        Err(e) => {
            sync(this);
            report(ctx, func, &e)?;
            Ok(Value::Bool(false))
        }
    }
}

/// A statement whose result is only success or failure (SET, COMMIT...).
fn simple(ctx: &mut dyn Ctx, this: &Rc<Object>, sql: &[u8], func: &str) -> R {
    connected(ctx, this)?;
    match run(this, sql) {
        Ok(_) => {
            sync(this);
            Ok(Value::Bool(true))
        }
        Err(e) => {
            sync(this);
            report(ctx, func, &e)?;
            Ok(Value::Bool(false))
        }
    }
}

fn select_db(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let l = connected(ctx, this)?;
    let db = str_arg(args, 0);
    let r = l.conn.borrow_mut().as_mut().expect("connected").select_db(db.as_bytes());
    l.set_error(r.as_ref().err());
    sync(this);
    match r {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            report(ctx, func, &e)?;
            Ok(Value::Bool(false))
        }
    }
}

fn set_charset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let l = connected(ctx, this)?;
    let name = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let r = l.conn.borrow_mut().as_mut().expect("connected").set_charset(&name);
    l.set_error(r.as_ref().err());
    sync(this);
    match r {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            report(ctx, func, &e)?;
            Ok(Value::Bool(false))
        }
    }
}

fn character_set_name(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    let nr = l.conn.borrow().as_ref().expect("connected").charset;
    Ok(Value::str(p8_mysqlnd::charset_by_nr(nr).map_or("", |c| c.name)))
}

/// get_charset(): the connection's character set (an stdClass, as mysqlnd
/// describes it).
fn get_charset(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    let nr = l.conn.borrow().as_ref().expect("connected").charset;
    let Some(cs) = p8_mysqlnd::charset_by_nr(nr) else { return Ok(Value::Null) };
    let class = ctx.class(b"stdClass").expect("stdClass");
    let o = ctx.new_object(&class);
    for (k, v) in [
        ("charset", Value::str(cs.name)),
        ("collation", Value::str(cs.collation)),
        ("dir", Value::str("")),
        ("min_length", Value::Int(1)),
        ("max_length", Value::Int(i64::from(cs.max_len))),
        ("number", Value::Int(i64::from(cs.nr))),
        ("state", Value::Int(1)),
        ("comment", Value::str(cs.comment)),
    ] {
        o.dynamic.borrow_mut().insert(Key::from_string(PhpStr::from(k)), v);
    }
    Ok(Value::Object(o))
}

fn escape(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    let nr = l.conn.borrow().as_ref().expect("connected").charset;
    let cs = p8_mysqlnd::charset_by_nr(nr).or_else(|| p8_mysqlnd::charset("utf8mb4")).expect("a known charset");
    Ok(Value::Str(PhpStr::new(p8_mysqlnd::escape(&cs, str_arg(args, 0).as_bytes()))))
}

fn errno(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::Int(connected(ctx, this)?.errno.get() as i64))
}

fn error(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::str(connected(ctx, this)?.error.borrow().as_str()))
}

fn sqlstate(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::str(connected(ctx, this)?.sqlstate.borrow().as_str()))
}

fn close(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    if let Some(c) = l.conn.borrow_mut().take() {
        c.close();
    }
    l.state.set(LinkState::Closed);
    Ok(Value::Bool(true))
}

fn ping(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], func: &str) -> R {
    let l = connected(ctx, this)?;
    let r = l.conn.borrow_mut().as_mut().expect("connected").ping();
    l.set_error(r.as_ref().err());
    sync(this);
    match r {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            report(ctx, func, &e)?;
            Ok(Value::Bool(false))
        }
    }
}

fn insert_id(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::Int(connected(ctx, this)?.insert_id.get() as i64))
}

fn affected_rows(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::Int(connected(ctx, this)?.affected.get()))
}

fn server_info(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    Ok(Value::str(l.conn.borrow().as_ref().expect("connected").server_version.as_str()))
}

fn server_version_op(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    Ok(Value::Int(server_version(&l.conn.borrow().as_ref().expect("connected").server_version)))
}

fn host_info(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    Ok(Value::str(l.conn.borrow().as_ref().expect("connected").host_info.as_str()))
}

fn proto_info(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    Ok(Value::Int(l.conn.borrow().as_ref().expect("connected").protocol_version as i64))
}

fn thread_id(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    Ok(Value::Int(l.conn.borrow().as_ref().expect("connected").thread_id as i64))
}

fn more_results(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    Ok(Value::Bool(l.conn.borrow().as_ref().expect("connected").more_results()))
}

/// next_result(): read the next result (mysqli then hands it out through
/// store_result(), which p8 does not have yet, so it is dropped).
fn next_result(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], func: &str) -> R {
    let l = connected(ctx, this)?;
    let r = l.conn.borrow_mut().as_mut().expect("connected").next_result();
    match r {
        None => Ok(Value::Bool(false)),
        Some(Ok(_)) => {
            l.set_error(None);
            sync(this);
            Ok(Value::Bool(true))
        }
        Some(Err(e)) => {
            l.set_error(Some(&e));
            sync(this);
            report(ctx, func, &e)?;
            Ok(Value::Bool(false))
        }
    }
}

fn field_count(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::Int(connected(ctx, this)?.field_count.get() as i64))
}

fn warning_count(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    Ok(Value::Int(l.conn.borrow().as_ref().expect("connected").warnings as i64))
}

fn info(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let l = connected(ctx, this)?;
    let info = l.conn.borrow().as_ref().expect("connected").info.clone();
    Ok(if info.is_empty() { Value::Null } else { Value::Str(PhpStr::new(info)) })
}

fn autocommit(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let sql = if arg(args, 0).to_bool() { "SET AUTOCOMMIT=1" } else { "SET AUTOCOMMIT=0" };
    simple(ctx, this, sql.as_bytes(), func)
}

fn commit(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], func: &str) -> R {
    simple(ctx, this, b"COMMIT", func)
}

fn rollback(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], func: &str) -> R {
    simple(ctx, this, b"ROLLBACK", func)
}

fn begin_transaction(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let flags = arg(args, 0).to_int();
    let mut sql = String::from("START TRANSACTION");
    let mut parts = Vec::new();
    if flags & 1 != 0 {
        parts.push(" WITH CONSISTENT SNAPSHOT");
    }
    if flags & 4 != 0 {
        parts.push(" READ WRITE");
    } else if flags & 2 != 0 {
        parts.push(" READ ONLY");
    }
    sql.push_str(&parts.join(","));
    simple(ctx, this, sql.as_bytes(), func)
}

fn mysqli_get_client_info(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::str(CLIENT_INFO))
}

fn mysqli_get_client_version(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::Int(CLIENT_VERSION))
}

fn m_get_client_info(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::str(CLIENT_INFO))
}

fn mysqli_escape_string(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    mysqli_real_escape_string(ctx, args)
}

// ---- results ----

macro_rules! result_op {
    ($op:ident, $func:ident, $method:ident, $fname:literal, $mname:literal) => {
        result_op!($op, $func, $fname);
        fn $method(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
            $op(ctx, this, args, $mname)
        }
    };
    ($op:ident, $func:ident, $fname:literal) => {
        fn $func(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
            let Value::Object(o) = arg(args, 0) else { unreachable!("checked against the stub") };
            $op(ctx, &o, &mut args[1..], $fname)
        }
    };
}

result_op!(fetch_assoc, mysqli_fetch_assoc, r_fetch_assoc, "mysqli_fetch_assoc", "mysqli_result::fetch_assoc");
result_op!(fetch_row, mysqli_fetch_row, r_fetch_row, "mysqli_fetch_row", "mysqli_result::fetch_row");
result_op!(fetch_array, mysqli_fetch_array, r_fetch_array, "mysqli_fetch_array", "mysqli_result::fetch_array");
result_op!(fetch_object, mysqli_fetch_object, r_fetch_object, "mysqli_fetch_object", "mysqli_result::fetch_object");
result_op!(fetch_all, mysqli_fetch_all, r_fetch_all, "mysqli_fetch_all", "mysqli_result::fetch_all");
result_op!(fetch_column, mysqli_fetch_column, r_fetch_column, "mysqli_fetch_column", "mysqli_result::fetch_column");
result_op!(fetch_field, mysqli_fetch_field, r_fetch_field, "mysqli_fetch_field", "mysqli_result::fetch_field");
result_op!(fetch_fields, mysqli_fetch_fields, r_fetch_fields, "mysqli_fetch_fields", "mysqli_result::fetch_fields");
result_op!(fetch_field_direct, mysqli_fetch_field_direct, r_fetch_field_direct, "mysqli_fetch_field_direct", "mysqli_result::fetch_field_direct");
result_op!(field_seek, mysqli_field_seek, r_field_seek, "mysqli_field_seek", "mysqli_result::field_seek");
result_op!(field_tell, mysqli_field_tell, "mysqli_field_tell");
result_op!(data_seek, mysqli_data_seek, r_data_seek, "mysqli_data_seek", "mysqli_result::data_seek");
result_op!(num_rows, mysqli_num_rows, "mysqli_num_rows");
result_op!(num_fields, mysqli_num_fields, "mysqli_num_fields");
result_op!(free, mysqli_free_result, r_free, "mysqli_free_result", "mysqli_result::free");
result_op!(fetch_lengths, mysqli_fetch_lengths, "mysqli_fetch_lengths");

fn live<'a>(ctx: &mut dyn Ctx, o: &'a Object) -> Result<&'a ResultData, Throw> {
    let r = result(o);
    if r.freed.get() {
        return Err(ctx.throw("Error", "mysqli_result object is already closed".into()));
    }
    Ok(r)
}

/// The next row, as (name, value) pairs, or None after the last.
fn next_row(r: &ResultData) -> Option<Vec<(Vec<u8>, Value)>> {
    let set = r.set.borrow();
    let i = r.row.get();
    let row = set.rows.get(i)?;
    r.row.set(i + 1);
    *r.lengths.borrow_mut() = Some(row.iter().map(|v| v.as_ref().map_or(0, |v| v.len() as i64)).collect());
    Some(
        set.columns
            .iter()
            .zip(row)
            .map(|(c, v)| (c.name.clone(), v.as_ref().map_or(Value::Null, |v| Value::Str(PhpStr::new(v.clone())))))
            .collect(),
    )
}

/// A row as fetch_array() builds it: numbered, named, or both.
fn row_array(row: Vec<(Vec<u8>, Value)>, mode: i64) -> PhpArray {
    let mut a = PhpArray::new();
    for (i, (name, value)) in row.into_iter().enumerate() {
        if mode & NUM != 0 {
            a.insert(Key::Int(i as i64), value.clone());
        }
        if mode & ASSOC != 0 {
            a.insert(Key::from_string(PhpStr::new(name)), value);
        }
    }
    a
}

fn fetch(ctx: &mut dyn Ctx, this: &Rc<Object>, mode: i64) -> R {
    let r = live(ctx, this)?;
    let row = next_row(r).map_or(Value::Null, |row| Value::Array(Rc::new(row_array(row, mode))));
    sync_result(this);
    Ok(row)
}

fn fetch_assoc(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    fetch(ctx, this, ASSOC)
}

fn fetch_row(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    fetch(ctx, this, NUM)
}

fn mode_arg(ctx: &mut dyn Ctx, args: &[Value], func: &str, default: i64) -> Result<i64, Throw> {
    let mode = if args.is_empty() { default } else { arg(args, 0).to_int() };
    if !matches!(mode, ASSOC | NUM | BOTH) {
        let n = if func.contains("::") { 1 } else { 2 };
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{n} ($mode) must be one of MYSQLI_NUM, MYSQLI_ASSOC, or MYSQLI_BOTH")));
    }
    Ok(mode)
}

fn fetch_array(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let mode = mode_arg(ctx, args, func, BOTH)?;
    fetch(ctx, this, mode)
}

fn fetch_all(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let mode = mode_arg(ctx, args, func, NUM)?;
    let r = live(ctx, this)?;
    let mut all = PhpArray::new();
    while let Some(row) = next_row(r) {
        let _ = all.push(Value::Array(Rc::new(row_array(row, mode))));
    }
    Ok(Value::Array(Rc::new(all)))
}

fn fetch_column(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let column = arg(args, 0).to_int();
    let r = live(ctx, this)?;
    let count = r.set.borrow().columns.len() as i64;
    if column < 0 || column >= count {
        let n = if func.contains("::") { 1 } else { 2 };
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{n} ($column) must be greater than or equal to 0 and less than the number of fields")));
    }
    Ok(next_row(r).map_or(Value::Bool(false), |mut row| row.swap_remove(column as usize).1))
}

fn fetch_object(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let class_name = opt_str(args, 0).unwrap_or_else(|| PhpStr::from("stdClass"));
    let Some(class) = ctx.class_load(class_name.as_bytes())? else {
        let n = if func.contains("::") { 1 } else { 2 };
        return Err(ctx.throw("TypeError", format!("{func}(): Argument #{n} ($class) must be a valid class name, {} given", String::from_utf8_lossy(class_name.as_bytes()))));
    };
    let r = live(ctx, this)?;
    let Some(row) = next_row(r) else { return Ok(Value::Null) };
    let object = ctx.new_instance(&class, &mut [], false)?;
    for (name, value) in row {
        match class.slot(&name, Some(&class.lower)) {
            Some(i) => object.slots.borrow_mut()[i] = value,
            None => {
                object.dynamic.borrow_mut().insert(Key::from_string(PhpStr::new(name)), value);
            }
        }
    }
    if class.method(b"__construct").is_some() {
        let mut ctor_args: Vec<Value> = match arg(args, 1) {
            Value::Array(a) => a.iter().map(|(_, v)| v.deref()).collect(),
            _ => Vec::new(),
        };
        let callable = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(Rc::clone(&object)), Value::str("__construct")])));
        ctx.call(&callable, &mut ctor_args)?;
    }
    Ok(Value::Object(object))
}

/// A column as fetch_field() describes it (an stdClass).
fn field_object(ctx: &mut dyn Ctx, r: &ResultData, i: usize) -> Value {
    let set = r.set.borrow();
    let c = &set.columns[i];
    let class = ctx.class(b"stdClass").expect("stdClass");
    let o = ctx.new_object(&class);
    let bytes = |b: &[u8]| Value::Str(PhpStr::new(b.to_vec()));
    let fields = [
        ("name", bytes(&c.name)),
        ("orgname", bytes(&c.org_name)),
        ("table", bytes(&c.table)),
        ("orgtable", bytes(&c.org_table)),
        ("def", Value::str("")),
        ("db", bytes(&c.db)),
        ("catalog", bytes(&c.catalog)),
        // Always 0 for a buffered result since PHP 8.1.
        ("max_length", Value::Int(0)),
        ("length", Value::Int(c.length as i64)),
        ("charsetnr", Value::Int(c.charsetnr as i64)),
        ("flags", Value::Int(c.flags as i64)),
        ("type", Value::Int(c.kind as i64)),
        ("decimals", Value::Int(c.decimals as i64)),
    ];
    let mut dynamic = o.dynamic.borrow_mut();
    for (name, value) in fields {
        dynamic.insert(Key::from_string(PhpStr::from(name)), value);
    }
    drop(dynamic);
    Value::Object(o)
}

fn fetch_field(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let r = live(ctx, this)?;
    let i = r.field.get();
    if i >= r.set.borrow().columns.len() {
        return Ok(Value::Bool(false));
    }
    r.field.set(i + 1);
    sync_result(this);
    Ok(field_object(ctx, r, i))
}

fn fetch_fields(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let r = live(ctx, this)?;
    let n = r.set.borrow().columns.len();
    let fields: Vec<Value> = (0..n).map(|i| field_object(ctx, r, i)).collect();
    Ok(Value::Array(Rc::new(PhpArray::from_list(fields))))
}

fn fetch_field_direct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let i = arg(args, 0).to_int();
    let r = live(ctx, this)?;
    if i < 0 || i >= r.set.borrow().columns.len() as i64 {
        let n = if func.contains("::") { 1 } else { 2 };
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{n} ($index) must be greater than or equal to 0 and less than the number of fields")));
    }
    Ok(field_object(ctx, r, i as usize))
}

fn field_seek(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let i = arg(args, 0).to_int();
    let r = live(ctx, this)?;
    if i < 0 || i >= r.set.borrow().columns.len() as i64 {
        let n = if func.contains("::") { 1 } else { 2 };
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{n} ($index) must be greater than or equal to 0 and less than the number of fields")));
    }
    r.field.set(i as usize);
    sync_result(this);
    Ok(Value::Bool(true))
}

fn field_tell(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::Int(live(ctx, this)?.field.get() as i64))
}

fn data_seek(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], func: &str) -> R {
    let i = arg(args, 0).to_int();
    let r = live(ctx, this)?;
    if i < 0 {
        let n = if func.contains("::") { 1 } else { 2 };
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{n} ($offset) must be greater than or equal to 0")));
    }
    if i >= r.set.borrow().rows.len() as i64 {
        return Ok(Value::Bool(false));
    }
    r.row.set(i as usize);
    Ok(Value::Bool(true))
}

fn num_rows(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::Int(live(ctx, this)?.set.borrow().rows.len() as i64))
}

fn num_fields(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    Ok(Value::Int(live(ctx, this)?.set.borrow().columns.len() as i64))
}

fn free(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let r = live(ctx, this)?;
    *r.set.borrow_mut() = ResultSet::default();
    r.freed.set(true);
    Ok(Value::Null)
}

fn fetch_lengths(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value], _: &str) -> R {
    let r = live(ctx, this)?;
    Ok(match &*r.lengths.borrow() {
        Some(l) if r.row.get() <= r.set.borrow().rows.len() => Value::Array(Rc::new(PhpArray::from_list(l.iter().map(|&n| Value::Int(n)).collect()))),
        _ => Value::Bool(false),
    })
}

/// getIterator(): the rows as fetch_assoc() gives them, from the first.
fn r_get_iterator(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = live(ctx, this)?;
    r.row.set(0);
    let mut rows = Vec::new();
    while let Some(row) = next_row(r) {
        rows.push(Value::Array(Rc::new(row_array(row, ASSOC))));
    }
    let class = ctx.class(b"ArrayIterator").expect("ArrayIterator");
    Ok(Value::Object(ctx.new_instance(&class, &mut [Value::Array(Rc::new(PhpArray::from_list(rows)))], true)?))
}

fn result_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "mysqli_result::__construct() is not supported by p8: mysqli_result objects come from mysqli_query()".into()))
}

fn exception_get_sql_state(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(this.class.read(this, b"sqlstate", b"mysqli_sql_exception"))
}

// ---- registration ----

static BUILTINS: &[Builtin] = &[
    Builtin { name: "mysqli_init", run: mysqli_init, by_ref: &[] },
    Builtin { name: "mysqli_connect", run: mysqli_connect, by_ref: &[] },
    Builtin { name: "mysqli_real_connect", run: mysqli_real_connect, by_ref: &[] },
    Builtin { name: "mysqli_connect_errno", run: mysqli_connect_errno, by_ref: &[] },
    Builtin { name: "mysqli_connect_error", run: mysqli_connect_error, by_ref: &[] },
    Builtin { name: "mysqli_report", run: mysqli_report, by_ref: &[] },
    Builtin { name: "mysqli_get_client_info", run: mysqli_get_client_info, by_ref: &[] },
    Builtin { name: "mysqli_get_client_version", run: mysqli_get_client_version, by_ref: &[] },
    Builtin { name: "mysqli_query", run: mysqli_query, by_ref: &[] },
    Builtin { name: "mysqli_select_db", run: mysqli_select_db, by_ref: &[] },
    Builtin { name: "mysqli_set_charset", run: mysqli_set_charset, by_ref: &[] },
    Builtin { name: "mysqli_character_set_name", run: mysqli_character_set_name, by_ref: &[] },
    Builtin { name: "mysqli_get_charset", run: mysqli_get_charset, by_ref: &[] },
    Builtin { name: "mysqli_real_escape_string", run: mysqli_real_escape_string, by_ref: &[] },
    Builtin { name: "mysqli_escape_string", run: mysqli_escape_string, by_ref: &[] },
    Builtin { name: "mysqli_errno", run: mysqli_errno, by_ref: &[] },
    Builtin { name: "mysqli_error", run: mysqli_error, by_ref: &[] },
    Builtin { name: "mysqli_sqlstate", run: mysqli_sqlstate, by_ref: &[] },
    Builtin { name: "mysqli_close", run: mysqli_close, by_ref: &[] },
    Builtin { name: "mysqli_ping", run: mysqli_ping, by_ref: &[] },
    Builtin { name: "mysqli_insert_id", run: mysqli_insert_id, by_ref: &[] },
    Builtin { name: "mysqli_affected_rows", run: mysqli_affected_rows, by_ref: &[] },
    Builtin { name: "mysqli_get_server_info", run: mysqli_get_server_info, by_ref: &[] },
    Builtin { name: "mysqli_get_server_version", run: mysqli_get_server_version, by_ref: &[] },
    Builtin { name: "mysqli_get_host_info", run: mysqli_get_host_info, by_ref: &[] },
    Builtin { name: "mysqli_get_proto_info", run: mysqli_get_proto_info, by_ref: &[] },
    Builtin { name: "mysqli_thread_id", run: mysqli_thread_id, by_ref: &[] },
    Builtin { name: "mysqli_more_results", run: mysqli_more_results, by_ref: &[] },
    Builtin { name: "mysqli_next_result", run: mysqli_next_result, by_ref: &[] },
    Builtin { name: "mysqli_field_count", run: mysqli_field_count, by_ref: &[] },
    Builtin { name: "mysqli_warning_count", run: mysqli_warning_count, by_ref: &[] },
    Builtin { name: "mysqli_info", run: mysqli_info, by_ref: &[] },
    Builtin { name: "mysqli_autocommit", run: mysqli_autocommit, by_ref: &[] },
    Builtin { name: "mysqli_commit", run: mysqli_commit, by_ref: &[] },
    Builtin { name: "mysqli_rollback", run: mysqli_rollback, by_ref: &[] },
    Builtin { name: "mysqli_begin_transaction", run: mysqli_begin_transaction, by_ref: &[] },
    Builtin { name: "mysqli_fetch_assoc", run: mysqli_fetch_assoc, by_ref: &[] },
    Builtin { name: "mysqli_fetch_row", run: mysqli_fetch_row, by_ref: &[] },
    Builtin { name: "mysqli_fetch_array", run: mysqli_fetch_array, by_ref: &[] },
    Builtin { name: "mysqli_fetch_object", run: mysqli_fetch_object, by_ref: &[] },
    Builtin { name: "mysqli_fetch_all", run: mysqli_fetch_all, by_ref: &[] },
    Builtin { name: "mysqli_fetch_column", run: mysqli_fetch_column, by_ref: &[] },
    Builtin { name: "mysqli_fetch_field", run: mysqli_fetch_field, by_ref: &[] },
    Builtin { name: "mysqli_fetch_fields", run: mysqli_fetch_fields, by_ref: &[] },
    Builtin { name: "mysqli_fetch_field_direct", run: mysqli_fetch_field_direct, by_ref: &[] },
    Builtin { name: "mysqli_field_seek", run: mysqli_field_seek, by_ref: &[] },
    Builtin { name: "mysqli_field_tell", run: mysqli_field_tell, by_ref: &[] },
    Builtin { name: "mysqli_data_seek", run: mysqli_data_seek, by_ref: &[] },
    Builtin { name: "mysqli_num_rows", run: mysqli_num_rows, by_ref: &[] },
    Builtin { name: "mysqli_num_fields", run: mysqli_num_fields, by_ref: &[] },
    Builtin { name: "mysqli_free_result", run: mysqli_free_result, by_ref: &[] },
    Builtin { name: "mysqli_fetch_lengths", run: mysqli_fetch_lengths, by_ref: &[] },
];

const PROP_NULL: Constant = Constant::Null;

static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "mysqli",
        parent: None,
        interfaces: &[],
        flags: 0,
        properties: &[
            ("affected_rows", PROP_NULL, Visibility::Public),
            ("client_info", PROP_NULL, Visibility::Public),
            ("client_version", PROP_NULL, Visibility::Public),
            ("connect_errno", PROP_NULL, Visibility::Public),
            ("connect_error", PROP_NULL, Visibility::Public),
            ("errno", PROP_NULL, Visibility::Public),
            ("error", PROP_NULL, Visibility::Public),
            ("error_list", PROP_NULL, Visibility::Public),
            ("field_count", PROP_NULL, Visibility::Public),
            ("host_info", PROP_NULL, Visibility::Public),
            ("info", PROP_NULL, Visibility::Public),
            ("insert_id", PROP_NULL, Visibility::Public),
            ("server_info", PROP_NULL, Visibility::Public),
            ("server_version", PROP_NULL, Visibility::Public),
            ("sqlstate", PROP_NULL, Visibility::Public),
            ("protocol_version", PROP_NULL, Visibility::Public),
            ("thread_id", PROP_NULL, Visibility::Public),
            ("warning_count", PROP_NULL, Visibility::Public),
        ],
        methods: &[
            ("__construct", Some(Native::Method(m_construct))),
            ("real_connect", Some(Native::Method(m_real_connect))),
            ("connect", Some(Native::Method(m_real_connect))),
            ("query", Some(Native::Method(m_query))),
            ("select_db", Some(Native::Method(m_select_db))),
            ("set_charset", Some(Native::Method(m_set_charset))),
            ("character_set_name", Some(Native::Method(m_character_set_name))),
            ("get_charset", Some(Native::Method(m_get_charset))),
            ("real_escape_string", Some(Native::Method(m_real_escape_string))),
            ("escape_string", Some(Native::Method(m_real_escape_string))),
            ("close", Some(Native::Method(m_close))),
            ("ping", Some(Native::Method(m_ping))),
            ("get_server_info", Some(Native::Method(m_get_server_info))),
            ("get_client_info", Some(Native::Method(m_get_client_info))),
            ("more_results", Some(Native::Method(m_more_results))),
            ("next_result", Some(Native::Method(m_next_result))),
            ("autocommit", Some(Native::Method(m_autocommit))),
            ("commit", Some(Native::Method(m_commit))),
            ("rollback", Some(Native::Method(m_rollback))),
            ("begin_transaction", Some(Native::Method(m_begin_transaction))),
        ],
        constants: &[],
        debug_info: Some(link_debug),
    },
    ClassSpec {
        name: "mysqli_result",
        parent: None,
        interfaces: &["IteratorAggregate"],
        flags: 0,
        properties: &[
            ("current_field", PROP_NULL, Visibility::Public),
            ("field_count", PROP_NULL, Visibility::Public),
            ("lengths", PROP_NULL, Visibility::Public),
            ("num_rows", PROP_NULL, Visibility::Public),
            ("type", PROP_NULL, Visibility::Public),
        ],
        methods: &[
            ("__construct", Some(Native::Method(result_construct))),
            ("fetch_assoc", Some(Native::Method(r_fetch_assoc))),
            ("fetch_row", Some(Native::Method(r_fetch_row))),
            ("fetch_array", Some(Native::Method(r_fetch_array))),
            ("fetch_object", Some(Native::Method(r_fetch_object))),
            ("fetch_all", Some(Native::Method(r_fetch_all))),
            ("fetch_column", Some(Native::Method(r_fetch_column))),
            ("fetch_field", Some(Native::Method(r_fetch_field))),
            ("fetch_fields", Some(Native::Method(r_fetch_fields))),
            ("fetch_field_direct", Some(Native::Method(r_fetch_field_direct))),
            ("field_seek", Some(Native::Method(r_field_seek))),
            ("data_seek", Some(Native::Method(r_data_seek))),
            ("free", Some(Native::Method(r_free))),
            ("close", Some(Native::Method(r_free))),
            ("free_result", Some(Native::Method(r_free))),
            ("getIterator", Some(Native::Method(r_get_iterator))),
        ],
        constants: &[],
        debug_info: Some(result_debug),
    },
    ClassSpec {
        name: "mysqli_sql_exception",
        parent: Some("RuntimeException"),
        interfaces: &[],
        flags: CLASS_FINAL,
        properties: &[("sqlstate", Constant::Str(b"00000"), Visibility::Protected)],
        methods: &[("getSqlState", Some(Native::Method(exception_get_sql_state)))],
        constants: &[],
        debug_info: None,
    },
];

/// What var_dump() shows of a link: its properties' current values.
fn link_debug(o: &Object) -> PhpArray {
    let mut a = PhpArray::new();
    for (name, value) in link_props(o) {
        a.insert(Key::from_string(PhpStr::from(name)), value);
    }
    a
}

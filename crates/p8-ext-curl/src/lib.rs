#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's curl extension over rsurl, a pure-Rust curl: the CurlHandle,
//! CurlMultiHandle and CurlShareHandle classes, CURLFile and CURLStringFile,
//! and the curl_* functions. HTTP and HTTPS only (and file://): no FTP, SSH
//! or other protocols. A transfer blocks until it is done; curl_multi_exec()
//! runs every added handle to completion in one call.
//!
//! Options follow PHP's own classification of them (`optkinds`): every
//! option PHP accepts is validated the same way and kept, the ones rsurl can
//! act on take effect (see `exec`), the rest are accepted and ignored.
//!
//! Reference: $PHP_SRC_PATH/ext/curl (interface.c, multi.c, share.c, curl_file.c)

mod c;
mod constants;
mod cookie;
mod exec;
mod form;
mod info;
mod multi;
mod optkinds;
mod setopt;
mod strerror;

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassHooks, ClassSpec, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_FINAL, CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Native, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "curl", register };

type R = Result<Value, Throw>;

/// The libcurl this extension reports being (the one php:8.4-cli ships).
pub(crate) const LIBCURL_VERSION: &str = "8.14.1";
pub(crate) const LIBCURL_VERSION_NUM: i64 = 0x080e01;

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    registry.add_functions(multi::BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_classes(multi::CLASSES);
    registry.add_data("CurlHandle", || Rc::new(Handle::default()));
    registry.add_hooks("CurlHandle", ClassHooks { clone_data: Some(clone_handle), cast_int: Some(object_id), ..ClassHooks::default() });
    registry.add_data("CurlMultiHandle", || Rc::new(multi::Multi::default()));
    registry.add_hooks("CurlMultiHandle", ClassHooks { cast_int: Some(object_id), ..ClassHooks::default() });
    registry.add_data("CurlShareHandle", || Rc::new(multi::Share::default()));
    for (name, value) in constants::CONSTANTS {
        registry.add_constant(name.as_bytes(), Constant::Int(match value {
            Constant::Int(i) => *i,
            _ => 0,
        }));
    }
    registry.add_ini("curl.cainfo", None);
    // Callbacks run PHP code; setopt checks callables.
    registry.mark_reenters(&["curl_exec", "curl_setopt", "curl_setopt_array", "curl_multi_exec", "curl_multi_setopt", "curl_close", "curl_reset"]);
    registry.add_module("curl");
}

// ---- state ----

/// How the response body leaves (CURLOPT_RETURNTRANSFER / _FILE / _WRITEFUNCTION).
#[derive(Clone, Default)]
pub(crate) enum Sink {
    #[default]
    Stdout,
    Return,
    File(Value),
    User(Value),
}

/// Where the response headers go (CURLOPT_WRITEHEADER / _HEADERFUNCTION).
#[derive(Clone, Default)]
pub(crate) enum HeaderSink {
    #[default]
    Ignore,
    File(Value),
    User(Value),
}

/// Where an upload's bytes come from: CURLOPT_READFUNCTION when set, else
/// the stream CURLOPT_INFILE gave (which the function is also handed).
#[derive(Clone, Default)]
pub(crate) struct Source {
    pub func: Option<Value>,
    pub file: Option<Value>,
}

/// libcurl's request kind, as the last option that set it left it.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Req {
    #[default]
    Get,
    Post,
    Put,
    Head,
}

#[derive(Clone)]
pub(crate) struct State {
    /// Every accepted option, as validated (ints, strings, arrays of strings).
    pub opts: HashMap<i64, Value>,
    pub req: Req,
    pub write: Sink,
    pub header: HeaderSink,
    pub read: Source,
    /// CURLINFO_HEADER_OUT: keep the request header text.
    pub header_out: bool,
    pub debug: Option<Value>,
    pub private: Option<Value>,
    pub errno: i64,
    pub error: String,
    pub info: info::Info,
    /// What the last transfer returned (CURLOPT_RETURNTRANSFER).
    pub content: Option<Vec<u8>>,
    pub share: Option<Rc<Object>>,
    pub jar: Option<Rc<RefCell<rsurl::CookieJar>>>,
}

impl Default for State {
    fn default() -> State {
        let mut opts = HashMap::new();
        // _php_curl_set_default_options() and libcurl's own defaults.
        opts.insert(c::CURLOPT_MAXREDIRS, Value::Int(20));
        opts.insert(c::CURLOPT_SSL_VERIFYPEER, Value::Int(1));
        opts.insert(c::CURLOPT_SSL_VERIFYHOST, Value::Int(2));
        State {
            opts,
            req: Req::Get,
            write: Sink::Stdout,
            header: HeaderSink::Ignore,
            read: Source::default(),
            header_out: false,
            debug: None,
            private: None,
            errno: 0,
            error: String::new(),
            info: info::Info::default(),
            content: None,
            share: None,
            jar: None,
        }
    }
}

/// A CurlHandle's native state.
#[derive(Default)]
pub struct Handle {
    pub(crate) st: RefCell<State>,
    pub(crate) in_callback: Cell<bool>,
    /// Added to a CurlMultiHandle.
    pub(crate) in_multi: Cell<bool>,
}

/// `(int) $handle` (curl_cast_object): the object's id, as the resource
/// number it was before PHP 8 (Composer keys its transfers by it).
fn object_id(o: &Object) -> i64 {
    i64::from(o.id)
}

fn clone_handle(o: &Object) -> Rc<dyn std::any::Any> {
    let h = handle(o);
    Rc::new(Handle { st: RefCell::new(h.st.borrow().clone()), in_callback: Cell::new(false), in_multi: Cell::new(false) })
}

pub(crate) fn handle(o: &Object) -> &Handle {
    o.native::<Handle>().expect("CurlHandle state")
}

pub(crate) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

pub(crate) fn this_arg(args: &[Value], i: usize) -> Rc<Object> {
    match arg(args, i) {
        Value::Object(o) => o,
        _ => unreachable!("checked against the stub"),
    }
}

pub(crate) fn bytes_of(v: &Value) -> Vec<u8> {
    v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()
}

/// zval_get_string(): __toString() for an object (an Error without one),
/// "Array" with a warning for an array.
pub(crate) fn to_str(ctx: &mut dyn Ctx, v: &Value) -> Result<PhpStr, Throw> {
    match v.deref() {
        Value::Object(o) if o.class.cast_string.is_none() && o.class.flags & p8_runtime::object::CLASS_RESOURCE == 0 => {
            if o.class.method(b"__tostring").is_none() {
                return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(o.class.shown()))));
            }
            let method = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(o), Value::str("__toString")])));
            Ok(ctx.call(&method, &mut [])?.to_php_string().unwrap_or_else(PhpStr::empty))
        }
        Value::Array(_) => {
            ctx.warn("Array to string conversion");
            Ok(PhpStr::from("Array"))
        }
        other => Ok(other.to_php_string().unwrap_or_else(PhpStr::empty)),
    }
}

pub(crate) fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

pub(crate) fn new_handle(ctx: &mut dyn Ctx, state: Option<State>) -> Rc<Object> {
    let class = ctx.class(b"CurlHandle").expect("CurlHandle");
    let o = ctx.new_object(&class);
    if let Some(state) = state {
        return o.sibling(class, Some(Rc::new(Handle { st: RefCell::new(state), in_callback: Cell::new(false), in_multi: Cell::new(false) })));
    }
    o
}

// ---- functions ----

fn curl_init(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = new_handle(ctx, None);
    if let Value::Str(url) = arg(args, 0) {
        let ok = setopt::set(ctx, &o, c::CURLOPT_URL, &Value::Str(url), None)?;
        if !ok {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Object(o))
}

fn curl_setopt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args, 0);
    let option = arg(args, 1).to_int();
    let value = arg(args, 2);
    Ok(Value::Bool(setopt::set(ctx, &o, option, &value, None)?))
}

fn curl_setopt_array(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args, 0);
    let Value::Array(options) = arg(args, 1) else { unreachable!("checked against the stub") };
    for (key, value) in options.iter() {
        let Key::Int(option) = key else {
            let name = ctx.builtin_name();
            return Err(ctx.throw("ValueError", format!("{name}(): Argument #2 ($options) contains an invalid cURL option")));
        };
        if !setopt::set(ctx, &o, option, &value.deref(), Some(()))? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn curl_exec(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args, 0);
    let h = handle(&o);
    verify_handlers(ctx, h);
    {
        let mut st = h.st.borrow_mut();
        st.errno = 0;
        st.error.clear();
        st.content = None;
    }
    let outcome = exec::perform(ctx, &o)?;
    let mut st = h.st.borrow_mut();
    match outcome {
        Err(e) => {
            st.errno = e.code;
            st.error = e.message;
            st.content = None;
            Ok(Value::Bool(false))
        }
        Ok(()) => {
            let content = st.content.take();
            match st.write {
                Sink::Return => Ok(Value::Str(PhpStr::new(content.unwrap_or_default()))),
                _ => Ok(Value::Bool(true)),
            }
        }
    }
}

/// _php_curl_verify_handlers(): a CURLOPT_FILE (or header, or input) stream
/// that was closed since is dropped with a warning.
pub(crate) fn verify_handlers(ctx: &mut dyn Ctx, h: &Handle) {
    let closed = |v: &Value| p8_runtime::stream::resource_of(v).is_none_or(|r| p8_runtime::stream::resource(&r).is_none_or(|s| s.closed.get()));
    let mut warnings = Vec::new();
    {
        let mut st = h.st.borrow_mut();
        if let Some(v) = &st.read.file
            && closed(v)
        {
            warnings.push("CURLOPT_INFILE resource has gone away, resetting to default");
            st.read.file = None;
        }
        if let HeaderSink::File(v) = &st.header
            && closed(v)
        {
            warnings.push("CURLOPT_WRITEHEADER resource has gone away, resetting to default");
            st.header = HeaderSink::Ignore;
        }
        if let Sink::File(v) = &st.write
            && closed(v)
        {
            warnings.push("CURLOPT_FILE resource has gone away, resetting to default");
            st.write = Sink::Stdout;
        }
    }
    for w in warnings {
        ctx.warn(&format!("curl_exec(): {w}"));
    }
}

fn curl_getinfo(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args, 0);
    let st = handle(&o).st.borrow();
    Ok(match arg(args, 1) {
        Value::Null => info::all(&st),
        v => info::one(ctx, &st, v.to_int()),
    })
}

fn curl_error(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args, 0);
    let st = handle(&o).st.borrow();
    if st.errno == 0 {
        return Ok(Value::str(""));
    }
    Ok(Value::str(if st.error.is_empty() { strerror(st.errno) } else { st.error.as_str() }))
}

fn curl_errno(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::Int(handle(&this_arg(args, 0)).st.borrow().errno))
}

fn curl_close(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args, 0);
    if handle(&o).in_callback.get() {
        let name = ctx.builtin_name();
        return Err(ctx.throw("Error", format!("{name}(): Attempt to close cURL handle from a callback")));
    }
    Ok(Value::Null)
}

fn curl_reset(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args, 0);
    let h = handle(&o);
    if h.in_callback.get() {
        let name = ctx.builtin_name();
        return Err(ctx.throw("Error", format!("{name}(): Attempt to reset cURL handle from a callback")));
    }
    let mut st = h.st.borrow_mut();
    let fresh = State::default();
    st.opts = fresh.opts;
    st.req = Req::Get;
    st.write = Sink::Stdout;
    st.header = HeaderSink::Ignore;
    st.read = Source::default();
    st.header_out = false;
    st.debug = None;
    st.info = info::Info::default();
    st.jar = None;
    Ok(Value::Null)
}

fn curl_copy_handle(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args, 0);
    let state = handle(&o).st.borrow().clone();
    Ok(Value::Object(new_handle(ctx, Some(state))))
}

fn curl_version(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| a.insert(Key::from_string(PhpStr::from(k)), v);
    put("version_number", Value::Int(LIBCURL_VERSION_NUM));
    put("age", Value::Int(11));
    // CURL_VERSION_IPV6 | SSL | LIBZ | IDN | LARGEFILE | UNIX_SOCKETS (what rsurl has)
    put("features", Value::Int(1 | 4 | 8 | 128 | 512 | 1 << 19));
    let mut features = PhpArray::new();
    for (name, on) in [
        ("AsynchDNS", false),
        ("CharConv", false),
        ("Debug", false),
        ("GSS-Negotiate", false),
        ("IDN", true),
        ("IPv6", true),
        ("krb4", false),
        ("Largefile", true),
        ("libz", true),
        ("NTLM", false),
        ("NTLMWB", false),
        ("SPNEGO", false),
        ("SSL", true),
        ("SSPI", false),
        ("TLS-SRP", false),
        ("HTTP2", true),
        ("GSSAPI", false),
        ("KERBEROS5", false),
        ("UNIX_SOCKETS", true),
        ("PSL", false),
        ("HTTPS_PROXY", true),
        ("MULTI_SSL", false),
        ("BROTLI", true),
        ("ALTSVC", false),
        ("HTTP3", true),
        ("UNICODE", false),
        ("ZSTD", true),
        ("HSTS", false),
        ("GSASL", false),
    ] {
        features.insert(Key::from_string(PhpStr::from(name)), Value::Bool(on));
    }
    put("feature_list", Value::Array(Rc::new(features)));
    put("ssl_version_number", Value::Int(0));
    put("version", Value::str(LIBCURL_VERSION));
    put("host", Value::str(host_triple()));
    put("ssl_version", Value::str("purecrypto"));
    put("libz_version", Value::str("1.3.1"));
    put("protocols", Value::Array(Rc::new(PhpArray::from_list(["file", "http", "https"].iter().map(|p| Value::str(*p)).collect()))));
    put("ares", Value::str(""));
    put("ares_num", Value::Int(0));
    put("libidn", Value::str(""));
    put("iconv_ver_num", Value::Int(0));
    put("libssh_version", Value::str(""));
    put("brotli_ver_num", Value::Int(0x1010000));
    put("brotli_version", Value::str("brotli/1.1.0"));
    Ok(Value::Array(Rc::new(a)))
}

fn host_triple() -> String {
    let os = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        other => other,
    };
    format!("{}-{os}", std::env::consts::ARCH)
}

/// curl_easy_strerror().
pub(crate) fn strerror(code: i64) -> &'static str {
    strerror::EASY.iter().find(|(c, _)| *c == code).map_or("Unknown error", |(_, s)| s)
}

fn curl_strerror(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::str(strerror(arg(args, 0).to_int())))
}

/// curl_easy_escape(): everything but the unreserved characters.
fn curl_escape(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = bytes_of(&arg(args, 1));
    let mut out = Vec::with_capacity(s.len());
    for b in s {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b);
        } else {
            out.extend_from_slice(format!("%{b:02X}").as_bytes());
        }
    }
    Ok(Value::Str(PhpStr::new(out)))
}

/// curl_easy_unescape(): %XX sequences decoded, anything else kept.
fn curl_unescape(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = bytes_of(&arg(args, 1));
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%'
            && i + 2 < s.len()
            && let (Some(h), Some(l)) = ((s[i + 1] as char).to_digit(16), (s[i + 2] as char).to_digit(16))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    Ok(Value::Str(PhpStr::new(out)))
}

fn curl_pause(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::Int(0))
}

fn curl_upkeep(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    handle(&this_arg(args, 0)).st.borrow_mut().errno = 0;
    Ok(Value::Bool(true))
}

// ---- CurlHandle & friends: classes that only their functions make ----

fn no_construct(class: &'static str, func: &'static str) -> impl Fn(&mut dyn Ctx) -> Throw {
    move |ctx| ctx.throw("Error", format!("Cannot directly construct {class}, use {func}() instead"))
}

fn handle_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(no_construct("CurlHandle", "curl_init")(ctx))
}

// ---- CURLFile, CURLStringFile ----

fn curlfile_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    curlfile_fill(ctx, this, args);
    Ok(Value::Null)
}

fn curlfile_fill(_: &mut dyn Ctx, o: &Rc<Object>, args: &[Value]) {
    o.class.write(o, b"name", b"CURLFile", arg(args, 0));
    if let Value::Str(s) = arg(args, 1) {
        o.class.write(o, b"mime", b"CURLFile", Value::Str(s));
    }
    if let Value::Str(s) = arg(args, 2) {
        o.class.write(o, b"postname", b"CURLFile", Value::Str(s));
    }
}

fn curl_file_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let class = ctx.class(b"CURLFile").expect("CURLFile");
    let o = ctx.new_object(&class);
    curlfile_fill(ctx, &o, args);
    Ok(Value::Object(o))
}

fn cf_get(name: &'static [u8]) -> impl Fn(&Rc<Object>) -> Value {
    move |o| o.class.read(o, name, b"CURLFile")
}

fn cf_get_name(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(cf_get(b"name")(this))
}

fn cf_get_mime(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(cf_get(b"mime")(this))
}

fn cf_get_postname(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(cf_get(b"postname")(this))
}

fn cf_set_mime(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    this.class.write(this, b"mime", b"CURLFile", arg(args, 0));
    Ok(Value::Null)
}

fn cf_set_postname(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    this.class.write(this, b"postname", b"CURLFile", arg(args, 0));
    Ok(Value::Null)
}

fn cf_wakeup(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Exception", "Unserialization of 'CURLFile' is not allowed".into()))
}

fn csf_wakeup(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Exception", "Unserialization of 'CURLStringFile' is not allowed".into()))
}

fn csf_construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    this.class.write(this, b"data", b"CURLStringFile", arg(args, 0));
    this.class.write(this, b"postname", b"CURLStringFile", arg(args, 1));
    let mime = match arg(args, 2) {
        Value::Str(s) => Value::Str(s),
        _ => Value::str("application/octet-stream"),
    };
    this.class.write(this, b"mime", b"CURLStringFile", mime);
    Ok(Value::Null)
}

// ---- registration ----

static BUILTINS: &[Builtin] = &[
    Builtin { name: "curl_close", run: curl_close, by_ref: &[] },
    Builtin { name: "curl_copy_handle", run: curl_copy_handle, by_ref: &[] },
    Builtin { name: "curl_errno", run: curl_errno, by_ref: &[] },
    Builtin { name: "curl_error", run: curl_error, by_ref: &[] },
    Builtin { name: "curl_escape", run: curl_escape, by_ref: &[] },
    Builtin { name: "curl_exec", run: curl_exec, by_ref: &[] },
    Builtin { name: "curl_file_create", run: curl_file_create, by_ref: &[] },
    Builtin { name: "curl_getinfo", run: curl_getinfo, by_ref: &[] },
    Builtin { name: "curl_init", run: curl_init, by_ref: &[] },
    Builtin { name: "curl_pause", run: curl_pause, by_ref: &[] },
    Builtin { name: "curl_reset", run: curl_reset, by_ref: &[] },
    Builtin { name: "curl_setopt", run: curl_setopt, by_ref: &[] },
    Builtin { name: "curl_setopt_array", run: curl_setopt_array, by_ref: &[] },
    Builtin { name: "curl_strerror", run: curl_strerror, by_ref: &[] },
    Builtin { name: "curl_unescape", run: curl_unescape, by_ref: &[] },
    Builtin { name: "curl_upkeep", run: curl_upkeep, by_ref: &[] },
    Builtin { name: "curl_version", run: curl_version, by_ref: &[] },
];

const EMPTY: Constant = Constant::Str(b"");

static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "CurlHandle",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL | CLASS_NOT_SERIALIZABLE,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(handle_construct)))],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "CURLFile",
        parent: None,
        interfaces: &[],
        flags: CLASS_NOT_SERIALIZABLE,
        properties: &[("name", EMPTY, Visibility::Public), ("mime", EMPTY, Visibility::Public), ("postname", EMPTY, Visibility::Public)],
        methods: &[
            ("__construct", Some(Native::Method(curlfile_construct))),
            ("getFilename", Some(Native::Method(cf_get_name))),
            ("getMimeType", Some(Native::Method(cf_get_mime))),
            ("getPostFilename", Some(Native::Method(cf_get_postname))),
            ("setMimeType", Some(Native::Method(cf_set_mime))),
            ("setPostFilename", Some(Native::Method(cf_set_postname))),
            ("__wakeup", Some(Native::Method(cf_wakeup))),
        ],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "CURLStringFile",
        parent: None,
        interfaces: &[],
        flags: CLASS_NOT_SERIALIZABLE,
        properties: &[("data", Constant::Null, Visibility::Public), ("postname", Constant::Null, Visibility::Public), ("mime", Constant::Null, Visibility::Public)],
        methods: &[("__construct", Some(Native::Method(csf_construct))), ("__wakeup", Some(Native::Method(csf_wakeup)))],
        constants: &[],
        debug_info: None,
    },
];

pub(crate) const UNCLONEABLE_FINAL: u32 = CLASS_FINAL | CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE;

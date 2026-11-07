//! curl_setopt(): _php_curl_setopt(), option by option. Every option is
//! validated as PHP validates it and kept in the handle; exec.rs acts on
//! the ones rsurl can honour.

use crate::optkinds::{KINDS, Kind, NAMES};
use crate::{HeaderSink, Req, Sink, bytes_of, c, handle, to_str};
use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::stream::{resource, resource_of};
use p8_runtime::value::Value;
use std::rc::Rc;

fn kind_of(option: i64) -> Option<Kind> {
    KINDS.binary_search_by_key(&option, |(n, _)| *n).ok().map(|i| KINDS[i].1)
}

fn name_of(option: i64) -> &'static str {
    NAMES.binary_search_by_key(&option, |(n, _)| *n).map_or("?", |i| NAMES[i].1)
}

/// The file-handle options' stream: Err for what PHP's
/// zend_fetch_resource2_ex() rejects, Ok(None) for null.
fn file_handle(ctx: &mut dyn Ctx, v: &Value) -> Result<Option<Rc<Object>>, Throw> {
    if v.is_null() {
        return Ok(None);
    }
    let name = ctx.builtin_name();
    let Some(r) = resource_of(v) else {
        return Err(ctx.throw("TypeError", format!("{name}(): supplied argument is not a valid File-Handle resource")));
    };
    match resource(&r) {
        Some(s) if !s.closed.get() && s.kind == "stream" => Ok(Some(r)),
        _ => Err(ctx.throw("TypeError", format!("{name}(): supplied resource is not a valid File-Handle resource"))),
    }
}

fn writable(r: &Object) -> bool {
    resource(r).is_some_and(|s| {
        let mode = s.stream.borrow().mode.clone();
        let b = mode.as_bytes();
        b.first() != Some(&b'r') || b.get(1) == Some(&b'+')
    })
}

fn string_of(ctx: &mut dyn Ctx, v: &Value) -> Result<Value, Throw> {
    Ok(Value::str(to_str(ctx, v)?))
}

/// _php_curl_setopt(): Ok(false) is PHP's FAILURE without an exception.
pub(crate) fn set(ctx: &mut dyn Ctx, o: &Rc<Object>, option: i64, value: &Value, array: Option<()>) -> Result<bool, Throw> {
    let h = handle(o);
    let func = ctx.builtin_name();
    let Some(kind) = kind_of(option) else {
        let msg = if array.is_some() { format!("{func}(): Argument #2 ($options) must contain only valid cURL options") } else { format!("{func}(): Argument #2 ($option) is not a valid cURL option") };
        return Err(ctx.throw("ValueError", msg));
    };
    let (argn, argname) = if array.is_some() { (2, "options") } else { (3, "value") };
    // open_basedir keeps file:// out of the protocols.
    let basedir = ctx.ini_get(b"open_basedir").is_some_and(|v| !bytes_of(&v).is_empty());
    if basedir {
        if (option == c::CURLOPT_PROTOCOLS || option == c::CURLOPT_REDIR_PROTOCOLS) && value.to_int() & (1 << 10) != 0 {
            ctx.warn(&format!("{func}(): CURLPROTO_FILE cannot be activated when an open_basedir is set"));
            return Ok(false);
        }
        if option == c::CURLOPT_PROTOCOLS_STR || option == c::CURLOPT_REDIR_PROTOCOLS_STR {
            let list = String::from_utf8_lossy(&bytes_of(value)).to_ascii_lowercase();
            if list.contains("file") || list.contains("all") {
                ctx.warn(&format!("{func}(): The FILE protocol cannot be activated when an open_basedir is set"));
                return Ok(false);
            }
        }
    }
    let nul_check = |ctx: &mut dyn Ctx, v: &Value| -> Result<(), Throw> {
        if bytes_of(v).contains(&0) { Err(ctx.throw("ValueError", format!("{func}(): cURL option must not contain any null bytes"))) } else { Ok(()) }
    };
    if option == c::CURLOPT_COOKIELIST {
        let cmd = to_str(ctx, value)?;
        crate::cookie::command(&mut h.st.borrow_mut(), cmd.as_bytes());
        h.st.borrow_mut().errno = 0;
        return Ok(true);
    }
    match kind {
        Kind::Long => {
            let n = value.to_int();
            let mut st = h.st.borrow_mut();
            match option {
                c::CURLOPT_POST => st.req = if n != 0 { Req::Post } else if st.req == Req::Post { Req::Get } else { st.req },
                c::CURLOPT_HTTPGET => {
                    if n != 0 {
                        st.req = Req::Get
                    }
                }
                c::CURLOPT_NOBODY => st.req = if n != 0 { Req::Head } else if st.req == Req::Head { Req::Get } else { st.req },
                c::CURLOPT_UPLOAD | c::CURLOPT_PUT => st.req = if n != 0 { Req::Put } else if st.req == Req::Put { Req::Get } else { st.req },
                _ => {}
            }
            st.opts.insert(option, Value::int(n));
        }
        Kind::VerifyHost => {
            let mut n = value.to_int();
            if n == 1 {
                ctx.notice(&format!("{func}(): CURLOPT_SSL_VERIFYHOST no longer accepts the value 1, value 2 will be used instead"));
                n = 2;
            }
            h.st.borrow_mut().opts.insert(option, Value::int(n));
        }
        Kind::Follow => {
            h.st.borrow_mut().opts.insert(option, Value::int(value.to_bool() as i64));
        }
        Kind::Str | Kind::PathStr | Kind::Url | Kind::Blob => {
            let s = string_of(ctx, value)?;
            if kind != Kind::Blob {
                nul_check(ctx, &s)?;
            }
            h.st.borrow_mut().opts.insert(option, s);
        }
        Kind::NStr => {
            if value.is_null() {
                h.st.borrow_mut().opts.remove(&option);
            } else {
                let s = string_of(ctx, value)?;
                nul_check(ctx, &s)?;
                h.st.borrow_mut().opts.insert(option, s);
            }
        }
        Kind::List => {
            let Some(a) = value.as_array() else {
                let name = name_of(option);
                return Err(ctx.throw("TypeError", format!("{func}(): The {name} option must have an array value")));
            };
            let mut items = Vec::new();
            for v in a.values() {
                items.push(string_of(ctx, &v.deref())?);
            }
            h.st.borrow_mut().opts.insert(option, Value::array(PhpArray::from_list(items)));
        }
        Kind::Ignore => {}
        Kind::PostFields => {
            let v = match value.as_array() {
                Some(a) if a.is_empty() => Value::str(""),
                Some(_) => value.clone(),
                None => string_of(ctx, value)?,
            };
            let mut st = h.st.borrow_mut();
            st.req = Req::Post;
            st.opts.insert(option, v);
        }
        Kind::ReturnTransfer => {
            h.st.borrow_mut().write = if value.to_bool() { Sink::Return } else { Sink::Stdout };
        }
        Kind::PostRedir => {
            h.st.borrow_mut().opts.insert(option, Value::int(value.to_int() & 7));
        }
        Kind::SafeUpload => {
            if !value.to_bool() {
                return Err(ctx.throw("ValueError", format!("{func}(): Disabling safe uploads is no longer supported")));
            }
        }
        Kind::Private => {
            h.st.borrow_mut().private = Some(value.clone());
            return Ok(true);
        }
        Kind::HeaderOut => {
            if h.st.borrow().debug.is_some() {
                return Err(ctx.throw("ValueError", "CURLINFO_HEADER_OUT option must not be set when the CURLOPT_DEBUGFUNCTION option is set".into()));
            }
            h.st.borrow_mut().header_out = value.to_bool();
        }
        Kind::Share => {
            if let Some(s) = value.as_object()
                && s.class.lower == b"curlsharehandle"
            {
                h.st.borrow_mut().share = Some(Rc::clone(s));
            }
        }
        Kind::File | Kind::WriteHeader | Kind::InFile | Kind::StdErr => {
            let stream = file_handle(ctx, value)?;
            let mut st = h.st.borrow_mut();
            match (kind, stream) {
                (Kind::File, None) => st.write = Sink::Stdout,
                (Kind::WriteHeader, None) => st.header = HeaderSink::Ignore,
                (Kind::InFile, None) => st.read.file = None,
                (Kind::StdErr, None) => {
                    st.opts.remove(&option);
                }
                (Kind::InFile, Some(_)) => st.read.file = Some(value.clone()),
                (k, Some(r)) => {
                    if !writable(&r) {
                        drop(st);
                        return Err(ctx.throw("ValueError", format!("{func}(): The provided file handle must be writable")));
                    }
                    match k {
                        Kind::File => st.write = Sink::File(value.clone()),
                        Kind::WriteHeader => st.header = HeaderSink::File(value.clone()),
                        _ => {
                            st.opts.insert(option, value.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        Kind::WriteFn | Kind::HeaderFn | Kind::ReadFn | Kind::Callable | Kind::DebugFn => {
            let default = |h: &crate::Handle| {
                let mut st = h.st.borrow_mut();
                match kind {
                    Kind::WriteFn => st.write = Sink::Stdout,
                    Kind::HeaderFn => st.header = HeaderSink::Ignore,
                    Kind::ReadFn => st.read.func = None,
                    Kind::DebugFn => st.debug = None,
                    _ => {
                        st.opts.remove(&option);
                    }
                }
            };
            if value.is_null() {
                default(h);
                return Ok(true);
            }
            if !ctx.is_callable(value) {
                default(h);
                let why = p8_runtime::args::callable_error(ctx, value);
                let name = name_of(option);
                return Err(ctx.throw("TypeError", format!("{func}(): Argument #{argn} (${argname}) must be a valid callback for option {name}, {why}")));
            }
            let mut st = h.st.borrow_mut();
            match kind {
                Kind::WriteFn => st.write = Sink::User(value.clone()),
                Kind::HeaderFn => st.header = HeaderSink::User(value.clone()),
                Kind::ReadFn => st.read.func = Some(value.clone()),
                Kind::DebugFn => {
                    st.debug = Some(value.clone());
                }
                _ => {
                    st.opts.insert(option, value.clone());
                }
            }
        }
    }
    h.st.borrow_mut().errno = 0;
    Ok(true)
}

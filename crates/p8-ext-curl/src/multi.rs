//! curl_multi_* and curl_share_*. rsurl blocks, so a multi handle runs every
//! handle added to it to completion in its first curl_multi_exec() and
//! reports nothing still running; curl_multi_info_read() then gives each
//! result.

use crate::{Handle, Sink, UNCLONEABLE_FINAL, arg, c, exec, handle, strerror, this_arg, verify_handlers};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Ctx, Throw};
use p8_runtime::object::{Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

type R = Result<Value, Throw>;

const CURLM_OK: i64 = 0;
const CURLM_UNKNOWN_OPTION: i64 = 6;
const CURLM_ADDED_ALREADY: i64 = 7;

#[derive(Default)]
pub struct Multi {
    easy: RefCell<Vec<Rc<Object>>>,
    /// Added and not run yet.
    waiting: RefCell<Vec<Rc<Object>>>,
    /// Finished: the handle and its CURLE_* code.
    done: RefCell<VecDeque<(Rc<Object>, i64)>>,
    errno: Cell<i64>,
}

#[derive(Default)]
pub struct Share {
    errno: Cell<i64>,
}

fn multi(o: &Object) -> &Multi {
    o.native::<Multi>().expect("CurlMultiHandle state")
}

fn share(o: &Object) -> &Share {
    o.native::<Share>().expect("CurlShareHandle state")
}

fn set_ref(v: &Value, n: i64) {
    if let Value::Ref(cell) = v {
        *cell.borrow_mut() = Value::Int(n);
    }
}

fn curl_multi_init(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let class = ctx.class(b"CurlMultiHandle").expect("CurlMultiHandle");
    Ok(Value::Object(ctx.new_object(&class)))
}

fn curl_multi_add_handle(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mh = this_arg(args, 0);
    let ch = this_arg(args, 1);
    let m = multi(&mh);
    verify_handlers(ctx, handle(&ch));
    {
        let mut st = handle(&ch).st.borrow_mut();
        st.errno = 0;
        st.error.clear();
    }
    if handle(&ch).in_multi.get() {
        m.errno.set(CURLM_ADDED_ALREADY);
        return Ok(Value::Int(CURLM_ADDED_ALREADY));
    }
    handle(&ch).in_multi.set(true);
    m.easy.borrow_mut().push(Rc::clone(&ch));
    m.waiting.borrow_mut().push(ch);
    m.errno.set(CURLM_OK);
    Ok(Value::Int(CURLM_OK))
}

fn curl_multi_remove_handle(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mh = this_arg(args, 0);
    let ch = this_arg(args, 1);
    let m = multi(&mh);
    m.easy.borrow_mut().retain(|e| !Rc::ptr_eq(e, &ch));
    m.waiting.borrow_mut().retain(|e| !Rc::ptr_eq(e, &ch));
    m.done.borrow_mut().retain(|(e, _)| !Rc::ptr_eq(e, &ch));
    handle(&ch).in_multi.set(false);
    m.errno.set(CURLM_OK);
    Ok(Value::Int(CURLM_OK))
}

fn curl_multi_exec(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mh = this_arg(args, 0);
    let m = multi(&mh);
    let easy: Vec<Rc<Object>> = m.easy.borrow().clone();
    for ch in &easy {
        verify_handlers(ctx, handle(ch));
    }
    let waiting: Vec<Rc<Object>> = std::mem::take(&mut *m.waiting.borrow_mut());
    for ch in waiting {
        let h: &Handle = handle(&ch);
        {
            let mut st = h.st.borrow_mut();
            st.errno = 0;
            st.error.clear();
            st.content = None;
        }
        let outcome = exec::perform(ctx, &ch)?;
        let code = {
            let mut st = h.st.borrow_mut();
            match outcome {
                Ok(()) => 0,
                Err(e) => {
                    st.errno = e.code;
                    st.error = e.message;
                    e.code
                }
            }
        };
        m.done.borrow_mut().push_back((ch, code));
    }
    if let Some(r) = args.get(1) {
        set_ref(r, 0);
    }
    m.errno.set(CURLM_OK);
    Ok(Value::Int(CURLM_OK))
}

fn curl_multi_select(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    if let Some(v) = args.get(1) {
        let t = v.deref().to_float();
        if !(0.0..=(i32::MAX as f64 / 1000.0)).contains(&t) {
            return Err(ctx.throw("ValueError", "curl_multi_select(): Argument #2 ($timeout) must be between 0 and 2147484".into()));
        }
    }
    Ok(Value::Int(0))
}

fn curl_multi_getcontent(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let ch = this_arg(args, 0);
    let st = handle(&ch).st.borrow();
    Ok(match st.write {
        Sink::Return => Value::Str(PhpStr::new(st.content.clone().unwrap_or_default())),
        _ => Value::Null,
    })
}

fn curl_multi_info_read(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mh = this_arg(args, 0);
    let m = multi(&mh);
    let Some((ch, code)) = m.done.borrow_mut().pop_front() else { return Ok(Value::Bool(false)) };
    if let Some(r) = args.get(1) {
        set_ref(r, m.done.borrow().len() as i64);
    }
    let mut a = PhpArray::new();
    a.insert(Key::from_string(PhpStr::from("msg")), Value::Int(1));
    a.insert(Key::from_string(PhpStr::from("result")), Value::Int(code));
    a.insert(Key::from_string(PhpStr::from("handle")), Value::Object(ch));
    Ok(Value::Array(Rc::new(a)))
}

fn curl_multi_close(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mh = this_arg(args, 0);
    let m = multi(&mh);
    let easy: Vec<Rc<Object>> = std::mem::take(&mut *m.easy.borrow_mut());
    for ch in easy {
        verify_handlers(ctx, handle(&ch));
        handle(&ch).in_multi.set(false);
    }
    m.waiting.borrow_mut().clear();
    m.done.borrow_mut().clear();
    Ok(Value::Null)
}

fn curl_multi_errno(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::Int(multi(&this_arg(args, 0)).errno.get()))
}

fn curl_multi_strerror(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let code = arg(args, 0).to_int();
    Ok(strerror::MULTI.iter().find(|(c, _)| *c == code).map_or(Value::str("Unknown error"), |(_, s)| Value::str(*s)))
}

fn curl_multi_setopt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mh = this_arg(args, 0);
    let m = multi(&mh);
    let option = arg(args, 1).to_int();
    let value = arg(args, 2);
    match option {
        c::CURLMOPT_PIPELINING | c::CURLMOPT_MAXCONNECTS | c::CURLMOPT_CHUNK_LENGTH_PENALTY_SIZE | c::CURLMOPT_CONTENT_LENGTH_PENALTY_SIZE | c::CURLMOPT_MAX_HOST_CONNECTIONS | c::CURLMOPT_MAX_PIPELINE_LENGTH | c::CURLMOPT_MAX_TOTAL_CONNECTIONS | c::CURLMOPT_MAX_CONCURRENT_STREAMS => {
            if option == c::CURLMOPT_PIPELINING && value.to_int() & 1 != 0 {
                ctx.warn("curl_multi_setopt(): CURLPIPE_HTTP1 is no longer supported");
            }
        }
        c::CURLMOPT_PUSHFUNCTION => {
            if !ctx.is_callable(&value) {
                let why = crate::setopt::callable_error(ctx, &value);
                return Err(ctx.throw("TypeError", format!("curl_multi_setopt(): Argument #2 ($option) must be a valid callback for option CURLMOPT_PUSHFUNCTION, {why}")));
            }
        }
        _ => {
            m.errno.set(CURLM_UNKNOWN_OPTION);
            return Err(ctx.throw("ValueError", "curl_multi_setopt(): Argument #2 ($option) is not a valid cURL multi option".into()));
        }
    }
    m.errno.set(CURLM_OK);
    Ok(Value::Bool(true))
}

fn multi_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Cannot directly construct CurlMultiHandle, use curl_multi_init() instead".into()))
}

// ---- share ----

fn curl_share_init(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let class = ctx.class(b"CurlShareHandle").expect("CurlShareHandle");
    Ok(Value::Object(ctx.new_object(&class)))
}

fn curl_share_close(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::Null)
}

fn curl_share_setopt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let sh = this_arg(args, 0);
    let s = share(&sh);
    let option = arg(args, 1).to_int();
    let value = arg(args, 2).to_int();
    if option != c::CURLSHOPT_SHARE && option != c::CURLSHOPT_UNSHARE {
        s.errno.set(1);
        return Err(ctx.throw("ValueError", "curl_share_setopt(): Argument #2 ($option) is not a valid cURL share option".into()));
    }
    // CURL_LOCK_DATA_COOKIE .. CURL_LOCK_DATA_PSL
    let ok = (2..=6).contains(&value);
    s.errno.set(if ok { 0 } else { 1 });
    Ok(Value::Bool(ok))
}

fn curl_share_errno(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::Int(share(&this_arg(args, 0)).errno.get()))
}

fn curl_share_strerror(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let code = arg(args, 0).to_int();
    Ok(Value::str(strerror::SHARE.iter().find(|(c, _)| *c == code).map_or("CURLSHcode unknown", |(_, s)| s)))
}

fn share_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Cannot directly construct CurlShareHandle, use curl_share_init() instead".into()))
}

pub(crate) static BUILTINS: &[Builtin] = &[
    Builtin { name: "curl_multi_add_handle", run: curl_multi_add_handle, by_ref: &[] },
    Builtin { name: "curl_multi_close", run: curl_multi_close, by_ref: &[] },
    Builtin { name: "curl_multi_errno", run: curl_multi_errno, by_ref: &[] },
    Builtin { name: "curl_multi_exec", run: curl_multi_exec, by_ref: &[1] },
    Builtin { name: "curl_multi_getcontent", run: curl_multi_getcontent, by_ref: &[] },
    Builtin { name: "curl_multi_info_read", run: curl_multi_info_read, by_ref: &[1] },
    Builtin { name: "curl_multi_init", run: curl_multi_init, by_ref: &[] },
    Builtin { name: "curl_multi_remove_handle", run: curl_multi_remove_handle, by_ref: &[] },
    Builtin { name: "curl_multi_select", run: curl_multi_select, by_ref: &[] },
    Builtin { name: "curl_multi_setopt", run: curl_multi_setopt, by_ref: &[] },
    Builtin { name: "curl_multi_strerror", run: curl_multi_strerror, by_ref: &[] },
    Builtin { name: "curl_share_close", run: curl_share_close, by_ref: &[] },
    Builtin { name: "curl_share_errno", run: curl_share_errno, by_ref: &[] },
    Builtin { name: "curl_share_init", run: curl_share_init, by_ref: &[] },
    Builtin { name: "curl_share_setopt", run: curl_share_setopt, by_ref: &[] },
    Builtin { name: "curl_share_strerror", run: curl_share_strerror, by_ref: &[] },
];

pub(crate) static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "CurlMultiHandle",
        parent: None,
        interfaces: &[],
        flags: UNCLONEABLE_FINAL,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(multi_construct)))],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "CurlShareHandle",
        parent: None,
        interfaces: &[],
        flags: UNCLONEABLE_FINAL,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(share_construct)))],
        constants: &[],
        debug_info: None,
    },
];

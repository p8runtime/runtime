//! SessionHandlerInterface, SessionIdInterface,
//! SessionUpdateTimestampHandlerInterface and SessionHandler (which wraps
//! the save handler that was in effect before a user one was set).
//!
//! Reference: $PHP_SRC_PATH/ext/session/mod_user_class.c

use crate::life::{self, Res};
use crate::state::{Module, Shared, Status, state};
use p8_runtime::ext::{ClassSpec, Ctx};
use p8_runtime::object::{CLASS_INTERFACE, Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

type R = Res<Value>;

macro_rules! m {
    ($name:literal, $f:ident) => {
        ($name, Some(Native::Method($f)))
    };
}

pub(crate) static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "SessionHandlerInterface",
        parent: None,
        interfaces: &[],
        flags: CLASS_INTERFACE,
        properties: &[],
        methods: &[("open", None), ("close", None), ("read", None), ("write", None), ("destroy", None), ("gc", None)],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "SessionIdInterface",
        parent: None,
        interfaces: &[],
        flags: CLASS_INTERFACE,
        properties: &[],
        methods: &[("create_sid", None)],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "SessionUpdateTimestampHandlerInterface",
        parent: None,
        interfaces: &[],
        flags: CLASS_INTERFACE,
        properties: &[],
        methods: &[("validateId", None), ("updateTimestamp", None)],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "SessionHandler",
        parent: None,
        interfaces: &["SessionHandlerInterface", "SessionIdInterface"],
        flags: 0,
        properties: &[],
        methods: &[
            m!("open", open),
            m!("close", close),
            m!("read", read),
            m!("write", write),
            m!("destroy", destroy),
            m!("gc", gc),
            m!("create_sid", create_sid),
        ],
        constants: &[],
        debug_info: None,
    },
];

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    args.get(i).map(Value::deref).and_then(|v| v.to_php_string()).map(|s| s.as_bytes().to_vec()).unwrap_or_default()
}

/// PS_SANITY_CHECK: the module to call.
fn sanity(ctx: &mut dyn Ctx) -> Res<(Shared, Module)> {
    let s = state(ctx);
    if s.borrow().status != Status::Active {
        return Err(ctx.throw("Error", "Session is not active".into()));
    }
    let Some(m) = s.borrow().default_mod else {
        return Err(ctx.throw("Error", "Cannot call default session handler".into()));
    };
    Ok((s, m))
}

/// PS_SANITY_CHECK_IS_OPEN: Ok(None) after the warning.
fn sanity_open(ctx: &mut dyn Ctx, method: &str) -> Res<Option<(Shared, Module)>> {
    let (s, m) = sanity(ctx)?;
    if !s.borrow().user_is_open {
        ctx.warn(&format!("SessionHandler::{method}(): Parent session handler is not open"));
        return Ok(None);
    }
    Ok(Some((s, m)))
}

fn open(ctx: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> R {
    let (s, m) = sanity(ctx)?;
    let ok = life::mod_open(ctx, &s, m, &bytes(args, 0), &bytes(args, 1));
    if ok.is_err() {
        s.borrow_mut().status = Status::None;
    }
    let ok = ok?;
    if ok {
        s.borrow_mut().user_is_open = true;
    }
    Ok(Value::Bool(ok))
}

fn close(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    let Some((s, m)) = sanity_open(ctx, "close")? else { return Ok(Value::Bool(false)) };
    s.borrow_mut().user_is_open = false;
    let r = life::mod_close(ctx, &s, m);
    if r.is_err() {
        s.borrow_mut().status = Status::None;
    }
    Ok(Value::Bool(r?))
}

fn read(ctx: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> R {
    let Some((s, m)) = sanity_open(ctx, "read")? else { return Ok(Value::Bool(false)) };
    Ok(match life::mod_read(ctx, &s, m, &bytes(args, 0))? {
        Some(v) => Value::Str(PhpStr::new(v)),
        None => Value::Bool(false),
    })
}

fn write(ctx: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> R {
    let Some((s, m)) = sanity_open(ctx, "write")? else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(life::mod_write(ctx, &s, m, &bytes(args, 0), &bytes(args, 1))?))
}

fn destroy(ctx: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> R {
    let Some((s, m)) = sanity_open(ctx, "destroy")? else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(life::mod_destroy(ctx, &s, m, &bytes(args, 0))?))
}

fn gc(ctx: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> R {
    let Some((s, m)) = sanity_open(ctx, "gc")? else { return Ok(Value::Bool(false)) };
    let lifetime = args.first().map_or(0, |v| v.deref().to_int());
    let n = life::mod_gc(ctx, &s, m, lifetime)?;
    Ok(if n == -1 { Value::Bool(false) } else { Value::Int(n) })
}

fn create_sid(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    let (s, m) = sanity(ctx)?;
    match life::mod_create_sid(ctx, &s, m)? {
        Some(id) => Ok(Value::Str(PhpStr::new(id))),
        None => Err(ctx.throw("Error", format!("Failed to create session ID: {}", m.name()))),
    }
}

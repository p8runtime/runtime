//! The user save handler: the callables session_set_save_handler() was
//! given, called with PHP's checks on what they return.
//!
//! Reference: $PHP_SRC_PATH/ext/session/mod_user.c

use crate::state::{Shared, deprecated, warn};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::value::Value;

/// ps_call_handler(): the handler's result; None when it did not run (a
/// handler called from inside a handler), an Err for what it threw.
fn call_handler(ctx: &mut dyn Ctx, s: &Shared, func: &Value, args: &mut [Value]) -> Result<Option<Value>, Throw> {
    if s.borrow().in_save_handler {
        s.borrow_mut().in_save_handler = false;
        warn(ctx, "Cannot call session save handler in a recursive manner");
        return Ok(None);
    }
    s.borrow_mut().in_save_handler = true;
    let r = ctx.call(func, args);
    s.borrow_mut().in_save_handler = false;
    match r {
        Ok(v) if v.is_undef() => Ok(Some(Value::NULL)),
        Ok(v) => Ok(Some(v.deref())),
        Err(t) => Err(t),
    }
}

/// verify_bool_return_type_userland_calls()
fn verify_bool(ctx: &mut dyn Ctx, ret: &Option<Value>) -> Result<bool, Throw> {
    let Some(value) = ret else { return Ok(false) };
    if let Some(b) = value.as_bool() {
        return Ok(b);
    }
    match value.as_int() {
        Some(-1) => {
            deprecated(ctx, &format!("Session callback must have a return value of type bool, {} returned", value.type_name()));
            Ok(false)
        }
        Some(0) => {
            deprecated(ctx, &format!("Session callback must have a return value of type bool, {} returned", value.type_name()));
            Ok(true)
        }
        _ => Err(ctx.throw("TypeError", format!("Session callback must have a return value of type bool, {} returned", value.type_name()))),
    }
}

fn handler(s: &Shared, pick: impl Fn(&crate::state::Handlers) -> &Option<Value>) -> Option<Value> {
    pick(&s.borrow().names).clone()
}

fn string(b: &[u8]) -> Value {
    Value::str(p8_runtime::string::PhpStr::from(b))
}

/// PS_OPEN_FUNC(user)
pub(crate) fn open(ctx: &mut dyn Ctx, s: &Shared, save_path: &[u8], name: &[u8]) -> Result<bool, Throw> {
    let Some(f) = handler(s, |n| &n.open) else { return Ok(false) };
    let r = call_handler(ctx, s, &f, &mut [string(save_path), string(name)]);
    if r.is_err() {
        s.borrow_mut().status = crate::state::Status::None;
    }
    let r = r?;
    s.borrow_mut().user_implemented = true;
    verify_bool(ctx, &r)
}

/// PS_CLOSE_FUNC(user)
pub(crate) fn close(ctx: &mut dyn Ctx, s: &Shared) -> Result<bool, Throw> {
    if !s.borrow().user_implemented {
        // already closed
        return Ok(true);
    }
    let Some(f) = handler(s, |n| &n.close) else { return Ok(false) };
    let r = call_handler(ctx, s, &f, &mut []);
    s.borrow_mut().user_implemented = false;
    verify_bool(ctx, &r?)
}

/// PS_READ_FUNC(user)
pub(crate) fn read(ctx: &mut dyn Ctx, s: &Shared, key: &[u8]) -> Result<Option<Vec<u8>>, Throw> {
    let Some(f) = handler(s, |n| &n.read) else { return Ok(None) };
    Ok(call_handler(ctx, s, &f, &mut [string(key)])?.as_ref().and_then(Value::as_str).map(|v| v.as_bytes().to_vec()))
}

/// PS_WRITE_FUNC(user)
pub(crate) fn write(ctx: &mut dyn Ctx, s: &Shared, key: &[u8], val: &[u8]) -> Result<bool, Throw> {
    let Some(f) = handler(s, |n| &n.write) else { return Ok(false) };
    let r = call_handler(ctx, s, &f, &mut [string(key), string(val)])?;
    verify_bool(ctx, &r)
}

/// PS_DESTROY_FUNC(user)
pub(crate) fn destroy(ctx: &mut dyn Ctx, s: &Shared, key: &[u8]) -> Result<bool, Throw> {
    let Some(f) = handler(s, |n| &n.destroy) else { return Ok(false) };
    let r = call_handler(ctx, s, &f, &mut [string(key)])?;
    verify_bool(ctx, &r)
}

/// PS_GC_FUNC(user): the number of sessions removed, -1 for an error.
pub(crate) fn gc(ctx: &mut dyn Ctx, s: &Shared, maxlifetime: i64) -> Result<i64, Throw> {
    let Some(f) = handler(s, |n| &n.gc) else { return Ok(-1) };
    let r = call_handler(ctx, s, &f, &mut [Value::int(maxlifetime)])?;
    Ok(match r.as_ref().map(|v| (v.as_int(), v.as_bool())) {
        Some((Some(n), _)) => n,
        // for older API compatibility
        Some((_, Some(true))) => 1,
        _ => -1,
    })
}

/// PS_CREATE_SID_FUNC(user): None after an Err-less failure is never
/// returned; a handler that is no good throws.
pub(crate) fn create_sid(ctx: &mut dyn Ctx, s: &Shared) -> Result<Option<Vec<u8>>, Throw> {
    let Some(f) = handler(s, |n| &n.create_sid) else {
        // function as defined by PS_MOD
        return crate::life::create_id(ctx).map(Some);
    };
    match call_handler(ctx, s, &f, &mut [])? {
        None => Err(ctx.throw("Error", "No session id returned by function".into())),
        Some(v) if v.is_str() => Ok(Some(v.as_str().expect("a string").as_bytes().to_vec())),
        Some(_) => Err(ctx.throw("Error", "Session id must be a string".into())),
    }
}

/// PS_VALIDATE_SID_FUNC(user)
pub(crate) fn validate_sid(ctx: &mut dyn Ctx, s: &Shared, key: &[u8]) -> Result<bool, Throw> {
    let Some(f) = handler(s, |n| &n.validate_sid) else {
        // a dummy function: any id is valid
        return Ok(true);
    };
    let r = call_handler(ctx, s, &f, &mut [string(key)])?;
    verify_bool(ctx, &r)
}

/// PS_UPDATE_TIMESTAMP_FUNC(user)
pub(crate) fn update_timestamp(ctx: &mut dyn Ctx, s: &Shared, key: &[u8], val: &[u8]) -> Result<bool, Throw> {
    let Some(f) = handler(s, |n| &n.update_timestamp).or_else(|| handler(s, |n| &n.write)) else { return Ok(false) };
    let r = call_handler(ctx, s, &f, &mut [string(key), string(val)])?;
    verify_bool(ctx, &r)
}

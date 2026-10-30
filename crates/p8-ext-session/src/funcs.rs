//! The session_* functions.
//!
//! Reference: $PHP_SRC_PATH/ext/session/session.c (the PHP_FUNCTIONs)

use crate::life::{self, Res};
use crate::state::{Module, Shared, Status, deprecated, flag, headers_sent, int, state, text, warn};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx};
use p8_runtime::object::Object;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

type R = Res<Value>;

macro_rules! f {
    ($name:literal, $run:ident) => {
        Builtin { name: $name, run: $run, by_ref: &[] }
    };
}

pub(crate) static BUILTINS: &[Builtin] = &[
    f!("session_name", session_name),
    f!("session_module_name", session_module_name),
    f!("session_save_path", session_save_path),
    f!("session_id", session_id),
    f!("session_create_id", session_create_id),
    f!("session_regenerate_id", session_regenerate_id),
    f!("session_decode", session_decode),
    f!("session_encode", session_encode),
    f!("session_destroy", session_destroy),
    f!("session_unset", session_unset),
    f!("session_gc", session_gc),
    f!("session_get_cookie_params", session_get_cookie_params),
    f!("session_write_close", session_write_close),
    f!("session_commit", session_write_close),
    f!("session_abort", session_abort),
    f!("session_reset", session_reset),
    f!("session_status", session_status),
    f!("session_register_shutdown", session_register_shutdown),
    f!("session_set_save_handler", session_set_save_handler),
    f!("session_cache_limiter", session_cache_limiter),
    f!("session_cache_expire", session_cache_expire),
    f!("session_set_cookie_params", session_set_cookie_params),
    f!("session_start", session_start),
];

/// The functions that reach the caller's variables or run PHP code.
pub(crate) fn names() -> Vec<&'static str> {
    BUILTINS.iter().map(|b| b.name).collect()
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

/// A string argument that may be null (`?string`).
fn opt_str(args: &[Value], i: usize) -> Option<PhpStr> {
    match arg(args, i) {
        Value::Null => None,
        v => v.to_php_string(),
    }
}

/// A `?string` argument PHP parses as a path ("P!"): no null bytes.
fn opt_path(ctx: &mut dyn Ctx, args: &[Value], i: usize, func: &str, name: &str) -> Res<Option<PhpStr>> {
    let v = opt_str(args, i);
    if v.as_ref().is_some_and(|v| v.as_bytes().contains(&0)) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{} (${name}) must not contain any null bytes", i + 1)));
    }
    Ok(v)
}

fn string(b: &[u8]) -> Value {
    Value::Str(PhpStr::from(b))
}

fn make(ctx: &mut dyn Ctx) -> Shared {
    state(ctx)
}

fn active(s: &Shared) -> bool {
    s.borrow().status == Status::Active
}

/// The reason a value is not a callable, as PHP words it.
pub(crate) fn callable_error(ctx: &mut dyn Ctx, v: &Value) -> String {
    match v {
        Value::Str(s) => {
            let name = String::from_utf8_lossy(s.as_bytes()).into_owned();
            match name.split_once("::") {
                Some((class, _)) if ctx.class(class.as_bytes()).is_none() => format!("class \"{class}\" not found"),
                Some((class, method)) => format!("class {class} does not have a method \"{method}\""),
                None => format!("function \"{name}\" not found or invalid function name"),
            }
        }
        Value::Array(a) if a.len() == 2 => match (a.get_int(0).map(Value::deref), a.get_int(1).map(Value::deref)) {
            (Some(Value::Str(class)), Some(Value::Str(method))) => {
                let class = String::from_utf8_lossy(class.as_bytes()).into_owned();
                if ctx.class(class.as_bytes()).is_none() {
                    format!("class \"{class}\" not found")
                } else {
                    format!("class {class} does not have a method \"{}\"", String::from_utf8_lossy(method.as_bytes()))
                }
            }
            (Some(Value::Object(o)), Some(Value::Str(method))) => {
                format!("class {} does not have a method \"{}\"", String::from_utf8_lossy(o.class.shown()), String::from_utf8_lossy(method.as_bytes()))
            }
            _ => "array callback must have exactly two members".into(),
        },
        Value::Array(_) => "array callback must have exactly two members".into(),
        _ => "no array or string given".into(),
    }
}

// ---- the settings ----

fn session_name(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let name = opt_path(ctx, args, 0, "session_name", "name")?;
    if name.is_some() && active(&s) {
        warn(ctx, "Session name cannot be changed when a session is active");
        return Ok(Value::Bool(false));
    }
    if name.is_some() && headers_sent(ctx) {
        warn(ctx, "Session name cannot be changed after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let current = text(ctx, "session.name");
    if let Some(name) = name {
        ctx.ini_set(b"session.name", Value::Str(name));
    }
    Ok(string(&current))
}

fn session_module_name(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let name = opt_str(args, 0);
    if name.is_some() && active(&s) {
        warn(ctx, "Session save handler module cannot be changed when a session is active");
        return Ok(Value::Bool(false));
    }
    if name.is_some() && headers_sent(ctx) {
        warn(ctx, "Session save handler module cannot be changed after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let current = s.borrow().module.map_or("", Module::name);
    if let Some(name) = name {
        if name.as_bytes().eq_ignore_ascii_case(b"user") {
            return Err(ctx.throw("ValueError", "session_module_name(): Argument #1 ($module) cannot be \"user\"".into()));
        }
        if Module::find(name.as_bytes()).is_none() {
            warn(ctx, &format!("Session handler module \"{}\" cannot be found", String::from_utf8_lossy(name.as_bytes())));
            return Ok(Value::Bool(false));
        }
        let open = s.borrow().has_mod_data();
        let m = s.borrow().module;
        if open && let Some(m) = m {
            life::mod_close(ctx, &s, m)?;
        }
        s.borrow_mut().files = None;
        ctx.ini_set(b"session.save_handler", Value::Str(name));
    }
    Ok(Value::str(current))
}

fn session_save_path(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let path = opt_path(ctx, args, 0, "session_save_path", "path")?;
    if path.is_some() && active(&s) {
        warn(ctx, "Session save path cannot be changed when a session is active");
        return Ok(Value::Bool(false));
    }
    if path.is_some() && headers_sent(ctx) {
        warn(ctx, "Session save path cannot be changed after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let current = text(ctx, "session.save_path");
    if let Some(path) = path {
        ctx.ini_set(b"session.save_path", Value::Str(path));
    }
    Ok(string(&current))
}

fn session_cache_limiter(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let limiter = opt_str(args, 0);
    if limiter.is_some() && active(&s) {
        warn(ctx, "Session cache limiter cannot be changed when a session is active");
        return Ok(Value::Bool(false));
    }
    if limiter.is_some() && headers_sent(ctx) {
        warn(ctx, "Session cache limiter cannot be changed after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let current = text(ctx, "session.cache_limiter");
    if let Some(limiter) = limiter {
        ctx.ini_set(b"session.cache_limiter", Value::Str(limiter));
    }
    Ok(string(&current))
}

fn session_cache_expire(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let expires = match arg(args, 0) {
        Value::Null => None,
        v => Some(v.to_int()),
    };
    if expires.is_some() && active(&s) {
        warn(ctx, "Session cache expiration cannot be changed when a session is active");
        return Ok(Value::Int(int(ctx, "session.cache_expire")));
    }
    if expires.is_some() && headers_sent(ctx) {
        warn(ctx, "Session cache expiration cannot be changed after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let current = int(ctx, "session.cache_expire");
    if let Some(e) = expires {
        ctx.ini_set(b"session.cache_expire", Value::str(e.to_string()));
    }
    Ok(Value::Int(current))
}

fn session_get_cookie_params(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    make(ctx);
    let mut a = PhpArray::new();
    let mut set = |k: &str, v: Value| a.insert(Key::Str(PhpStr::from(k)), v);
    set("lifetime", Value::Int(int(ctx, "session.cookie_lifetime")));
    set("path", string(&text(ctx, "session.cookie_path")));
    set("domain", string(&text(ctx, "session.cookie_domain")));
    set("secure", Value::Bool(flag(ctx, "session.cookie_secure")));
    set("httponly", Value::Bool(flag(ctx, "session.cookie_httponly")));
    set("samesite", string(&text(ctx, "session.cookie_samesite")));
    Ok(Value::Array(Rc::new(a)))
}

fn session_set_cookie_params(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let first = arg(args, 0);
    let str_arg = |i: usize| opt_str(args, i);
    let bool_arg = |i: usize| match arg(args, i) {
        Value::Null => None,
        v => Some(v.to_bool()),
    };
    if !flag(ctx, "session.use_cookies") {
        warn(ctx, "Session cookies cannot be used when session.use_cookies is disabled");
        return Ok(Value::Bool(false));
    }
    if active(&s) {
        warn(ctx, "Session cookie parameters cannot be changed when a session is active");
        return Ok(Value::Bool(false));
    }
    if headers_sent(ctx) {
        warn(ctx, "Session cookie parameters cannot be changed after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let (mut lifetime, mut path, mut domain, mut samesite) = (None, str_arg(1), str_arg(2), None::<PhpStr>);
    let (mut secure, mut httponly) = (bool_arg(3), bool_arg(4));
    if let Value::Array(options) = &first {
        for (i, what) in [(1, path.is_some()), (2, domain.is_some()), (3, secure.is_some()), (4, httponly.is_some())] {
            if what {
                let name = ["", "path", "domain", "secure", "httponly"][i];
                return Err(ctx.throw("ValueError", format!("session_set_cookie_params(): Argument #{} (${name}) must be null when argument #1 ($lifetime_or_options) is an array", i + 1)));
            }
        }
        let mut found = 0;
        for (key, value) in options.iter() {
            let value = value.deref();
            let Key::Str(key) = key else {
                warn(ctx, "Argument #1 ($lifetime_or_options) cannot contain numeric keys");
                continue;
            };
            let as_str = || value.to_php_string().unwrap_or_else(PhpStr::empty);
            match key.as_bytes().to_ascii_lowercase().as_slice() {
                b"lifetime" => lifetime = Some(as_str()),
                b"path" => path = Some(as_str()),
                b"domain" => domain = Some(as_str()),
                b"secure" => secure = Some(value.to_bool()),
                b"httponly" => httponly = Some(value.to_bool()),
                b"samesite" => samesite = Some(as_str()),
                _ => {
                    warn(ctx, &format!("Argument #1 ($lifetime_or_options) contains an unrecognized key \"{}\"", String::from_utf8_lossy(key.as_bytes())));
                    continue;
                }
            }
            found += 1;
        }
        if found == 0 {
            return Err(ctx.throw("ValueError", "session_set_cookie_params(): Argument #1 ($lifetime_or_options) must contain at least 1 valid key".into()));
        }
    } else {
        lifetime = Some(PhpStr::from(first.to_int().to_string()));
    }
    let bool_text = |b: bool| Value::str(if b { "1" } else { "0" });
    let mut changes: Vec<(&[u8], Value)> = Vec::new();
    if let Some(v) = lifetime {
        changes.push((b"session.cookie_lifetime", Value::Str(v)));
    }
    if let Some(v) = path {
        changes.push((b"session.cookie_path", Value::Str(v)));
    }
    if let Some(v) = domain {
        changes.push((b"session.cookie_domain", Value::Str(v)));
    }
    if let Some(v) = secure {
        changes.push((b"session.cookie_secure", bool_text(v)));
    }
    if let Some(v) = httponly {
        changes.push((b"session.cookie_httponly", bool_text(v)));
    }
    if let Some(v) = samesite {
        changes.push((b"session.cookie_samesite", Value::Str(v)));
    }
    for (name, value) in changes {
        if ctx.ini_set(name, value).is_none() {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

// ---- the id ----

fn session_id(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let new = opt_str(args, 0);
    if new.is_some() && active(&s) {
        warn(ctx, "Session ID cannot be changed when a session is active");
        return Ok(Value::Bool(false));
    }
    if new.is_some() && flag(ctx, "session.use_cookies") && headers_sent(ctx) {
        warn(ctx, "Session ID cannot be changed after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let current = s.borrow().id.clone().unwrap_or_default();
    // keep compatibility for "\0" characters
    let current = &current[..current.iter().position(|c| *c == 0).unwrap_or(current.len())];
    let current = string(current);
    if let Some(new) = new {
        s.borrow_mut().id = Some(new.as_bytes().to_vec());
    }
    Ok(current)
}

fn session_create_id(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let prefix = opt_path(ctx, args, 0, "session_create_id", "prefix")?.unwrap_or_else(PhpStr::empty);
    let mut id = Vec::new();
    if !prefix.is_empty() {
        if prefix.len() > crate::PS_MAX_SID_LENGTH {
            return Err(ctx.throw("ValueError", format!("session_create_id(): Argument #1 ($prefix) cannot be longer than {} characters", crate::PS_MAX_SID_LENGTH)));
        }
        let key = &prefix.as_bytes()[..prefix.as_bytes().iter().position(|c| *c == 0).unwrap_or(prefix.len())];
        if !crate::files::valid_key(key) {
            // E_ERROR raised for security reason.
            warn(ctx, "Prefix cannot contain special characters. Only the A-Z, a-z, 0-9, \"-\", and \",\" characters are allowed");
            return Ok(Value::Bool(false));
        }
        id.extend_from_slice(prefix.as_bytes());
    }
    let new_id = if !s.borrow().in_save_handler && active(&s) {
        let m = s.borrow().module;
        let mut found = None;
        if let Some(m) = m {
            for _ in 0..3 {
                let new = life::mod_create_sid(ctx, &s, m)?;
                let Some(new) = new else { break };
                if m == Module::User && s.borrow().user_implemented && s.borrow().names.validate_sid.is_none() {
                    found = Some(new);
                    break;
                }
                // Detect collision and retry
                if life::mod_validate_sid(ctx, &s, m, &new)? {
                    continue;
                }
                found = Some(new);
                break;
            }
        }
        found
    } else {
        Some(life::create_id(ctx)?)
    };
    match new_id {
        Some(new) => {
            id.extend_from_slice(&new);
            Ok(string(&id))
        }
        None => {
            warn(ctx, "Failed to create new ID");
            Ok(Value::Bool(false))
        }
    }
}

fn session_regenerate_id(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let delete = arg(args, 0).to_bool();
    if !active(&s) {
        warn(ctx, "Session ID cannot be regenerated when there is no active session");
        return Ok(Value::Bool(false));
    }
    if headers_sent(ctx) {
        warn(ctx, "Session ID cannot be regenerated after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let Some(m) = s.borrow().module else { return Ok(Value::Bool(false)) };
    let save_path = String::from_utf8_lossy(&text(ctx, "session.save_path")).into_owned();
    let old_id = s.borrow().id.clone().unwrap_or_default();
    // Process old session data
    if delete {
        if !life::mod_destroy(ctx, &s, m, &old_id)? {
            life::mod_close(ctx, &s, m)?;
            s.borrow_mut().status = Status::None;
            warn(ctx, &format!("Session object destruction failed. ID: {} (path: {save_path})", m.name()));
            return Ok(Value::Bool(false));
        }
    } else {
        let data = life::encode(ctx, &s)?;
        let ok = life::mod_write(ctx, &s, m, &old_id, data.as_deref().unwrap_or(&[]))?;
        if !ok {
            life::mod_close(ctx, &s, m)?;
            s.borrow_mut().status = Status::None;
            warn(ctx, &format!("Session write failed. ID: {} (path: {save_path})", m.name()));
            return Ok(Value::Bool(false));
        }
    }
    life::mod_close(ctx, &s, m)?;
    // New session data
    {
        let mut st = s.borrow_mut();
        st.original = None;
        st.id = None;
    }
    let name = text(ctx, "session.name");
    if !life::mod_open(ctx, &s, m, save_path.as_bytes(), &name)? {
        s.borrow_mut().status = Status::None;
        return Err(ctx.throw("Error", format!("Failed to open session: {} (path: {save_path})", m.name())));
    }
    let Some(id) = life::mod_create_sid(ctx, &s, m)? else {
        s.borrow_mut().status = Status::None;
        return Err(ctx.throw("Error", format!("Failed to create new session ID: {} (path: {save_path})", m.name())));
    };
    s.borrow_mut().id = Some(id.clone());
    if flag(ctx, "session.use_strict_mode") {
        let can_validate = (m != Module::User || !s.borrow().user_implemented) || s.borrow().names.validate_sid.is_some();
        if can_validate {
            let mut limit = 3;
            // Try to generate non-existing ID
            while limit > 0 {
                limit -= 1;
                let current = s.borrow().id.clone().unwrap_or_default();
                if !life::mod_validate_sid(ctx, &s, m, &current)? {
                    break;
                }
                s.borrow_mut().id = None;
                match life::mod_create_sid(ctx, &s, m)? {
                    Some(new) => s.borrow_mut().id = Some(new),
                    None => {
                        life::mod_close(ctx, &s, m)?;
                        s.borrow_mut().status = Status::None;
                        return Err(ctx.throw("Error", format!("Failed to create session ID by collision: {} (path: {save_path})", m.name())));
                    }
                }
            }
        }
    }
    // Read is required to make new session data at this point.
    let current = s.borrow().id.clone().unwrap_or_default();
    if life::mod_read(ctx, &s, m, &current)?.is_none() {
        life::mod_close(ctx, &s, m)?;
        s.borrow_mut().status = Status::None;
        return Err(ctx.throw("Error", format!("Failed to create(read) session ID: {} (path: {save_path})", m.name())));
    }
    if flag(ctx, "session.use_cookies") {
        s.borrow_mut().send_cookie = true;
    }
    Ok(Value::Bool(life::reset_id(ctx, &s)))
}

// ---- the data ----

fn session_encode(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let s = make(ctx);
    Ok(match life::encode(ctx, &s)? {
        Some(data) => Value::Str(PhpStr::new(data)),
        None => Value::Bool(false),
    })
}

fn session_decode(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let data = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    if !active(&s) {
        warn(ctx, "Session data cannot be decoded when there is no active session");
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(life::decode(ctx, &s, data.as_bytes())?))
}

fn session_unset(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let s = make(ctx);
    if !active(&s) {
        return Ok(Value::Bool(false));
    }
    s.borrow().edit_vars(|a| *a = PhpArray::new());
    Ok(Value::Bool(true))
}

fn session_gc(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let s = make(ctx);
    if !active(&s) {
        warn(ctx, "Session cannot be garbage collected when there is no active session");
        return Ok(Value::Bool(false));
    }
    let n = life::gc(ctx, &s, true)?;
    Ok(if n < 0 { Value::Bool(false) } else { Value::Int(n) })
}

// ---- the session's life ----

fn session_start(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    let options = match arg(args, 0) {
        Value::Array(a) => Some(a),
        _ => None,
    };
    if active(&s) {
        let started = s.borrow().started.clone();
        match started {
            Some((file, line)) => crate::state::notice(ctx, &format!("Ignoring session_start() because a session is already active (started from {file} on line {line})")),
            None if flag(ctx, "session.auto_start") => crate::state::notice(ctx, "Ignoring session_start() because a session is already automatically active"),
            None => crate::state::notice(ctx, "Ignoring session_start() because a session is already active"),
        }
        return Ok(Value::Bool(true));
    }
    if flag(ctx, "session.use_cookies") && headers_sent(ctx) {
        warn(ctx, "Session cannot be started after headers have already been sent");
        return Ok(Value::Bool(false));
    }
    let mut read_and_close = 0;
    if let Some(options) = options {
        for (key, value) in options.iter() {
            let Key::Str(key) = key else { continue };
            let value = value.deref();
            match &value {
                Value::Str(_) | Value::Bool(_) | Value::Int(_) => {
                    if key.as_bytes() == b"read_and_close" {
                        read_and_close = value.to_int();
                    } else {
                        let shown = String::from_utf8_lossy(key.as_bytes()).into_owned();
                        let name = [b"session.".as_slice(), key.as_bytes()].concat();
                        if ctx.ini_set(&name, value.clone()).is_none() {
                            warn(ctx, &format!("Setting option \"{shown}\" failed"));
                        }
                    }
                }
                other => {
                    return Err(ctx.throw(
                        "TypeError",
                        format!("session_start(): Option \"{}\" must be of type string|int|bool, {} given", String::from_utf8_lossy(key.as_bytes()), other.type_name()),
                    ));
                }
            }
        }
    }
    life::start(ctx, &s)?;
    if !active(&s) {
        s.borrow().edit_vars(|a| *a = PhpArray::new());
        return Ok(Value::Bool(false));
    }
    if read_and_close != 0 {
        life::flush(ctx, &s, false)?;
    }
    Ok(Value::Bool(true))
}

fn session_destroy(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let s = make(ctx);
    Ok(Value::Bool(life::destroy(ctx, &s)?))
}

fn session_write_close(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let s = make(ctx);
    if !active(&s) {
        return Ok(Value::Bool(false));
    }
    life::flush(ctx, &s, true)?;
    Ok(Value::Bool(true))
}

fn session_abort(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let s = make(ctx);
    Ok(Value::Bool(life::abort(ctx, &s)?))
}

fn session_reset(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let s = make(ctx);
    if !active(&s) {
        return Ok(Value::Bool(false));
    }
    life::reset(ctx, &s)?;
    Ok(Value::Bool(true))
}

fn session_status(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    let s = make(ctx);
    let status = s.borrow().status;
    Ok(Value::Int(status as i64))
}

fn session_register_shutdown(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    // The shutdown function this is registered as by session_set_save_handler()
    // runs first; another one registered later may expect the session still
    // to be there, so the write is registered again now.
    ctx.on_shutdown(Value::str("session_write_close"), Vec::new());
    Ok(Value::Null)
}

// ---- the save handler ----

/// can_session_handler_be_changed()
fn can_handler_change(ctx: &mut dyn Ctx, s: &Shared) -> bool {
    if active(s) {
        warn(ctx, "Session save handler cannot be changed when a session is active");
        return false;
    }
    if headers_sent(ctx) {
        warn(ctx, "Session save handler cannot be changed after headers have already been sent");
        return false;
    }
    true
}

/// set_user_save_handler_ini()
fn set_user_save_handler_ini(ctx: &mut dyn Ctx, s: &Shared) {
    s.borrow_mut().set_handler = true;
    ctx.ini_set(b"session.save_handler", Value::str("user"));
    s.borrow_mut().set_handler = false;
}

fn method_callable(obj: &Rc<Object>, method: &str) -> Value {
    Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(Rc::clone(obj)), Value::str(method)])))
}

fn session_set_save_handler(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = make(ctx);
    if args.len() <= 2 {
        // OOP version
        let obj = match arg(args, 0) {
            Value::Object(o) if o.class.is_a(b"sessionhandlerinterface") => o,
            other => {
                return Err(ctx.throw(
                    "TypeError",
                    format!("session_set_save_handler(): Argument #1 ($open) must be of type SessionHandlerInterface, {} given", p8_runtime::args::value_name(&other)),
                ));
            }
        };
        let register_shutdown = if args.len() > 1 {
            match arg(args, 1) {
                Value::Bool(b) => b,
                Value::Int(_) | Value::Float(_) | Value::Str(_) => arg(args, 1).to_bool(),
                other => {
                    return Err(ctx.throw(
                        "TypeError",
                        format!("session_set_save_handler(): Argument #2 ($close) must be of type bool, {} given", p8_runtime::args::value_name(&other)),
                    ));
                }
            }
        } else {
            true
        };
        if !can_handler_change(ctx, &s) {
            return Ok(Value::Bool(false));
        }
        {
            let mut st = s.borrow_mut();
            st.class_name = Some(obj.class.shown().to_vec());
            st.names.open = Some(method_callable(&obj, "open"));
            st.names.close = Some(method_callable(&obj, "close"));
            st.names.read = Some(method_callable(&obj, "read"));
            st.names.write = Some(method_callable(&obj, "write"));
            st.names.destroy = Some(method_callable(&obj, "destroy"));
            st.names.gc = Some(method_callable(&obj, "gc"));
            // SessionIdInterface, or (for BC) a method of that name
            st.names.create_sid = (obj.class.is_a(b"sessionidinterface") || obj.class.method(b"create_sid").is_some()).then(|| method_callable(&obj, "create_sid"));
            st.names.validate_sid = None;
            st.names.update_timestamp = None;
            let both = obj.class.is_a(b"sessionupdatetimestamphandlerinterface");
            if both || obj.class.method(b"validateid").is_some() {
                st.names.validate_sid = Some(method_callable(&obj, "validateId"));
            }
            if both || obj.class.method(b"updatetimestamp").is_some() {
                st.names.update_timestamp = Some(method_callable(&obj, "updateTimestamp"));
            }
        }
        if register_shutdown {
            // add the shutdown function, replacing the old one if there is one
            ctx.shutdown_named("session_shutdown", Some((Value::str("session_register_shutdown"), Vec::new())));
        } else {
            ctx.shutdown_named("session_shutdown", None);
        }
        let not_user = s.borrow().module != Some(Module::User);
        if !active(&s) && not_user {
            set_user_save_handler_ini(ctx, &s);
        }
        return Ok(Value::Bool(true));
    }
    deprecated(ctx, "Providing individual callbacks instead of an object implementing SessionHandlerInterface is deprecated");
    // Procedural version
    if args.len() < 6 {
        return Err(ctx.throw("ArgumentCountError", format!("session_set_save_handler() expects at least 6 arguments, {} given", args.len())));
    }
    let names = ["open", "close", "read", "write", "destroy", "gc", "create_sid", "validate_sid", "update_timestamp"];
    let mut callables: Vec<Option<Value>> = Vec::new();
    for (i, name) in names.iter().enumerate().take(args.len()) {
        let v = arg(args, i);
        if i >= 6 && matches!(v, Value::Null) {
            callables.push(None);
            continue;
        }
        if !ctx.is_callable(&v) {
            let why = callable_error(ctx, &v);
            let or_null = if i >= 6 { " or null" } else { "" };
            return Err(ctx.throw("TypeError", format!("session_set_save_handler(): Argument #{} (${name}) must be a valid callback{or_null}, {why}", i + 1)));
        }
        callables.push(Some(v));
    }
    if !can_handler_change(ctx, &s) {
        return Ok(Value::Bool(false));
    }
    s.borrow_mut().class_name = None;
    ctx.shutdown_named("session_shutdown", None);
    let not_user = s.borrow().module != Some(Module::User);
    if not_user {
        set_user_save_handler_ini(ctx, &s);
    }
    let mut st = s.borrow_mut();
    let mut it = callables.into_iter();
    st.names.open = it.next().flatten();
    st.names.close = it.next().flatten();
    st.names.read = it.next().flatten();
    st.names.write = it.next().flatten();
    st.names.destroy = it.next().flatten();
    st.names.gc = it.next().flatten();
    // the optional handlers: set only when given
    if let Some(v) = it.next().flatten() {
        st.names.create_sid = Some(v);
    }
    if let Some(v) = it.next().flatten() {
        st.names.validate_sid = Some(v);
    }
    if let Some(v) = it.next().flatten() {
        st.names.update_timestamp = Some(v);
    }
    Ok(Value::Bool(true))
}

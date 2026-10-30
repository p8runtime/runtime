//! The session's life: starting, reading, writing and ending it, the
//! storage module dispatch, the serializers, the id cookie and the cache
//! limiters (the part of session.c the functions share).
//!
//! Reference: $PHP_SRC_PATH/ext/session/session.c

use crate::files;
use crate::state::{Module, Serializer, Shared, Status, flag, int, notice, state, text, warn};
use crate::user;
use p8_ext_standard::{Serializer as Writer, Unserializer as Reader};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub(crate) type Res<T> = Result<T, Throw>;

// ---- ids ----

const HEXCONVTAB: &[u8; 64] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ,-";

/// bin_to_readable(): `outlen` characters of `nbits` bits each.
fn bin_to_readable(input: &[u8], outlen: usize, nbits: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(outlen);
    let mut bytes = input.iter();
    let mask = (1u16 << nbits) - 1;
    let mut w: u16 = 0;
    let mut have = 0u32;
    for _ in 0..outlen {
        if have < nbits {
            let Some(b) = bytes.next() else { break };
            w |= u16::from(*b) << have;
            have += 8;
        }
        out.push(HEXCONVTAB[usize::from(w & mask)]);
        w >>= nbits;
        have -= nbits;
    }
    out
}

/// php_session_create_id()
pub(crate) fn create_id(ctx: &mut dyn Ctx) -> Res<Vec<u8>> {
    let length = int(ctx, "session.sid_length").clamp(1, crate::PS_MAX_SID_LENGTH as i64) as usize;
    let bits = int(ctx, "session.sid_bits_per_character").clamp(4, 6) as u32;
    let mut random = vec![0u8; length];
    if getrandom::getrandom(&mut random).is_err() {
        return Err(ctx.throw("Exception", "Cannot gather sufficient random data".into()));
    }
    Ok(bin_to_readable(&random, length, bits))
}

// ---- the storage module ----

fn module(s: &Shared) -> Option<Module> {
    s.borrow().module
}

/// Run `f` on the files module's data, which is put back after (a handler
/// it reaches may not find it taken).
fn with_files<T>(ctx: &mut dyn Ctx, s: &Shared, f: impl FnOnce(&mut dyn Ctx, &mut files::Files) -> T) -> Option<T> {
    let mut data = s.borrow_mut().files.take()?;
    let r = f(ctx, &mut data);
    let mut st = s.borrow_mut();
    if st.files.is_none() {
        st.files = Some(data);
    }
    Some(r)
}

/// `m`'s s_open
pub(crate) fn mod_open(ctx: &mut dyn Ctx, s: &Shared, m: Module, save_path: &[u8], name: &[u8]) -> Res<bool> {
    match m {
        Module::Files => match files::open(ctx, save_path) {
            Some(data) => {
                // a data left open is closed by the one replacing it
                s.borrow_mut().files = Some(data);
                Ok(true)
            }
            None => Ok(false),
        },
        Module::User => user::open(ctx, s, save_path, name),
    }
}

/// `m`'s s_close
pub(crate) fn mod_close(ctx: &mut dyn Ctx, s: &Shared, m: Module) -> Res<bool> {
    match m {
        Module::Files => {
            s.borrow_mut().files = None;
            Ok(true)
        }
        Module::User => user::close(ctx, s),
    }
}

pub(crate) fn mod_read(ctx: &mut dyn Ctx, s: &Shared, m: Module, key: &[u8]) -> Res<Option<Vec<u8>>> {
    match m {
        Module::Files => Ok(with_files(ctx, s, |ctx, f| f.read(ctx, key)).flatten()),
        Module::User => user::read(ctx, s, key),
    }
}

pub(crate) fn mod_write(ctx: &mut dyn Ctx, s: &Shared, m: Module, key: &[u8], val: &[u8]) -> Res<bool> {
    match m {
        Module::Files => Ok(with_files(ctx, s, |ctx, f| f.write(ctx, key, val)).unwrap_or(false)),
        Module::User => user::write(ctx, s, key, val),
    }
}

pub(crate) fn mod_destroy(ctx: &mut dyn Ctx, s: &Shared, m: Module, key: &[u8]) -> Res<bool> {
    match m {
        Module::Files => Ok(with_files(ctx, s, |_, f| f.destroy(key)).unwrap_or(false)),
        Module::User => user::destroy(ctx, s, key),
    }
}

/// s_gc: the number of sessions removed, -1 for an error.
pub(crate) fn mod_gc(ctx: &mut dyn Ctx, s: &Shared, m: Module, maxlifetime: i64) -> Res<i64> {
    match m {
        Module::Files => Ok(with_files(ctx, s, |ctx, f| f.gc(ctx, maxlifetime)).unwrap_or(-1)),
        Module::User => user::gc(ctx, s, maxlifetime),
    }
}

pub(crate) fn mod_create_sid(ctx: &mut dyn Ctx, s: &Shared, m: Module) -> Res<Option<Vec<u8>>> {
    match m {
        Module::Files => {
            let mut maxfail = 3;
            loop {
                let id = create_id(ctx)?;
                // Check collision
                let taken = s.borrow().files.as_ref().is_some_and(|f| f.key_exists(&id));
                if !taken {
                    return Ok(Some(id));
                }
                maxfail -= 1;
                if maxfail < 0 {
                    return Ok(None);
                }
            }
        }
        Module::User => user::create_sid(ctx, s),
    }
}

pub(crate) fn mod_validate_sid(ctx: &mut dyn Ctx, s: &Shared, m: Module, key: &[u8]) -> Res<bool> {
    match m {
        Module::Files => Ok(s.borrow().files.as_ref().is_some_and(|f| f.key_exists(key))),
        Module::User => user::validate_sid(ctx, s, key),
    }
}

pub(crate) fn mod_update_timestamp(ctx: &mut dyn Ctx, s: &Shared, m: Module, key: &[u8], val: &[u8]) -> Res<bool> {
    match m {
        Module::Files => Ok(with_files(ctx, s, |ctx, f| f.update_timestamp(ctx, key, val)).unwrap_or(false)),
        Module::User => user::update_timestamp(ctx, s, key, val),
    }
}

// ---- state transitions ----

/// php_rinit_session_globals(): what a session leaves behind going away.
fn rinit_globals(s: &Shared) {
    let mut st = s.borrow_mut();
    st.id = None;
    st.status = Status::None;
    st.in_save_handler = false;
    st.set_handler = false;
    st.files = None;
    st.user_is_open = false;
    st.define_sid = true;
    st.original = None;
    st.vars = None;
}

/// php_rshutdown_session_globals()
pub(crate) fn rshutdown_globals(ctx: &mut dyn Ctx, s: &Shared) -> Res<()> {
    s.borrow_mut().vars = None;
    let m = module(s);
    let closed = if s.borrow().has_mod_data() { m.map(|m| mod_close(ctx, s, m)) } else { None };
    {
        let mut st = s.borrow_mut();
        st.id = None;
        st.original = None;
        st.class_name = None;
        st.started = None;
        st.status = Status::None;
    }
    if let Some(r) = closed {
        r?;
    }
    Ok(())
}

/// php_session_destroy()
pub(crate) fn destroy(ctx: &mut dyn Ctx, s: &Shared) -> Res<bool> {
    if s.borrow().status != Status::Active {
        warn(ctx, "Trying to destroy uninitialized session");
        return Ok(false);
    }
    let mut ok = true;
    let id = s.borrow().id.clone();
    let destroyed = match (id, module(s)) {
        (Some(id), Some(m)) => Some(mod_destroy(ctx, s, m, &id)),
        _ => None,
    };
    match destroyed {
        Some(Ok(false)) => {
            ok = false;
            warn(ctx, "Session object destruction failed");
        }
        Some(Err(t)) => {
            // (no handler is called with an exception pending)
            s.borrow_mut().user_implemented = false;
            rshutdown_globals(ctx, s)?;
            rinit_globals(s);
            return Err(t);
        }
        _ => {}
    }
    rshutdown_globals(ctx, s)?;
    rinit_globals(s);
    Ok(ok)
}

/// php_session_abort()
pub(crate) fn abort(ctx: &mut dyn Ctx, s: &Shared) -> Res<bool> {
    if s.borrow().status != Status::Active {
        return Ok(false);
    }
    let closed = match module(s) {
        Some(m) if s.borrow().has_mod_data() => mod_close(ctx, s, m),
        _ => Ok(true),
    };
    s.borrow_mut().status = Status::None;
    closed?;
    Ok(true)
}

/// php_session_track_init(): $_SESSION is a new, empty array.
fn track_init(ctx: &mut dyn Ctx, s: &Shared) {
    let cell = ctx.global_cell(b"_SESSION");
    *cell.borrow_mut() = Value::Array(Rc::new(PhpArray::new()));
    s.borrow_mut().vars = Some(cell);
}

/// php_session_cancel_decode()
fn cancel_decode(ctx: &mut dyn Ctx, s: &Shared) -> Res<()> {
    destroy(ctx, s)?;
    track_init(ctx, s);
    warn(ctx, "Failed to decode session object. Session has been destroyed");
    Ok(())
}

// ---- the serializers ----

fn serializer(ctx: &dyn Ctx) -> Option<Serializer> {
    Serializer::find(&text(ctx, "session.serialize_handler"))
}

/// php_session_encode(): None when the session has no variables to encode
/// (or a handler cannot encode them).
pub(crate) fn encode(ctx: &mut dyn Ctx, s: &Shared) -> Res<Option<Vec<u8>>> {
    let Some(vars) = s.borrow().vars() else {
        warn(ctx, "Cannot encode non-existent session");
        return Ok(None);
    };
    let Some(ser) = serializer(ctx) else { return Ok(None) };
    match ser {
        Serializer::PhpSerialize => {
            let mut w = Writer::new(ctx);
            w.value(&Value::Array(vars))?;
            Ok(Some(w.finish()))
        }
        Serializer::Php | Serializer::PhpBinary => {
            // PS_ENCODE_LOOP: a copy of the array, each variable fetched by name
            let mut out = Vec::new();
            let mut numeric = Vec::new();
            let mut w = Writer::new(ctx);
            for (key, _) in vars.iter() {
                let Key::Str(name) = key else {
                    if let Key::Int(n) = key {
                        numeric.push(n);
                    }
                    continue;
                };
                let name_bytes = name.as_bytes();
                let Some(value) = s.borrow().vars().and_then(|a| a.get(&Key::Str(name.clone())).cloned()) else { continue };
                if ser == Serializer::Php {
                    w.raw(name_bytes);
                    if name_bytes.contains(&b'|') {
                        return Ok(None);
                    }
                    w.raw(b"|");
                } else {
                    if name_bytes.len() > 127 {
                        continue;
                    }
                    w.raw(&[name_bytes.len() as u8]);
                    w.raw(name_bytes);
                }
                w.value(&value)?;
            }
            out.extend_from_slice(&w.finish());
            for n in numeric {
                // reported in the order met: after the others is close enough
                warn(ctx, &format!("Skipping numeric key {n}"));
            }
            // (nothing written is no string, not an empty one)
            Ok(if out.is_empty() { None } else { Some(out) })
        }
    }
}

/// php_session_decode(): false after "Failed to decode session object".
pub(crate) fn decode(ctx: &mut dyn Ctx, s: &Shared, data: &[u8]) -> Res<bool> {
    let Some(ser) = serializer(ctx) else { return Ok(false) };
    let ok = match decode_with(ctx, s, ser, data) {
        Ok(ok) => ok,
        Err(t) => {
            cancel_decode(ctx, s)?;
            return Err(t);
        }
    };
    if !ok {
        cancel_decode(ctx, s)?;
        return Ok(false);
    }
    Ok(true)
}

fn set_var(s: &Shared, name: Vec<u8>, value: Value) {
    s.borrow().edit_vars(|a| a.insert(Key::Str(PhpStr::new(name)), value));
}

fn decode_with(ctx: &mut dyn Ctx, s: &Shared, ser: Serializer, data: &[u8]) -> Res<bool> {
    match ser {
        Serializer::PhpSerialize => {
            let mut r = Reader::new(data);
            let value = r.value(ctx)?;
            let result = value.is_ok();
            let mut value = value.unwrap_or(Value::Null);
            if result {
                r.finish(ctx)?;
            }
            if matches!(value, Value::Null) {
                value = Value::Array(Rc::new(PhpArray::new()));
            }
            let cell = ctx.global_cell(b"_SESSION");
            *cell.borrow_mut() = value;
            s.borrow_mut().vars = Some(cell);
            Ok(result || data.is_empty())
        }
        Serializer::PhpBinary => {
            let mut r = Reader::new(data);
            let mut p = 0;
            let mut ok = true;
            while p < data.len() {
                let namelen = usize::from(data[p] & 0x7f);
                if namelen > 127 || p + namelen >= data.len() {
                    ok = false;
                    break;
                }
                let name = data[p + 1..p + 1 + namelen].to_vec();
                p += namelen + 1;
                r.seek(p);
                match r.value(ctx)? {
                    Ok(v) => {
                        set_var(s, name, v);
                        p = r.at();
                    }
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            r.finish(ctx)?;
            Ok(ok)
        }
        Serializer::Php => {
            let mut r = Reader::new(data);
            let mut p = 0;
            let mut ok = true;
            while p < data.len() {
                let Some(q) = data[p..].iter().position(|c| *c == b'|').map(|i| p + i) else {
                    ok = false;
                    break;
                };
                let name = data[p..q].to_vec();
                r.seek(q + 1);
                match r.value(ctx)? {
                    Ok(v) => {
                        set_var(s, name, v);
                        p = r.at();
                    }
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            r.finish(ctx)?;
            Ok(ok)
        }
    }
}

// ---- cookies and headers ----

/// A header added as header() adds one (nothing under a host that sends none).
fn add_header(ctx: &mut dyn Ctx, line: &[u8], replace: bool) -> bool {
    p8_ext_standard::header_line(ctx, line, replace)
}

fn gmt(ts: i64) -> String {
    p8_ext_standard::cookie_date(ts)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// last_modified(): the script's modification time.
fn last_modified(ctx: &mut dyn Ctx) {
    let path = match ctx.global_value(b"_SERVER").map(|v| v.deref()) {
        Some(Value::Array(server)) => ["PATH_TRANSLATED", "SCRIPT_FILENAME"]
            .iter()
            .find_map(|k| server.get(&Key::Str(PhpStr::from(*k))).and_then(|v| v.deref().to_php_string())),
        _ => None,
    };
    let Some(path) = path else { return };
    let mtime = std::fs::metadata(String::from_utf8_lossy(path.as_bytes()).as_ref()).and_then(|m| m.modified());
    if let Ok(t) = mtime {
        let secs = t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        add_header(ctx, format!("Last-Modified: {}", gmt(secs)).as_bytes(), true);
    }
}

/// php_session_cache_limiter(): 0 applied (or none wanted), -1 not
/// applicable, -2 headers were sent.
fn cache_limiter(ctx: &mut dyn Ctx, s: &Shared) -> Res<i32> {
    let limiter = text(ctx, "session.cache_limiter");
    if limiter.is_empty() {
        return Ok(0);
    }
    if s.borrow().status != Status::Active {
        return Ok(-1);
    }
    if let Some((file, line)) = ctx.output_started() {
        abort(ctx, s)?;
        if file.is_empty() {
            warn(ctx, "Session cache limiter cannot be sent after headers have already been sent");
        } else {
            warn(ctx, &format!("Session cache limiter cannot be sent after headers have already been sent (output started at {file}:{line})"));
        }
        return Ok(-2);
    }
    let expire = int(ctx, "session.cache_expire");
    let h = |ctx: &mut dyn Ctx, line: &str| {
        add_header(ctx, line.as_bytes(), true);
    };
    match limiter.to_ascii_lowercase().as_slice() {
        b"public" => {
            h(ctx, &format!("Expires: {}", gmt(now() + expire * 60)));
            h(ctx, &format!("Cache-Control: public, max-age={}", expire * 60));
            last_modified(ctx);
        }
        b"private" => {
            h(ctx, "Expires: Thu, 19 Nov 1981 08:52:00 GMT");
            h(ctx, &format!("Cache-Control: private, max-age={}", expire * 60));
            last_modified(ctx);
        }
        b"private_no_expire" => {
            h(ctx, &format!("Cache-Control: private, max-age={}", expire * 60));
            last_modified(ctx);
        }
        b"nocache" => {
            h(ctx, "Expires: Thu, 19 Nov 1981 08:52:00 GMT");
            h(ctx, "Cache-Control: no-store, no-cache, must-revalidate");
            h(ctx, "Pragma: no-cache");
        }
        _ => return Ok(-1),
    }
    Ok(0)
}

/// php_url_encode()
fn url_encode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(c),
            b' ' => out.push(b'+'),
            _ => out.extend_from_slice(format!("%{c:02X}").as_bytes()),
        }
    }
    out
}

/// php_session_send_cookie()
fn send_cookie(ctx: &mut dyn Ctx, s: &Shared) -> bool {
    if let Some((file, line)) = ctx.output_started() {
        if file.is_empty() {
            warn(ctx, "Session cookie cannot be sent after headers have already been sent");
        } else {
            warn(ctx, &format!("Session cookie cannot be sent after headers have already been sent (output started at {file}:{line})"));
        }
        return false;
    }
    let name = text(ctx, "session.name");
    let id = s.borrow().id.clone().unwrap_or_default();
    let mut cookie = b"Set-Cookie: ".to_vec();
    cookie.extend_from_slice(&name);
    cookie.push(b'=');
    cookie.extend_from_slice(&url_encode(&id));
    let lifetime = int(ctx, "session.cookie_lifetime");
    if lifetime > 0 {
        let t = now() + lifetime;
        if t > 0 {
            cookie.extend_from_slice(format!("; expires={}; Max-Age={lifetime}", gmt(t)).as_bytes());
        }
    }
    for (setting, label) in [("session.cookie_path", "; path="), ("session.cookie_domain", "; domain=")] {
        let v = text(ctx, setting);
        if !v.is_empty() {
            cookie.extend_from_slice(label.as_bytes());
            cookie.extend_from_slice(&v);
        }
    }
    if flag(ctx, "session.cookie_secure") {
        cookie.extend_from_slice(b"; secure");
    }
    if flag(ctx, "session.cookie_httponly") {
        cookie.extend_from_slice(b"; HttpOnly");
    }
    let samesite = text(ctx, "session.cookie_samesite");
    if !samesite.is_empty() {
        cookie.extend_from_slice(b"; SameSite=");
        cookie.extend_from_slice(&samesite);
    }
    // php_session_remove_cookie(): the session cookie sent already goes
    let prefix = [b"Set-Cookie: ".as_slice(), &name, b"="].concat();
    ctx.response().headers.retain(|h| !h.starts_with(&prefix));
    // replace is 0: a cookie setcookie() sent stays
    add_header(ctx, &cookie, false)
}

/// php_session_reset_id()
pub(crate) fn reset_id(ctx: &mut dyn Ctx, s: &Shared) -> bool {
    let Some(id) = s.borrow().id.clone() else {
        warn(ctx, "Cannot set session ID - session ID is not initialized");
        return false;
    };
    if flag(ctx, "session.use_cookies") && s.borrow().send_cookie {
        if !send_cookie(ctx, s) {
            return false;
        }
        s.borrow_mut().send_cookie = false;
    }
    let sid = if s.borrow().define_sid {
        let mut v = text(ctx, "session.name");
        v.push(b'=');
        v.extend_from_slice(&id);
        v
    } else {
        Vec::new()
    };
    ctx.set_constant(b"SID", Value::Str(PhpStr::new(sid)));
    true
}

// ---- starting and ending ----

/// The id a request variable (cookie, query, form) names `lookup`, if it
/// is there: ppid2sid() on it.
fn request_id(ctx: &mut dyn Ctx, array: &[u8], name: &[u8]) -> Option<Option<Vec<u8>>> {
    let Some(Value::Array(a)) = ctx.global_value(array).map(|v| v.deref()) else { return None };
    let v = a.get(&Key::from_string(PhpStr::from(name)))?.deref();
    Some(match v {
        Value::Str(s) => Some(s.as_bytes().to_vec()),
        _ => None,
    })
}

/// php_session_initialize(): the session is not active after an exception
/// from a handler (and no other handler is called then).
pub(crate) fn initialize(ctx: &mut dyn Ctx, s: &Shared) -> Res<bool> {
    let r = initialize_inner(ctx, s);
    if r.is_err() {
        let mut st = s.borrow_mut();
        st.status = Status::None;
        st.user_implemented = false;
    }
    r
}

fn initialize_inner(ctx: &mut dyn Ctx, s: &Shared) -> Res<bool> {
    s.borrow_mut().status = Status::Active;
    let Some(m) = module(s) else {
        s.borrow_mut().status = Status::Disabled;
        warn(ctx, "No storage module chosen - failed to initialize session");
        return Ok(false);
    };
    let save_path = text(ctx, "session.save_path");
    let name = text(ctx, "session.name");
    let module_name = m.name();
    let failure = |ctx: &mut dyn Ctx, what: &str| {
        warn(ctx, &format!("{what}: {module_name} (path: {})", String::from_utf8_lossy(&save_path)));
    };
    // Open session handler first
    if !mod_open(ctx, s, m, &save_path, &name)? {
        abort(ctx, s)?;
        failure(ctx, "Failed to initialize storage module");
        return Ok(false);
    }
    // If there is no ID, use session module to create one
    let strict = flag(ctx, "session.use_strict_mode");
    let id = s.borrow().id.clone();
    let need_new = match &id {
        None => true,
        Some(id) if id.is_empty() => true,
        Some(id) => strict && !mod_validate_sid(ctx, s, m, id)?,
    };
    if need_new {
        s.borrow_mut().id = None;
        let new = mod_create_sid(ctx, s, m)?;
        match new {
            Some(new) => s.borrow_mut().id = Some(new),
            None => {
                abort(ctx, s)?;
                return Err(ctx.throw("Error", format!("Failed to create session ID: {module_name} (path: {})", String::from_utf8_lossy(&save_path))));
            }
        }
        if flag(ctx, "session.use_cookies") {
            s.borrow_mut().send_cookie = true;
        }
    }
    if !reset_id(ctx, s) {
        abort(ctx, s)?;
        return Ok(false);
    }
    // Read data
    track_init(ctx, s);
    let id = s.borrow().id.clone().unwrap_or_default();
    let val = mod_read(ctx, s, m, &id)?;
    let Some(val) = val else {
        abort(ctx, s)?;
        warn(ctx, &format!("Failed to read session data: {module_name} (path: {})", String::from_utf8_lossy(&save_path)));
        return Ok(false);
    };
    // GC must be done after read
    gc(ctx, s, false)?;
    s.borrow_mut().original = None;
    if flag(ctx, "session.lazy_write") {
        s.borrow_mut().original = Some(val.clone());
    }
    if !val.is_empty() {
        decode(ctx, s, &val)?;
    }
    let (file, line) = ctx.location();
    // (no script runs while a session starts by session.auto_start)
    s.borrow_mut().started = if file.is_empty() || (&*file == "Unknown" && line == 0) { None } else { Some((file, line)) };
    Ok(true)
}

/// php_session_gc(): the number of sessions removed (-1 when none were
/// looked for or the handler failed).
pub(crate) fn gc(ctx: &mut dyn Ctx, s: &Shared, immediate: bool) -> Res<i64> {
    let mut num = -1;
    if !s.borrow().has_mod_data() {
        return Ok(num);
    }
    let mut collect = immediate;
    if !collect {
        let probability = int(ctx, "session.gc_probability");
        if probability > 0 {
            let divisor = int(ctx, "session.gc_divisor").max(1);
            let mut b = [0u8; 8];
            let _ = getrandom::getrandom(&mut b);
            collect = ((u64::from_le_bytes(b) % divisor as u64) as i64) < probability;
        }
    }
    if collect && let Some(m) = module(s) {
        let lifetime = int(ctx, "session.gc_maxlifetime");
        num = mod_gc(ctx, s, m, lifetime)?;
    }
    Ok(num)
}

/// php_session_start()
pub(crate) fn start(ctx: &mut dyn Ctx, s: &Shared) -> Res<bool> {
    let status = s.borrow().status;
    match status {
        Status::Active => {
            let started = s.borrow().started.clone();
            match started {
                Some((file, line)) => notice(ctx, &format!("Ignoring session_start() because a session has already been started (started from {file} on line {line})")),
                None if flag(ctx, "session.auto_start") => notice(ctx, "Ignoring session_start() because a session has already been started automatically"),
                None => notice(ctx, "Ignoring session_start() because a session has already been started"),
            }
            return Ok(false);
        }
        Status::Disabled => {
            let handler = text(ctx, "session.save_handler");
            if s.borrow().module.is_none() && !handler.is_empty() {
                let m = Module::find(&handler);
                if m.is_none() {
                    warn(ctx, &format!("Cannot find session save handler \"{}\" - session startup failed", String::from_utf8_lossy(&handler)));
                    return Ok(false);
                }
                s.borrow_mut().module = m;
            }
            let ser = text(ctx, "session.serialize_handler");
            if !ser.is_empty() && Serializer::find(&ser).is_none() {
                warn(ctx, &format!("Cannot find session serialization handler \"{}\" - session startup failed", String::from_utf8_lossy(&ser)));
                return Ok(false);
            }
            s.borrow_mut().status = Status::None;
        }
        Status::None => {}
    }
    {
        // Setup internal flags
        let only = flag(ctx, "session.use_only_cookies");
        let cookies = flag(ctx, "session.use_cookies");
        let mut st = s.borrow_mut();
        st.define_sid = !only;
        st.send_cookie = cookies || only;
    }
    let name = text(ctx, "session.name");
    // Cookies are preferred, because initially cookie and get variables
    // will be available. URL/POST session ID may be used when
    // use_only_cookies=Off.
    if s.borrow().id.is_none() {
        if flag(ctx, "session.use_cookies")
            && let Some(id) = request_id(ctx, b"_COOKIE", &name)
        {
            let mut st = s.borrow_mut();
            st.id = id;
            st.send_cookie = false;
            st.define_sid = false;
        }
        if !flag(ctx, "session.use_only_cookies") {
            for array in [b"_GET".as_slice(), b"_POST"] {
                if s.borrow().id.is_none()
                    && let Some(id) = request_id(ctx, array, &name)
                {
                    let mut st = s.borrow_mut();
                    st.send_cookie = id.is_none();
                    st.id = id;
                }
            }
            // an external site that referred to this request invalidates the id
            let referer_check = text(ctx, "session.referer_check");
            if s.borrow().id.is_some() && !referer_check.is_empty() {
                let referer = match ctx.global_value(b"_SERVER").map(|v| v.deref()) {
                    Some(Value::Array(server)) => match server.get(&Key::Str(PhpStr::from("HTTP_REFERER"))).map(Value::deref) {
                        Some(Value::Str(r)) => Some(r.as_bytes().to_vec()),
                        _ => None,
                    },
                    _ => None,
                };
                if referer.is_some_and(|r| !r.is_empty() && !r.windows(referer_check.len()).any(|w| w == referer_check.as_slice())) {
                    s.borrow_mut().id = None;
                }
            }
        }
    }
    // Finally check session id for dangerous characters
    let dangerous = s.borrow().id.as_ref().is_some_and(|id| id.iter().any(|c| b"\r\n\t <>'\"\\".contains(c)));
    if dangerous {
        s.borrow_mut().id = None;
    }
    let started = initialize(ctx, s).and_then(|ok| if ok { cache_limiter(ctx, s).map(|l| l != -2) } else { Ok(false) });
    match started {
        Ok(true) => Ok(true),
        Ok(false) | Err(_) => {
            let mut st = s.borrow_mut();
            st.status = Status::None;
            st.id = None;
            started
        }
    }
}

/// php_session_save_current_state()
fn save_current_state(ctx: &mut dyn Ctx, s: &Shared, write: bool) -> Res<()> {
    let m = module(s);
    if write && s.borrow().has_vars() {
        let class_name = s.borrow().class_name.clone();
        if s.borrow().has_mod_data()
            && let Some(m) = m
        {
            let id = s.borrow().id.clone().unwrap_or_default();
            let val = encode(ctx, s)?;
            let mut handler = "write";
            let ok = match val {
                Some(val) => {
                    let unchanged = flag(ctx, "session.lazy_write") && s.borrow().original.as_deref() == Some(val.as_slice());
                    // the files and user modules both update timestamps
                    if unchanged {
                        handler = if class_name.is_some() { "updateTimestamp" } else { "update_timestamp" };
                        mod_update_timestamp(ctx, s, m, &id, &val)?
                    } else {
                        mod_write(ctx, s, m, &id, &val)?
                    }
                }
                None => mod_write(ctx, s, m, &id, &[])?,
            };
            if !ok {
                let save_path = String::from_utf8_lossy(&text(ctx, "session.save_path")).into_owned();
                if m == Module::Files {
                    warn(ctx, &format!("Failed to write session data ({}). Please verify that the current setting of session.save_path is correct ({save_path})", m.name()));
                } else if let Some(class) = class_name {
                    warn(ctx, &format!("Failed to write session data using user defined save handler. (session.save_path: {save_path}, handler: {}::{handler})", String::from_utf8_lossy(&class)));
                } else {
                    warn(ctx, &format!("Failed to write session data using user defined save handler. (session.save_path: {save_path}, handler: {handler})"));
                }
            }
        }
    }
    if s.borrow().has_mod_data()
        && let Some(m) = m
    {
        mod_close(ctx, s, m)?;
    }
    Ok(())
}

/// php_session_flush()
pub(crate) fn flush(ctx: &mut dyn Ctx, s: &Shared, write: bool) -> Res<bool> {
    if s.borrow().status == Status::Active {
        let saved = save_current_state(ctx, s, write);
        s.borrow_mut().status = Status::None;
        saved?;
        return Ok(true);
    }
    Ok(false)
}

/// php_session_reset()
pub(crate) fn reset(ctx: &mut dyn Ctx, s: &Shared) -> Res<bool> {
    if s.borrow().status == Status::Active {
        return initialize(ctx, s);
    }
    Ok(false)
}

// ---- the request's ends ----

/// RINIT: a session started by session.auto_start.
pub(crate) fn request_start(ctx: &mut dyn Ctx) -> Res<()> {
    if !flag(ctx, "session.auto_start") {
        return Ok(());
    }
    let s = state(ctx);
    if s.borrow().status == Status::Disabled {
        return Ok(());
    }
    start(ctx, &s)?;
    Ok(())
}

/// RSHUTDOWN: an active session is written, then the session goes.
pub(crate) fn request_end(ctx: &mut dyn Ctx) -> Res<()> {
    let Some(s) = crate::state::peek(ctx) else { return Ok(()) };
    s.borrow_mut().ending = true;
    let mut result = Ok(());
    if s.borrow().status == Status::Active {
        result = flush(ctx, &s, true).map(|_| ());
    }
    let closed = rshutdown_globals(ctx, &s);
    {
        // Free user defined handlers
        let mut st = s.borrow_mut();
        st.names = Default::default();
    }
    result.and(closed)
}

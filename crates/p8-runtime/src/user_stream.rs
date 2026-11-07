//! User-space stream wrappers: a class stream_wrapper_register() names
//! serves a URL scheme, PHP calling its methods (stream_open, stream_read,
//! url_stat, unlink, dir_opendir, ...) on an instance made for each
//! operation. A request keeps its own registrations, and the built-in
//! schemes it unregistered.
//!
//! A stream a wrapper opened keeps PHP's read buffer in front of
//! stream_read (what the method is asked for depends on it) and writes
//! through in chunk_size pieces, as streams.c does for user-space streams.
//!
//! Reference: $PHP_SRC_PATH/main/streams/userspace.c,
//! $PHP_SRC_PATH/main/streams/streams.c (php_stream_locate_url_wrapper,
//! _php_stream_read, _php_stream_get_line, _php_stream_copy_to_mem),
//! $PHP_SRC_PATH/ext/standard/filestat.c (the stat cache)

use crate::array::{Key, PhpArray};
use crate::ext::{Ctx, Throw};
use crate::object::{CLASS_ABSTRACT, CLASS_ALLOW_DYNAMIC, CLASS_INTERFACE, CLASS_TRAIT, Class, Object, Visibility};
use crate::stream::{Stream, Target};
use crate::string::PhpStr;
use crate::value::Value;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// stream_open's options: STREAM_USE_PATH, and what include adds
/// (STREAM_OPEN_FOR_INCLUDE, STREAM_OPEN_FOR_ZEND_STREAM).
pub const USE_PATH: i64 = 1;
pub const OPEN_FOR_INCLUDE: i64 = 0x80;
pub const OPEN_FOR_ZEND_STREAM: i64 = 0x10000;
/// stream_wrapper_register's flag: the wrapper is remote (STREAM_IS_URL).
pub const IS_URL: i64 = 1;
/// url_stat's flags.
pub const STAT_LINK: i64 = 1;
pub const STAT_QUIET: i64 = 2;
pub const STAT_IGNORE_OPEN_BASEDIR: i64 = 4;
/// mkdir's and rmdir's options (REPORT_ERRORS is always on).
pub const MKDIR_RECURSIVE: i64 = 1;
const REPORT_ERRORS: i64 = 8;
/// stream_metadata's options.
pub const META_TOUCH: i64 = 1;
pub const META_OWNER_NAME: i64 = 2;
pub const META_OWNER: i64 = 3;
pub const META_GROUP_NAME: i64 = 4;
pub const META_GROUP: i64 = 5;
pub const META_ACCESS: i64 = 6;
/// stream_set_option's options.
pub const OPTION_BLOCKING: i64 = 1;
pub const OPTION_READ_BUFFER: i64 = 2;
pub const OPTION_WRITE_BUFFER: i64 = 3;
pub const OPTION_READ_TIMEOUT: i64 = 4;
/// What a buffer option without a size passes (the C library's BUFSIZ).
const BUFSIZ: i64 = 8192;

/// The schemes p8 opens by itself, in PHP's order (no https, ftp(s) or
/// phar yet), then those extensions registered (zip).
pub fn builtin_schemes() -> Vec<&'static str> {
    let mut names = vec!["compress.zlib", "php", "file", "glob", "data", "http"];
    names.extend(crate::stream::wrapper_schemes());
    names
}

/// The schemes PHP itself has whether or not p8 opens them: a URL of one
/// of these goes on to p8's own handling, not to "Unable to find the
/// wrapper".
const PHP_SCHEMES: &[&str] = &["https", "ftps", "compress.zlib", "php", "file", "glob", "data", "http", "ftp", "phar"];

/// A class registered for a scheme.
pub struct Wrapper {
    pub class: Rc<Class>,
    /// Registered with STREAM_IS_URL: include refuses it (allow_url_include).
    pub is_url: bool,
}

/// stat()'s values, as a wrapper's url_stat or stream_stat gives them (each
/// as the C struct stat holds it).
#[derive(Debug, Clone, Copy, Default)]
pub struct Stat {
    pub dev: i64,
    pub ino: i64,
    pub mode: i64,
    pub nlink: i64,
    pub uid: i64,
    pub gid: i64,
    pub rdev: i64,
    pub size: i64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub blksize: i64,
    pub blocks: i64,
}

/// S_IFMT and the file types stat()'s mode tells.
pub const S_IFMT: i64 = 0o170000;
pub const S_IFDIR: i64 = 0o040000;
pub const S_IFREG: i64 = 0o100000;
pub const S_IFLNK: i64 = 0o120000;

impl Stat {
    const NAMES: [&'static str; 13] = ["dev", "ino", "mode", "nlink", "uid", "gid", "rdev", "size", "atime", "mtime", "ctime", "blksize", "blocks"];

    fn values(&self) -> [i64; 13] {
        [self.dev, self.ino, self.mode, self.nlink, self.uid, self.gid, self.rdev, self.size, self.atime, self.mtime, self.ctime, self.blksize, self.blocks]
    }

    /// statbuf_from_array: the fields an array names (others 0), cut to
    /// the C types (mode, uid and gid are 32 bits).
    fn from_array(a: &PhpArray) -> Stat {
        let get = |k: &str| a.get(&Key::Str(PhpStr::from(k))).map_or(0, Value::to_int);
        Stat {
            dev: get("dev"),
            ino: get("ino"),
            mode: i64::from(get("mode") as u32),
            nlink: get("nlink"),
            uid: i64::from(get("uid") as u32),
            gid: i64::from(get("gid") as u32),
            rdev: get("rdev"),
            size: get("size"),
            atime: get("atime"),
            mtime: get("mtime"),
            ctime: get("ctime"),
            blksize: get("blksize"),
            blocks: get("blocks"),
        }
    }

    /// stat()'s array: the 13 values by number, then by name.
    pub fn to_array(&self) -> PhpArray {
        let values = self.values();
        let mut out = PhpArray::new();
        for (i, v) in values.iter().enumerate() {
            out.insert(Key::Int(i as i64), Value::int(*v));
        }
        for (k, v) in Stat::NAMES.iter().zip(values) {
            out.insert(Key::Str(PhpStr::from(*k)), Value::int(v));
        }
        out
    }

    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }

    pub fn is_file(&self) -> bool {
        self.mode & S_IFMT == S_IFREG
    }

    pub fn is_link(&self) -> bool {
        self.mode & S_IFMT == S_IFLNK
    }
}

/// A stream a wrapper opened: its instance, and the read buffer PHP
/// keeps in front of stream_read. The stream's `buffer` holds the bytes
/// not yet read; `pos` is PHP's position.
pub struct UserStream {
    pub object: Rc<Object>,
    /// Opened by dir_opendir (read with dir_readdir), not stream_open.
    pub dir: bool,
    /// The read buffer's size and where its unread bytes start in it
    /// (readbuflen, readpos): what stream_read is asked for comes of them.
    buf_len: usize,
    buf_start: usize,
    /// Written since the last flush: a close flushes first.
    written: bool,
    /// stream_seek is missing: seeking forward reads instead.
    no_seek: bool,
}

impl UserStream {
    fn new(object: Rc<Object>, dir: bool) -> UserStream {
        UserStream { object, dir, buf_len: 0, buf_start: 0, written: false, no_seek: false }
    }

    pub fn seekable(&self) -> bool {
        !self.no_seek
    }
}

/// What a request has registered.
#[derive(Default)]
struct State {
    /// The wrappers by scheme as registered, in the order registered.
    user: Vec<(Vec<u8>, Rc<Wrapper>)>,
    /// The built-in schemes stream_wrapper_unregister() took away.
    removed: Vec<Vec<u8>>,
    /// The URL being opened (FG(user_stream_current_filename)): opening
    /// it again from its own wrapper is refused.
    opening: Option<String>,
    /// The last stat and lstat (the path as given, its values): PHP's
    /// stat cache (BG(CurrentStatFile), BG(CurrentLStatFile)).
    stat: Option<(Vec<u8>, Stat)>,
    lstat: Option<(Vec<u8>, Stat)>,
    /// The stream resources open through a wrapper: the request's end
    /// closes those still open.
    open: Vec<Weak<Object>>,
}

const KEY: &str = "user stream wrappers";

fn state(ctx: &mut dyn Ctx) -> &mut State {
    let slot = ctx.ext_data(KEY);
    if !slot.as_ref().is_some_and(|s| s.is::<State>()) {
        *slot = Some(Box::new(State::default()));
    }
    slot.as_mut().and_then(|s| s.downcast_mut::<State>()).expect("just made")
}

/// The wrapper `scheme` names: as registered, else in lowercase.
fn user_wrapper(st: &State, scheme: &[u8]) -> Option<Rc<Wrapper>> {
    let lower = scheme.to_ascii_lowercase();
    st.user.iter().find(|(s, _)| s == scheme).or_else(|| st.user.iter().find(|(s, _)| *s == lower)).map(|(_, w)| Rc::clone(w))
}

fn builtin_enabled(st: &State, scheme: &[u8]) -> bool {
    builtin_schemes().iter().any(|b| b.as_bytes() == scheme) && !st.removed.iter().any(|r| r == scheme)
}

/// stream_wrapper_register(): false after PHP's warning.
pub fn register(ctx: &mut dyn Ctx, func: &str, protocol: &[u8], class: Rc<Class>, flags: i64) -> bool {
    let shown = String::from_utf8_lossy(protocol).into_owned();
    let st = state(ctx);
    if st.user.iter().any(|(s, _)| s == protocol) || builtin_enabled(st, protocol) {
        ctx.warn(&format!("{func}(): Protocol {shown}:// is already defined."));
        return false;
    }
    if !protocol.iter().all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')) {
        let name = String::from_utf8_lossy(class.name.as_bytes()).into_owned();
        ctx.warn(&format!("{func}(): Invalid protocol scheme specified. Unable to register wrapper class {name} to {shown}://"));
        return false;
    }
    st.user.push((protocol.to_vec(), Rc::new(Wrapper { class, is_url: flags & IS_URL != 0 })));
    true
}

/// stream_wrapper_unregister(): a registered wrapper goes, or a built-in
/// scheme is disabled for the rest of the request.
pub fn unregister(ctx: &mut dyn Ctx, protocol: &[u8]) -> bool {
    let st = state(ctx);
    if let Some(i) = st.user.iter().position(|(s, _)| s == protocol) {
        st.user.remove(i);
        return true;
    }
    if builtin_enabled(st, protocol) {
        st.removed.push(protocol.to_vec());
        return true;
    }
    ctx.warn(&format!("stream_wrapper_unregister(): Unable to unregister protocol {}://", String::from_utf8_lossy(protocol)));
    false
}

/// stream_wrapper_restore(): a built-in scheme back as it was.
pub fn restore(ctx: &mut dyn Ctx, protocol: &[u8]) -> bool {
    let shown = String::from_utf8_lossy(protocol).into_owned();
    if !builtin_schemes().iter().any(|b| b.as_bytes() == protocol) {
        ctx.warn(&format!("stream_wrapper_restore(): {shown}:// never existed, nothing to restore"));
        return false;
    }
    let st = state(ctx);
    let overridden = st.user.iter().any(|(s, _)| s == protocol);
    if !overridden && !st.removed.iter().any(|r| r == protocol) {
        ctx.notice(&format!("stream_wrapper_restore(): {shown}:// was never changed, nothing to restore"));
        return true;
    }
    st.user.retain(|(s, _)| s != protocol);
    st.removed.retain(|r| r != protocol);
    true
}

/// stream_get_wrappers(): the built-in schemes still registered, then
/// the user ones.
pub fn schemes(ctx: &mut dyn Ctx) -> Vec<Vec<u8>> {
    let st = state(ctx);
    let mut out: Vec<Vec<u8>> = builtin_schemes().into_iter().map(|s| s.as_bytes().to_vec()).filter(|s| !st.removed.contains(s)).collect();
    out.extend(st.user.iter().map(|(s, _)| s.clone()));
    out
}

/// What serves a path.
pub enum Located {
    /// A user wrapper: the path its methods are given where PHP strips
    /// "file://" (stream_open, url_stat), and the wrapper.
    User(Rc<Wrapper>, String),
    /// p8's own handling (a plain file, php://, ...).
    Builtin,
    /// The file:// wrapper is unregistered and nothing replaced it.
    Disabled,
}

/// php_stream_locate_url_wrapper: the scheme a path starts with ("x://",
/// two characters at least, or "data:"), served by a registered class or
/// by p8; a plain path is file://'s. A scheme nothing serves is warned of
/// and the path taken as a plain file.
pub fn locate(ctx: &mut dyn Ctx, func: &str, path: &str) -> Located {
    find(ctx, Some(func), path)
}

/// [`locate`] without its warning: what serves a path, to decide by.
pub fn peek(ctx: &mut dyn Ctx, path: &str) -> Located {
    find(ctx, None, path)
}

fn find(ctx: &mut dyn Ctx, func: Option<&str>, path: &str) -> Located {
    let b = path.as_bytes();
    let n = b.iter().take_while(|&&c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')).count();
    let scheme = (b.get(n) == Some(&b':') && n > 1 && (b[n + 1..].starts_with(b"//") || (n == 4 && b.starts_with(b"data:")))).then(|| &b[..n]);
    let has_state = ctx.ext_data(KEY).is_some();
    let mut file_url = false;
    if let Some(scheme) = scheme {
        let lower = scheme.to_ascii_lowercase();
        // (A file:// URL goes on below, its path taken out of it.)
        if has_state
            && lower != b"file"
            && let Some(w) = user_wrapper(state(ctx), scheme)
        {
            return Located::User(w, path.to_string());
        }
        let known = if has_state {
            let st = state(ctx);
            user_wrapper(st, scheme).is_some()
                || builtin_enabled(st, scheme)
                || builtin_enabled(st, &lower)
                || PHP_SCHEMES.iter().any(|s| s.as_bytes() == lower) && !st.removed.contains(&lower)
        } else {
            builtin_schemes().iter().any(|s| s.as_bytes() == lower) || PHP_SCHEMES.iter().any(|s| s.as_bytes() == lower)
        };
        if known && lower != b"file" {
            return Located::Builtin;
        }
        if !known && let Some(func) = func {
            let name: String = String::from_utf8_lossy(&scheme[..scheme.len().min(31)]).into_owned();
            ctx.warn(&format!("{func}(): Unable to find the wrapper \"{name}\" - did you forget to enable it when you configured PHP?"));
        }
        file_url = known;
    }
    if !has_state {
        return Located::Builtin;
    }
    let st = state(ctx);
    if let Some(w) = user_wrapper(st, b"file") {
        // file://host/path names /path.
        let local = if file_url { path.get(7..).map(|rest| rest.find('/').map_or("", |i| &rest[i..])).unwrap_or(path).to_string() } else { path.to_string() };
        return Located::User(w, local);
    }
    if st.removed.iter().any(|r| r == b"file") {
        return Located::Disabled;
    }
    Located::Builtin
}

/// The warning a disabled file:// gives (php_stream_locate_url_wrapper
/// with REPORT_ERRORS).
pub fn warn_disabled(ctx: &mut dyn Ctx, func: &str) {
    ctx.warn(&format!("{func}(): file:// wrapper is disabled in the server configuration"));
}

/// The request's default stream context (FG(default_context)), made when
/// first asked for.
pub fn default_context(ctx: &mut dyn Ctx) -> Value {
    let v = ctx.state("stream_context_default").clone();
    if v.is_object() {
        return v;
    }
    let v = new_context(ctx);
    *ctx.state("stream_context_default") = v.clone();
    v
}

/// A new, empty stream-context resource.
pub fn new_context(ctx: &mut dyn Ctx) -> Value {
    ctx.new_resource(crate::stream::Resource {
        kind: "stream-context",
        closed: std::cell::Cell::new(false),
        stream: RefCell::new(Stream::new(Target::Memory(Vec::new()), "", "r")),
    })
}

fn class_name(o: &Object) -> String {
    String::from_utf8_lossy(o.class.name.as_bytes()).into_owned()
}

/// user_stream_create_object: an instance of the wrapper's class with
/// `$context` set, then its constructor run; None for a class that
/// cannot be instantiated.
fn instance(ctx: &mut dyn Ctx, w: &Wrapper, context: Value) -> Result<Option<Rc<Object>>, Throw> {
    if w.class.flags & (CLASS_INTERFACE | CLASS_TRAIT | CLASS_ABSTRACT) != 0 {
        return Ok(None);
    }
    let o = ctx.new_instance(&w.class, &mut [], false)?;
    // Its destructor runs when the last reference goes, as any object's.
    o.destructed.set(false);
    match o.class.slot(b"context", None) {
        Some(i) => o.slots.borrow_mut()[i] = context,
        None => {
            let key = Key::Str(PhpStr::from("context"));
            if !o.dynamic.borrow().contains_key(&key) && o.class.flags & CLASS_ALLOW_DYNAMIC == 0 && o.class.lower != b"stdclass" {
                ctx.deprecated(&format!("Creation of dynamic property {}::$context is deprecated", class_name(&o)));
            }
            o.dynamic.borrow_mut().insert(key, context);
        }
    }
    if o.class.method(b"__construct").is_some() {
        ctx.invoke(&crate::reflect::Callee::Method(Rc::clone(&w.class), PhpStr::from("__construct")), Some(Rc::clone(&o)), &mut [])?;
    }
    Ok(Some(o))
}

/// call_method_if_exists: the method's result, None when the object has
/// no such public method (and no __call).
fn call(ctx: &mut dyn Ctx, o: &Rc<Object>, name: &str, args: &mut [Value]) -> Result<Option<Value>, Throw> {
    let public = o.class.method(name.as_bytes()).is_some_and(|m| m.visibility == Visibility::Public && !m.is_abstract);
    if !public {
        if o.class.method(b"__call").is_none() {
            return Ok(None);
        }
        // __call gets the arguments as an array, a reference (stream_open's
        // fourth) kept one, as PHP's trampoline copies them.
        let packed = Value::array(PhpArray::from_list(args.to_vec()));
        let callable = Value::array(PhpArray::from_list(vec![Value::object(Rc::clone(o)), Value::str("__call")]));
        return ctx.call(&callable, &mut [Value::str(name), packed]).map(Some);
    }
    let callable = Value::array(PhpArray::from_list(vec![Value::object(Rc::clone(o)), Value::str(name)]));
    ctx.call(&callable, args).map(Some)
}

/// "%s::%s is not implemented!" (the class's name).
fn not_implemented(ctx: &mut dyn Ctx, func: &str, o: &Object, method: &str) {
    ctx.warn(&format!("{func}(): {}::{method} is not implemented!", class_name(o)));
}

/// try_convert_to_string on what stream_read gave.
fn to_bytes(ctx: &mut dyn Ctx, v: &Value) -> Result<Vec<u8>, Throw> {
    let v = v.deref();
    if v.is_array() {
        ctx.warn("Array to string conversion");
        return Ok(b"Array".to_vec());
    }
    if let Some(o) = v.as_object() {
        if o.class.method(b"__tostring").is_some() {
            let s = ctx.call(&Value::array(PhpArray::from_list(vec![Value::object(Rc::clone(o)), Value::str("__toString")])), &mut [])?;
            return Ok(s.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default());
        }
        if o.class.cast_string.is_none() && o.class.flags & crate::object::CLASS_RESOURCE == 0 {
            return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", class_name(o))));
        }
    }
    Ok(v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default())
}

/// Opens `url` with the wrapper (stream_open, or dir_opendir for a
/// directory): the stream, or why not (PHP's "Failed to open stream:"
/// reason). `opened` receives what stream_open set its fourth argument to.
pub fn open(ctx: &mut dyn Ctx, w: &Rc<Wrapper>, url: &str, mode: &str, options: i64, context: Value, dir: bool) -> Result<Result<(Stream, Option<PhpStr>), String>, Throw> {
    if state(ctx).opening.as_deref() == Some(url) {
        return Ok(Err("infinite recursion prevented".into()));
    }
    state(ctx).opening = Some(url.to_string());
    let made = instance(ctx, w, context);
    let opened = Value::new_reference(Value::NULL);
    let r = match made {
        Ok(Some(o)) => {
            let r = if dir {
                call(ctx, &o, "dir_opendir", &mut [Value::str(url), Value::int(options)])
            } else {
                call(ctx, &o, "stream_open", &mut [Value::str(url), Value::str(mode), Value::int(options), opened.clone()])
            };
            r.map(|v| Some((o, v)))
        }
        Ok(None) => Ok(None),
        Err(t) => Err(t),
    };
    state(ctx).opening = None;
    let Some((o, result)) = r? else { return Ok(Err("operation failed".into())) };
    if !result.is_some_and(|v| v.to_bool()) {
        let method = if dir { "dir_opendir" } else { "stream_open" };
        return Ok(Err(format!("\"{}::{method}\" call failed", class_name(&o))));
    }
    let opened = opened.deref().into_str().ok();
    Ok(Ok((Stream::new(Target::User(Box::new(UserStream::new(o, dir))), url, mode), opened)))
}

/// Keep a stream resource a wrapper opened, for the request's end.
pub fn adopt(ctx: &mut dyn Ctx, resource: &Value) {
    if let Some(o) = resource.as_object() {
        let st = state(ctx);
        st.open.retain(|w| w.strong_count() > 0);
        st.open.push(Rc::downgrade(o));
    }
}

/// The request's end: the streams still open close (flushed first), as
/// PHP frees its resources.
pub fn close_open(ctx: &mut dyn Ctx) -> Result<(), Throw> {
    let Some(st) = ctx.ext_data(KEY).as_mut().and_then(|s| s.downcast_mut::<State>()) else { return Ok(()) };
    let open = std::mem::take(&mut st.open);
    for o in open.iter().filter_map(Weak::upgrade) {
        free(ctx, &o, &mut [])?;
    }
    Ok(())
}

/// A stream resource a wrapper opened goes (its last reference): closed
/// as fclose would.
pub fn free(ctx: &mut dyn Ctx, o: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let Some(r) = crate::stream::resource(o) else { return Ok(Value::NULL) };
    if r.closed.replace(true) {
        return Ok(Value::NULL);
    }
    let Ok(mut s) = r.stream.try_borrow_mut() else { return Ok(Value::NULL) };
    close(ctx, &mut s)?;
    Ok(Value::NULL)
}

fn user(s: &mut Stream) -> &mut UserStream {
    match &mut s.target {
        Target::User(u) => u,
        _ => unreachable!("a user-space stream"),
    }
}

/// The function a warning on an open stream names (fread, fgets, ...).
fn running(ctx: &dyn Ctx, func: &str) -> String {
    if func.is_empty() { ctx.builtin_name().to_string() } else { func.to_string() }
}

/// php_userstreamop_read: up to `count` bytes, then stream_eof asked
/// whether that was the end. None: the read failed.
fn op_read(ctx: &mut dyn Ctx, func: &str, s: &mut Stream, count: usize) -> Result<Option<Vec<u8>>, Throw> {
    let o = Rc::clone(&user(s).object);
    let Some(v) = call(ctx, &o, "stream_read", &mut [Value::int(count as i64)])? else {
        not_implemented(ctx, func, &o, "stream_read");
        return Ok(None);
    };
    if v.deref().as_bool() == Some(false) {
        return Ok(None);
    }
    let mut bytes = to_bytes(ctx, &v)?;
    if bytes.len() > count {
        ctx.warn(&format!(
            "{func}(): {}::stream_read - read {} bytes more data than requested ({} read, {count} max) - excess data will be lost",
            class_name(&o),
            bytes.len() - count,
            bytes.len()
        ));
        bytes.truncate(count);
    }
    match call(ctx, &o, "stream_eof", &mut [])? {
        Some(e) if e.to_bool() => s.eof = true,
        Some(_) => {}
        None => {
            ctx.warn(&format!("{func}(): {}::stream_eof is not implemented! Assuming EOF", class_name(&o)));
            s.eof = true;
        }
    }
    Ok(Some(bytes))
}

/// _php_stream_fill_read_buffer: at least `size` bytes buffered, if a
/// read gives them; false when the read failed.
fn fill(ctx: &mut dyn Ctx, func: &str, s: &mut Stream, size: usize) -> Result<bool, Throw> {
    if s.buffer.len() >= size {
        return Ok(true);
    }
    let chunk = s.chunk_size.max(1) as usize;
    let unread = s.buffer.len();
    let u = user(s);
    // Move what is left to the front when the end is short of a chunk,
    // then grow by a chunk if it still is.
    if u.buf_len > 0 && u.buf_len - (u.buf_start + unread) < chunk {
        u.buf_start = 0;
    }
    if u.buf_len - (u.buf_start + unread) < chunk {
        u.buf_len += chunk;
    }
    let want = u.buf_len - (u.buf_start + unread);
    match op_read(ctx, func, s, want)? {
        Some(bytes) => {
            s.buffer.extend(bytes);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// `n` buffered bytes taken by the script.
fn take(s: &mut Stream, n: usize) -> Vec<u8> {
    let n = n.min(s.buffer.len());
    user(s).buf_start += n;
    s.pos += n;
    s.buffer.drain(..n).collect()
}

/// _php_stream_read: up to `size` bytes, from the buffer and at most one
/// read. None: the read failed and nothing was read.
pub fn read(ctx: &mut dyn Ctx, func: &str, s: &mut Stream, size: usize) -> Result<Option<Vec<u8>>, Throw> {
    let func = running(ctx, func);
    let mut out = take(s, size);
    if out.len() == size {
        return Ok(Some(out));
    }
    let want = size - out.len();
    if s.chunk_size == 1 {
        return Ok(match op_read(ctx, &func, s, want)? {
            Some(bytes) => {
                s.pos += bytes.len();
                out.extend(bytes);
                Some(out)
            }
            None if out.is_empty() => None,
            None => Some(out),
        });
    }
    if !fill(ctx, &func, s, want)? && out.is_empty() {
        return Ok(None);
    }
    out.extend(take(s, want));
    Ok(Some(out))
}

/// _php_stream_get_line: through the next newline, or `limit - 1` bytes
/// (fgets's length); None at the end.
pub fn gets(ctx: &mut dyn Ctx, s: &mut Stream, limit: Option<usize>) -> Result<Option<Vec<u8>>, Throw> {
    let func = running(ctx, "");
    let chunk = s.chunk_size.max(1) as usize;
    let mut left = limit.unwrap_or(0);
    let mut out = Vec::new();
    loop {
        if !s.buffer.is_empty() {
            let (mut n, mut done) = match s.buffer.iter().position(|&b| b == b'\n') {
                Some(i) => (i + 1, true),
                None => (s.buffer.len(), false),
            };
            if limit.is_some() && n >= left.saturating_sub(1) {
                n = left.saturating_sub(1);
                done = true;
            }
            out.extend(take(s, n));
            left = left.saturating_sub(n);
            if done {
                break;
            }
        } else if s.eof {
            break;
        } else {
            let want = if limit.is_some() { left.saturating_sub(1).min(chunk) } else { chunk };
            fill(ctx, &func, s, want)?;
            if s.buffer.is_empty() {
                break;
            }
        }
    }
    Ok((!out.is_empty()).then_some(out))
}

/// _php_stream_copy_to_mem: what is left (up to `max` bytes), the stream's
/// size asked first when it is all.
pub fn read_all(ctx: &mut dyn Ctx, s: &mut Stream, max: Option<usize>) -> Result<Vec<u8>, Throw> {
    const STEP: usize = 8192;
    let func = running(ctx, "");
    let mut out = Vec::new();
    match max {
        Some(0) => return Ok(out),
        Some(max) if max < 4 * STEP => {
            while out.len() < max && !at_end(ctx, s)? {
                match read(ctx, &func, s, max - out.len())? {
                    Some(b) if !b.is_empty() => out.extend(b),
                    _ => break,
                }
            }
            return Ok(out);
        }
        _ => {}
    }
    let mut buf_len = match stat(ctx, &func, s)? {
        Some(st) if st.size > 0 => (st.size - s.pos as i64).max(0) as usize + STEP,
        _ => STEP,
    };
    if let Some(max) = max {
        buf_len = buf_len.min(max);
    }
    loop {
        match read(ctx, &func, s, buf_len - out.len())? {
            Some(b) if !b.is_empty() => out.extend(b),
            _ => break,
        }
        if out.len() + STEP / 4 >= buf_len {
            if max == Some(out.len()) {
                break;
            }
            buf_len = match max {
                Some(max) if buf_len + STEP > max => max,
                _ => buf_len + STEP,
            };
        }
    }
    Ok(out)
}

/// _php_stream_write: `bytes` through stream_write, chunk_size at a time.
/// None: nothing was written (the first write failed).
pub fn write(ctx: &mut dyn Ctx, s: &mut Stream, bytes: &[u8]) -> Result<Option<usize>, Throw> {
    if bytes.is_empty() {
        return Ok(Some(0));
    }
    let func = running(ctx, "");
    let o = Rc::clone(&user(s).object);
    if user(s).dir {
        ctx.notice(&format!("{func}(): Stream is not writable"));
        return Ok(None);
    }
    // Writing after reading ahead: back to where the script is.
    if !user(s).no_seek && !s.buffer.is_empty() {
        s.buffer.clear();
        user(s).buf_start = 0;
        let pos = s.pos as i64;
        op_seek(ctx, &func, s, pos, 0)?;
    }
    let chunk = s.chunk_size.max(1) as usize;
    let mut done = 0;
    while done < bytes.len() {
        let piece = &bytes[done..(done + chunk).min(bytes.len())];
        let wrote = match call(ctx, &o, "stream_write", &mut [Value::str(PhpStr::from(piece))])? {
            Some(v) if v.as_bool() == Some(false) => -1,
            Some(v) => v.to_int(),
            None => {
                not_implemented(ctx, &func, &o, "stream_write");
                -1
            }
        };
        let wrote = if wrote > piece.len() as i64 {
            ctx.warn(&format!(
                "{func}(): {}::stream_write wrote {} bytes more data than requested ({wrote} written, {} max)",
                class_name(&o),
                wrote - piece.len() as i64,
                piece.len()
            ));
            piece.len() as i64
        } else {
            wrote
        };
        if wrote <= 0 {
            if done == 0 {
                return Ok((wrote == 0).then_some(0));
            }
            break;
        }
        done += wrote as usize;
        s.pos += wrote as usize;
        user(s).written = true;
    }
    Ok(Some(done))
}

/// php_userstreamop_seek: stream_seek, then stream_tell for where it
/// went. Ok(None) when stream_seek is missing.
fn op_seek(ctx: &mut dyn Ctx, func: &str, s: &mut Stream, offset: i64, whence: i64) -> Result<Option<bool>, Throw> {
    let o = Rc::clone(&user(s).object);
    let Some(moved) = call(ctx, &o, "stream_seek", &mut [Value::int(offset), Value::int(whence)])? else {
        user(s).no_seek = true;
        return Ok(None);
    };
    if !moved.to_bool() {
        return Ok(Some(false));
    }
    match call(ctx, &o, "stream_tell", &mut [])? {
        Some(v) if v.is_int() => {
            s.pos = v.as_int().expect("an int").max(0) as usize;
            Ok(Some(true))
        }
        Some(_) => Ok(Some(false)),
        None => {
            not_implemented(ctx, func, &o, "stream_tell");
            Ok(Some(false))
        }
    }
}

/// _php_stream_seek (whence: SEEK_SET 0, SEEK_CUR 1, SEEK_END 2): within
/// the buffer when it can, else stream_seek; forward with reads when the
/// wrapper cannot seek.
pub fn seek(ctx: &mut dyn Ctx, s: &mut Stream, offset: i64, whence: i64) -> Result<bool, Throw> {
    let func = running(ctx, "");
    if user(s).dir {
        // rewinddir.
        let o = Rc::clone(&user(s).object);
        call(ctx, &o, "dir_rewinddir", &mut [])?;
        return Ok(true);
    }
    let here = s.pos as i64;
    let ahead = s.buffer.len() as i64;
    match whence {
        1 if offset > 0 && offset <= ahead => {
            take(s, offset as usize);
            s.eof = false;
            return Ok(true);
        }
        0 if offset > here && offset <= here + ahead => {
            take(s, (offset - here) as usize);
            s.eof = false;
            return Ok(true);
        }
        _ => {}
    }
    let (mut offset, mut whence) = (offset, whence);
    if !user(s).no_seek {
        if whence == 1 {
            (offset, whence) = (here + offset, 0);
        }
        let moved = op_seek(ctx, &func, s, offset, whence)?;
        if let Some(moved) = moved {
            if moved {
                s.eof = false;
            }
            s.buffer.clear();
            user(s).buf_start = 0;
            return Ok(moved);
        }
        // stream_seek is missing: from now on as for a wrapper that cannot
        // seek (this seek, made absolute, is not emulated).
    }
    if whence == 1 && offset >= 0 {
        let mut left = offset as usize;
        while left > 0 {
            match read(ctx, &func, s, left.min(8192))? {
                Some(b) if !b.is_empty() => left -= b.len(),
                _ => return Ok(false),
            }
        }
        s.eof = false;
        return Ok(true);
    }
    ctx.warn(&format!("{func}(): Stream does not support seeking"));
    Ok(false)
}

/// php_stream_eof: nothing buffered and the end reached, stream_eof
/// asked when the stream has not said so yet.
pub fn at_end(ctx: &mut dyn Ctx, s: &mut Stream) -> Result<bool, Throw> {
    if !s.buffer.is_empty() {
        return Ok(false);
    }
    if !s.eof {
        let o = Rc::clone(&user(s).object);
        match call(ctx, &o, "stream_eof", &mut [])?.and_then(|v| v.as_bool()) {
            Some(b) => s.eof = b,
            None => {
                let func = running(ctx, "");
                ctx.warn(&format!("{func}(): {}::stream_eof is not implemented! Assuming EOF", class_name(&o)));
                s.eof = true;
            }
        }
    }
    Ok(s.eof)
}

/// php_userstreamop_flush: stream_flush's answer.
pub fn flush(ctx: &mut dyn Ctx, s: &mut Stream) -> Result<bool, Throw> {
    if user(s).dir {
        return Ok(false);
    }
    user(s).written = false;
    let o = Rc::clone(&user(s).object);
    Ok(call(ctx, &o, "stream_flush", &mut [])?.is_some_and(|v| v.to_bool()))
}

/// php_stream_free: flushed if written to, then stream_close (or
/// dir_closedir); the wrapper's instance let go.
pub fn close(ctx: &mut dyn Ctx, s: &mut Stream) -> Result<(), Throw> {
    if !matches!(s.target, Target::User(_)) {
        return Ok(());
    }
    if user(s).written {
        flush(ctx, s)?;
    }
    let u = user(s);
    let (o, method) = (Rc::clone(&u.object), if u.dir { "dir_closedir" } else { "stream_close" });
    let r = call(ctx, &o, method, &mut []);
    s.target = Target::Memory(Vec::new());
    s.buffer.clear();
    r.map(|_| ())
}

/// php_userstreamop_stat: stream_stat's values (fstat); None when it
/// gives none.
pub fn stat(ctx: &mut dyn Ctx, func: &str, s: &mut Stream) -> Result<Option<Stat>, Throw> {
    let func = running(ctx, func);
    let o = Rc::clone(&user(s).object);
    Ok(match call(ctx, &o, "stream_stat", &mut [])? {
        Some(v) if v.is_array() => Some(Stat::from_array(v.as_array().expect("an array"))),
        Some(_) => None,
        None => {
            not_implemented(ctx, &func, &o, "stream_stat");
            None
        }
    })
}

/// ftruncate: whether the wrapper can (it has stream_truncate), and
/// whether it did.
pub fn truncate(ctx: &mut dyn Ctx, s: &mut Stream, size: i64) -> Result<Option<bool>, Throw> {
    let o = Rc::clone(&user(s).object);
    let public = o.class.method(b"stream_truncate").is_some_and(|m| m.visibility == Visibility::Public);
    if !public && o.class.method(b"__call").is_none() {
        return Ok(None);
    }
    let func = running(ctx, "");
    Ok(Some(match call(ctx, &o, "stream_truncate", &mut [Value::int(size)])? {
        Some(v) if v.is_bool() => v.as_bool().expect("a bool"),
        Some(_) => {
            ctx.warn(&format!("{func}(): {}::stream_truncate did not return a boolean!", class_name(&o)));
            false
        }
        None => {
            not_implemented(ctx, &func, &o, "stream_truncate");
            false
        }
    }))
}

/// flock (`operation`: LOCK_SH 1, LOCK_EX 2, LOCK_UN 3, LOCK_NB 4 added):
/// stream_lock's answer; operation 0 only asks whether locking works.
pub fn lock(ctx: &mut dyn Ctx, s: &mut Stream, operation: i64) -> Result<bool, Throw> {
    let o = Rc::clone(&user(s).object);
    match call(ctx, &o, "stream_lock", &mut [Value::int(operation)])? {
        Some(v) if v.is_bool() => Ok(v.as_bool().expect("a bool")),
        Some(_) => Ok(false),
        None if operation == 0 => Ok(true),
        None => {
            let func = running(ctx, "");
            not_implemented(ctx, &func, &o, "stream_lock");
            Ok(false)
        }
    }
}

/// php_userstreamop_set_option for the options a script sets
/// (OPTION_BLOCKING, OPTION_READ_TIMEOUT, OPTION_READ_BUFFER,
/// OPTION_WRITE_BUFFER): stream_set_option's answer.
pub fn set_option(ctx: &mut dyn Ctx, func: &str, s: &mut Stream, option: i64, arg1: Value, arg2: Option<i64>) -> Result<bool, Throw> {
    let func = running(ctx, func);
    let o = Rc::clone(&user(s).object);
    let arg2 = match option {
        OPTION_READ_BUFFER | OPTION_WRITE_BUFFER => Value::int(arg2.unwrap_or(BUFSIZ)),
        _ => arg2.map_or(Value::NULL, Value::int),
    };
    match call(ctx, &o, "stream_set_option", &mut [Value::int(option), arg1, arg2])? {
        Some(v) => Ok(v.to_bool()),
        None => {
            not_implemented(ctx, &func, &o, "stream_set_option");
            Ok(false)
        }
    }
}

/// readdir: dir_readdir's next name (cut to 255 bytes, as a dirent
/// holds it); None at the end.
pub fn readdir(ctx: &mut dyn Ctx, s: &mut Stream) -> Result<Option<Vec<u8>>, Throw> {
    let o = Rc::clone(&user(s).object);
    match call(ctx, &o, "dir_readdir", &mut [])? {
        Some(v) if v.is_bool() => Ok(None),
        Some(v) => {
            let mut name = to_bytes(ctx, &v)?;
            name.truncate(255);
            Ok(Some(name))
        }
        None => {
            let func = running(ctx, "");
            not_implemented(ctx, &func, &o, "dir_readdir");
            Ok(None)
        }
    }
}

/// The wrapper's url_stat for `path` through PHP's stat cache: `flags`
/// STAT_LINK (lstat) and STAT_QUIET. None: no stat (warned of when the
/// method is missing).
pub fn url_stat(ctx: &mut dyn Ctx, func: &str, w: &Wrapper, path: &str, flags: i64, context: Value) -> Result<Option<Stat>, Throw> {
    let key = path.as_bytes();
    let link = flags & STAT_LINK != 0;
    {
        let st = state(ctx);
        let cached = if link { &st.lstat } else { &st.stat };
        if let Some((p, stat)) = cached
            && p == key
        {
            return Ok(Some(*stat));
        }
    }
    let Some(o) = instance(ctx, w, context)? else { return Ok(None) };
    let stat = match call(ctx, &o, "url_stat", &mut [Value::str(path), Value::int(flags)])? {
        Some(v) if v.is_array() => Stat::from_array(v.as_array().expect("an array")),
        Some(_) => return Ok(None),
        None => {
            not_implemented(ctx, func, &o, "url_stat");
            return Ok(None);
        }
    };
    let st = state(ctx);
    if link {
        st.lstat = Some((key.to_vec(), stat));
    }
    if !link || !stat.is_link() {
        st.stat = Some((key.to_vec(), stat));
    }
    Ok(Some(stat))
}

/// php_clear_stat_cache: the next stat asks again.
pub fn clear_stat_cache(ctx: &mut dyn Ctx) {
    if let Some(st) = ctx.ext_data(KEY).as_mut().and_then(|s| s.downcast_mut::<State>()) {
        st.stat = None;
        st.lstat = None;
    }
}

/// A wrapper method that takes the URL(s) and answers true or false:
/// unlink, rename, mkdir, rmdir, stream_metadata. False when the method
/// is missing (warned of) or does not answer with a bool.
pub fn url_op(ctx: &mut dyn Ctx, func: &str, w: &Wrapper, method: &str, args: &mut [Value], context: Value) -> Result<bool, Throw> {
    let Some(o) = instance(ctx, w, context)? else { return Ok(false) };
    match call(ctx, &o, method, args)? {
        Some(v) if v.is_bool() => Ok(v.as_bool().expect("a bool")),
        Some(_) => Ok(false),
        None => {
            not_implemented(ctx, func, &o, method);
            Ok(false)
        }
    }
}

/// mkdir's and rmdir's options: REPORT_ERRORS and what was asked.
pub fn dir_options(recursive: bool) -> i64 {
    REPORT_ERRORS | if recursive { MKDIR_RECURSIVE } else { 0 }
}

/// php_userstreamop_cast for stream_select (STREAM_CAST_FOR_SELECT, 3):
/// the stream resource stream_cast hands over, which is what is waited
/// on; None (warned of) when it gives none. `this` is the stream's own
/// resource, which it must not give back.
pub fn cast(ctx: &mut dyn Ctx, func: &str, s: &mut Stream, this: &Rc<Object>) -> Result<Option<Value>, Throw> {
    let o = Rc::clone(&user(s).object);
    let Some(v) = call(ctx, &o, "stream_cast", &mut [Value::int(3)])? else {
        not_implemented(ctx, func, &o, "stream_cast");
        return Ok(None);
    };
    if !v.to_bool() {
        return Ok(None);
    }
    match crate::stream::resource_of(&v) {
        Some(r) if crate::stream::resource(&r).is_some_and(|r| r.kind == "stream") => {
            if Rc::ptr_eq(&r, this) {
                ctx.warn(&format!("{func}(): {}::stream_cast must not return itself", class_name(&o)));
                return Ok(None);
            }
            // Another user-space stream is cast in turn.
            let inner = crate::stream::resource(&r).expect("a stream resource");
            if let Ok(mut s) = inner.stream.try_borrow_mut()
                && s.user().is_some()
            {
                let cast = cast(ctx, func, &mut s, &r)?;
                if cast.is_none() {
                    ctx.warn(&format!("{func}(): Cannot represent a stream of type user-space as a select()able descriptor"));
                }
                return Ok(cast);
            }
            Ok(Some(v.deref()))
        }
        _ => {
            ctx.warn(&format!("{func}(): {}::stream_cast must return a stream resource", class_name(&o)));
            Ok(None)
        }
    }
}

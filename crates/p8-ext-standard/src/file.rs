//! Files and streams: fopen and its family, file_get_contents and
//! file_put_contents, directories, paths. Streams are resources
//! (p8_runtime::stream); php://memory, php://temp, php://stdin,
//! php://stdout, php://stderr and php://output are known.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/file.c,
//! $PHP_SRC_PATH/ext/standard/filestat.c, $PHP_SRC_PATH/ext/standard/dir.c,
//! $PHP_SRC_PATH/main/streams/plain_wrapper.c

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::{Native, Object, Visibility};
use p8_runtime::stream::{Resource, Stream, Target, resource, resource_of};
use p8_runtime::string::PhpStr;
use p8_runtime::user_stream::{self, Located, Stat};
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "fopen", run: fopen, by_ref: &[] },
    Builtin { name: "fclose", run: fclose, by_ref: &[] },
    Builtin { name: "fwrite", run: fwrite, by_ref: &[] },
    Builtin { name: "fputs", run: fwrite, by_ref: &[] },
    Builtin { name: "fread", run: fread, by_ref: &[] },
    Builtin { name: "fgets", run: fgets, by_ref: &[] },
    Builtin { name: "fgetc", run: fgetc, by_ref: &[] },
    Builtin { name: "feof", run: feof, by_ref: &[] },
    Builtin { name: "stream_get_meta_data", run: stream_get_meta_data, by_ref: &[] },
    Builtin { name: "stream_isatty", run: stream_isatty, by_ref: &[] },
    Builtin { name: "stream_resolve_include_path", run: stream_resolve_include_path, by_ref: &[] },
    Builtin { name: "fflush", run: fflush, by_ref: &[] },
    Builtin { name: "ftell", run: ftell, by_ref: &[] },
    Builtin { name: "fseek", run: fseek, by_ref: &[] },
    Builtin { name: "rewind", run: rewind, by_ref: &[] },
    Builtin { name: "ftruncate", run: ftruncate, by_ref: &[] },
    Builtin { name: "flock", run: flock, by_ref: &[] },
    Builtin { name: "fprintf", run: fprintf, by_ref: &[] },
    Builtin { name: "fpassthru", run: fpassthru, by_ref: &[] },
    Builtin { name: "stream_get_contents", run: stream_get_contents, by_ref: &[] },
    Builtin { name: "stream_copy_to_stream", run: stream_copy_to_stream, by_ref: &[] },
    Builtin { name: "stream_is_local", run: stream_is_local, by_ref: &[] },
    Builtin { name: "tmpfile", run: tmpfile, by_ref: &[] },
    Builtin { name: "file_get_contents", run: file_get_contents, by_ref: &[] },
    Builtin { name: "file_put_contents", run: file_put_contents, by_ref: &[] },
    Builtin { name: "file", run: file, by_ref: &[] },
    Builtin { name: "readfile", run: readfile, by_ref: &[] },
    Builtin { name: "unlink", run: unlink, by_ref: &[] },
    Builtin { name: "mkdir", run: mkdir, by_ref: &[] },
    Builtin { name: "rmdir", run: rmdir, by_ref: &[] },
    Builtin { name: "rename", run: rename, by_ref: &[] },
    Builtin { name: "is_uploaded_file", run: is_uploaded_file, by_ref: &[] },
    Builtin { name: "move_uploaded_file", run: move_uploaded_file, by_ref: &[] },
    Builtin { name: "copy", run: copy, by_ref: &[] },
    Builtin { name: "touch", run: touch, by_ref: &[] },
    Builtin { name: "is_file", run: is_file, by_ref: &[] },
    Builtin { name: "is_dir", run: is_dir, by_ref: &[] },
    Builtin { name: "is_link", run: is_link, by_ref: &[] },
    Builtin { name: "file_exists", run: file_exists, by_ref: &[] },
    Builtin { name: "is_readable", run: is_readable, by_ref: &[] },
    Builtin { name: "is_writable", run: is_writable, by_ref: &[] },
    Builtin { name: "is_writeable", run: is_writable, by_ref: &[] },
    Builtin { name: "is_executable", run: is_executable, by_ref: &[] },
    Builtin { name: "filesize", run: filesize, by_ref: &[] },
    Builtin { name: "filemtime", run: filemtime, by_ref: &[] },
    Builtin { name: "fileatime", run: filemtime, by_ref: &[] },
    Builtin { name: "filectime", run: filemtime, by_ref: &[] },
    Builtin { name: "fileperms", run: fileperms, by_ref: &[] },
    Builtin { name: "chmod", run: chmod, by_ref: &[] },
    Builtin { name: "umask", run: umask, by_ref: &[] },
    Builtin { name: "clearstatcache", run: clearstatcache, by_ref: &[] },
    Builtin { name: "tempnam", run: tempnam, by_ref: &[] },
    Builtin { name: "sys_get_temp_dir", run: sys_get_temp_dir, by_ref: &[] },
    Builtin { name: "getcwd", run: getcwd, by_ref: &[] },
    Builtin { name: "chdir", run: chdir, by_ref: &[] },
    Builtin { name: "realpath", run: realpath, by_ref: &[] },
    Builtin { name: "pathinfo", run: pathinfo, by_ref: &[] },
    Builtin { name: "scandir", run: scandir, by_ref: &[] },
    Builtin { name: "glob", run: glob, by_ref: &[] },
    Builtin { name: "fnmatch", run: fnmatch, by_ref: &[] },
    Builtin { name: "opendir", run: opendir, by_ref: &[] },
    Builtin { name: "stream_get_wrappers", run: stream_get_wrappers, by_ref: &[] },
    Builtin { name: "readdir", run: readdir, by_ref: &[] },
    Builtin { name: "rewinddir", run: rewinddir, by_ref: &[] },
    Builtin { name: "dir", run: dir, by_ref: &[] },
    Builtin { name: "closedir", run: fclose, by_ref: &[] },
    Builtin { name: "stream_wrapper_register", run: stream_wrapper_register, by_ref: &[] },
    Builtin { name: "stream_register_wrapper", run: stream_wrapper_register, by_ref: &[] },
    Builtin { name: "stream_wrapper_unregister", run: stream_wrapper_unregister, by_ref: &[] },
    Builtin { name: "stream_wrapper_restore", run: stream_wrapper_restore, by_ref: &[] },
];

/// The functions that take a path a user-space wrapper may serve: they
/// run its methods (PHP code). Those that take a stream resource already
/// may (an object argument). stream_wrapper_register autoloads its class.
pub(crate) const REENTER: &[&str] = &[
    "fopen", "file_get_contents", "file_put_contents", "file", "readfile", "unlink", "mkdir", "rmdir", "rename", "copy", "touch", "chmod", "is_file", "is_dir",
    "is_link", "file_exists", "is_readable", "is_writable", "is_writeable", "is_executable", "filesize", "filemtime", "fileatime", "filectime", "fileperms",
    "scandir", "opendir", "dir", "stream_is_local", "stream_select", "stream_wrapper_register", "stream_register_wrapper",
];

fn text(s: &PhpStr) -> String {
    String::from_utf8_lossy(s.as_bytes()).into_owned()
}

/// The stream resource in argument `i`: TypeError for anything else, and
/// for a closed one.
/// The name the running builtin was called by (an alias, gzread for
/// fread, names itself), else `default`.
fn called<'a>(ctx: &dyn Ctx, default: &'a str) -> &'a str {
    match ctx.builtin_name() {
        "" => default,
        name => name,
    }
}

pub(crate) fn stream_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value], i: usize) -> Result<Rc<Object>, Throw> {
    let func = called(ctx, func);
    let v = arg(args, i);
    match resource_of(&v) {
        Some(o) if resource(&o).is_some_and(|r| !r.closed.get()) => Ok(o),
        Some(_) => Err(ctx.throw("TypeError", format!("{func}(): supplied resource is not a valid stream resource"))),
        None => Err(ctx.throw(
            "TypeError",
            format!("{func}(): Argument #{} ($stream) must be of type resource, {} given", i + 1, v.type_name()),
        )),
    }
}

fn with_stream<R>(o: &Object, f: impl FnOnce(&mut Stream) -> R) -> R {
    let r = resource(o).expect("a stream resource");
    f(&mut r.stream.borrow_mut())
}

/// Whether the stream is a user-space wrapper's (its operations run PHP
/// code, so they go through the `_in` methods with the context).
pub(crate) fn is_user(o: &Object) -> bool {
    resource(o).is_some_and(|r| r.stream.try_borrow().is_ok_and(|s| s.user().is_some()))
}

/// An operation on a stream that may run PHP code (a user-space
/// wrapper's), the stream borrowed meanwhile: the wrapper's own code
/// reaching for it again finds it busy (`busy` is the answer) rather than
/// the borrow panicking.
pub(crate) fn with_user<R>(ctx: &mut dyn Ctx, o: &Object, busy: R, f: impl FnOnce(&mut dyn Ctx, &mut Stream) -> Result<R, Throw>) -> Result<R, Throw> {
    let r = resource(o).expect("a stream resource");
    match r.stream.try_borrow_mut() {
        Ok(mut s) => f(ctx, &mut s),
        Err(_) => Ok(busy),
    }
}

pub(crate) fn new_stream(ctx: &mut dyn Ctx, stream: Stream) -> Value {
    let user = stream.user().is_some();
    let v = ctx.new_resource(Resource { kind: "stream", closed: Cell::new(false), stream: RefCell::new(stream) });
    if user {
        user_stream::adopt(ctx, &v);
    }
    v
}

/// The stream context argument `i`, if one was given (null is none).
fn context_arg(args: &[Value], i: usize) -> Option<Value> {
    args.get(i).map(Value::deref).filter(|v| resource_of(v).is_some())
}

/// A user-space wrapper's context: the one given, else the request's default.
fn context_or_default(ctx: &mut dyn Ctx, context: Option<Value>) -> Value {
    context.unwrap_or_else(|| user_stream::default_context(ctx))
}

/// "Failed to open stream" for a path no wrapper serves (file://
/// unregistered).
fn disabled(ctx: &mut dyn Ctx, func: &str, shown: &str) {
    user_stream::warn_disabled(ctx, func);
    ctx.warn(&format!("{func}({shown}): Failed to open stream: no suitable wrapper could be found"));
}

/// `path` opened by the user-space wrapper serving it (stream_open given
/// `options` and the context; PHP's warning when it fails), or refused
/// when file:// is unregistered: None when p8's own streams serve it.
fn open_user(ctx: &mut dyn Ctx, func: &str, path: &str, mode: &str, options: i64, context: Option<Value>) -> Result<Option<Option<Stream>>, Throw> {
    match user_stream::locate(ctx, func, path) {
        Located::User(w, local) => {
            let context = context_or_default(ctx, context);
            Ok(Some(match user_stream::open(ctx, &w, &local, mode, options, context, false)? {
                Ok((s, _)) => Some(s),
                Err(reason) => {
                    ctx.warn(&format!("{func}({path}): Failed to open stream: {reason}"));
                    None
                }
            }))
        }
        Located::Disabled => {
            disabled(ctx, func, path);
            Ok(Some(None))
        }
        Located::Builtin => Ok(None),
    }
}

/// php_stat for a path a user-space wrapper serves (its url_stat, through
/// PHP's stat cache): None when p8 serves the path itself. A failed stat
/// is warned of unless `quiet` (the is_* and file_exists checks).
pub(crate) fn user_stat(ctx: &mut dyn Ctx, func: &str, path: &str, link: bool, quiet: bool) -> Result<Option<Option<Stat>>, Throw> {
    let stat = match user_stream::locate(ctx, func, path) {
        Located::User(w, local) => {
            let flags = user_stream::STAT_IGNORE_OPEN_BASEDIR | if link { user_stream::STAT_LINK } else { 0 } | if quiet { user_stream::STAT_QUIET } else { 0 };
            user_stream::url_stat(ctx, func, &w, &local, flags, Value::NULL)?
        }
        Located::Disabled => None,
        Located::Builtin => return Ok(None),
    };
    if stat.is_none() && !quiet {
        ctx.warn(&format!("{func}(): {}stat failed for {path}", if link { "L" } else { "" }));
    }
    Ok(Some(stat))
}

/// The uid this process runs as, which php_stat compares a file's owner
/// with (getuid()): a new file's owner, learnt once.
#[cfg(unix)]
fn process_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    static UID: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *UID.get_or_init(|| {
        let probe = std::env::temp_dir().join(format!("p8-uid-{}", std::process::id()));
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).ok()?;
        let uid = file.metadata().ok().map(|m| m.uid());
        let _ = std::fs::remove_file(&probe);
        uid
    })
}

/// The uid this process runs as: WASI has no users, and wasi-libc's stat()
/// gives every file the owner 0.
#[cfg(target_os = "wasi")]
fn process_uid() -> Option<u32> {
    Some(0)
}

/// is_readable/is_writable/is_executable for a user-space wrapper's stat:
/// the owner's bits when the process owns it, else the others' (`mask`:
/// the owner's bit).
fn user_access(st: &Stat, mask: i64) -> bool {
    let owner = process_uid().is_some_and(|uid| i64::from(uid) == st.uid);
    st.mode & if owner { mask } else { mask >> 6 } != 0
}

/// A user-space wrapper's stat, or None when p8 serves the path: the
/// answer of an is_* check.
fn user_check(ctx: &mut dyn Ctx, func: &str, path: &str, link: bool, check: impl FnOnce(&Stat) -> bool) -> Result<Option<Value>, Throw> {
    Ok(user_stat(ctx, func, path, link, true)?.map(|st| Value::bool(st.as_ref().is_some_and(check))))
}

/// A user-space wrapper's stat value for a function that warns when there
/// is none (filesize, filemtime, ...); None when p8 serves the path.
pub(crate) fn user_field(ctx: &mut dyn Ctx, func: &str, path: &str, link: bool, field: impl FnOnce(&Stat) -> Value) -> Result<Option<Value>, Throw> {
    Ok(user_stat(ctx, func, path, link, false)?.map(|st| st.as_ref().map_or(Value::FALSE, field)))
}

/// A user-space wrapper's url operation (unlink, rename, mkdir, rmdir,
/// stream_metadata) for `path`: None when p8 serves the path.
fn user_op(ctx: &mut dyn Ctx, func: &str, path: &str, method: &str, args: &mut [Value], context: Option<Value>) -> Result<Option<Value>, Throw> {
    match user_stream::locate(ctx, func, path) {
        Located::User(w, _) => {
            let context = match method {
                // touch, chmod and chown pass no context.
                "stream_metadata" => Value::NULL,
                _ => context_or_default(ctx, context),
            };
            Ok(Some(Value::bool(user_stream::url_op(ctx, func, &w, method, args, context)?)))
        }
        Located::Disabled => {
            ctx.warn(&format!("{func}(): Unable to locate stream wrapper"));
            Ok(Some(Value::FALSE))
        }
        Located::Builtin => Ok(None),
    }
}

/// The OS error PHP shows ("No such file or directory").
pub(crate) fn os_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        std::io::ErrorKind::AlreadyExists => "File exists".into(),
        std::io::ErrorKind::DirectoryNotEmpty => "Directory not empty".into(),
        std::io::ErrorKind::IsADirectory => "Is a directory".into(),
        std::io::ErrorKind::NotADirectory => "Not a directory".into(),
        // What std says of a call WASI lacks (a process, a socket), as
        // wasi-libc's strerror(ENOTSUP) says it.
        #[cfg(target_os = "wasi")]
        std::io::ErrorKind::Unsupported => "Not supported".into(),
        _ => {
            let s = e.to_string();
            match s.find(" (os error") {
                Some(at) => s[..at].to_string(),
                None => s,
            }
        }
    }
}

/// Open `path` with an fopen mode (php:// names included).
fn open(ctx: &mut dyn Ctx, func: &str, path: &PhpStr, mode: &str) -> Result<Option<Stream>, Throw> {
    open_with(ctx, func, path, mode, 0, None)
}

/// Open `path` with an fopen mode: a user-space wrapper's URL is opened by
/// its class (stream_open given `options` and the context), anything else
/// as [`open`] does.
fn open_with(ctx: &mut dyn Ctx, func: &str, path: &PhpStr, mode: &str, options: i64, context: Option<Value>) -> Result<Option<Stream>, Throw> {
    let p = text(path);
    if let Some(opened) = open_user(ctx, func, &p, mode, options, context)? {
        return Ok(opened);
    }
    open_builtin(ctx, func, p, mode)
}

/// `p` opened by p8's own streams (php://, compress.zlib://, http://, an
/// extension's scheme, a file).
fn open_builtin(ctx: &mut dyn Ctx, func: &str, p: String, mode: &str) -> Result<Option<Stream>, Throw> {
    if let Some(gz) = p.strip_prefix("compress.zlib://") {
        return Ok(crate::zlib::open_gz(ctx, func, gz, mode));
    }
    let lower = p.to_ascii_lowercase();
    if lower.starts_with("http://") && !mode.starts_with(['w', 'a', 'x', 'c']) {
        return Ok(http_get(ctx, func, &p).map(|body| Stream::new(Target::Memory(body), &p, mode)));
    }
    if let Some(rest) = lower.strip_prefix("php://") {
        let target = match rest.split('/').next().unwrap_or("") {
            "memory" | "temp" => Target::Memory(Vec::new()),
            "stdin" => Target::Stdin,
            "stdout" | "output" => Target::Stdout,
            "stderr" => Target::Stderr,
            // The request body (what the host gave a web request).
            "input" => Target::Memory(match ctx.state("php_input").as_str() {
                Some(s) => s.as_bytes().to_vec(),
                None => Vec::new(),
            }),
            _ => {
                ctx.warn(&format!("{func}(): Invalid php:// URL specified"));
                ctx.warn(&format!("{func}({p}): Failed to open stream: operation failed"));
                return Ok(None);
            }
        };
        return Ok(Some(Stream::new(target, &p, mode)));
    }
    if let Some((scheme, _)) = p.split_once("://")
        && let Some(open) = p8_runtime::stream::wrapper(&scheme.to_ascii_lowercase())
    {
        return Ok(open(ctx, func, &p, mode));
    }
    let p = p.strip_prefix("file://").map(str::to_string).unwrap_or(p);
    // The first letter and whether a '+' is anywhere in it ("rb+" is "r+"),
    // as php_stream_parse_fopen_modes.
    let first = mode.get(..1).unwrap_or("");
    let plus = if mode.contains('+') { "+" } else { "" };
    let base = format!("{first}{plus}");
    let base = base.as_str();
    let mut options = std::fs::OpenOptions::new();
    match base {
        "r" => options.read(true),
        "r+" => options.read(true).write(true),
        "w" => options.write(true).create(true).truncate(true),
        "w+" => options.read(true).write(true).create(true).truncate(true),
        "a" => options.append(true).create(true),
        "a+" => options.read(true).append(true).create(true),
        "x" => options.write(true).create_new(true),
        "x+" => options.read(true).write(true).create_new(true),
        "c" => options.write(true).create(true),
        "c+" => options.read(true).write(true).create(true),
        _ => {
            return Err(ctx.throw(
                "ValueError",
                format!("{func}(): Argument #2 ($mode) must be a valid mode"),
            ));
        }
    };
    if Path::new(&p).is_dir() && base != "r" {
        ctx.warn(&format!("{func}({p}): Failed to open stream: Is a directory"));
        return Ok(None);
    }
    match options.open(&p) {
        Ok(f) => Ok(Some(Stream::new(Target::File(f), &p, mode))),
        Err(e) => {
            ctx.warn(&format!("{func}({p}): Failed to open stream: {}", os_error(&e)));
            Ok(None)
        }
    }
}

/// fopen(string $filename, string $mode, bool $use_include_path = false, $context = null)
fn fopen(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = str_arg(args, 0);
    if path.is_empty() {
        return Err(ctx.throw("ValueError", "Path cannot be empty".into()));
    }
    let mode = text(&str_arg(args, 1));
    let options = if args.len() > 2 && arg(args, 2).to_bool() { user_stream::USE_PATH } else { 0 };
    match open_with(ctx, "fopen", &path, &mode, options, context_arg(args, 3))? {
        Some(s) => Ok(new_stream(ctx, s)),
        None => Ok(Value::FALSE),
    }
}

pub(crate) fn fclose(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fclose", args, 0)?;
    let r = resource(&o).expect("a stream resource");
    r.closed.set(true);
    // The file itself closes now (the resource value lives on, closed).
    with_user(ctx, &o, (), |ctx, s| s.close_in(ctx))?;
    Ok(Value::TRUE)
}

/// fwrite(resource $stream, string $data, ?int $length = null)
pub(crate) fn fwrite(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fwrite", args, 0)?;
    let data = str_arg(args, 1);
    let mut bytes = data.as_bytes();
    if args.len() > 2 && !arg(args, 2).is_null() {
        let n = arg(args, 2).to_int().max(0) as usize;
        bytes = &bytes[..n.min(bytes.len())];
    }
    let (writable, zlib) = with_stream(&o, |s| (s.writable(), s.zlib));
    if !writable && zlib {
        // A gzip file being read takes nothing.
        return Ok(Value::int(0));
    }
    // A user-space wrapper decides for itself what its mode allows.
    if !writable && !is_user(&o) {
        ctx.report(8, &format!("fwrite(): Write of {} bytes failed with errno=9 Bad file descriptor", bytes.len()));
        return Ok(Value::FALSE);
    }
    let written = with_user(ctx, &o, None, |ctx, s| s.write(ctx, bytes))?;
    Ok(written.map_or(Value::FALSE, |n| Value::int(n as i64)))
}

pub(crate) fn fread(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fread", args, 0)?;
    let n = arg(args, 1).to_int();
    if n <= 0 {
        return Err(ctx.throw("ValueError", format!("{}(): Argument #2 ($length) must be greater than 0", called(ctx, "fread"))));
    }
    if with_stream(&o, |s| s.zlib && !s.readable()) {
        return Ok(Value::FALSE);
    }
    let read = with_user(ctx, &o, None, |ctx, s| s.read_in(ctx, n as usize))?;
    Ok(read.map_or(Value::FALSE, |b| Value::str(PhpStr::new(b))))
}

pub(crate) fn fgets(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fgets", args, 0)?;
    let limit = match args.get(1).map(Value::deref) {
        Some(v) if !v.is_null() => {
            let n = v.to_int();
            if n <= 0 {
                return Err(ctx.throw("ValueError", format!("{}(): Argument #2 ($length) must be greater than 0", called(ctx, "fgets"))));
            }
            Some(n as usize)
        }
        _ => None,
    };
    let line = with_user(ctx, &o, None, |ctx, s| s.gets_in(ctx, limit))?;
    Ok(line.map_or(Value::FALSE, |b| Value::str(PhpStr::new(b))))
}

pub(crate) fn fgetc(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fgetc", args, 0)?;
    let b = with_user(ctx, &o, None, |ctx, s| s.read_in(ctx, 1))?.unwrap_or_default();
    Ok(if b.is_empty() { Value::FALSE } else { Value::str(PhpStr::new(b)) })
}

pub(crate) fn feof(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "feof", args, 0)?;
    Ok(Value::bool(with_user(ctx, &o, true, |ctx, s| s.at_end_in(ctx))?))
}

fn fflush(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fflush", args, 0)?;
    Ok(Value::bool(with_user(ctx, &o, false, |ctx, s| s.flush_in(ctx))?))
}

pub(crate) fn ftell(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "ftell", args, 0)?;
    Ok(Value::int(with_stream(&o, |s| s.tell())))
}

pub(crate) fn fseek(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fseek", args, 0)?;
    let offset = arg(args, 1).to_int();
    let whence = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    if with_stream(&o, |s| s.zlib) {
        if whence == 2 {
            ctx.warn(&format!("{}(): SEEK_END is not supported", called(ctx, "gzseek")));
            return Ok(Value::int(-1));
        }
        // Writing, zlib only goes forward.
        let (writing, here) = with_stream(&o, |s| (s.writable(), s.tell()));
        let to = if whence == 1 { here + offset } else { offset };
        if writing && to < here {
            return Ok(Value::int(-1));
        }
    }
    Ok(Value::int(if with_user(ctx, &o, false, |ctx, s| s.seek_in(ctx, offset, whence))? { 0 } else { -1 }))
}

pub(crate) fn rewind(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "rewind", args, 0)?;
    if with_stream(&o, |s| s.zlib && s.writable()) {
        return Ok(Value::FALSE);
    }
    Ok(Value::bool(with_user(ctx, &o, false, |ctx, s| s.seek_in(ctx, 0, 0))?))
}

fn ftruncate(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "ftruncate", args, 0)?;
    let size = arg(args, 1).to_int();
    if size < 0 {
        return Err(ctx.throw("ValueError", "ftruncate(): Argument #2 ($size) must be greater than or equal to 0".into()));
    }
    if is_user(&o) {
        // stream_truncate, if the wrapper has one.
        return match with_user(ctx, &o, Some(false), |ctx, s| user_stream::truncate(ctx, s, size))? {
            Some(done) => Ok(Value::bool(done)),
            None => {
                ctx.warn("ftruncate(): Can't truncate this stream!");
                Ok(Value::FALSE)
            }
        };
    }
    Ok(Value::bool(with_stream(&o, |s| s.truncate(size as usize))))
}

/// flock(resource $stream, int $operation, int &$would_block = null): a
/// user-space wrapper's stream_lock decides; p8 holds no other locks.
fn flock(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    const LOCK_UN: i64 = 3;
    let o = stream_arg(ctx, "flock", args, 0)?;
    let operation = arg(args, 1).to_int();
    if !(1..=3).contains(&(operation & LOCK_UN)) {
        return Err(ctx.throw("ValueError", "flock(): Argument #2 ($operation) must be one of LOCK_SH, LOCK_EX, or LOCK_UN".into()));
    }
    if is_user(&o) {
        return Ok(Value::bool(with_user(ctx, &o, false, |ctx, s| user_stream::lock(ctx, s, operation))?));
    }
    Ok(Value::TRUE)
}

/// fprintf(resource $stream, string $format, mixed ...$values)
fn fprintf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fprintf", args, 0)?;
    let bytes = crate::printf::formatted(ctx, "fprintf", crate::str_arg(args, 1).as_bytes(), &args[2..], crate::printf::Missing::Count(2))?;
    with_user(ctx, &o, None, |ctx, s| s.write(ctx, &bytes))?;
    Ok(Value::int(bytes.len() as i64))
}

pub(crate) fn fpassthru(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fpassthru", args, 0)?;
    if is_user(&o) {
        return Ok(Value::int(with_user(ctx, &o, 0, passthru)? as i64));
    }
    let rest = with_stream(&o, |s| s.read_all());
    ctx.echo(&rest);
    Ok(Value::int(rest.len() as i64))
}

/// _php_stream_passthru: what is left, echoed a read (8 KiB) at a time;
/// how much.
fn passthru(ctx: &mut dyn Ctx, s: &mut Stream) -> Result<usize, Throw> {
    let mut n = 0;
    while let Some(b) = s.read_in(ctx, 8192)?.filter(|b| !b.is_empty()) {
        ctx.echo(&b);
        n += b.len();
    }
    Ok(n)
}

/// _php_stream_copy_to_stream_ex: up to `max` bytes (all: None) read 8 KiB
/// at a time and written on; how many, None when a write failed.
fn copy_streams(ctx: &mut dyn Ctx, from: &mut Stream, to: &mut Stream, max: Option<usize>) -> Result<Option<usize>, Throw> {
    let mut n = 0;
    loop {
        let chunk = max.map_or(8192, |m| (m - n).min(8192));
        if chunk == 0 {
            break;
        }
        let Some(bytes) = from.read_in(ctx, chunk)?.filter(|b| !b.is_empty()) else { break };
        n += bytes.len();
        if to.write(ctx, &bytes)?.is_none_or(|w| w == 0) {
            return Ok(None);
        }
    }
    Ok(Some(n))
}

/// stream_get_contents(resource $stream, ?int $length = null, int $offset = -1)
fn stream_get_contents(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_get_contents", args, 0)?;
    let offset = if args.len() > 2 { arg(args, 2).to_int() } else { -1 };
    let length = match args.get(1).map(Value::deref) {
        Some(v) if !v.is_null() => Some(v.to_int()),
        _ => None,
    };
    if is_user(&o) {
        let read = with_user(ctx, &o, None, |ctx, s| {
            let here = s.tell();
            if offset >= 0 && offset != here {
                let moved = if offset > here { s.seek_in(ctx, offset - here, 1)? } else { s.seek_in(ctx, offset, 0)? };
                if !moved {
                    ctx.warn(&format!("stream_get_contents(): Failed to seek to position {offset} in the stream"));
                    return Ok(None);
                }
            }
            s.read_all_in(ctx, length.filter(|&n| n >= 0).map(|n| n as usize)).map(Some)
        })?;
        return Ok(read.map_or(Value::FALSE, |b| Value::str(PhpStr::new(b))));
    }
    Ok(Value::str(PhpStr::new(with_stream(&o, |s| {
        if offset >= 0 {
            s.seek(offset, 0);
        }
        match length {
            Some(n) if n >= 0 => s.read(n as usize),
            _ => s.read_all(),
        }
    }))))
}

/// stream_copy_to_stream(resource $from, resource $to, ?int $length =
/// null, int $offset = 0): int|false, the bytes copied.
fn stream_copy_to_stream(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let from = stream_arg(ctx, "stream_copy_to_stream", args, 0)?;
    let to = stream_arg(ctx, "stream_copy_to_stream", args, 1)?;
    let length = match args.get(2).map(Value::deref) {
        Some(v) if !v.is_null() => Some(v.to_int()),
        _ => None,
    };
    let offset = if args.len() > 3 { arg(args, 3).to_int() } else { 0 };
    if is_user(&from) || is_user(&to) {
        let (Some(rf), Some(rt)) = (resource(&from), resource(&to)) else { return Ok(Value::FALSE) };
        let (Ok(mut src), Ok(mut dst)) = (rf.stream.try_borrow_mut(), rt.stream.try_borrow_mut()) else { return Ok(Value::FALSE) };
        if offset > 0 && !src.seek_in(ctx, offset, 0)? {
            ctx.warn(&format!("stream_copy_to_stream(): Failed to seek to position {offset} in the stream"));
            return Ok(Value::FALSE);
        }
        let max = length.filter(|&n| n >= 0).map(|n| n as usize);
        return Ok(copy_streams(ctx, &mut src, &mut dst, max)?.map_or(Value::FALSE, |n| Value::int(n as i64)));
    }
    let bytes = with_stream(&from, |s| {
        if offset > 0 && !s.seek(offset, 0) {
            return None;
        }
        Some(match length {
            Some(n) if n >= 0 => s.read(n as usize),
            _ => s.read_all(),
        })
    });
    let Some(bytes) = bytes else {
        ctx.warn(&format!("stream_copy_to_stream(): Failed to seek to position {offset} in the stream"));
        return Ok(Value::FALSE);
    };
    if bytes.is_empty() {
        return Ok(Value::int(0));
    }
    let r = resource(&to).expect("a stream resource");
    let written = r.stream.borrow_mut().write(ctx, &bytes)?;
    Ok(written.map_or(Value::FALSE, |n| Value::int(n as i64)))
}

/// stream_is_local(resource|string $stream): whether it is not over a
/// network (a URL wrapper: http, https, ftp, data; a socket; a user-space
/// wrapper registered with STREAM_IS_URL).
fn stream_is_local(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    if let Some(o) = resource_of(&arg(args, 0)) {
        if is_user(&o) {
            let url = with_stream(&o, |s| s.path.clone());
            return Ok(Value::bool(!matches!(user_stream::locate(ctx, "stream_is_local", &url), Located::User(w, _) if w.is_url)));
        }
        return Ok(Value::bool(resource(&o).is_some_and(|r| !matches!(r.stream.borrow().target, Target::Tcp(_)))));
    }
    if let Located::User(w, _) = user_stream::locate(ctx, "stream_is_local", &text(&str_arg(args, 0))) {
        return Ok(Value::bool(!w.is_url));
    }
    let url = text(&str_arg(args, 0)).to_ascii_lowercase();
    let scheme = url.split_once("://").map(|(s, _)| s).or_else(|| url.starts_with("data:").then_some("data"));
    Ok(Value::bool(!matches!(scheme, Some("http" | "https" | "ftp" | "ftps" | "data"))))
}

/// tmpfile(): a new file in the temporary directory, open for reading and
/// writing, removed when it is closed (as PHP's: its path is a real file's,
/// which stream_get_meta_data() gives).
fn tmpfile(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let dir = temp_dir();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for i in 0..1000u32 {
        let path = format!("{dir}/php{:06x}", (stamp as u32).wrapping_add(i.wrapping_mul(7919)) & 0xff_ffff);
        if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path) {
            let mut s = Stream::new(Target::File(f), &path, "r+b");
            let gone = path.clone();
            s.on_close = Some(Box::new(move |_| {
                let _ = std::fs::remove_file(&gone);
            }));
            return Ok(new_stream(ctx, s));
        }
    }
    ctx.warn("tmpfile(): Unable to create temporary file, Check permissions in temporary files directory.");
    Ok(Value::FALSE)
}

/// The bytes of a file (or php:// stream), or None after PHP's warning.
fn read_path(ctx: &mut dyn Ctx, func: &str, path: &PhpStr, options: i64, context: Option<Value>) -> Result<Option<Vec<u8>>, Throw> {
    if path.is_empty() {
        return Err(ctx.throw("ValueError", "Path cannot be empty".into()));
    }
    let p = text(path);
    if let Some(opened) = open_user(ctx, func, &p, "rb", options, context)? {
        let Some(mut s) = opened else { return Ok(None) };
        let bytes = s.read_all_in(ctx, None)?;
        s.close_in(ctx)?;
        return Ok(Some(bytes));
    }
    read_builtin(ctx, func, p)
}

/// The bytes p8's own streams have for `p` (no user-space wrapper serves
/// it), or None after PHP's warning.
fn read_builtin(ctx: &mut dyn Ctx, func: &str, p: String) -> Result<Option<Vec<u8>>, Throw> {
    if p.starts_with("php://") {
        return Ok(open_builtin(ctx, func, p, "r")?.map(|mut s| s.read_all()));
    }
    if let Some(gz) = p.strip_prefix("compress.zlib://") {
        return Ok(crate::zlib::open_gz(ctx, func, gz, "r").map(|mut s| s.read_all()));
    }
    if p.to_ascii_lowercase().starts_with("http://") {
        return Ok(http_get(ctx, func, &p));
    }
    if let Some((scheme, _)) = p.split_once("://")
        && let Some(open_wrapper) = p8_runtime::stream::wrapper(&scheme.to_ascii_lowercase())
    {
        return Ok(open_wrapper(ctx, func, &p, "r").map(|mut s| s.read_all()));
    }
    let p = p.strip_prefix("file://").map(str::to_string).unwrap_or(p);
    if Path::new(&p).is_dir() {
        ctx.warn(&format!("{func}(): Read of 8192 bytes failed with errno=21 Is a directory"));
        return Ok(Some(Vec::new()));
    }
    match std::fs::read(&p) {
        Ok(b) => Ok(Some(b)),
        Err(e) => {
            ctx.warn(&format!("{func}({p}): Failed to open stream: {}", os_error(&e)));
            Ok(None)
        }
    }
}

/// file_get_contents(string $filename, bool $use_include_path = false, $context = null,
/// int $offset = 0, ?int $length = null)
fn file_get_contents(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = str_arg(args, 0);
    let options = if args.len() > 1 && arg(args, 1).to_bool() { user_stream::USE_PATH } else { 0 };
    let offset = if args.len() > 3 { arg(args, 3).to_int() } else { 0 };
    let length = args.get(4).map(Value::deref).filter(|v| !v.is_null()).map(|v| v.to_int());
    if length.is_some_and(|n| n < 0) {
        return Err(ctx.throw("ValueError", "file_get_contents(): Argument #5 ($length) must be greater than or equal to 0".into()));
    }
    if !path.is_empty()
        && let Some(opened) = open_user(ctx, "file_get_contents", &text(&path), "rb", options, context_arg(args, 2))?
    {
        // Through the wrapper: seek to the offset, read up to the length.
        let Some(mut s) = opened else { return Ok(Value::FALSE) };
        if offset != 0 && !s.seek_in(ctx, offset, if offset > 0 { 0 } else { 2 })? {
            ctx.warn(&format!("file_get_contents(): Failed to seek to position {offset} in the stream"));
            s.close_in(ctx)?;
            return Ok(Value::FALSE);
        }
        let bytes = s.read_all_in(ctx, length.map(|n| n as usize))?;
        s.close_in(ctx)?;
        return Ok(Value::str(PhpStr::new(bytes)));
    }
    if path.is_empty() {
        return Err(ctx.throw("ValueError", "Path cannot be empty".into()));
    }
    let Some(mut bytes) = read_builtin(ctx, "file_get_contents", text(&path))? else { return Ok(Value::FALSE) };
    if offset != 0 {
        let at = if offset < 0 { (bytes.len() as i64 + offset).max(0) as usize } else { (offset as usize).min(bytes.len()) };
        bytes.drain(..at);
    }
    if let Some(n) = length {
        bytes.truncate(n as usize);
    }
    Ok(Value::str(PhpStr::new(bytes)))
}

const FILE_USE_INCLUDE_PATH: i64 = 1;
const FILE_IGNORE_NEW_LINES: i64 = 2;
const FILE_SKIP_EMPTY_LINES: i64 = 4;
const FILE_APPEND: i64 = 8;

/// file_put_contents(string $filename, mixed $data, int $flags = 0, $context = null)
fn file_put_contents(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    const LOCK_EX: i64 = 2;
    let path = str_arg(args, 0);
    let p = text(&path);
    let flags = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    if flags & LOCK_EX != 0 && p.contains("://") && !p.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("file://")) {
        ctx.warn("file_put_contents(): Exclusive locks may only be set for regular files");
        return Ok(Value::FALSE);
    }
    let options = if flags & FILE_USE_INCLUDE_PATH != 0 { user_stream::USE_PATH } else { 0 };
    let mode = if flags & FILE_APPEND != 0 { "ab" } else if flags & LOCK_EX != 0 { "cb" } else { "wb" };
    let user = match open_user(ctx, "file_put_contents", &p, mode, options, context_arg(args, 3))? {
        Some(None) => return Ok(Value::FALSE),
        Some(Some(s)) => Some(s),
        None => None,
    };
    let Some(mut stream) = user.map_or_else(|| open_builtin(ctx, "file_put_contents", p.clone(), if flags & FILE_APPEND != 0 { "a" } else { "w" }), |s| Ok(Some(s)))? else {
        return Ok(Value::FALSE);
    };
    if stream.user().is_some() && flags & LOCK_EX != 0 {
        // A file:// wrapper's file: locked (if it can be), then emptied.
        if !(user_stream::lock(ctx, &mut stream, 0)? && user_stream::lock(ctx, &mut stream, LOCK_EX)?) {
            stream.close_in(ctx)?;
            ctx.warn("file_put_contents(): Exclusive locks are not supported for this stream");
            return Ok(Value::FALSE);
        }
        user_stream::truncate(ctx, &mut stream, 0)?;
    }
    let written = match arg(args, 1).into_array() {
        Ok(a) => {
            // Each element written as it is.
            let mut n = 0;
            for v in a.values() {
                let piece = v.to_php_string().unwrap_or_else(PhpStr::empty);
                if piece.is_empty() {
                    continue;
                }
                if stream.write(ctx, piece.as_bytes())? != Some(piece.len()) {
                    ctx.warn(&format!("file_put_contents(): Failed to write {} bytes to {p}", piece.len()));
                    stream.close_in(ctx)?;
                    return Ok(Value::FALSE);
                }
                n += piece.len();
            }
            Some(n)
        }
        Err(v) => match resource_of(&v) {
            Some(o) if is_user(&o) || stream.user().is_some() => {
                let r = resource(&o).expect("a stream resource");
                let Ok(mut from) = r.stream.try_borrow_mut() else { return Ok(Value::FALSE) };
                copy_streams(ctx, &mut from, &mut stream, None)?
            }
            Some(o) => {
                let data = with_stream(&o, |s| s.read_all());
                stream.write(ctx, &data)?
            }
            None => {
                let data = v.to_php_string().unwrap_or_else(PhpStr::empty);
                let n = if data.is_empty() { Some(0) } else { stream.write(ctx, data.as_bytes())? };
                if let Some(n) = n.filter(|&n| n != data.len()) {
                    ctx.warn(&format!("file_put_contents(): Only {n} of {} bytes written, possibly out of free disk space", data.len()));
                    stream.close_in(ctx)?;
                    return Ok(Value::FALSE);
                }
                n
            }
        },
    };
    stream.close_in(ctx)?;
    Ok(written.map_or(Value::FALSE, |n| Value::int(n as i64)))
}

/// file(string $filename, int $flags = 0, $context = null): the lines.
fn file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    if flags & !(FILE_USE_INCLUDE_PATH | FILE_IGNORE_NEW_LINES | FILE_SKIP_EMPTY_LINES) != 0 {
        return Err(ctx.throw("ValueError", "file(): Argument #2 ($flags) must be a valid flag value".into()));
    }
    let options = if flags & FILE_USE_INCLUDE_PATH != 0 { user_stream::USE_PATH } else { 0 };
    let Some(bytes) = read_path(ctx, "file", &str_arg(args, 0), options, context_arg(args, 2))? else { return Ok(Value::FALSE) };
    let mut lines = Vec::new();
    let mut start = 0;
    let mut push = |line: &[u8]| {
        let mut line = line;
        if flags & FILE_IGNORE_NEW_LINES != 0 {
            line = line.strip_suffix(b"\n").unwrap_or(line);
            line = line.strip_suffix(b"\r").unwrap_or(line);
            if flags & FILE_SKIP_EMPTY_LINES != 0 && line.is_empty() {
                return;
            }
        }
        lines.push(Value::str(PhpStr::from(line)));
    };
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            push(&bytes[start..=i]);
            start = i + 1;
        }
    }
    if start < bytes.len() {
        push(&bytes[start..]);
    }
    Ok(Value::array(PhpArray::from_list(lines)))
}

fn readfile(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = str_arg(args, 0);
    let options = if args.len() > 1 && arg(args, 1).to_bool() { user_stream::USE_PATH } else { 0 };
    if !path.is_empty()
        && let Some(opened) = open_user(ctx, "readfile", &text(&path), "rb", options, context_arg(args, 2))?
    {
        let Some(mut s) = opened else { return Ok(Value::FALSE) };
        let n = passthru(ctx, &mut s)?;
        s.close_in(ctx)?;
        return Ok(Value::int(n as i64));
    }
    if path.is_empty() {
        return Err(ctx.throw("ValueError", "Path cannot be empty".into()));
    }
    let Some(bytes) = read_builtin(ctx, "readfile", text(&path))? else { return Ok(Value::FALSE) };
    ctx.echo(&bytes);
    Ok(Value::int(bytes.len() as i64))
}

/// A filesystem change: true, or false after PHP's warning `func(path): error`.
fn done(ctx: &mut dyn Ctx, func: &str, shown: &str, r: std::io::Result<()>) -> Value {
    match r {
        Ok(()) => Value::TRUE,
        Err(e) => {
            ctx.warn(&format!("{func}({shown}): {}", os_error(&e)));
            Value::FALSE
        }
    }
}

fn unlink(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = text(&str_arg(args, 0));
    if let Some(done) = user_op(ctx, "unlink", &p, "unlink", &mut [Value::str(p.as_str())], context_arg(args, 1))? {
        return Ok(done);
    }
    user_stream::clear_stat_cache(ctx);
    // A symlink to a directory is unlinked (the link, not what it names).
    if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
        ctx.warn(&format!("unlink({p}): Is a directory"));
        return Ok(Value::FALSE);
    }
    let r = std::fs::remove_file(&p);
    Ok(done(ctx, "unlink", &p, r))
}

/// mkdir(string $directory, int $permissions = 0777, bool $recursive = false)
fn mkdir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = text(&str_arg(args, 0));
    let recursive = args.len() > 2 && arg(args, 2).to_bool();
    let mode = if args.len() > 1 { arg(args, 1).to_int() } else { 0o777 };
    let mut call = [Value::str(p.as_str()), Value::int(mode), Value::int(user_stream::dir_options(recursive))];
    if let Some(done) = user_op(ctx, "mkdir", &p, "mkdir", &mut call, context_arg(args, 3))? {
        return Ok(done);
    }
    user_stream::clear_stat_cache(ctx);
    let r = if recursive {
        if Path::new(&p).exists() {
            Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
        } else {
            std::fs::create_dir_all(&p)
        }
    } else {
        std::fs::create_dir(&p)
    };
    // PHP names no path in mkdir's warnings.
    Ok(match r {
        Ok(()) => Value::TRUE,
        Err(e) => {
            ctx.warn(&format!("mkdir(): {}", os_error(&e)));
            Value::FALSE
        }
    })
}

fn rmdir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = text(&str_arg(args, 0));
    let mut call = [Value::str(p.as_str()), Value::int(user_stream::dir_options(false))];
    if let Some(done) = user_op(ctx, "rmdir", &p, "rmdir", &mut call, context_arg(args, 1))? {
        return Ok(done);
    }
    user_stream::clear_stat_cache(ctx);
    let r = std::fs::remove_dir(&p);
    Ok(done(ctx, "rmdir", &p, r))
}

fn rename(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (from, to) = (text(&str_arg(args, 0)), text(&str_arg(args, 1)));
    let (source, dest) = (user_stream::locate(ctx, "rename", &from), user_stream::locate(ctx, "rename", &to));
    match (source, dest) {
        (Located::User(a, _), Located::User(b, _)) if Rc::ptr_eq(&a, &b) => {
            let context = context_or_default(ctx, context_arg(args, 2));
            let mut call = [Value::str(from.as_str()), Value::str(to.as_str())];
            return Ok(Value::bool(user_stream::url_op(ctx, "rename", &a, "rename", &mut call, context)?));
        }
        (Located::Disabled, _) => {
            ctx.warn("rename(): Unable to locate stream wrapper");
            return Ok(Value::FALSE);
        }
        (Located::User(..), _) | (_, Located::User(..)) => {
            ctx.warn("rename(): Cannot rename a file across wrapper types");
            return Ok(Value::FALSE);
        }
        _ => {}
    }
    user_stream::clear_stat_cache(ctx);
    let r = std::fs::rename(&from, &to);
    Ok(done(ctx, "rename", &format!("{from},{to}"), r))
}

/// The files a web request's uploads were saved in (the host gives them).
const UPLOADED: &str = "rfc1867_uploaded_files";

fn is_upload(ctx: &mut dyn Ctx, path: &PhpStr) -> bool {
    match ctx.state(UPLOADED).as_array() {
        Some(a) => a.values().any(|v| v.as_str().is_some_and(|s| s.as_bytes() == path.as_bytes())),
        None => false,
    }
}

/// is_uploaded_file(string $filename): whether the request uploaded it.
fn is_uploaded_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::bool(is_upload(ctx, &str_arg(args, 0))))
}

/// move_uploaded_file(string $from, string $to): an upload moves, anything
/// else does not.
fn move_uploaded_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (from, to) = (str_arg(args, 0), str_arg(args, 1));
    if !is_upload(ctx, &from) {
        return Ok(Value::FALSE);
    }
    let (from_text, to_text) = (text(&from), text(&to));
    let moved = std::fs::rename(&from_text, &to_text).is_ok() || std::fs::copy(&from_text, &to_text).is_ok() && std::fs::remove_file(&from_text).is_ok();
    if !moved {
        ctx.warn(&format!("move_uploaded_file(): Unable to move \"{from_text}\" to \"{to_text}\""));
        return Ok(Value::FALSE);
    }
    let uploaded = ctx.state(UPLOADED);
    if let Some(a) = uploaded.as_array() {
        let rest: Vec<Value> = a.values().filter(|v| !v.as_str().is_some_and(|s| s.as_bytes() == from.as_bytes())).cloned().collect();
        *uploaded = Value::array(PhpArray::from_list(rest));
    }
    Ok(Value::TRUE)
}

fn copy(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (from, to) = (text(&str_arg(args, 0)), text(&str_arg(args, 1)));
    let context = context_arg(args, 2);
    let (source, dest) = (user_stream::peek(ctx, &from), user_stream::peek(ctx, &to));
    if matches!(source, Located::User(..)) || matches!(dest, Located::User(..)) {
        return copy_through(ctx, (&from, source), (&to, dest), context);
    }
    if from.contains("://") || to.contains("://") {
        // Through the streams: a wrapper (compress.zlib://, php://) at either end.
        let Some(bytes) = read_path(ctx, "copy", &str_arg(args, 0), 0, None)? else { return Ok(Value::FALSE) };
        let Some(mut out) = open(ctx, "copy", &str_arg(args, 1), "wb")? else { return Ok(Value::FALSE) };
        let ok = out.write(ctx, &bytes)?.is_some();
        out.close();
        return Ok(Value::bool(ok));
    }
    match std::fs::copy(&from, &to) {
        Ok(_) => Ok(Value::TRUE),
        Err(e) => {
            ctx.warn(&format!("copy({from}): Failed to open stream: {}", os_error(&e)));
            Ok(Value::FALSE)
        }
    }
}

/// php_copy_file_ctx with a user-space wrapper at either end: neither may
/// be a directory, nor both the same file (their stats say); then the
/// bytes go across, a read at a time.
fn copy_through(ctx: &mut dyn Ctx, (from, source): (&str, Located), (to, dest): (&str, Located), context: Option<Value>) -> Result<Value, Throw> {
    let ctx_value = context_or_default(ctx, context.clone());
    let src = match &source {
        Located::User(w, local) => user_stream::url_stat(ctx, "copy", w, local, 0, ctx_value.clone())?,
        _ => None,
    };
    if src.is_some_and(|s| s.is_dir()) {
        ctx.warn("copy(): The first argument to copy() function cannot be a directory");
        return Ok(Value::FALSE);
    }
    let dst = match &dest {
        Located::User(w, local) => user_stream::url_stat(ctx, "copy", w, local, user_stream::STAT_QUIET, ctx_value)?,
        _ => None,
    };
    if dst.is_some_and(|s| s.is_dir()) {
        ctx.warn("copy(): The second argument to copy() function cannot be a directory");
        return Ok(Value::FALSE);
    }
    if let (Some(a), Some(b)) = (src, dst)
        && a.ino != 0
        && a.ino == b.ino
        && a.dev == b.dev
    {
        return Ok(Value::FALSE);
    }
    let Some(mut input) = open_with(ctx, "copy", &PhpStr::from(from), "rb", 0, context.clone())? else { return Ok(Value::FALSE) };
    let Some(mut output) = open_with(ctx, "copy", &PhpStr::from(to), "wb", 0, context)? else {
        input.close_in(ctx)?;
        return Ok(Value::FALSE);
    };
    let copied = copy_streams(ctx, &mut input, &mut output, None)?;
    input.close_in(ctx)?;
    output.close_in(ctx)?;
    Ok(Value::bool(copied.is_some()))
}

/// touch(string $filename, ?int $mtime = null, ?int $atime = null)
fn touch(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = text(&str_arg(args, 0));
    let int_or_null = |i: usize| match args.get(i).map(Value::deref) {
        Some(v) if !v.is_null() => Some(v.to_int()),
        _ => None,
    };
    let (mtime, atime) = (int_or_null(1), int_or_null(2));
    if mtime.is_none() && atime.is_some() {
        return Err(ctx.throw("ValueError", "touch(): Argument #2 ($mtime) cannot be null when argument #3 ($atime) is an integer".into()));
    }
    // stream_metadata's value: [mtime, atime], or [] for now.
    let times: Vec<Value> = mtime.map(|m| vec![Value::int(m), Value::int(atime.unwrap_or(m))]).unwrap_or_default();
    let mut call = [Value::str(p.as_str()), Value::int(user_stream::META_TOUCH), Value::array(PhpArray::from_list(times))];
    if let Some(done) = user_op(ctx, "touch", &p, "stream_metadata", &mut call, None)? {
        return Ok(done);
    }
    user_stream::clear_stat_cache(ctx);
    let file = match std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        Ok(f) => f,
        Err(e) => {
            ctx.warn(&format!("touch(): Unable to create file {p} because {}", os_error(&e)));
            return Ok(Value::FALSE);
        }
    };
    let mtime = match args.get(1).map(Value::deref) {
        Some(v) if !v.is_null() => std::time::UNIX_EPOCH + std::time::Duration::from_secs(v.to_int().max(0) as u64),
        _ => std::time::SystemTime::now(),
    };
    let _ = file.set_modified(mtime);
    Ok(Value::TRUE)
}

fn path_arg(args: &[Value]) -> String {
    text(&str_arg(args, 0))
}

fn is_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if let Some(v) = user_check(ctx, "is_file", &p, false, Stat::is_file)? {
        return Ok(v);
    }
    Ok(Value::bool(Path::new(&p).is_file()))
}

fn is_dir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if let Some(v) = user_check(ctx, "is_dir", &p, false, Stat::is_dir)? {
        return Ok(v);
    }
    Ok(Value::bool(Path::new(&p).is_dir()))
}

fn is_link(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if let Some(v) = user_check(ctx, "is_link", &p, true, Stat::is_link)? {
        return Ok(v);
    }
    Ok(Value::bool(std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())))
}

fn file_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if p.is_empty() {
        return Ok(Value::FALSE);
    }
    if let Some(v) = user_check(ctx, "file_exists", &p, false, |_| true)? {
        return Ok(v);
    }
    Ok(Value::bool(Path::new(&p).exists()))
}

pub(crate) fn mode_bits(p: &str) -> Option<u32> {
    use p8_runtime::os::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.mode())
}

/// Whether the file allows what `bits` of a mode say (0o444 reading,
/// 0o222 writing, 0o111 executing): is_readable() and the like.
#[cfg(unix)]
pub(crate) fn access(p: &str, bits: u32) -> bool {
    mode_bits(p).is_some_and(|m| m & bits != 0)
}

/// Whether the file allows what `bits` of a mode say, as wasi-libc's
/// access() answers: WASI has no permission bits, so a file may be read
/// or written when its directory's rights let it be opened so, and
/// executing is never refused.
#[cfg(target_os = "wasi")]
pub(crate) fn access(p: &str, bits: u32) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    if bits & 0o222 != 0 {
        if m.is_dir() { !m.permissions().readonly() } else { std::fs::OpenOptions::new().write(true).open(p).is_ok() }
    } else if bits & 0o444 != 0 {
        if m.is_dir() { std::fs::read_dir(p).is_ok() } else { std::fs::File::open(p).is_ok() }
    } else {
        true
    }
}

fn is_readable(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if let Some(v) = user_check(ctx, "is_readable", &p, false, |st| user_access(st, 0o400))? {
        return Ok(v);
    }
    Ok(Value::bool(access(&p, 0o444)))
}

fn is_writable(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    let func = called(ctx, "is_writable");
    if let Some(v) = user_check(ctx, func, &p, false, |st| user_access(st, 0o200))? {
        return Ok(v);
    }
    Ok(Value::bool(access(&p, 0o222)))
}

fn is_executable(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if let Some(v) = user_check(ctx, "is_executable", &p, false, |st| user_access(st, 0o100))? {
        return Ok(v);
    }
    Ok(Value::bool(Path::new(&p).is_file() && access(&p, 0o111)))
}

/// A stat of `p`, or PHP's "stat failed" warning.
fn stat(ctx: &mut dyn Ctx, func: &str, p: &str) -> Option<std::fs::Metadata> {
    match std::fs::metadata(p) {
        Ok(m) => Some(m),
        Err(_) => {
            ctx.warn(&format!("{func}(): stat failed for {p}"));
            None
        }
    }
}

fn filesize(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if let Some(v) = user_field(ctx, "filesize", &p, false, |st| Value::int(st.size))? {
        return Ok(v);
    }
    Ok(stat(ctx, "filesize", &p).map_or(Value::FALSE, |m| Value::int(m.len() as i64)))
}

/// filemtime, fileatime and filectime (a user-space wrapper's stat gives
/// each its own; p8's files answer the modification time for all three).
fn filemtime(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    let func = called(ctx, "filemtime");
    if let Some(v) = user_field(ctx, func, &p, false, |st| {
        Value::int(match func {
            "fileatime" => st.atime,
            "filectime" => st.ctime,
            _ => st.mtime,
        })
    })? {
        return Ok(v);
    }
    Ok(stat(ctx, "filemtime", &p).map_or(Value::FALSE, |m| {
        let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        Value::int(t as i64)
    }))
}

fn fileperms(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if let Some(v) = user_field(ctx, "fileperms", &p, false, |st| Value::int(st.mode))? {
        return Ok(v);
    }
    Ok(stat(ctx, "fileperms", &p).map_or(Value::FALSE, |_| Value::int(i64::from(mode_bits(&p).unwrap_or(0)))))
}

fn chmod(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    let mut call = [Value::str(p.as_str()), Value::int(user_stream::META_ACCESS), Value::int(arg(args, 1).to_int())];
    if let Some(done) = user_op(ctx, "chmod", &p, "stream_metadata", &mut call, None)? {
        return Ok(done);
    }
    user_stream::clear_stat_cache(ctx);
    let mode = arg(args, 1).to_int() as u32;
    let r = p8_runtime::os::set_mode(&p, mode);
    Ok(match r {
        Ok(()) => Value::TRUE,
        Err(e) => {
            ctx.warn(&format!("chmod(): {}", os_error(&e)));
            Value::FALSE
        }
    })
}

/// umask: p8 does not change the process's (a host runs many requests);
/// it reports the usual 022.
fn umask(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::int(0o022))
}

fn clearstatcache(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    user_stream::clear_stat_cache(ctx);
    Ok(Value::NULL)
}

fn temp_dir() -> String {
    let d = p8_runtime::os::temp_dir().to_string_lossy().into_owned();
    d.trim_end_matches('/').to_string()
}

/// tempnam(string $directory, string $prefix): a new empty file's path.
fn tempnam(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut dir = path_arg(args);
    if dir.is_empty() || !Path::new(&dir).is_dir() {
        if !dir.is_empty() {
            ctx.report(8, "tempnam(): file created in the system's temporary directory");
        }
        dir = temp_dir();
    }
    let prefix: String = text(&str_arg(args, 1)).chars().take(63).collect();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for i in 0..1000u32 {
        let name = format!("{}/{prefix}{:06x}", dir.trim_end_matches('/'), (stamp as u32).wrapping_add(i.wrapping_mul(7919)) & 0xff_ffff);
        if std::fs::OpenOptions::new().write(true).create_new(true).open(&name).is_ok() {
            return Ok(Value::str(PhpStr::from(name)));
        }
    }
    Ok(Value::FALSE)
}

fn sys_get_temp_dir(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str(PhpStr::from(temp_dir())))
}

fn getcwd(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(std::env::current_dir().map_or(Value::FALSE, |d| Value::str(PhpStr::from(d.to_string_lossy().into_owned()))))
}

/// chdir: changes the process's directory (the CLI's one request).
fn chdir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    match std::env::set_current_dir(&p) {
        Ok(()) => Ok(Value::TRUE),
        Err(e) => {
            ctx.warn(&format!("chdir(): {} (errno 2)", os_error(&e)));
            Ok(Value::FALSE)
        }
    }
}

fn realpath(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    let p = if p.is_empty() { ".".to_string() } else { p };
    Ok(p8_runtime::os::canonicalize(&p).map_or(Value::FALSE, |d| Value::str(PhpStr::from(d.to_string_lossy().into_owned()))))
}

const PATHINFO_DIRNAME: i64 = 1;
const PATHINFO_BASENAME: i64 = 2;
const PATHINFO_EXTENSION: i64 = 4;
const PATHINFO_FILENAME: i64 = 8;

/// pathinfo(string $path, int $flags = PATHINFO_ALL)
fn pathinfo(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = str_arg(args, 0);
    let p = path.as_bytes();
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 15 };
    let key = |k: &str| Key::from_string(PhpStr::from(k));
    let mut out = PhpArray::new();
    let trimmed = {
        let mut end = p.len();
        while end > 1 && p[end - 1] == b'/' {
            end -= 1;
        }
        &p[..end]
    };
    let base_start = trimmed.iter().rposition(|&c| c == b'/').map_or(0, |i| i + 1);
    let base = &trimmed[base_start..];
    if flags & PATHINFO_DIRNAME != 0 && p.contains(&b'/') || flags & PATHINFO_DIRNAME != 0 && !p.is_empty() {
        let dir = match trimmed[..base_start].iter().rposition(|&c| c != b'/') {
            Some(i) => &trimmed[..=i],
            None if trimmed.starts_with(b"/") => b"/".as_slice(),
            None => b".".as_slice(),
        };
        if p.contains(&b'/') || !p.is_empty() {
            out.insert(key("dirname"), Value::str(PhpStr::from(dir)));
        }
    }
    if flags & PATHINFO_BASENAME != 0 {
        out.insert(key("basename"), Value::str(PhpStr::from(base)));
    }
    let dot = base.iter().rposition(|&c| c == b'.');
    if flags & PATHINFO_EXTENSION != 0
        && let Some(d) = dot
    {
        out.insert(key("extension"), Value::str(PhpStr::from(&base[d + 1..])));
    }
    if flags & PATHINFO_FILENAME != 0 {
        out.insert(key("filename"), Value::str(PhpStr::from(&base[..dot.unwrap_or(base.len())])));
    }
    if flags != 15 {
        return Ok(out.values().next().cloned().unwrap_or(Value::str(PhpStr::empty())));
    }
    Ok(Value::array(out))
}

/// A directory's entries as the system lists them (readdir's order, which
/// PHP keeps and code like WordPress's theme scan shows), "." and ".."
/// first.
pub(crate) fn entries(p: &str) -> std::io::Result<Vec<Vec<u8>>> {
    use p8_runtime::os::OsStrExt;
    let mut names: Vec<Vec<u8>> = vec![b".".to_vec(), b"..".to_vec()];
    for e in std::fs::read_dir(p)? {
        names.push(e?.file_name().as_bytes().to_vec());
    }
    Ok(names)
}

/// scandir(string $directory, int $sorting_order = SCANDIR_SORT_ASCENDING)
fn scandir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if p.is_empty() {
        return Err(ctx.throw("ValueError", "scandir(): Argument #1 ($directory) cannot be empty".into()));
    }
    let listed = match user_stream::locate(ctx, "scandir", &p) {
        Located::User(w, local) => {
            let context = context_or_default(ctx, context_arg(args, 2));
            match user_stream::open(ctx, &w, &local, "r", 0, context, true)? {
                Ok((mut dir, _)) => {
                    let mut names = Vec::new();
                    while let Some(name) = user_stream::readdir(ctx, &mut dir)? {
                        names.push(name);
                    }
                    dir.close_in(ctx)?;
                    Ok(names)
                }
                Err(reason) => Err(reason),
            }
        }
        Located::Disabled => Err("no suitable wrapper could be found".to_string()),
        Located::Builtin => entries(&p).map_err(|e| os_error(&e)),
    };
    match listed {
        Ok(mut names) => {
            // SCANDIR_SORT_ASCENDING (0), _DESCENDING (1), _NONE (2).
            match if args.len() > 1 { arg(args, 1).to_int() } else { 0 } {
                2 => {}
                1 => names.sort_by(|a, b| b.cmp(a)),
                _ => names.sort(),
            }
            Ok(Value::array(PhpArray::from_list(names.into_iter().map(|n| Value::str(PhpStr::new(n))).collect())))
        }
        Err(reason) => {
            ctx.warn(&format!("scandir({p}): Failed to open directory: {reason}"));
            ctx.warn("scandir(): (errno 2): No such file or directory");
            Ok(Value::FALSE)
        }
    }
}

/// Whether `name` matches a shell wildcard (`*`, `?`, `[...]`).
fn wildcard(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|i| wildcard(rest, &name[i..])),
        Some((b'?', rest)) => !name.is_empty() && wildcard(rest, &name[1..]),
        Some((b'[', rest)) => {
            let Some(close) = rest.iter().position(|&c| c == b']') else { return name.first() == Some(&b'[') && wildcard(rest, &name[1..]) };
            let (set, after) = (&rest[..close], &rest[close + 1..]);
            let Some((&c, tail)) = name.split_first() else { return false };
            let (negate, set) = match set.split_first() {
                Some((b'!' | b'^', s)) => (true, s),
                _ => (false, set),
            };
            let mut hit = false;
            let mut i = 0;
            while i < set.len() {
                if i + 2 < set.len() && set[i + 1] == b'-' {
                    hit |= set[i] <= c && c <= set[i + 2];
                    i += 3;
                } else {
                    hit |= set[i] == c;
                    i += 1;
                }
            }
            hit != negate && wildcard(after, tail)
        }
        Some((&p, rest)) => name.first() == Some(&p) && wildcard(rest, &name[1..]),
    }
}

const FNM_NOESCAPE: i64 = 2;
const FNM_PATHNAME: i64 = 1;
const FNM_PERIOD: i64 = 4;
const FNM_CASEFOLD: i64 = 16;

/// fnmatch(string $pattern, string $filename, int $flags = 0): glibc's
/// fnmatch, with FNM_NOESCAPE, FNM_PATHNAME, FNM_PERIOD and FNM_CASEFOLD.
fn fnmatch(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let pattern = str_arg(args, 0);
    let name = str_arg(args, 1);
    let flags = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    Ok(Value::bool(fn_match(pattern.as_bytes(), name.as_bytes(), flags, true)))
}

/// Whether `name` matches `pattern`; `start`: `name` begins a path
/// component (where FNM_PERIOD wants a leading dot matched literally).
fn fn_match(pattern: &[u8], name: &[u8], flags: i64, start: bool) -> bool {
    let fold = |c: u8| if flags & FNM_CASEFOLD != 0 { c.to_ascii_lowercase() } else { c };
    let hidden = start && flags & FNM_PERIOD != 0 && name.first() == Some(&b'.');
    // A character a wildcard may stand for here.
    let wild_ok = |c: u8| !(flags & FNM_PATHNAME != 0 && c == b'/');
    let next_start = |c: u8| flags & FNM_PATHNAME != 0 && c == b'/';
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => {
            if hidden {
                return false;
            }
            let mut i = 0;
            loop {
                if fn_match(rest, &name[i..], flags, i > 0 && next_start(name[i - 1]) || (i == 0 && start)) {
                    return true;
                }
                if i == name.len() || !wild_ok(name[i]) {
                    return false;
                }
                i += 1;
            }
        }
        Some((b'?', rest)) => match name.split_first() {
            Some((&c, tail)) if wild_ok(c) && !hidden => fn_match(rest, tail, flags, next_start(c)),
            _ => false,
        },
        Some((b'[', rest)) => {
            let Some((&c, tail)) = name.split_first() else { return false };
            if !wild_ok(c) || hidden {
                return false;
            }
            let (negate, body) = match rest.split_first() {
                Some((b'!' | b'^', b)) => (true, b),
                _ => (false, rest),
            };
            // `]` first in the set is a member; find the closing one.
            let Some(close) = body.iter().skip(1).position(|&x| x == b']').map(|p| p + 1) else {
                return c == b'[' && fn_match(rest, tail, flags, false);
            };
            let set = &body[..close];
            let mut hit = false;
            let mut i = 0;
            while i < set.len() {
                if i + 2 < set.len() && set[i + 1] == b'-' {
                    hit |= fold(set[i]) <= fold(c) && fold(c) <= fold(set[i + 2]);
                    i += 3;
                } else {
                    hit |= fold(set[i]) == fold(c);
                    i += 1;
                }
            }
            hit != negate && fn_match(&body[close + 1..], tail, flags, next_start(c))
        }
        Some((b'\\', rest)) if flags & FNM_NOESCAPE == 0 && !rest.is_empty() => match name.split_first() {
            Some((&c, tail)) => fold(c) == fold(rest[0]) && fn_match(&rest[1..], tail, flags, next_start(c)),
            None => false,
        },
        Some((&p, rest)) => match name.split_first() {
            Some((&c, tail)) => fold(c) == fold(p) && fn_match(rest, tail, flags, next_start(c)),
            None => false,
        },
    }
}

use p8_runtime::glob::{MARK as GLOB_MARK, ONLYDIR as GLOB_ONLYDIR};

/// glob(string $pattern, int $flags = 0): the paths matching, sorted.
fn glob(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let pattern = path_arg(args);
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    let out = glob_paths(&pattern, flags).into_iter().map(|p| Value::str(PhpStr::from(p))).collect();
    Ok(Value::array(PhpArray::from_list(out)))
}

/// GLOB_BRACE's first group of alternatives expanded: "a{b,c}d" is "abd"
/// and "acd" (None: no group; "{}" is literal, as in glibc).
fn expand_braces(pattern: &str) -> Option<Vec<String>> {
    let b = pattern.as_bytes();
    let open = (0..b.len()).find(|&i| b[i] == b'{' && b.get(i + 1) != Some(&b'}'))?;
    let mut depth = 0;
    let mut close = None;
    let mut cuts = vec![open];
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            b',' if depth == 1 => cuts.push(i),
            _ => {}
        }
    }
    let close = close?;
    cuts.push(close);
    let (head, tail) = (&pattern[..open], &pattern[close + 1..]);
    Some(cuts.windows(2).map(|w| format!("{head}{}{tail}", &pattern[w[0] + 1..w[1]])).collect())
}

/// The paths `pattern` matches, sorted (glob(), GlobIterator).
pub(crate) fn glob_paths(pattern: &str, flags: i64) -> Vec<String> {
    use p8_runtime::glob::BRACE as GLOB_BRACE;
    if flags & GLOB_BRACE != 0
        && let Some(alternatives) = expand_braces(pattern)
    {
        return alternatives.iter().flat_map(|p| glob_paths(p, flags)).collect();
    }
    let absolute = pattern.starts_with('/');
    let mut found: Vec<String> = vec![if absolute { "/".into() } else { String::new() }];
    for part in pattern.split('/').filter(|s| !s.is_empty()) {
        let mut next = Vec::new();
        for base in &found {
            let dir = if base.is_empty() { ".".to_string() } else { base.clone() };
            let join = |n: &str| if base.is_empty() { n.to_string() } else if base.ends_with('/') { format!("{base}{n}") } else { format!("{base}/{n}") };
            if !part.contains(['*', '?', '[']) {
                let p = join(part);
                if Path::new(&p).exists() {
                    next.push(p);
                }
                continue;
            }
            if let Ok(mut names) = entries(&dir) {
                names.sort();
                for n in names {
                    let n = String::from_utf8_lossy(&n).into_owned();
                    if (n.starts_with('.') && !part.starts_with('.')) || n == "." || n == ".." {
                        continue;
                    }
                    if wildcard(part.as_bytes(), n.as_bytes()) {
                        next.push(join(&n));
                    }
                }
            }
        }
        found = next;
    }
    let mut out = Vec::new();
    for p in found {
        let dir = Path::new(&p).is_dir();
        if flags & GLOB_ONLYDIR != 0 && !dir {
            continue;
        }
        out.push(if flags & GLOB_MARK != 0 && dir { format!("{p}/") } else { p });
    }
    out
}

/// stream_get_wrappers: the URL schemes p8's streams open, in PHP's order
/// (no https, ftp(s) or phar yet), then those extensions registered (zip).
/// The user-space wrappers registered come last; a scheme
/// stream_wrapper_unregister() took away is left out.
fn stream_get_wrappers(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let names = user_stream::schemes(ctx);
    Ok(Value::array(PhpArray::from_list(names.into_iter().map(|n| Value::str(PhpStr::new(n))).collect())))
}

/// stream_wrapper_register(string $protocol, string $class, int $flags = 0)
fn stream_wrapper_register(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let func = called(ctx, "stream_wrapper_register");
    let (protocol, name) = (str_arg(args, 0), str_arg(args, 1));
    let Some(class) = ctx.class_load(name.as_bytes())? else {
        return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($class) must be a valid class name, {} given", crate::class::class_name_given(&Value::str(name)))));
    };
    // PHP keeps a registration as a resource ("stream factory"), failed
    // or not: it takes a number.
    ctx.new_resource(Resource { kind: "stream factory", closed: Cell::new(true), stream: RefCell::new(Stream::new(Target::Memory(Vec::new()), "", "r")) });
    let flags = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    Ok(Value::bool(user_stream::register(ctx, func, protocol.as_bytes(), class, flags)))
}

/// stream_wrapper_unregister(string $protocol)
fn stream_wrapper_unregister(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::bool(user_stream::unregister(ctx, str_arg(args, 0).as_bytes())))
}

/// stream_wrapper_restore(string $protocol)
fn stream_wrapper_restore(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::bool(user_stream::restore(ctx, str_arg(args, 0).as_bytes())))
}

/// opendir: a stream whose lines are the directory's entries (as readdir
/// gives); a user-space wrapper's dir_opendir.
fn opendir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    let func = called(ctx, "opendir");
    match user_stream::locate(ctx, func, &p) {
        Located::User(w, local) => {
            let context = context_or_default(ctx, context_arg(args, 1));
            return Ok(match user_stream::open(ctx, &w, &local, "r", 0, context, true)? {
                Ok((dir, _)) => new_stream(ctx, dir),
                Err(reason) => {
                    ctx.warn(&format!("{func}({p}): Failed to open directory: {reason}"));
                    Value::FALSE
                }
            });
        }
        Located::Disabled => {
            user_stream::warn_disabled(ctx, func);
            ctx.warn(&format!("{func}({p}): Failed to open directory: no suitable wrapper could be found"));
            return Ok(Value::FALSE);
        }
        Located::Builtin => {}
    }
    match entries(&p) {
        Ok(names) => {
            let mut data = Vec::new();
            for n in names {
                data.extend_from_slice(&n);
                data.push(0);
            }
            Ok(new_stream(ctx, Stream::new(Target::Memory(data), &p, "r")))
        }
        Err(e) => {
            ctx.warn(&format!("opendir({p}): Failed to open directory: {}", os_error(&e)));
            Ok(Value::FALSE)
        }
    }
}

fn readdir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "readdir", args, 0)?;
    if is_user(&o) {
        let name = with_user(ctx, &o, None, |ctx, s| if s.user().is_some_and(|u| u.dir) { user_stream::readdir(ctx, s) } else { Ok(None) })?;
        return Ok(name.map_or(Value::FALSE, |n| Value::str(PhpStr::new(n))));
    }
    let mut name = Vec::new();
    loop {
        let b = with_stream(&o, |s| s.read(1));
        match b.first() {
            None if name.is_empty() => return Ok(Value::FALSE),
            None | Some(0) => break,
            Some(&c) => name.push(c),
        }
    }
    Ok(Value::str(PhpStr::new(name)))
}

fn rewinddir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "rewinddir", args, 0)?;
    with_user(ctx, &o, false, |ctx, s| s.seek_in(ctx, 0, 0))?;
    Ok(Value::NULL)
}

/// dir($directory): a Directory holding opendir()'s handle, or false.
fn dir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = arg(args, 0);
    let handle = opendir(ctx, args)?;
    if handle.as_bool() == Some(false) {
        return Ok(handle);
    }
    let class = ctx.class(b"Directory").expect("Directory");
    let o = ctx.new_object(&class);
    o.class.write(&o, b"path", b"Directory", path);
    o.class.write(&o, b"handle", b"Directory", handle);
    Ok(Value::object(o))
}

/// A Directory method's handle, while it is open.
fn dir_handle(ctx: &mut dyn Ctx, this: &Rc<Object>, func: &str) -> Result<Value, Throw> {
    let handle = this.class.read(this, b"handle", b"Directory");
    match resource_of(&handle) {
        Some(o) if resource(&o).is_some_and(|r| !r.closed.get()) => Ok(handle),
        _ => Err(ctx.throw("TypeError", format!("Directory::{func}(): supplied resource is not a valid Directory resource"))),
    }
}

fn directory_read(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let handle = dir_handle(ctx, this, "read")?;
    readdir(ctx, &mut [handle])
}

fn directory_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let handle = dir_handle(ctx, this, "rewind")?;
    rewinddir(ctx, &mut [handle])
}

fn directory_close(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let handle = dir_handle(ctx, this, "close")?;
    fclose(ctx, &mut [handle]).map(|_| Value::NULL)
}

pub(crate) static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "Directory",
    parent: None,
    interfaces: &[],
    flags: 0,
    properties: &[("path", Constant::Null, Visibility::Public), ("handle", Constant::Null, Visibility::Public)],
    methods: &[
        ("read", Some(Native::Method(directory_read))),
        ("rewind", Some(Native::Method(directory_rewind))),
        ("close", Some(Native::Method(directory_close))),
    ],
    constants: &[],
    debug_info: None,
}];

/// stream_resolve_include_path($filename): where an include would find it.
fn stream_resolve_include_path(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
    Ok(match ctx.resolve_include(&name) {
        Some(p) => Value::str(PhpStr::from(p.to_string_lossy().as_bytes())),
        None => Value::FALSE,
    })
}

/// stream_isatty($stream): whether the stream is a terminal (a file opened
/// on one, or the process's own STDIN/STDOUT/STDERR on one).
fn stream_isatty(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    use std::io::IsTerminal;
    let o = stream_arg(ctx, "stream_isatty", args, 0)?;
    Ok(Value::bool(with_stream(&o, |s| match &s.target {
        Target::File(f) => f.is_terminal(),
        Target::Stdin => std::io::stdin().is_terminal(),
        Target::Stdout => std::io::stdout().is_terminal(),
        Target::Stderr => std::io::stderr().is_terminal(),
        _ => false,
    })))
}

/// stream_get_meta_data($stream): what the stream is (wrapper, type, uri,
/// mode) and where it is (eof, bytes read ahead).
fn stream_get_meta_data(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_get_meta_data", args, 0)?;
    // php_stream_eof: a user-space stream is asked (stream_eof).
    if is_user(&o) {
        with_user(ctx, &o, true, |ctx, s| s.at_end_in(ctx))?;
    }
    let entries = with_stream(&o, |s| {
        let (wrapper, kind, seekable) = match &s.target {
            Target::User(u) => ("user-space", if u.dir { "user-space-dir" } else { "user-space" }, u.seekable()),
            Target::File(_) => ("plainfile", "STDIO", true),
            Target::Memory(_) if s.path.starts_with("php://temp") => ("PHP", "TEMP", true),
            Target::Memory(_) if s.path.starts_with("php://memory") => ("PHP", "MEMORY", true),
            Target::Memory(_) if s.path.starts_with("data:") => ("RFC2397", "RFC2397", true),
            Target::Memory(_) => ("plainfile", "STDIO", true),
            Target::Stdin | Target::Stdout | Target::Stderr => ("PHP", "STDIO", false),
            Target::Tcp(_) => ("", "tcp_socket", false),
            Target::PipeOut(_) | Target::PipeIn(_) | Target::Process(_) => ("", "STDIO", false),
        };
        let text = |t: &str| Value::str(PhpStr::from(t));
        let mut entries = vec![
            ("timed_out", Value::FALSE),
            ("blocked", Value::TRUE),
            // At the end for the script: nothing read ahead is left.
            ("eof", Value::bool(s.eof && s.buffer.is_empty())),
        ];
        // A user-space stream's wrapper instance.
        if let Some(u) = s.user() {
            entries.push(("wrapper_data", Value::object(Rc::clone(&u.object))));
        }
        entries.extend([
            ("wrapper_type", text(wrapper)),
            ("stream_type", text(kind)),
            // Memory and temp streams are always read-write.
            ("mode", text(if matches!(kind, "MEMORY" | "TEMP") { "w+b" } else { &s.mode })),
            ("unread_bytes", Value::int(s.buffer.len() as i64)),
            ("seekable", Value::bool(seekable)),
            ("uri", text(&s.path)),
        ]);
        entries
    });
    let mut out = PhpArray::new();
    for (k, v) in entries {
        out.insert(Key::Str(PhpStr::from(k)), v);
    }
    Ok(Value::array(out))
}

/// The http:// wrapper, for reading: a GET over a plain connection (no
/// redirects, no TLS). The body, with `$http_response_header` set in the
/// calling code; None after PHP's warning.
fn http_get(ctx: &mut dyn Ctx, func: &str, url: &str) -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    let rest = url.get(7..)?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let has_port = match hostport.strip_prefix('[') {
        Some(v6) => v6.contains("]:"),
        None => hostport.contains(':'),
    };
    let addr = if has_port { hostport.to_string() } else { format!("{hostport}:80") };
    let timeout = std::time::Duration::from_secs(60);
    let connected = std::net::ToSocketAddrs::to_socket_addrs(&addr).and_then(|addrs| {
        let mut last = std::io::Error::from(std::io::ErrorKind::NotFound);
        for a in addrs {
            match std::net::TcpStream::connect_timeout(&a, timeout) {
                Ok(s) => return Ok(s),
                Err(e) => last = e,
            }
        }
        Err(last)
    });
    let mut sock = match connected {
        Ok(s) => s,
        Err(e) => {
            ctx.warn(&format!("{func}({url}): Failed to open stream: {}", os_error(&e)));
            return None;
        }
    };
    let _ = sock.set_read_timeout(Some(timeout));
    let request = format!("GET {path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n");
    let mut raw = Vec::new();
    if sock.write_all(request.as_bytes()).is_err() || sock.read_to_end(&mut raw).is_err() && raw.is_empty() {
        ctx.warn(&format!("{func}({url}): Failed to open stream: Connection reset by peer"));
        return None;
    }
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |i| i + 4);
    let head = String::from_utf8_lossy(&raw[..split.saturating_sub(4).min(raw.len())]).into_owned();
    let mut body = raw[split.min(raw.len())..].to_vec();
    let lines: Vec<&str> = head.split("\r\n").collect();
    let status = lines.first().copied().unwrap_or("");
    let code: i64 = status.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    if lines.iter().any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked") {
        let mut out = Vec::new();
        let mut rest = body.as_slice();
        while let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") {
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..eol]).split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
            if size == 0 || rest.len() < eol + 2 + size {
                break;
            }
            out.extend_from_slice(&rest[eol + 2..eol + 2 + size]);
            rest = rest.get(eol + 2 + size + 2..).unwrap_or(&[]);
        }
        body = out;
    }
    let header_lines: Vec<Value> = lines.iter().map(|l| Value::str(PhpStr::from(*l))).collect();
    ctx.set_local(b"http_response_header", Value::array(PhpArray::from_list(header_lines)));
    if code >= 400 {
        ctx.warn(&format!("{func}({url}): Failed to open stream: HTTP request failed! {status}\n"));
        return None;
    }
    Some(body)
}

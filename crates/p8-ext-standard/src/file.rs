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
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::stream::{Resource, Stream, Target, resource, resource_of};
use p8_runtime::string::PhpStr;
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
    Builtin { name: "fflush", run: fflush, by_ref: &[] },
    Builtin { name: "ftell", run: ftell, by_ref: &[] },
    Builtin { name: "fseek", run: fseek, by_ref: &[] },
    Builtin { name: "rewind", run: rewind, by_ref: &[] },
    Builtin { name: "ftruncate", run: ftruncate, by_ref: &[] },
    Builtin { name: "flock", run: flock, by_ref: &[] },
    Builtin { name: "fprintf", run: fprintf, by_ref: &[] },
    Builtin { name: "fpassthru", run: fpassthru, by_ref: &[] },
    Builtin { name: "stream_get_contents", run: stream_get_contents, by_ref: &[] },
    Builtin { name: "tmpfile", run: tmpfile, by_ref: &[] },
    Builtin { name: "file_get_contents", run: file_get_contents, by_ref: &[] },
    Builtin { name: "file_put_contents", run: file_put_contents, by_ref: &[] },
    Builtin { name: "file", run: file, by_ref: &[] },
    Builtin { name: "readfile", run: readfile, by_ref: &[] },
    Builtin { name: "unlink", run: unlink, by_ref: &[] },
    Builtin { name: "mkdir", run: mkdir, by_ref: &[] },
    Builtin { name: "rmdir", run: rmdir, by_ref: &[] },
    Builtin { name: "rename", run: rename, by_ref: &[] },
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
    Builtin { name: "opendir", run: opendir, by_ref: &[] },
    Builtin { name: "readdir", run: readdir, by_ref: &[] },
    Builtin { name: "rewinddir", run: rewinddir, by_ref: &[] },
    Builtin { name: "closedir", run: fclose, by_ref: &[] },
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

pub(crate) fn new_stream(ctx: &mut dyn Ctx, stream: Stream) -> Value {
    ctx.new_resource(Resource { kind: "stream", closed: Cell::new(false), stream: RefCell::new(stream) })
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
    let p = text(path);
    if let Some(gz) = p.strip_prefix("compress.zlib://") {
        return Ok(crate::zlib::open_gz(ctx, func, gz, mode));
    }
    let lower = p.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("php://") {
        let target = match rest.split('/').next().unwrap_or("") {
            "memory" | "temp" => Target::Memory(Vec::new()),
            "stdin" => Target::Stdin,
            "stdout" | "output" => Target::Stdout,
            "stderr" => Target::Stderr,
            "input" => Target::Memory(Vec::new()),
            _ => {
                ctx.warn(&format!("{func}(): Invalid php:// URL specified"));
                ctx.warn(&format!("{func}({p}): Failed to open stream: operation failed"));
                return Ok(None);
            }
        };
        return Ok(Some(Stream::new(target, &p, mode)));
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
    match open(ctx, "fopen", &path, &mode)? {
        Some(s) => Ok(new_stream(ctx, s)),
        None => Ok(Value::Bool(false)),
    }
}

pub(crate) fn fclose(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fclose", args, 0)?;
    let r = resource(&o).expect("a stream resource");
    r.closed.set(true);
    // The file itself closes now (the resource value lives on, closed).
    r.stream.borrow_mut().close();
    Ok(Value::Bool(true))
}

/// fwrite(resource $stream, string $data, ?int $length = null)
pub(crate) fn fwrite(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fwrite", args, 0)?;
    let data = str_arg(args, 1);
    let mut bytes = data.as_bytes();
    if args.len() > 2 && !matches!(arg(args, 2), Value::Null) {
        let n = arg(args, 2).to_int().max(0) as usize;
        bytes = &bytes[..n.min(bytes.len())];
    }
    let (writable, zlib) = with_stream(&o, |s| (s.writable(), s.zlib));
    if !writable && zlib {
        // A gzip file being read takes nothing.
        return Ok(Value::Int(0));
    }
    if !writable {
        ctx.report(8, &format!("fwrite(): Write of {} bytes failed with errno=9 Bad file descriptor", bytes.len()));
        return Ok(Value::Bool(false));
    }
    let r = resource(&o).expect("a stream resource");
    let written = r.stream.borrow_mut().write(ctx, bytes);
    Ok(written.map_or(Value::Bool(false), |n| Value::Int(n as i64)))
}

pub(crate) fn fread(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fread", args, 0)?;
    let n = arg(args, 1).to_int();
    if n <= 0 {
        return Err(ctx.throw("ValueError", format!("{}(): Argument #2 ($length) must be greater than 0", called(ctx, "fread"))));
    }
    if with_stream(&o, |s| s.zlib && !s.readable()) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Str(PhpStr::new(with_stream(&o, |s| s.read(n as usize)))))
}

pub(crate) fn fgets(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fgets", args, 0)?;
    let limit = match args.get(1).map(Value::deref) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v.to_int();
            if n <= 0 {
                return Err(ctx.throw("ValueError", format!("{}(): Argument #2 ($length) must be greater than 0", called(ctx, "fgets"))));
            }
            Some(n as usize)
        }
    };
    Ok(with_stream(&o, |s| s.gets(limit)).map_or(Value::Bool(false), |b| Value::Str(PhpStr::new(b))))
}

pub(crate) fn fgetc(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fgetc", args, 0)?;
    let b = with_stream(&o, |s| s.read(1));
    Ok(if b.is_empty() { Value::Bool(false) } else { Value::Str(PhpStr::new(b)) })
}

pub(crate) fn feof(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "feof", args, 0)?;
    Ok(Value::Bool(with_stream(&o, |s| s.at_end())))
}

fn fflush(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fflush", args, 0)?;
    Ok(Value::Bool(with_stream(&o, |s| s.flush())))
}

pub(crate) fn ftell(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "ftell", args, 0)?;
    Ok(Value::Int(with_stream(&o, |s| s.tell())))
}

pub(crate) fn fseek(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fseek", args, 0)?;
    let offset = arg(args, 1).to_int();
    let whence = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    if with_stream(&o, |s| s.zlib) {
        if whence == 2 {
            ctx.warn(&format!("{}(): SEEK_END is not supported", called(ctx, "gzseek")));
            return Ok(Value::Int(-1));
        }
        // Writing, zlib only goes forward.
        let (writing, here) = with_stream(&o, |s| (s.writable(), s.tell()));
        let to = if whence == 1 { here + offset } else { offset };
        if writing && to < here {
            return Ok(Value::Int(-1));
        }
    }
    Ok(Value::Int(if with_stream(&o, |s| s.seek(offset, whence)) { 0 } else { -1 }))
}

pub(crate) fn rewind(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "rewind", args, 0)?;
    if with_stream(&o, |s| s.zlib && s.writable()) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(with_stream(&o, |s| s.seek(0, 0))))
}

fn ftruncate(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "ftruncate", args, 0)?;
    let size = arg(args, 1).to_int();
    if size < 0 {
        return Err(ctx.throw("ValueError", "ftruncate(): Argument #2 ($size) must be greater than or equal to 0".into()));
    }
    Ok(Value::Bool(with_stream(&o, |s| s.truncate(size as usize))))
}

fn flock(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    stream_arg(ctx, "flock", args, 0)?;
    Ok(Value::Bool(true))
}

/// fprintf(resource $stream, string $format, mixed ...$values)
fn fprintf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fprintf", args, 0)?;
    let bytes = crate::format(&args[1..]);
    let r = resource(&o).expect("a stream resource");
    r.stream.borrow_mut().write(ctx, &bytes);
    Ok(Value::Int(bytes.len() as i64))
}

pub(crate) fn fpassthru(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "fpassthru", args, 0)?;
    let rest = with_stream(&o, |s| s.read_all());
    ctx.echo(&rest);
    Ok(Value::Int(rest.len() as i64))
}

/// stream_get_contents(resource $stream, ?int $length = null, int $offset = -1)
fn stream_get_contents(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_get_contents", args, 0)?;
    let offset = if args.len() > 2 { arg(args, 2).to_int() } else { -1 };
    let length = match args.get(1).map(Value::deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.to_int()),
    };
    Ok(Value::Str(PhpStr::new(with_stream(&o, |s| {
        if offset >= 0 {
            s.seek(offset, 0);
        }
        match length {
            Some(n) if n >= 0 => s.read(n as usize),
            _ => s.read_all(),
        }
    }))))
}

fn tmpfile(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(new_stream(ctx, Stream::new(Target::Memory(Vec::new()), "php://temp", "w+b")))
}

/// The bytes of a file (or php:// stream), or None after PHP's warning.
fn read_path(ctx: &mut dyn Ctx, func: &str, path: &PhpStr) -> Result<Option<Vec<u8>>, Throw> {
    if path.is_empty() {
        return Err(ctx.throw("ValueError", "Path cannot be empty".into()));
    }
    let p = text(path);
    if p.starts_with("php://") {
        return Ok(open(ctx, func, path, "r")?.map(|mut s| s.read_all()));
    }
    if let Some(gz) = p.strip_prefix("compress.zlib://") {
        return Ok(crate::zlib::open_gz(ctx, func, gz, "r").map(|mut s| s.read_all()));
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
    let Some(mut bytes) = read_path(ctx, "file_get_contents", &str_arg(args, 0))? else { return Ok(Value::Bool(false)) };
    let offset = if args.len() > 3 { arg(args, 3).to_int() } else { 0 };
    if offset != 0 {
        let at = if offset < 0 { (bytes.len() as i64 + offset).max(0) as usize } else { (offset as usize).min(bytes.len()) };
        bytes.drain(..at);
    }
    if let Some(n) = args.get(4).map(Value::deref).filter(|v| !matches!(v, Value::Null)) {
        let n = n.to_int();
        if n < 0 {
            return Err(ctx.throw("ValueError", "file_get_contents(): Argument #5 ($length) must be greater than or equal to 0".into()));
        }
        bytes.truncate(n as usize);
    }
    Ok(Value::Str(PhpStr::new(bytes)))
}

const FILE_USE_INCLUDE_PATH: i64 = 1;
const FILE_IGNORE_NEW_LINES: i64 = 2;
const FILE_SKIP_EMPTY_LINES: i64 = 4;
const FILE_APPEND: i64 = 8;

/// file_put_contents(string $filename, mixed $data, int $flags = 0, $context = null)
fn file_put_contents(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = str_arg(args, 0);
    let flags = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    let data: Vec<u8> = match arg(args, 1) {
        Value::Array(a) => a.values().flat_map(|v| v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()).collect(),
        v => match resource_of(&v) {
            Some(o) => with_stream(&o, |s| s.read_all()),
            None => v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec(),
        },
    };
    let mode = if flags & FILE_APPEND != 0 { "a" } else { "w" };
    let Some(mut stream) = open(ctx, "file_put_contents", &path, mode)? else { return Ok(Value::Bool(false)) };
    Ok(stream.write(ctx, &data).map_or(Value::Bool(false), |n| Value::Int(n as i64)))
}

/// file(string $filename, int $flags = 0, $context = null): the lines.
fn file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    if flags & !(FILE_USE_INCLUDE_PATH | FILE_IGNORE_NEW_LINES | FILE_SKIP_EMPTY_LINES) != 0 {
        return Err(ctx.throw("ValueError", "file(): Argument #2 ($flags) must be a valid flag value".into()));
    }
    let Some(bytes) = read_path(ctx, "file", &str_arg(args, 0))? else { return Ok(Value::Bool(false)) };
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
        lines.push(Value::Str(PhpStr::from(line)));
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
    Ok(Value::Array(Rc::new(PhpArray::from_list(lines))))
}

fn readfile(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(bytes) = read_path(ctx, "readfile", &str_arg(args, 0))? else { return Ok(Value::Bool(false)) };
    ctx.echo(&bytes);
    Ok(Value::Int(bytes.len() as i64))
}

/// A filesystem change: true, or false after PHP's warning `func(path): error`.
fn done(ctx: &mut dyn Ctx, func: &str, shown: &str, r: std::io::Result<()>) -> Value {
    match r {
        Ok(()) => Value::Bool(true),
        Err(e) => {
            ctx.warn(&format!("{func}({shown}): {}", os_error(&e)));
            Value::Bool(false)
        }
    }
}

fn unlink(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = text(&str_arg(args, 0));
    // A symlink to a directory is unlinked (the link, not what it names).
    if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
        ctx.warn(&format!("unlink({p}): Is a directory"));
        return Ok(Value::Bool(false));
    }
    let r = std::fs::remove_file(&p);
    Ok(done(ctx, "unlink", &p, r))
}

/// mkdir(string $directory, int $permissions = 0777, bool $recursive = false)
fn mkdir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = text(&str_arg(args, 0));
    let recursive = args.len() > 2 && arg(args, 2).to_bool();
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
        Ok(()) => Value::Bool(true),
        Err(e) => {
            ctx.warn(&format!("mkdir(): {}", os_error(&e)));
            Value::Bool(false)
        }
    })
}

fn rmdir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = text(&str_arg(args, 0));
    let r = std::fs::remove_dir(&p);
    Ok(done(ctx, "rmdir", &p, r))
}

fn rename(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (from, to) = (text(&str_arg(args, 0)), text(&str_arg(args, 1)));
    let r = std::fs::rename(&from, &to);
    Ok(done(ctx, "rename", &format!("{from},{to}"), r))
}

fn copy(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (from, to) = (text(&str_arg(args, 0)), text(&str_arg(args, 1)));
    if from.contains("://") || to.contains("://") {
        // Through the streams: a wrapper (compress.zlib://, php://) at either end.
        let Some(bytes) = read_path(ctx, "copy", &str_arg(args, 0))? else { return Ok(Value::Bool(false)) };
        let Some(mut out) = open(ctx, "copy", &str_arg(args, 1), "wb")? else { return Ok(Value::Bool(false)) };
        let ok = out.write(ctx, &bytes).is_some();
        out.close();
        return Ok(Value::Bool(ok));
    }
    match std::fs::copy(&from, &to) {
        Ok(_) => Ok(Value::Bool(true)),
        Err(e) => {
            ctx.warn(&format!("copy({from}): Failed to open stream: {}", os_error(&e)));
            Ok(Value::Bool(false))
        }
    }
}

/// touch(string $filename, ?int $mtime = null, ?int $atime = null)
fn touch(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = text(&str_arg(args, 0));
    let file = match std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        Ok(f) => f,
        Err(e) => {
            ctx.warn(&format!("touch(): Unable to create file {p} because {}", os_error(&e)));
            return Ok(Value::Bool(false));
        }
    };
    let mtime = match args.get(1).map(Value::deref) {
        Some(Value::Null) | None => std::time::SystemTime::now(),
        Some(v) => std::time::UNIX_EPOCH + std::time::Duration::from_secs(v.to_int().max(0) as u64),
    };
    let _ = file.set_modified(mtime);
    Ok(Value::Bool(true))
}

fn path_arg(args: &[Value]) -> String {
    text(&str_arg(args, 0))
}

fn is_file(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(Path::new(&path_arg(args)).is_file()))
}

fn is_dir(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(Path::new(&path_arg(args)).is_dir()))
}

fn is_link(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(std::fs::symlink_metadata(path_arg(args)).is_ok_and(|m| m.file_type().is_symlink())))
}

fn file_exists(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    Ok(Value::Bool(!p.is_empty() && Path::new(&p).exists()))
}

fn mode_bits(p: &str) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).ok().map(|m| m.permissions().mode())
}

fn is_readable(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(mode_bits(&path_arg(args)).is_some_and(|m| m & 0o444 != 0)))
}

fn is_writable(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(mode_bits(&path_arg(args)).is_some_and(|m| m & 0o222 != 0)))
}

fn is_executable(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    Ok(Value::Bool(Path::new(&p).is_file() && mode_bits(&p).is_some_and(|m| m & 0o111 != 0)))
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
    Ok(stat(ctx, "filesize", &p).map_or(Value::Bool(false), |m| Value::Int(m.len() as i64)))
}

fn filemtime(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    Ok(stat(ctx, "filemtime", &p).map_or(Value::Bool(false), |m| {
        let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        Value::Int(t as i64)
    }))
}

fn fileperms(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    Ok(stat(ctx, "fileperms", &p).map_or(Value::Bool(false), |_| Value::Int(i64::from(mode_bits(&p).unwrap_or(0)))))
}

fn chmod(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    use std::os::unix::fs::PermissionsExt;
    let p = path_arg(args);
    let mode = arg(args, 1).to_int() as u32;
    let r = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode));
    Ok(match r {
        Ok(()) => Value::Bool(true),
        Err(e) => {
            ctx.warn(&format!("chmod(): {}", os_error(&e)));
            Value::Bool(false)
        }
    })
}

/// umask: p8 does not change the process's (a host runs many requests);
/// it reports the usual 022.
fn umask(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(0o022))
}

fn clearstatcache(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

fn temp_dir() -> String {
    let d = std::env::temp_dir().to_string_lossy().into_owned();
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
            return Ok(Value::Str(PhpStr::from(name)));
        }
    }
    Ok(Value::Bool(false))
}

fn sys_get_temp_dir(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Str(PhpStr::from(temp_dir())))
}

fn getcwd(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(std::env::current_dir().map_or(Value::Bool(false), |d| Value::Str(PhpStr::from(d.to_string_lossy().into_owned()))))
}

/// chdir: changes the process's directory (the CLI's one request).
fn chdir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    match std::env::set_current_dir(&p) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            ctx.warn(&format!("chdir(): {} (errno 2)", os_error(&e)));
            Ok(Value::Bool(false))
        }
    }
}

fn realpath(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    let p = if p.is_empty() { ".".to_string() } else { p };
    Ok(std::fs::canonicalize(&p).map_or(Value::Bool(false), |d| Value::Str(PhpStr::from(d.to_string_lossy().into_owned()))))
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
            out.insert(key("dirname"), Value::Str(PhpStr::from(dir)));
        }
    }
    if flags & PATHINFO_BASENAME != 0 {
        out.insert(key("basename"), Value::Str(PhpStr::from(base)));
    }
    let dot = base.iter().rposition(|&c| c == b'.');
    if flags & PATHINFO_EXTENSION != 0
        && let Some(d) = dot
    {
        out.insert(key("extension"), Value::Str(PhpStr::from(&base[d + 1..])));
    }
    if flags & PATHINFO_FILENAME != 0 {
        out.insert(key("filename"), Value::Str(PhpStr::from(&base[..dot.unwrap_or(base.len())])));
    }
    if flags != 15 {
        return Ok(out.values().next().cloned().unwrap_or(Value::Str(PhpStr::empty())));
    }
    Ok(Value::Array(Rc::new(out)))
}

/// A directory's entries, "." and ".." included, sorted.
fn entries(p: &str) -> std::io::Result<Vec<Vec<u8>>> {
    use std::os::unix::ffi::OsStrExt;
    let mut names: Vec<Vec<u8>> = vec![b".".to_vec(), b"..".to_vec()];
    for e in std::fs::read_dir(p)? {
        names.push(e?.file_name().as_bytes().to_vec());
    }
    names.sort();
    Ok(names)
}

/// scandir(string $directory, int $sorting_order = SCANDIR_SORT_ASCENDING)
fn scandir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
    if p.is_empty() {
        return Err(ctx.throw("ValueError", "scandir(): Argument #1 ($directory) cannot be empty".into()));
    }
    match entries(&p) {
        Ok(mut names) => {
            if args.len() > 1 && arg(args, 1).to_int() == 1 {
                names.reverse();
            }
            Ok(Value::Array(Rc::new(PhpArray::from_list(names.into_iter().map(|n| Value::Str(PhpStr::new(n))).collect()))))
        }
        Err(e) => {
            ctx.warn(&format!("scandir({p}): Failed to open directory: {}", os_error(&e)));
            ctx.warn("scandir(): (errno 2): No such file or directory");
            Ok(Value::Bool(false))
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

const GLOB_MARK: i64 = 2;
const GLOB_ONLYDIR: i64 = 1 << 13;

/// glob(string $pattern, int $flags = 0): the paths matching, sorted.
fn glob(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let pattern = path_arg(args);
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
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
            if let Ok(names) = entries(&dir) {
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
    let mut out: Vec<Value> = Vec::new();
    for p in found {
        let dir = Path::new(&p).is_dir();
        if flags & GLOB_ONLYDIR != 0 && !dir {
            continue;
        }
        let shown = if flags & GLOB_MARK != 0 && dir { format!("{p}/") } else { p };
        out.push(Value::Str(PhpStr::from(shown)));
    }
    Ok(Value::Array(Rc::new(PhpArray::from_list(out))))
}

/// opendir: a stream whose lines are the directory's entries (as readdir gives).
fn opendir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path_arg(args);
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
            Ok(Value::Bool(false))
        }
    }
}

fn readdir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "readdir", args, 0)?;
    let mut name = Vec::new();
    loop {
        let b = with_stream(&o, |s| s.read(1));
        match b.first() {
            None if name.is_empty() => return Ok(Value::Bool(false)),
            None | Some(0) => break,
            Some(&c) => name.push(c),
        }
    }
    Ok(Value::Str(PhpStr::new(name)))
}

fn rewinddir(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "rewinddir", args, 0)?;
    with_stream(&o, |s| s.seek(0, 0));
    Ok(Value::Null)
}

/// stream_get_meta_data($stream): what the stream is (wrapper, type, uri,
/// mode) and where it is (eof, bytes read ahead).
fn stream_get_meta_data(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = stream_arg(ctx, "stream_get_meta_data", args, 0)?;
    let entries = with_stream(&o, |s| {
        let (wrapper, kind, seekable) = match &s.target {
            Target::File(_) => ("plainfile", "STDIO", true),
            Target::Memory(_) if s.path.starts_with("php://temp") => ("PHP", "TEMP", true),
            Target::Memory(_) if s.path.starts_with("php://memory") => ("PHP", "MEMORY", true),
            Target::Memory(_) if s.path.starts_with("data:") => ("RFC2397", "RFC2397", true),
            Target::Memory(_) => ("plainfile", "STDIO", true),
            Target::Stdin | Target::Stdout | Target::Stderr => ("PHP", "STDIO", false),
        };
        let text = |t: &str| Value::Str(PhpStr::from(t));
        vec![
            ("timed_out", Value::Bool(false)),
            ("blocked", Value::Bool(true)),
            // At the end for the script: nothing read ahead is left.
            ("eof", Value::Bool(s.eof && s.buffer.is_empty())),
            ("wrapper_type", text(wrapper)),
            ("stream_type", text(kind)),
            // Memory and temp streams are always read-write.
            ("mode", text(if matches!(kind, "MEMORY" | "TEMP") { "w+b" } else { &s.mode })),
            ("unread_bytes", Value::Int(s.buffer.len() as i64)),
            ("seekable", Value::Bool(seekable)),
            ("uri", text(&s.path)),
        ]
    });
    let mut out = PhpArray::new();
    for (k, v) in entries {
        out.insert(Key::Str(PhpStr::from(k)), v);
    }
    Ok(Value::Array(Rc::new(out)))
}

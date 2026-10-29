//! stat and links, backtraces, included files, vfprintf.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/filestat.c, link.c,
//! $PHP_SRC_PATH/Zend/zend_builtin_functions.c (debug_backtrace)

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::stream::{Target, resource, resource_of};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::os::unix::fs::MetadataExt;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "stat", run: stat, by_ref: &[] },
    Builtin { name: "lstat", run: lstat, by_ref: &[] },
    Builtin { name: "fstat", run: fstat, by_ref: &[] },
    Builtin { name: "symlink", run: symlink, by_ref: &[] },
    Builtin { name: "link", run: link, by_ref: &[] },
    Builtin { name: "readlink", run: readlink, by_ref: &[] },
    Builtin { name: "fileinode", run: fileinode, by_ref: &[] },
    Builtin { name: "fileowner", run: fileowner, by_ref: &[] },
    Builtin { name: "chown", run: chown, by_ref: &[] },
    Builtin { name: "chgrp", run: chgrp, by_ref: &[] },
    Builtin { name: "lchown", run: lchown, by_ref: &[] },
    Builtin { name: "lchgrp", run: lchgrp, by_ref: &[] },
    Builtin { name: "filegroup", run: filegroup, by_ref: &[] },
    Builtin { name: "filetype", run: filetype, by_ref: &[] },
    Builtin { name: "debug_backtrace", run: debug_backtrace, by_ref: &[] },
    Builtin { name: "debug_print_backtrace", run: debug_print_backtrace, by_ref: &[] },
    Builtin { name: "get_included_files", run: get_included_files, by_ref: &[] },
    Builtin { name: "get_required_files", run: get_included_files, by_ref: &[] },
    Builtin { name: "vfprintf", run: vfprintf, by_ref: &[] },
];

/// These need the running frames' registers (backtrace arguments).
pub const REENTER: &[&str] = &["debug_backtrace", "debug_print_backtrace"];

fn path(args: &[Value]) -> String {
    String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned()
}

/// stat's array: the 13 values by number, then by name.
fn stat_array(m: &std::fs::Metadata) -> Value {
    let values = [
        ("dev", m.dev() as i64),
        ("ino", m.ino() as i64),
        ("mode", i64::from(m.mode())),
        ("nlink", m.nlink() as i64),
        ("uid", i64::from(m.uid())),
        ("gid", i64::from(m.gid())),
        ("rdev", m.rdev() as i64),
        ("size", m.size() as i64),
        ("atime", m.atime()),
        ("mtime", m.mtime()),
        ("ctime", m.ctime()),
        ("blksize", m.blksize() as i64),
        ("blocks", m.blocks() as i64),
    ];
    let mut out = PhpArray::new();
    for (i, (_, v)) in values.iter().enumerate() {
        out.insert(Key::Int(i as i64), Value::Int(*v));
    }
    for (k, v) in values {
        out.insert(Key::from_string(PhpStr::from(k)), Value::Int(v));
    }
    Value::Array(Rc::new(out))
}

fn stat_of(ctx: &mut dyn Ctx, func: &str, p: &str, follow: bool) -> Option<std::fs::Metadata> {
    let m = if follow { std::fs::metadata(p) } else { std::fs::symlink_metadata(p) };
    match m {
        Ok(m) => Some(m),
        Err(_) => {
            let what = if follow { "stat" } else { "Lstat" };
            ctx.warn(&format!("{func}(): {what} failed for {p}"));
            None
        }
    }
}

fn stat(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path(args);
    Ok(stat_of(ctx, "stat", &p, true).map_or(Value::Bool(false), |m| stat_array(&m)))
}

fn lstat(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path(args);
    Ok(stat_of(ctx, "lstat", &p, false).map_or(Value::Bool(false), |m| stat_array(&m)))
}

fn fstat(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let Some(o) = resource_of(&v) else {
        return Err(ctx.throw("TypeError", format!("fstat(): Argument #1 ($stream) must be of type resource, {} given", v.type_name())));
    };
    let Some(r) = resource(&o) else { return Ok(Value::Bool(false)) };
    let s = r.stream.borrow();
    match &s.target {
        Target::File(f) => Ok(f.metadata().map_or(Value::Bool(false), |m| stat_array(&m))),
        Target::Memory(data) => {
            // A memory stream: its size, and a file's mode.
            let mut out = PhpArray::new();
            let names = ["dev", "ino", "mode", "nlink", "uid", "gid", "rdev", "size", "atime", "mtime", "ctime", "blksize", "blocks"];
            let value = |k: &str| match k {
                "mode" => 0o100666,
                "nlink" => 1,
                "size" => data.len() as i64,
                "blksize" | "blocks" => -1,
                _ => 0,
            };
            for (i, k) in names.iter().enumerate() {
                out.insert(Key::Int(i as i64), Value::Int(value(k)));
            }
            for k in names {
                out.insert(Key::from_string(PhpStr::from(k)), Value::Int(value(k)));
            }
            Ok(Value::Array(Rc::new(out)))
        }
        _ => Ok(Value::Bool(false)),
    }
}

fn os_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".into(),
        std::io::ErrorKind::AlreadyExists => "File exists".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        _ => e.to_string(),
    }
}

fn symlink(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (target, link) = (path(args), String::from_utf8_lossy(str_arg(args, 1).as_bytes()).into_owned());
    match std::os::unix::fs::symlink(&target, &link) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            ctx.warn(&format!("symlink(): {}", os_error(&e)));
            Ok(Value::Bool(false))
        }
    }
}

fn link(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (target, link) = (path(args), String::from_utf8_lossy(str_arg(args, 1).as_bytes()).into_owned());
    match std::fs::hard_link(&target, &link) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            ctx.warn(&format!("link(): {}", os_error(&e)));
            Ok(Value::Bool(false))
        }
    }
}

fn readlink(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path(args);
    match std::fs::read_link(&p) {
        Ok(t) => Ok(Value::Str(PhpStr::from(t.to_string_lossy().into_owned()))),
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::InvalidInput { "Invalid argument".to_string() } else { os_error(&e) };
            ctx.warn(&format!("readlink(): {msg}"));
            Ok(Value::Bool(false))
        }
    }
}

fn stat_field(ctx: &mut dyn Ctx, func: &str, args: &[Value], f: impl Fn(&std::fs::Metadata) -> i64) -> Value {
    let p = path(args);
    stat_of(ctx, func, &p, true).map_or(Value::Bool(false), |m| Value::Int(f(&m)))
}

fn fileinode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(stat_field(ctx, "fileinode", args, |m| m.ino() as i64))
}

fn fileowner(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(stat_field(ctx, "fileowner", args, |m| i64::from(m.uid())))
}

fn filegroup(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(stat_field(ctx, "filegroup", args, |m| i64::from(m.gid())))
}

fn filetype(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path(args);
    let Some(m) = stat_of(ctx, "filetype", &p, false) else { return Ok(Value::Bool(false)) };
    let t = m.file_type();
    Ok(Value::str(if t.is_symlink() {
        "link"
    } else if t.is_dir() {
        "dir"
    } else if t.is_file() {
        "file"
    } else {
        "unknown"
    }))
}

const DEBUG_BACKTRACE_IGNORE_ARGS: i64 = 2;

/// debug_backtrace(int $options = DEBUG_BACKTRACE_PROVIDE_OBJECT, int $limit = 0)
fn debug_backtrace(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let options = if args.is_empty() { 1 } else { arg(args, 0).to_int() };
    let limit = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    let trace = ctx.backtrace();
    let mut out = PhpArray::new();
    for (i, (_, frame)) in trace.iter().enumerate() {
        if limit > 0 && i as i64 >= limit {
            break;
        }
        let mut frame = match frame {
            Value::Array(f) => (**f).clone(),
            _ => continue,
        };
        if options & DEBUG_BACKTRACE_IGNORE_ARGS != 0 {
            frame.remove(&Key::from_string(PhpStr::from("args")));
        }
        let _ = out.push(Value::Array(Rc::new(frame)));
    }
    Ok(Value::Array(Rc::new(out)))
}

/// debug_print_backtrace: the trace as getTraceAsString shows it, no {main}.
fn debug_print_backtrace(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let trace = Value::Array(Rc::new(ctx.backtrace()));
    let shown = p8_runtime::trace::trace_string(&trace);
    // Everything up to the "#N {main}" line.
    let cut = shown.rfind('#').unwrap_or(0);
    ctx.echo(&shown.as_bytes()[..cut]);
    Ok(Value::Null)
}

fn get_included_files(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let files: Vec<Value> = ctx.included_files().into_iter().map(|f| Value::Str(PhpStr::from(f))).collect();
    Ok(Value::Array(Rc::new(PhpArray::from_list(files))))
}

/// vfprintf(resource $stream, string $format, array $values)
fn vfprintf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let Some(o) = resource_of(&v) else {
        return Err(ctx.throw("TypeError", format!("vfprintf(): Argument #1 ($stream) must be of type resource, {} given", v.type_name())));
    };
    let values = crate::printf::array_values(&arg(args, 2));
    let fmt = arg(args, 1).to_php_string().unwrap_or_else(PhpStr::empty);
    let bytes = crate::printf::formatted(ctx, "vfprintf", fmt.as_bytes(), &values, crate::printf::Missing::Array)?;
    if let Some(r) = resource(&o) {
        r.stream.borrow_mut().write(ctx, &bytes);
    }
    Ok(Value::Int(bytes.len() as i64))
}

/// A user or group given by number, or by name from /etc/passwd or
/// /etc/group (the name's third field).
fn id_of(ctx: &mut dyn Ctx, func: &str, v: &Value, table: &str, what: &str) -> Option<u32> {
    match v {
        Value::Int(i) => Some(*i as u32),
        v => {
            let name = v.to_php_string().unwrap_or_else(PhpStr::empty);
            let found = std::fs::read_to_string(table).ok().and_then(|t| {
                t.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
                    let mut f = l.split(':');
                    (f.next()? .as_bytes() == name.as_bytes()).then(|| f.nth(1)?.parse::<u32>().ok()).flatten()
                })
            });
            if found.is_none() {
                ctx.warn(&format!("{func}(): Unable to find {what} for {}", String::from_utf8_lossy(name.as_bytes())));
            }
            found
        }
    }
}

/// chown/chgrp/lchown/lchgrp: the owner or group changed; false (with
/// the system's reason) when it can't be.
fn change_owner(ctx: &mut dyn Ctx, args: &mut [Value], func: &str, group: bool, link: bool) -> Result<Value, Throw> {
    let path = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    let id = if group { id_of(ctx, func, &arg(args, 1), "/etc/group", "gid") } else { id_of(ctx, func, &arg(args, 1), "/etc/passwd", "uid") };
    let Some(id) = id else { return Ok(Value::Bool(false)) };
    let (uid, gid) = if group { (None, Some(id)) } else { (Some(id), None) };
    let r = if link { std::os::unix::fs::lchown(&path, uid, gid) } else { std::os::unix::fs::chown(&path, uid, gid) };
    match r {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            ctx.warn(&format!("{func}(): {}", crate::file::os_error(&e)));
            Ok(Value::Bool(false))
        }
    }
}

fn chown(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    change_owner(ctx, args, "chown", false, false)
}

fn chgrp(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    change_owner(ctx, args, "chgrp", true, false)
}

fn lchown(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    change_owner(ctx, args, "lchown", false, true)
}

fn lchgrp(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    change_owner(ctx, args, "lchgrp", true, true)
}

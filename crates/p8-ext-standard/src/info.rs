//! stat and links, backtraces, included files, vfprintf.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/filestat.c, link.c,
//! $PHP_SRC_PATH/Zend/zend_builtin_functions.c (debug_backtrace)

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use crate::file::user_field;
use p8_runtime::stream::{Target, resource, resource_of};
use p8_runtime::user_stream::{self, Located, Stat};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use p8_runtime::os::MetadataExt;

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

/// These need the running frames' registers (backtrace arguments), or
/// take a path a user-space wrapper may serve (its methods are PHP code).
pub const REENTER: &[&str] = &[
    "debug_backtrace", "debug_print_backtrace", "stat", "lstat", "fileinode", "fileowner", "filegroup", "filetype", "chown", "chgrp", "lchown", "lchgrp",
];

fn path(args: &[Value]) -> String {
    String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned()
}

/// stat's array: the 13 values by number, then by name.
fn stat_array(m: &std::fs::Metadata) -> Value {
    let st = Stat {
        dev: m.dev() as i64,
        ino: m.ino() as i64,
        mode: i64::from(m.mode()),
        nlink: m.nlink() as i64,
        uid: i64::from(m.uid()),
        gid: i64::from(m.gid()),
        rdev: m.rdev() as i64,
        size: m.size() as i64,
        atime: m.atime(),
        mtime: m.mtime(),
        ctime: m.ctime(),
        blksize: m.blksize() as i64,
        blocks: m.blocks() as i64,
    };
    Value::array(st.to_array())
}

fn stat_of(ctx: &mut dyn Ctx, func: &str, p: &str, follow: bool) -> Option<std::fs::Metadata> {
    let os = p8_runtime::cwd::resolve(ctx, p);
    let m = if follow { std::fs::metadata(&os) } else { std::fs::symlink_metadata(&os) };
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
    if let Some(v) = user_field(ctx, "stat", &p, false, |st| Value::array(st.to_array()))? {
        return Ok(v);
    }
    Ok(stat_of(ctx, "stat", &p, true).map_or(Value::FALSE, |m| stat_array(&m)))
}

fn lstat(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path(args);
    if let Some(v) = user_field(ctx, "lstat", &p, true, |st| Value::array(st.to_array()))? {
        return Ok(v);
    }
    Ok(stat_of(ctx, "lstat", &p, false).map_or(Value::FALSE, |m| stat_array(&m)))
}

fn fstat(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let Some(o) = resource_of(&v) else {
        return Err(ctx.throw("TypeError", format!("fstat(): Argument #1 ($stream) must be of type resource, {} given", v.type_name())));
    };
    if crate::file::is_user(&o) {
        // The wrapper's stream_stat.
        let st = crate::file::with_user(ctx, &o, None, |ctx, s| user_stream::stat(ctx, "fstat", s))?;
        return Ok(st.map_or(Value::FALSE, |st| Value::array(st.to_array())));
    }
    let Some(r) = resource(&o) else { return Ok(Value::FALSE) };
    let s = r.stream.borrow();
    match &s.target {
        Target::File(f) => Ok(f.metadata().map_or(Value::FALSE, |m| stat_array(&m))),
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
                out.insert(Key::Int(i as i64), Value::int(value(k)));
            }
            for k in names {
                out.insert(Key::from_string(PhpStr::from(k)), Value::int(value(k)));
            }
            Ok(Value::array(out))
        }
        _ => Ok(Value::FALSE),
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
    // The link's own path is the request's; its target is stored as given
    // (php_sys_symlink(topath, source_p): a relative target is relative to
    // the link).
    match p8_runtime::os::symlink(&target, p8_runtime::cwd::resolve(ctx, &link)) {
        Ok(()) => Ok(Value::TRUE),
        Err(e) => {
            ctx.warn(&format!("symlink(): {}", os_error(&e)));
            Ok(Value::FALSE)
        }
    }
}

fn link(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (target, link) = (path(args), String::from_utf8_lossy(str_arg(args, 1).as_bytes()).into_owned());
    match std::fs::hard_link(p8_runtime::cwd::resolve(ctx, &target), p8_runtime::cwd::resolve(ctx, &link)) {
        Ok(()) => Ok(Value::TRUE),
        Err(e) => {
            ctx.warn(&format!("link(): {}", os_error(&e)));
            Ok(Value::FALSE)
        }
    }
}

fn readlink(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path(args);
    match std::fs::read_link(p8_runtime::cwd::resolve(ctx, &p)) {
        Ok(t) => Ok(Value::str(PhpStr::from(t.to_string_lossy().into_owned()))),
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::InvalidInput { "Invalid argument".to_string() } else { os_error(&e) };
            ctx.warn(&format!("readlink(): {msg}"));
            Ok(Value::FALSE)
        }
    }
}

/// A stat field: from a user-space wrapper's url_stat (`user`), else the
/// file's metadata (`f`).
fn stat_field(ctx: &mut dyn Ctx, func: &str, args: &[Value], user: impl FnOnce(&Stat) -> i64, f: impl Fn(&std::fs::Metadata) -> i64) -> Result<Value, Throw> {
    let p = path(args);
    if let Some(v) = user_field(ctx, func, &p, false, |st| Value::int(user(st)))? {
        return Ok(v);
    }
    Ok(stat_of(ctx, func, &p, true).map_or(Value::FALSE, |m| Value::int(f(&m))))
}

fn fileinode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    stat_field(ctx, "fileinode", args, |st| st.ino, |m| m.ino() as i64)
}

fn fileowner(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    stat_field(ctx, "fileowner", args, |st| st.uid, |m| i64::from(m.uid()))
}

fn filegroup(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    stat_field(ctx, "filegroup", args, |st| st.gid, |m| i64::from(m.gid()))
}

/// What a stat's mode says the file is (filetype()).
fn type_of_mode(ctx: &mut dyn Ctx, mode: i64) -> &'static str {
    match mode & user_stream::S_IFMT {
        0o120000 => "link",
        0o010000 => "fifo",
        0o020000 => "char",
        0o040000 => "dir",
        0o060000 => "block",
        0o100000 => "file",
        0o140000 => "socket",
        other => {
            ctx.notice(&format!("filetype(): Unknown file type ({other})"));
            "unknown"
        }
    }
}

fn filetype(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let p = path(args);
    if let Some(st) = crate::file::user_stat(ctx, "filetype", &p, true, false)? {
        return Ok(st.map_or(Value::FALSE, |st| Value::str(type_of_mode(ctx, st.mode))));
    }
    let Some(m) = stat_of(ctx, "filetype", &p, false) else { return Ok(Value::FALSE) };
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
    const DEBUG_BACKTRACE_PROVIDE_OBJECT: i64 = 1;
    let mut trace = ctx.backtrace(options & DEBUG_BACKTRACE_PROVIDE_OBJECT != 0, usize::try_from(limit).unwrap_or(0));
    let mut out = PhpArray::new();
    // ("args", hashed once per thread: PHPUnit asks for a trace per test.)
    thread_local! {
        static ARGS: [PhpStr; 1] = p8_runtime::array::record_keys(["args"]);
    }
    let args_key = ARGS.with(|k| Key::Str(k[0].clone()));
    // Each frame is moved out of the trace (the only holder), so dropping
    // "args" edits it in place: no frame is copied.
    for i in 0..trace.len() {
        if limit > 0 && i as i64 >= limit {
            break;
        }
        let Some(slot) = trace.get_int_mut(i as i64) else { continue };
        let mut frame = std::mem::take(slot);
        let Some(fields) = frame.array_mut() else { continue };
        if options & DEBUG_BACKTRACE_IGNORE_ARGS != 0 {
            fields.remove(&args_key);
        }
        let _ = out.push(frame);
    }
    Ok(Value::array(out))
}

/// debug_print_backtrace: the trace as getTraceAsString shows it, no {main}.
fn debug_print_backtrace(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let trace = Value::array(ctx.backtrace(false, 0));
    let shown = p8_runtime::trace::trace_string(&trace);
    // Everything up to the "#N {main}" line.
    let cut = shown.rfind('#').unwrap_or(0);
    ctx.echo(&shown.as_bytes()[..cut]);
    Ok(Value::NULL)
}

fn get_included_files(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let files: Vec<Value> = ctx.included_files().into_iter().map(|f| Value::str(PhpStr::from(f))).collect();
    Ok(Value::array(PhpArray::from_list(files)))
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
    if resource(&o).is_some() {
        crate::file::with_user(ctx, &o, None, |ctx, s| s.write(ctx, &bytes))?;
    }
    Ok(Value::int(bytes.len() as i64))
}

/// A user or group given by number, or by name from /etc/passwd or
/// /etc/group (the name's third field).
fn id_of(ctx: &mut dyn Ctx, func: &str, v: &Value, table: &str, what: &str) -> Option<u32> {
    match v.as_int() {
        Some(i) => Some(i as u32),
        None => {
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
    if let Located::User(w, _) = user_stream::locate(ctx, func, &path) {
        // stream_metadata with the name or the number.
        let who = arg(args, 1);
        let option = match (group, who.is_int()) {
            (false, false) => user_stream::META_OWNER_NAME,
            (false, true) => user_stream::META_OWNER,
            (true, false) => user_stream::META_GROUP_NAME,
            (true, true) => user_stream::META_GROUP,
        };
        let value = if who.is_int() { who } else { Value::str(who.to_php_string().unwrap_or_else(PhpStr::empty)) };
        let mut call = [Value::str(path.as_str()), Value::int(option), value];
        return Ok(Value::bool(user_stream::url_op(ctx, func, &w, "stream_metadata", &mut call, Value::NULL)?));
    }
    user_stream::clear_stat_cache(ctx);
    let id = if group { id_of(ctx, func, &arg(args, 1), "/etc/group", "gid") } else { id_of(ctx, func, &arg(args, 1), "/etc/passwd", "uid") };
    let Some(id) = id else { return Ok(Value::FALSE) };
    let (uid, gid) = if group { (None, Some(id)) } else { (Some(id), None) };
    let os = p8_runtime::cwd::resolve(ctx, &path);
    let r = if link { p8_runtime::os::lchown(&os, uid, gid) } else { p8_runtime::os::chown(&os, uid, gid) };
    match r {
        Ok(()) => Ok(Value::TRUE),
        Err(e) => {
            ctx.warn(&format!("{func}(): {}", crate::file::os_error(&e)));
            Ok(Value::FALSE)
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

//! The deprecated procedural zip_* functions: zip_open() and friends, read-only
//! views of an archive's entries, as resources of the types "Zip Directory"
//! and "Zip Entry".
//!
//! Reference: $PHP_SRC_PATH/ext/zip/php_zip.c (zip_open, zip_read, zip_entry_*)

use crate::archive::{Archive, Stat};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::stream::{Resource, Stream, Target, resource, resource_of};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

type R = Result<Value, Throw>;

/// An archive opened by zip_open(), and the next entry zip_read() gives.
struct Dir {
    a: Archive,
    next: usize,
}

/// A zip_read() entry: what it was when read, and how much was read of it.
struct Entry {
    dir: u64,
    stat: Stat,
    data: Vec<u8>,
    pos: usize,
}

thread_local! {
    static DIRS: RefCell<HashMap<u64, Dir>> = RefCell::new(HashMap::new());
    static ENTRIES: RefCell<HashMap<u64, Entry>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

fn next_id() -> u64 {
    NEXT.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    })
}

/// A resource of type `kind` standing for table entry `id`; the entry goes
/// when the resource does (an entry then names an archive that is gone).
fn new_resource(ctx: &mut dyn Ctx, kind: &'static str, id: u64) -> Value {
    let mut stream = Stream::new(Target::Memory(Vec::new()), &id.to_string(), "r");
    stream.on_close = Some(Box::new(move |_| {
        if kind == "Zip Directory" {
            DIRS.with(|d| d.borrow_mut().remove(&id));
        } else {
            ENTRIES.with(|e| e.borrow_mut().remove(&id));
        }
    }));
    ctx.new_resource(Resource { kind, closed: Cell::new(false), stream: RefCell::new(stream) })
}

/// The table id of the live resource of type `kind` in argument `i`.
fn resource_id(ctx: &mut dyn Ctx, args: &[Value], i: usize, kind: &'static str) -> Result<u64, Throw> {
    let func = ctx.builtin_name();
    let name = if kind == "Zip Directory" { if i == 0 { "zip" } else { "zip_entry" } } else { "zip_entry" };
    let v = args.get(i).map(Value::deref).unwrap_or(Value::Null);
    match resource_of(&v) {
        Some(o) => match resource(&o) {
            Some(r) if r.kind == kind && !r.closed.get() => Ok(r.stream.borrow().path.parse().unwrap_or(0)),
            _ => Err(ctx.throw("TypeError", format!("{func}(): supplied resource is not a valid {kind} resource"))),
        },
        None => Err(ctx.throw("TypeError", format!("{func}(): Argument #{} (${name}) must be of type resource, {} given", i + 1, v.type_name()))),
    }
}

fn close_resource(v: &Value) {
    if let Some(o) = resource_of(v)
        && let Some(r) = resource(&o) {
            r.closed.set(true);
        }
}

fn zip_open(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let name = args.first().and_then(|v| v.deref().to_php_string()).unwrap_or_else(PhpStr::empty);
    let f = ctx.builtin_name();
    if name.is_empty() {
        return Err(ctx.throw("ValueError", format!("{f}(): Argument #1 ($filename) must not be empty")));
    }
    if name.as_bytes().contains(&0) {
        return Err(ctx.throw("ValueError", format!("{f}(): Argument #1 ($filename) must not contain any null bytes")));
    }
    match crate::open_archive(ctx, "zip_open", &String::from_utf8_lossy(name.as_bytes()), 0) {
        Ok(zip) => {
            let id = next_id();
            DIRS.with(|d| d.borrow_mut().insert(id, Dir { a: zip.a, next: 0 }));
            Ok(new_resource(ctx, "Zip Directory", id))
        }
        Err(code) => Ok(Value::Int(i64::from(code))),
    }
}

fn zip_close(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    resource_id(ctx, args, 0, "Zip Directory")?;
    close_resource(&args[0]);
    Ok(Value::Null)
}

fn zip_read(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let id = resource_id(ctx, args, 0, "Zip Directory")?;
    let entry = DIRS.with(|d| {
        let mut d = d.borrow_mut();
        let dir = d.get_mut(&id)?;
        if dir.next >= dir.a.count() {
            return None;
        }
        let i = dir.next;
        let stat = dir.a.stat(i, false).ok()?;
        let data = dir.a.read(i, false).ok()?;
        dir.next += 1;
        Some(Entry { dir: id, stat, data, pos: 0 })
    });
    let Some(entry) = entry else { return Ok(Value::Bool(false)) };
    let eid = next_id();
    ENTRIES.with(|e| e.borrow_mut().insert(eid, entry));
    Ok(new_resource(ctx, "Zip Entry", eid))
}

fn zip_entry_open(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    resource_id(ctx, args, 0, "Zip Directory")?;
    resource_id(ctx, args, 1, "Zip Entry")?;
    Ok(Value::Bool(true))
}

fn zip_entry_close(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    resource_id(ctx, args, 0, "Zip Entry")?;
    close_resource(&args[0]);
    Ok(Value::Bool(true))
}

fn zip_entry_read(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let id = resource_id(ctx, args, 0, "Zip Entry")?;
    let len = match args.get(1).map_or(0, |v| v.deref().to_int()) {
        n if n <= 0 => 1024,
        n => n as usize,
    };
    let chunk = ENTRIES.with(|e| {
        let mut e = e.borrow_mut();
        let entry = e.get_mut(&id)?;
        let n = len.min(entry.data.len() - entry.pos);
        let chunk = entry.data[entry.pos..entry.pos + n].to_vec();
        entry.pos += n;
        Some(chunk)
    });
    Ok(match chunk {
        Some(c) => Value::Str(PhpStr::new(c)),
        None => Value::Bool(false),
    })
}

/// What an entry's info functions show: its stat, while its archive lives.
fn info<T>(ctx: &mut dyn Ctx, args: &[Value], f: impl FnOnce(&Stat) -> T) -> Result<Option<T>, Throw> {
    let id = resource_id(ctx, args, 0, "Zip Entry")?;
    Ok(ENTRIES.with(|e| {
        let e = e.borrow();
        let entry = e.get(&id)?;
        DIRS.with(|d| d.borrow().contains_key(&entry.dir)).then(|| f(&entry.stat))
    }))
}

fn zip_entry_name(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(info(ctx, args, |s| Value::Str(PhpStr::new(s.name.clone())))?.unwrap_or(Value::Bool(false)))
}

fn zip_entry_filesize(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(info(ctx, args, |s| Value::Int(s.size as i64))?.unwrap_or(Value::Bool(false)))
}

fn zip_entry_compressedsize(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(info(ctx, args, |s| Value::Int(s.comp_size))?.unwrap_or(Value::Bool(false)))
}

fn zip_entry_compressionmethod(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(info(ctx, args, |s| match s.comp_method {
        0 => Value::str("stored"),
        1 => Value::str("shrunk"),
        2..=5 => Value::str("reduced"),
        6 => Value::str("imploded"),
        7 => Value::str("tokenized"),
        8 => Value::str("deflated"),
        9 => Value::str("deflatedX"),
        10 => Value::str("implodedX"),
        _ => Value::Bool(false),
    })?
    .unwrap_or(Value::Bool(false)))
}

macro_rules! b {
    ($name:literal, $f:ident) => {
        Builtin { name: $name, run: $f, by_ref: &[] }
    };
}

pub static BUILTINS: &[Builtin] = &[
    b!("zip_open", zip_open),
    b!("zip_close", zip_close),
    b!("zip_read", zip_read),
    b!("zip_entry_open", zip_entry_open),
    b!("zip_entry_close", zip_entry_close),
    b!("zip_entry_read", zip_entry_read),
    b!("zip_entry_name", zip_entry_name),
    b!("zip_entry_compressedsize", zip_entry_compressedsize),
    b!("zip_entry_filesize", zip_entry_filesize),
    b!("zip_entry_compressionmethod", zip_entry_compressionmethod),
];

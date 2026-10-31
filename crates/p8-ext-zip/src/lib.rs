#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's zip extension: the ZipArchive class and the zip_* functions, over a
//! zip container read and written here and pure-Rust deflate (miniz_oxide,
//! crc32fast): no libzip. What is observable follows libzip as php-src's
//! ext/zip drives it: indexes, stat arrays, flags, error codes and strings,
//! the state ZipArchive's properties show. The compressed bytes differ from
//! libzip's. Not implemented: encryption (setPassword is accepted,
//! setEncryption* and encrypted entries are refused), the compression
//! methods other than store and deflate, multi-volume archives, the
//! progress and cancel callbacks outliving their archive's object, and
//! stat/seek on zip:// streams.
//!
//! Reference: $PHP_SRC_PATH/ext/zip/php_zip.c, ext/zip/zip_stream.c

mod archive;
mod procedural;

use archive::{Archive, Error, Source};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{ClassHooks, ClassSpec, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_UNCLONEABLE, Native, Object, Visibility};
use p8_runtime::ops::Diag;
use p8_runtime::stream::{MAXPATHLEN, Resource, Stream, Target};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "zip", register };

type R = Result<Value, Throw>;

const CREATE: i64 = 1;
const EXCL: i64 = 2;
const CHECKCONS: i64 = 4;
const OVERWRITE: i64 = 8;
const RDONLY: i64 = 16;

const FL_NOCASE: i64 = 1;
const FL_NODIR: i64 = 2;
const FL_COMPRESSED: i64 = 4;
const FL_UNCHANGED: i64 = 8;
const FL_OVERWRITE: i64 = 8192;
const FL_OPEN_FILE_NOW: i64 = 1 << 30;
const FL_ENC_CP437: i64 = 4096;

const AFL_RDONLY: i64 = 2;
const AFL_IS_TORRENTZIP: i64 = 4;
const AFL_WANT_TORRENTZIP: i64 = 8;
const AFL_CREATE_OR_KEEP_FILE_FOR_EMPTY_ARCHIVE: i64 = 16;

const GLOB_ERR: i64 = 1 << 0;
const GLOB_MARK: i64 = 1 << 1;
const GLOB_NOSORT: i64 = 1 << 2;
const GLOB_NOCHECK: i64 = 1 << 4;
const GLOB_NOESCAPE: i64 = 1 << 6;
const GLOB_BRACE: i64 = 1 << 10;
const GLOB_ONLYDIR: i64 = 1 << 13;

fn register(registry: &mut Registry) {
    registry.add_functions(procedural::BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_data("ZipArchive", || Rc::new(Handle::default()));
    registry.add_dtor("ZipArchive", destruct);
    registry.add_hooks("ZipArchive", ClassHooks { get_properties: Some(properties), ..ClassHooks::default() });
    p8_runtime::stream::register_wrapper("zip", open_wrapper);
    registry.add_module("zip");
}

/// A ZipArchive's native state.
struct Handle {
    z: RefCell<Option<Zip>>,
    last_id: Cell<i64>,
    /// The error the last close left (the properties and getStatusString
    /// show it once the archive is closed).
    closed_err: Cell<Error>,
    /// close() is running (its callbacks may not use the archive).
    closing: Cell<bool>,
}

impl Default for Handle {
    fn default() -> Handle {
        Handle { z: RefCell::new(None), last_id: Cell::new(-1), closed_err: Cell::new(Error::default()), closing: Cell::new(false) }
    }
}

impl Drop for Handle {
    /// An archive let go of while open is written, as PHP's free_obj does
    /// (without callbacks or warnings: there is no context to run them in).
    fn drop(&mut self) {
        if let Some(mut zip) = self.z.get_mut().take() {
            let _ = write_file(&mut zip);
        }
    }
}

/// An open archive.
pub(crate) struct Zip {
    pub(crate) a: Archive,
    /// The file it is written to on close.
    path: String,
    err: Error,
    /// Opened with OVERWRITE (ZIP_TRUNCATE): the file is replaced on close.
    truncated: bool,
    /// Opened with RDONLY: the flag can not be cleared.
    opened_rdonly: bool,
    keep_empty: bool,
    progress: Option<(f64, Value)>,
    cancel: Option<Value>,
}

fn handle(o: &Object) -> &Handle {
    o.native::<Handle>().expect("ZipArchive state")
}

const fn c(v: i64) -> Constant {
    Constant::Int(v)
}

static CLASS_CONSTANTS: &[(&str, Constant)] = &[
    ("CREATE", c(1)), ("EXCL", c(2)), ("CHECKCONS", c(4)), ("OVERWRITE", c(8)), ("RDONLY", c(16)),
    ("FL_NOCASE", c(1)), ("FL_NODIR", c(2)), ("FL_COMPRESSED", c(4)), ("FL_UNCHANGED", c(8)), ("FL_RECOMPRESS", c(16)),
    ("FL_ENCRYPTED", c(32)), ("FL_OVERWRITE", c(8192)), ("FL_LOCAL", c(256)), ("FL_CENTRAL", c(512)),
    ("FL_ENC_GUESS", c(0)), ("FL_ENC_RAW", c(64)), ("FL_ENC_STRICT", c(128)), ("FL_ENC_UTF_8", c(2048)),
    ("FL_ENC_CP437", c(4096)), ("FL_OPEN_FILE_NOW", c(1073741824)),
    ("CM_DEFAULT", c(-1)), ("CM_STORE", c(0)), ("CM_SHRINK", c(1)), ("CM_REDUCE_1", c(2)), ("CM_REDUCE_2", c(3)),
    ("CM_REDUCE_3", c(4)), ("CM_REDUCE_4", c(5)), ("CM_IMPLODE", c(6)), ("CM_DEFLATE", c(8)), ("CM_DEFLATE64", c(9)),
    ("CM_PKWARE_IMPLODE", c(10)), ("CM_BZIP2", c(12)), ("CM_LZMA", c(14)), ("CM_LZMA2", c(33)), ("CM_ZSTD", c(93)),
    ("CM_XZ", c(95)), ("CM_TERSE", c(18)), ("CM_LZ77", c(19)), ("CM_WAVPACK", c(97)), ("CM_PPMD", c(98)),
    ("ER_OK", c(0)), ("ER_MULTIDISK", c(1)), ("ER_RENAME", c(2)), ("ER_CLOSE", c(3)), ("ER_SEEK", c(4)),
    ("ER_READ", c(5)), ("ER_WRITE", c(6)), ("ER_CRC", c(7)), ("ER_ZIPCLOSED", c(8)), ("ER_NOENT", c(9)),
    ("ER_EXISTS", c(10)), ("ER_OPEN", c(11)), ("ER_TMPOPEN", c(12)), ("ER_ZLIB", c(13)), ("ER_MEMORY", c(14)),
    ("ER_CHANGED", c(15)), ("ER_COMPNOTSUPP", c(16)), ("ER_EOF", c(17)), ("ER_INVAL", c(18)), ("ER_NOZIP", c(19)),
    ("ER_INTERNAL", c(20)), ("ER_INCONS", c(21)), ("ER_REMOVE", c(22)), ("ER_DELETED", c(23)),
    ("ER_ENCRNOTSUPP", c(24)), ("ER_RDONLY", c(25)), ("ER_NOPASSWD", c(26)), ("ER_WRONGPASSWD", c(27)),
    ("ER_OPNOTSUPP", c(28)), ("ER_INUSE", c(29)), ("ER_TELL", c(30)), ("ER_COMPRESSED_DATA", c(31)),
    ("ER_CANCELLED", c(32)), ("ER_DATA_LENGTH", c(33)), ("ER_NOT_ALLOWED", c(34)), ("ER_TRUNCATED_ZIP", c(35)),
    ("AFL_RDONLY", c(2)), ("AFL_IS_TORRENTZIP", c(4)), ("AFL_WANT_TORRENTZIP", c(8)),
    ("AFL_CREATE_OR_KEEP_FILE_FOR_EMPTY_ARCHIVE", c(16)),
    ("OPSYS_DOS", c(0)), ("OPSYS_AMIGA", c(1)), ("OPSYS_OPENVMS", c(2)), ("OPSYS_UNIX", c(3)), ("OPSYS_VM_CMS", c(4)),
    ("OPSYS_ATARI_ST", c(5)), ("OPSYS_OS_2", c(6)), ("OPSYS_MACINTOSH", c(7)), ("OPSYS_Z_SYSTEM", c(8)),
    ("OPSYS_CPM", c(9)), ("OPSYS_WINDOWS_NTFS", c(10)), ("OPSYS_MVS", c(11)), ("OPSYS_VSE", c(12)),
    ("OPSYS_ACORN_RISC", c(13)), ("OPSYS_VFAT", c(14)), ("OPSYS_ALTERNATE_MVS", c(15)), ("OPSYS_BEOS", c(16)),
    ("OPSYS_TANDEM", c(17)), ("OPSYS_OS_400", c(18)), ("OPSYS_OS_X", c(19)), ("OPSYS_DEFAULT", c(3)),
    ("EM_NONE", c(0)), ("EM_TRAD_PKWARE", c(1)), ("EM_AES_128", c(257)), ("EM_AES_192", c(258)),
    ("EM_AES_256", c(259)), ("EM_UNKNOWN", c(65535)),
    ("LIBZIP_VERSION", Constant::Str(b"1.11.3")), ("LENGTH_TO_END", c(0)), ("LENGTH_UNCHECKED", c(-2)),
];

const PUB: Visibility = Visibility::Public;

macro_rules! m {
    ($name:literal, $f:ident) => {
        ($name, Some(Native::Method($f)))
    };
}

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "ZipArchive",
    parent: None,
    interfaces: &["Countable"],
    flags: CLASS_UNCLONEABLE,
    properties: &[
        ("lastId", Constant::Int(-1), PUB),
        ("status", Constant::Int(0), PUB),
        ("statusSys", Constant::Int(0), PUB),
        ("numFiles", Constant::Int(0), PUB),
        ("filename", Constant::Str(b""), PUB),
        ("comment", Constant::Str(b""), PUB),
    ],
    methods: &[
        m!("$lastId::get", get_last_id),
        m!("$lastId::set", set_last_id),
        m!("$status::get", get_status),
        m!("$status::set", set_status),
        m!("$statusSys::get", get_status_sys),
        m!("$statusSys::set", set_status_sys),
        m!("$numFiles::get", get_num_files),
        m!("$numFiles::set", set_num_files),
        m!("$filename::get", get_filename),
        m!("$filename::set", set_filename),
        m!("$comment::get", get_comment_prop),
        m!("$comment::set", set_comment_prop),
        m!("open", open),
        m!("setPassword", set_password),
        m!("close", close),
        m!("count", count),
        m!("getStatusString", get_status_string),
        m!("clearError", clear_error),
        m!("addEmptyDir", add_empty_dir),
        m!("addFromString", add_from_string),
        m!("addFile", add_file),
        m!("replaceFile", replace_file),
        m!("addGlob", add_glob),
        m!("addPattern", add_pattern),
        m!("renameIndex", rename_index),
        m!("renameName", rename_name),
        m!("setArchiveComment", set_archive_comment),
        m!("getArchiveComment", get_archive_comment),
        m!("setArchiveFlag", set_archive_flag),
        m!("getArchiveFlag", get_archive_flag),
        m!("setCommentIndex", set_comment_index),
        m!("setCommentName", set_comment_name),
        m!("setMtimeIndex", set_mtime_index),
        m!("setMtimeName", set_mtime_name),
        m!("getCommentIndex", get_comment_index),
        m!("getCommentName", get_comment_name),
        m!("deleteIndex", delete_index),
        m!("deleteName", delete_name),
        m!("statName", stat_name),
        m!("statIndex", stat_index),
        m!("locateName", locate_name),
        m!("getNameIndex", get_name_index),
        m!("unchangeArchive", unchange_archive),
        m!("unchangeAll", unchange_all),
        m!("unchangeIndex", unchange_index),
        m!("unchangeName", unchange_name),
        m!("extractTo", extract_to),
        m!("getFromName", get_from_name),
        m!("getFromIndex", get_from_index),
        m!("getStreamIndex", get_stream_index),
        m!("getStreamName", get_stream_name),
        m!("getStream", get_stream),
        m!("setExternalAttributesName", set_ext_attr_name),
        m!("setExternalAttributesIndex", set_ext_attr_index),
        m!("getExternalAttributesName", get_ext_attr_name),
        m!("getExternalAttributesIndex", get_ext_attr_index),
        m!("setCompressionName", set_compression_name),
        m!("setCompressionIndex", set_compression_index),
        m!("setEncryptionName", set_encryption_name),
        m!("setEncryptionIndex", set_encryption_index),
        m!("registerProgressCallback", register_progress_callback),
        m!("registerCancelCallback", register_cancel_callback),
        ("isCompressionMethodSupported", Some(Native::Static(is_compression_supported))),
        ("isEncryptionMethodSupported", Some(Native::Static(is_encryption_supported))),
    ],
    constants: CLASS_CONSTANTS,
    debug_info: None,
}];

// ---- the properties: virtual, read from the archive ----

fn get_last_id(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Int(handle(this).last_id.get()))
}

fn error_of(this: &Object) -> Error {
    let h = handle(this);
    match &*h.z.borrow() {
        Some(z) => z.err,
        None => h.closed_err.get(),
    }
}

fn get_status(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Int(i64::from(error_of(this).code)))
}

fn get_status_sys(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Int(i64::from(error_of(this).sys)))
}

fn get_num_files(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Int(handle(this).z.borrow().as_ref().map_or(0, |z| z.a.count() as i64)))
}

fn get_filename(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(handle(this).z.borrow().as_ref().map_or_else(|| Value::str(""), |z| Value::str(z.path.clone())))
}

fn get_comment_prop(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(handle(this).z.borrow().as_ref().map_or_else(|| Value::str(""), |z| string(z.a.comment.clone())))
}

fn read_only(ctx: &mut dyn Ctx, this: &Rc<Object>, name: &str) -> R {
    Err(ctx.throw("Error", format!("Cannot write read-only property {}::${name}", text(this.class.name.as_bytes()))))
}

macro_rules! read_only_setter {
    ($f:ident, $name:literal) => {
        fn $f(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
            read_only(ctx, this, $name)
        }
    };
}

read_only_setter!(set_last_id, "lastId");
read_only_setter!(set_status, "status");
read_only_setter!(set_status_sys, "statusSys");
read_only_setter!(set_num_files, "numFiles");
read_only_setter!(set_filename, "filename");
read_only_setter!(set_comment_prop, "comment");

/// The object's properties as PHP lists them: the six, then a subclass's own
/// (with the mangled names of private and protected ones), then dynamic ones.
fn properties(o: &Object) -> PhpArray {
    let mut a = PhpArray::new();
    let h = handle(o);
    {
        let z = h.z.borrow();
        let err = z.as_ref().map_or(h.closed_err.get(), |z| z.err);
        let mut set = |k: &str, v: Value| a.insert(Key::from_string(k.into()), v);
        set("lastId", Value::Int(h.last_id.get()));
        set("status", Value::Int(i64::from(err.code)));
        set("statusSys", Value::Int(i64::from(err.sys)));
        set("numFiles", Value::Int(z.as_ref().map_or(0, |z| z.a.count() as i64)));
        set("filename", z.as_ref().map_or_else(|| Value::str(""), |z| Value::str(z.path.clone())));
        set("comment", z.as_ref().map_or_else(|| Value::str(""), |z| string(z.a.comment.clone())));
    }
    let slots = o.slots.borrow();
    for (p, v) in o.class.properties.iter().zip(slots.iter()) {
        if p.hooks != 0 || p.is_internal() || matches!(v, Value::Undef) {
            continue;
        }
        let name = match p.visibility {
            Visibility::Public => p.name.as_bytes().to_vec(),
            Visibility::Protected => [&b"\0*\0"[..], p.name.as_bytes()].concat(),
            Visibility::Private => [&b"\0"[..], p.declaring_name.as_bytes(), b"\0", p.name.as_bytes()].concat(),
        };
        a.insert(Key::from_string(PhpStr::new(name)), v.clone());
    }
    for (k, v) in o.dynamic.borrow().iter() {
        a.insert(k, v.clone());
    }
    a
}

// ---- argument helpers ----

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

fn int(args: &[Value], i: usize) -> i64 {
    arg(args, i).to_int()
}

fn opt_int(args: &[Value], i: usize, default: i64) -> i64 {
    if args.len() <= i { default } else { arg(args, i).to_int() }
}

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    arg(args, i).to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn string(b: Vec<u8>) -> Value {
    Value::Str(PhpStr::new(b))
}

/// PHP's argument checks for the method `stub` ("ZipArchive::open").
fn prep(ctx: &mut dyn Ctx, stub: &str, args: &mut [Value]) -> Result<(), Throw> {
    let sig = p8_runtime::stubs::lookup(stub.as_bytes()).expect("a stub signature");
    let mut diags = Vec::new();
    let checked = p8_runtime::args::check(sig, args, &mut diags);
    for d in diags {
        match d {
            Diag::Warning(m) => ctx.warn(&m),
            Diag::Deprecated(m) => ctx.deprecated(&m),
        }
    }
    checked.map_err(|(class, message)| ctx.throw(class, message))
}

fn value_error(ctx: &mut dyn Ctx, stub: &str, n: usize, name: &str, msg: &str) -> Throw {
    ctx.throw("ValueError", format!("{stub}(): Argument #{n} (${name}) {msg}"))
}

fn not_empty(ctx: &mut dyn Ctx, stub: &str, n: usize, name: &str) -> Throw {
    value_error(ctx, stub, n, name, "must not be empty")
}

fn uninitialized(ctx: &mut dyn Ctx) -> Throw {
    ctx.throw("ValueError", "Invalid or uninitialized Zip object".into())
}

/// The check every method makes first: the archive is open.
fn require_open(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<(), Throw> {
    let h = handle(this);
    if h.closing.get() {
        return Err(ctx.throw("Error", "Already being closed".into()));
    }
    if h.z.borrow().is_none() {
        return Err(uninitialized(ctx));
    }
    Ok(())
}

/// Run `f` on the open archive; an archive not open is an error.
fn with_zip(ctx: &mut dyn Ctx, this: &Rc<Object>, f: impl FnOnce(&mut Zip, &Handle) -> R) -> R {
    require_open(ctx, this)?;
    let h = handle(this);
    let mut z = h.z.borrow_mut();
    f(z.as_mut().expect("checked open"), h)
}

fn fail(z: &mut Zip, code: i32) -> R {
    z.err = Error::new(code);
    Ok(Value::Bool(false))
}

/// Success leaves the archive's error as it was (libzip clears none).
fn ok(_: &mut Zip) -> R {
    Ok(Value::Bool(true))
}

/// A read-only archive refuses changes.
fn modify(z: &mut Zip) -> bool {
    if z.a.readonly {
        z.err = Error::new(archive::ER_RDONLY);
        return false;
    }
    true
}

fn index(z: &Zip, i: i64) -> Option<usize> {
    if i < 0 || i as usize >= z.a.count() { None } else { Some(i as usize) }
}

fn locate(z: &Zip, name: &[u8], flags: i64) -> Option<usize> {
    z.a.locate(name, flags & FL_NOCASE != 0, flags & FL_NODIR != 0, flags & FL_UNCHANGED != 0)
}

fn warn_all(ctx: &mut dyn Ctx, warns: Vec<String>) {
    for w in warns {
        ctx.warn(&w);
    }
}

// ---- opening and closing ----

/// The absolute form of a path (expand_filepath): joined to the working
/// directory, `.` and `..` folded away.
pub(crate) fn expand_path(path: &str) -> String {
    let full = if path.starts_with('/') {
        path.to_string()
    } else {
        let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        format!("{cwd}/{path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for p in full.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// zip_open(): the archive at `path`, or libzip's error code.
pub(crate) fn open_archive(ctx: &mut dyn Ctx, func: &str, path: &str, flags: i64) -> Result<Zip, i32> {
    let resolved = expand_path(path);
    let meta = std::fs::metadata(&resolved);
    let mut flags = flags;
    // An empty file is no archive to libzip 1.6+; PHP makes it a new one.
    if flags & (OVERWRITE | RDONLY) == 0
        && let Ok(m) = &meta
            && m.is_file() && m.len() == 0 {
                ctx.deprecated(&format!("{func}(): Using empty file as ZipArchive is deprecated"));
                flags |= OVERWRITE;
            }
    let new = |truncated| Zip {
        a: Archive::new(),
        path: resolved.clone(),
        err: Error::default(),
        truncated,
        opened_rdonly: flags & RDONLY != 0,
        keep_empty: false,
        progress: None,
        cancel: None,
    };
    if flags & OVERWRITE != 0 && flags & RDONLY == 0 {
        return Ok(new(meta.is_ok()));
    }
    match meta {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if flags & CREATE != 0 && flags & RDONLY == 0 { Ok(new(false)) } else { Err(archive::ER_NOENT) }
        }
        Err(_) => Err(archive::ER_OPEN),
        Ok(m) => {
            if flags & EXCL != 0 {
                return Err(archive::ER_EXISTS);
            }
            if m.is_dir() {
                return Err(archive::ER_READ);
            }
            let raw = std::fs::read(&resolved).map_err(|_| archive::ER_OPEN)?;
            if raw.is_empty() {
                return Err(archive::ER_NOZIP);
            }
            let mut zip = new(false);
            zip.a = Archive::parse(raw, flags & CHECKCONS != 0).map_err(|e| e.code)?;
            zip.a.readonly = flags & RDONLY != 0;
            Ok(zip)
        }
    }
}

fn open(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::open", args)?;
    let name = bytes(args, 0);
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::open", 1, "filename"));
    }
    if name.contains(&0) {
        return Err(value_error(ctx, "ZipArchive::open", 1, "filename", "must not contain any null bytes"));
    }
    let flags = opt_int(args, 1, 0);
    let h = handle(this);
    if h.closing.get() {
        return Err(ctx.throw("Error", "Already being closed".into()));
    }
    // An archive still open is closed first.
    if h.z.borrow().is_some() && !finish(ctx, this, "ZipArchive::open")? {
        ctx.warn("ZipArchive::open(): Empty string as source");
        return Ok(Value::Bool(false));
    }
    Ok(match open_archive(ctx, "ZipArchive::open", &text(&name), flags) {
        Ok(zip) => {
            *h.z.borrow_mut() = Some(zip);
            Value::Bool(true)
        }
        Err(code) => Value::Int(i64::from(code)),
    })
}

/// Write the archive out and let it go; false when that failed (a warning
/// said why, and the error stays for the properties).
fn finish(ctx: &mut dyn Ctx, this: &Rc<Object>, func: &str) -> Result<bool, Throw> {
    let h = handle(this);
    let Some(mut zip) = h.z.borrow_mut().take() else { return Ok(false) };
    h.closing.set(true);
    let r = write_archive(ctx, &mut zip);
    h.closing.set(false);
    match r {
        Err(thrown) => {
            *h.z.borrow_mut() = Some(zip);
            Err(thrown)
        }
        Ok(Ok(())) => {
            h.closed_err.set(Error::default());
            Ok(true)
        }
        Ok(Err(e)) => {
            h.closed_err.set(e);
            ctx.warn(&format!("{func}(): {}", e.message()));
            Ok(false)
        }
    }
}

/// Whether closing has anything to write: a change, a truncation, or an
/// empty archive asked to be kept that is not on disk yet.
fn needs_write(zip: &Zip) -> bool {
    zip.truncated || zip.a.is_changed() || (zip.keep_empty && !std::path::Path::new(&zip.path).exists())
}

/// zip_close(), with the progress and cancel callbacks.
fn write_archive(ctx: &mut dyn Ctx, zip: &mut Zip) -> Result<Result<(), Error>, Throw> {
    if !needs_write(zip) {
        return Ok(Ok(()));
    }
    let live = zip.a.slots.iter().filter(|s| !s.deleted).count();
    if live > 0 {
        let progress = zip.progress.clone();
        let cancel = zip.cancel.clone();
        let tell = |ctx: &mut dyn Ctx, p: f64| -> Result<(), Throw> {
            if let Some((_, cb)) = &progress {
                ctx.call(cb, &mut [Value::Float(p)])?;
            }
            Ok(())
        };
        tell(ctx, 0.0)?;
        let rate = progress.as_ref().map_or(1.0, |(r, _)| *r);
        let mut last = 0.0;
        for i in 0..live {
            if let Some(cb) = &cancel
                && ctx.call(cb, &mut [])?.to_int() != 0 {
                    return Ok(Err(Error::new(32)));
                }
            let p = (i + 1) as f64 / live as f64;
            if p < 1.0 && p - last >= rate {
                last = p;
                tell(ctx, p)?;
            }
        }
        if let Some(cb) = &cancel
            && ctx.call(cb, &mut [])?.to_int() != 0 {
                return Ok(Err(Error::new(32)));
            }
        let r = write_file(zip);
        if r.is_ok() {
            tell(ctx, 1.0)?;
        }
        return Ok(r);
    }
    Ok(write_file(zip))
}

/// The file side of closing: the archive built and put in place.
fn write_file(zip: &mut Zip) -> Result<(), Error> {
    if !needs_write(zip) {
        return Ok(());
    }
    let live = zip.a.slots.iter().filter(|s| !s.deleted).count();
    if live == 0 && !zip.keep_empty {
        // libzip deletes the file of an archive left empty.
        if std::path::Path::new(&zip.path).exists() {
            std::fs::remove_file(&zip.path).map_err(|e| Error::io(22, &e))?;
        }
        return Ok(());
    }
    let data = zip.a.build()?;
    let tmp = format!("{}.{:x}", zip.path, p8_runtime::os::process_id().unwrap_or(0));
    std::fs::write(&tmp, &data).map_err(|e| Error::io(11, &e))?;
    if let Ok(m) = std::fs::metadata(&zip.path) {
        let _ = std::fs::set_permissions(&tmp, m.permissions());
    }
    std::fs::rename(&tmp, &zip.path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(2, &e)
    })
}

fn close(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::close", args)?;
    require_open(ctx, this)?;
    Ok(Value::Bool(finish(ctx, this, "ZipArchive::close")?))
}

/// The object goes away: an open archive is written (a subclass with its own
/// __destruct sees it open first; the handle's Drop then writes it).
fn destruct(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    if this.class.method(b"__destruct").is_some() || handle(this).z.borrow().is_none() {
        return Ok(Value::Null);
    }
    let h = handle(this);
    let Some(mut zip) = h.z.borrow_mut().take() else { return Ok(Value::Null) };
    h.closing.set(true);
    let r = write_archive(ctx, &mut zip);
    h.closing.set(false);
    match r {
        Ok(Err(e)) => ctx.warn(&format!("ZipArchive::__destruct(): Cannot destroy the zip context: {}", e.message())),
        Err(thrown) => return Err(thrown),
        Ok(Ok(())) => {}
    }
    Ok(Value::Null)
}

fn set_password(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setPassword", args)?;
    let pw = bytes(args, 0);
    with_zip(ctx, this, |z, _| {
        if pw.is_empty() {
            return Ok(Value::Bool(false));
        }
        z.a.password = Some(pw);
        Ok(Value::Bool(true))
    })
}

fn count(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::count", args)?;
    with_zip(ctx, this, |z, _| Ok(Value::Int(z.a.count() as i64)))
}

fn get_status_string(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getStatusString", args)?;
    Ok(Value::str(error_of(this).message()))
}

fn clear_error(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::clearError", args)?;
    let h = handle(this);
    match h.z.borrow_mut().as_mut() {
        Some(z) => z.err = Error::default(),
        None => h.closed_err.set(Error::default()),
    }
    Ok(Value::Null)
}

// ---- adding ----


/// A name given with ZipArchive::FL_ENC_CP437, as the UTF-8 libzip stores.
fn encode_name(name: Vec<u8>, flags: i64) -> Vec<u8> {
    const CP437: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";
    if flags & FL_ENC_CP437 == 0 {
        return name;
    }
    let high: Vec<char> = CP437.chars().collect();
    name.iter().map(|&b| if b < 0x80 { char::from(b) } else { high[usize::from(b - 0x80)] }).collect::<String>().into_bytes()
}

/// zip_file_add: a new entry, or with FL_OVERWRITE a replacement.
fn add(z: &mut Zip, h: &Handle, name: Vec<u8>, source: Source, flags: i64) -> R {
    let name = encode_name(name, flags);
    let existing = locate(z, &name, 0);
    let r = if !modify(z) {
        None
    } else {
        match existing {
            Some(_) if flags & FL_OVERWRITE == 0 => {
                z.err = Error::new(archive::ER_EXISTS);
                None
            }
            at => Some(z.a.set_data(at, name, source)),
        }
    };
    match r {
        Some(i) => {
            h.last_id.set(i as i64);
            z.err = Error::default();
            Ok(Value::Bool(true))
        }
        None => {
            h.last_id.set(-1);
            Ok(Value::Bool(false))
        }
    }
}

fn add_empty_dir(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::addEmptyDir", args)?;
    let mut name = bytes(args, 0);
    let flags = opt_int(args, 1, 0);
    with_zip(ctx, this, |z, h| {
        if name.is_empty() {
            return Ok(Value::Bool(false));
        }
        if !name.ends_with(b"/") {
            name.push(b'/');
        }
        let r = add(z, h, name, Source::Memory(Rc::new(Vec::new())), flags)?;
        if matches!(r, Value::Bool(true)) {
            z.a.slots[h.last_id.get() as usize].cur.attr = 0x41FF_0000;
        }
        Ok(r)
    })
}

fn add_from_string(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::addFromString", args)?;
    let name = bytes(args, 0);
    let content = bytes(args, 1);
    let flags = opt_int(args, 2, FL_OVERWRITE);
    with_zip(ctx, this, |z, h| {
        let r = add(z, h, name, Source::Memory(Rc::new(content)), flags)?;
        if matches!(r, Value::Bool(true)) {
            z.a.slots[h.last_id.get() as usize].cur.attr = 0x81B6_0000;
        }
        Ok(r)
    })
}

/// The OS error text PHP shows ("No such file or directory").
fn os_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        std::io::ErrorKind::AlreadyExists => "File exists".into(),
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

/// php_zip_add_file's source: the file, read from `start` for `len`
/// (0: to its end). None: it can not be a source (a warning said so, or
/// libzip refuses the range).
fn file_source(ctx: &mut dyn Ctx, func: &str, path: &[u8], start: i64, len: i64, flags: i64) -> Option<Source> {
    let p = expand_path(&text(path));
    let Ok(meta) = std::fs::metadata(&p) else {
        ctx.warn(&format!("{func}(): No such file or directory"));
        return None;
    };
    if start < 0 || len < -2 || len == -1 {
        return None;
    }
    if meta.is_file() {
        let size = meta.len();
        if start as u64 > size || (len > 0 && start as u64 + len as u64 > size) {
            return None;
        }
    }
    let mut loaded = None;
    if flags & FL_OPEN_FILE_NOW != 0 {
        let data = std::fs::read(&p).ok()?;
        let from = (start as usize).min(data.len());
        let to = if len > 0 { from + len as usize } else { data.len() };
        loaded = Some(Rc::new(data[from..to.min(data.len())].to_vec()));
    }
    Some(Source::File { path: p, start: start as u64, len, loaded })
}

/// File attributes and time an added file carries.
fn file_attrs(z: &mut Zip, i: usize) {
    use p8_runtime::os::MetadataExt;
    let path = match &z.a.slots[i].cur.source {
        Source::File { path, .. } => path.clone(),
        _ => return,
    };
    if let Ok(m) = std::fs::metadata(&path) {
        z.a.slots[i].cur.attr = (m.mode() & 0xFFFF) << 16;
        if let Some(d) = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
            z.a.slots[i].cur.mtime = d.as_secs() as i64;
        }
    }
}

fn add_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::addFile", args)?;
    let path = bytes(args, 0);
    if path.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::addFile", 1, "filepath"));
    }
    if path.contains(&0) {
        return Err(value_error(ctx, "ZipArchive::addFile", 1, "filepath", "must not contain any null bytes"));
    }
    let mut name = bytes(args, 1);
    if name.is_empty() {
        name = path.clone();
    }
    let (start, len, flags) = (opt_int(args, 2, 0), opt_int(args, 3, 0), opt_int(args, 4, FL_OVERWRITE));
    let Some(source) = file_source(ctx, "ZipArchive::addFile", &path, start, len, flags) else { return Ok(Value::Bool(false)) };
    let h = handle(this);
    if h.closing.get() {
        return Err(ctx.throw("Error", "Already being closed".into()));
    }
    let mut z = h.z.borrow_mut();
    let Some(zip) = z.as_mut() else { return Ok(Value::Bool(false)) };
    let r = add(zip, h, name, source, flags & !FL_OPEN_FILE_NOW)?;
    if matches!(r, Value::Bool(true)) {
        file_attrs(zip, h.last_id.get() as usize);
    }
    Ok(r)
}

fn replace_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::replaceFile", args)?;
    let path = bytes(args, 0);
    if path.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::replaceFile", 1, "filepath"));
    }
    if path.contains(&0) {
        return Err(value_error(ctx, "ZipArchive::replaceFile", 1, "filepath", "must not contain any null bytes"));
    }
    let idx = int(args, 1);
    if idx < 0 {
        return Err(value_error(ctx, "ZipArchive::replaceFile", 2, "index", "must be greater than or equal to 0"));
    }
    let (start, len, flags) = (opt_int(args, 2, 0), opt_int(args, 3, 0), opt_int(args, 4, 0));
    let Some(source) = file_source(ctx, "ZipArchive::replaceFile", &path, start, len, flags) else { return Ok(Value::Bool(false)) };
    let h = handle(this);
    if h.closing.get() {
        return Err(ctx.throw("Error", "Already being closed".into()));
    }
    let mut z = h.z.borrow_mut();
    let Some(zip) = z.as_mut() else { return Ok(Value::Bool(false)) };
    let Some(i) = index(zip, idx).filter(|&i| !zip.a.slots[i].deleted) else { return fail(zip, archive::ER_INVAL) };
    if !modify(zip) {
        return Ok(Value::Bool(false));
    }
    let name = zip.a.slots[i].cur.name.clone();
    zip.a.set_data(Some(i), name, source);
    file_attrs(zip, i);
    zip.err = Error::default();
    Ok(Value::Bool(true))
}

/// The options of addGlob and addPattern (php_zip_parse_options).
struct Options {
    remove_all_path: bool,
    remove_path: Option<Vec<u8>>,
    add_path: Option<Vec<u8>>,
    flags: i64,
    comp_method: i64,
    comp_flags: i64,
}

fn parse_options(ctx: &mut dyn Ctx, func: &str, opts: &Value) -> Result<Options, Throw> {
    let mut o = Options { remove_all_path: false, remove_path: None, add_path: None, flags: FL_OVERWRITE, comp_method: -1, comp_flags: 0 };
    let Value::Array(a) = opts else { return Ok(o) };
    let get = |name: &str| a.get(&Key::from_string(name.into())).map(Value::deref);
    if let Some(v) = get("remove_all_path") {
        if !matches!(v, Value::Bool(_)) {
            ctx.warn(&format!("{func}(): Option \"remove_all_path\" must be of type bool, {} given", p8_runtime::args::value_name(&v)));
        }
        o.remove_all_path = v.to_int() != 0;
    }
    if let Some(v) = get("comp_method") {
        if !matches!(v, Value::Int(_)) {
            ctx.warn(&format!("{func}(): Option \"comp_method\" must be of type int, {} given", p8_runtime::args::value_name(&v)));
        }
        o.comp_method = v.to_int();
        if let Some(f) = get("comp_flags") {
            if !matches!(f, Value::Int(_)) {
                ctx.warn(&format!("{func}(): Option \"comp_flags\" must be of type int, {} given", p8_runtime::args::value_name(&f)));
            }
            o.comp_flags = f.to_int();
        }
    }
    for key in ["remove_path", "add_path"] {
        let Some(v) = get(key) else { continue };
        let Value::Str(s) = &v else {
            return Err(ctx.throw("TypeError", format!("Option \"{key}\" must be of type string, {} given", p8_runtime::args::value_name(&v))));
        };
        if s.is_empty() {
            return Err(ctx.throw("ValueError", format!("Option \"{key}\" must not be empty")));
        }
        if s.len() >= MAXPATHLEN {
            return Err(ctx.throw("ValueError", format!("Option \"{key}\" must be less than {} bytes", MAXPATHLEN - 1)));
        }
        if key == "remove_path" { o.remove_path = Some(s.as_bytes().to_vec()) } else { o.add_path = Some(s.as_bytes().to_vec()) }
    }
    if let Some(v) = get("flags") {
        if !matches!(v, Value::Int(_)) {
            return Err(ctx.throw("TypeError", format!("Option \"flags\" must be of type int, {} given", p8_runtime::args::value_name(&v))));
        }
        o.flags = v.to_int();
    }
    Ok(o)
}

/// Add each of `found` (php_zip_add_from_pattern's loop): the paths on
/// success, false when one cannot be added.
fn add_found(ctx: &mut dyn Ctx, this: &Rc<Object>, func: &str, found: Vec<Vec<u8>>, o: &Options) -> R {
    for file in &found {
        let stripped: Vec<u8> = if o.remove_all_path {
            archive::basename(file).to_vec()
        } else if let Some(rm) = o.remove_path.as_ref().filter(|rm| file.len() > rm.len() && file.starts_with(rm)) {
            let skip = if file[rm.len()] == b'/' { rm.len() + 1 } else { rm.len() };
            file[skip..].to_vec()
        } else {
            file.clone()
        };
        let mut name = stripped.clone();
        if let Some(add) = &o.add_path {
            if add.len() + stripped.len() > MAXPATHLEN {
                ctx.warn(&format!("{func}(): Entry name too long (max: {}, {} given)", MAXPATHLEN - 1, add.len() + stripped.len()));
                return Ok(Value::Bool(false));
            }
            name = [&add[..], &stripped[..]].concat();
        }
        name.truncate(MAXPATHLEN - 1);
        let Some(source) = file_source(ctx, func, file, 0, 0, o.flags) else { return Ok(Value::Bool(false)) };
        let h = handle(this);
        let mut z = h.z.borrow_mut();
        let Some(zip) = z.as_mut() else { return Ok(Value::Bool(false)) };
        let r = add(zip, h, name, source, o.flags & !FL_OPEN_FILE_NOW)?;
        if !matches!(r, Value::Bool(true)) {
            return Ok(r);
        }
        let i = h.last_id.get() as usize;
        file_attrs(zip, i);
        if o.comp_method >= 0 && !matches!(set_compression(zip, i as i64, o.comp_method, o.comp_flags)?, Value::Bool(true)) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Array(Rc::new(PhpArray::from_list(found.into_iter().map(string).collect()))))
}

fn add_glob(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::addGlob", args)?;
    let pattern = bytes(args, 0);
    if pattern.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::addGlob", 1, "pattern"));
    }
    let glob_flags = opt_int(args, 1, 0);
    let o = parse_options(ctx, "ZipArchive::addGlob", &arg(args, 2))?;
    if pattern.len() >= MAXPATHLEN {
        ctx.warn(&format!("ZipArchive::addGlob(): Pattern exceeds the maximum allowed length of {MAXPATHLEN} characters"));
        return Ok(Value::Bool(false));
    }
    let available = GLOB_BRACE | GLOB_MARK | GLOB_NOSORT | GLOB_NOCHECK | GLOB_NOESCAPE | GLOB_ERR | GLOB_ONLYDIR;
    if glob_flags & available != glob_flags {
        ctx.warn("ZipArchive::addGlob(): At least one of the passed flags is invalid or not supported on this platform");
        return Ok(Value::Bool(false));
    }
    let list = ctx.call(&Value::str("glob"), &mut [string(pattern), Value::Int(glob_flags & !GLOB_ONLYDIR)])?;
    let mut found: Vec<Vec<u8>> = match list {
        Value::Array(a) => a.values().filter_map(|v| v.to_php_string()).map(|s| s.as_bytes().to_vec()).collect(),
        _ => return Ok(Value::Array(Rc::new(PhpArray::new()))),
    };
    if glob_flags & GLOB_ONLYDIR != 0 {
        found.retain(|f| std::fs::metadata(text(f)).is_ok_and(|m| m.is_dir()));
    }
    if found.is_empty() {
        return Ok(Value::Array(Rc::new(PhpArray::new())));
    }
    add_found(ctx, this, "ZipArchive::addGlob", found, &o)
}

fn add_pattern(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::addPattern", args)?;
    let pattern = bytes(args, 0);
    if pattern.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::addPattern", 1, "pattern"));
    }
    let path = if args.len() > 1 { bytes(args, 1) } else { b".".to_vec() };
    let o = parse_options(ctx, "ZipArchive::addPattern", &arg(args, 2))?;
    let dir = text(&path);
    let listing = ctx.call(&Value::str("scandir"), &mut [Value::str(dir.clone())])?;
    let names: Vec<Vec<u8>> = match listing {
        Value::Array(a) => a.values().filter_map(|v| v.to_php_string()).map(|s| s.as_bytes().to_vec()).collect(),
        _ => return Ok(Value::Array(Rc::new(PhpArray::new()))),
    };
    if names.is_empty() {
        return Ok(Value::Array(Rc::new(PhpArray::new())));
    }
    let Some(re) = p8_ext_pcre::Pattern::compile(ctx, "ZipArchive::addPattern", &pattern) else {
        ctx.warn("ZipArchive::addPattern(): Invalid expression");
        return Ok(Value::Bool(false));
    };
    let mut found = Vec::new();
    for name in names {
        if name == b"." || name == b".." {
            continue;
        }
        if path.len() + name.len() + 1 >= MAXPATHLEN {
            ctx.warn(&format!("ZipArchive::addPattern(): add_path string too long (max: {}, {} given)", MAXPATHLEN - 1, path.len() + name.len() + 1));
            break;
        }
        if !re.is_match(ctx, &name) {
            continue;
        }
        let full = [&path[..], b"/", &name[..]].concat();
        match std::fs::metadata(text(&full)) {
            Err(_) => ctx.warn(&format!("ZipArchive::addPattern(): Cannot read <{}>", text(&full))),
            Ok(m) if m.is_dir() => {}
            Ok(_) => found.push(full),
        }
    }
    add_found(ctx, this, "ZipArchive::addPattern", found, &o)
}

// ---- renaming, comments, deleting ----

fn rename(z: &mut Zip, i: i64, new_name: Vec<u8>) -> R {
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    if z.a.slots[i].deleted {
        return fail(z, archive::ER_DELETED);
    }
    if new_name.is_empty() {
        return fail(z, archive::ER_INVAL);
    }
    if !modify(z) {
        return Ok(Value::Bool(false));
    }
    if let Some(j) = locate(z, &new_name, 0)
        && j != i {
            return fail(z, archive::ER_EXISTS);
        }
    z.a.slots[i].cur.name = new_name;
    ok(z)
}

fn rename_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::renameIndex", args)?;
    let (i, n) = (int(args, 0), bytes(args, 1));
    if i < 0 {
        return Ok(Value::Bool(false));
    }
    require_open(ctx, this)?;
    if n.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::renameIndex", 2, "new_name"));
    }
    with_zip(ctx, this, |z, _| rename(z, i, n))
}

fn rename_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::renameName", args)?;
    let (name, n) = (bytes(args, 0), bytes(args, 1));
    require_open(ctx, this)?;
    if n.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::renameName", 2, "new_name"));
    }
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::renameName", 1, "name"));
    }
    with_zip(ctx, this, |z, _| match locate(z, &name, 0) {
        Some(i) => rename(z, i as i64, n),
        None => fail(z, archive::ER_NOENT),
    })
}

fn set_archive_comment(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setArchiveComment", args)?;
    let c = bytes(args, 0);
    require_open(ctx, this)?;
    if c.len() > 65535 {
        return Err(value_error(ctx, "ZipArchive::setArchiveComment", 1, "comment", "must be less than 65535 bytes"));
    }
    with_zip(ctx, this, |z, _| {
        if !modify(z) {
            return Ok(Value::Bool(false));
        }
        z.a.comment = c;
        ok(z)
    })
}

fn get_archive_comment(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getArchiveComment", args)?;
    let flags = opt_int(args, 0, 0);
    with_zip(ctx, this, |z, _| Ok(string(if flags & FL_UNCHANGED != 0 { z.a.orig_comment.clone() } else { z.a.comment.clone() })))
}

fn set_archive_flag(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setArchiveFlag", args)?;
    let (flag, value) = (int(args, 0), int(args, 1));
    with_zip(ctx, this, |z, _| match flag {
        AFL_RDONLY => {
            if value != 0 && z.a.is_changed() {
                return fail(z, archive::ER_CHANGED);
            }
            if value == 0 && z.opened_rdonly {
                return fail(z, archive::ER_RDONLY);
            }
            z.a.readonly = value != 0;
            ok(z)
        }
        AFL_WANT_TORRENTZIP => {
            z.a.want_torrent = value != 0;
            ok(z)
        }
        AFL_CREATE_OR_KEEP_FILE_FOR_EMPTY_ARCHIVE => {
            z.keep_empty = value != 0;
            ok(z)
        }
        _ => fail(z, archive::ER_INVAL),
    })
}

fn get_archive_flag(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getArchiveFlag", args)?;
    let flag = int(args, 0);
    with_zip(ctx, this, |z, _| {
        Ok(Value::Int(match flag {
            AFL_RDONLY => i64::from(z.a.readonly),
            AFL_IS_TORRENTZIP => i64::from(z.a.torrent),
            AFL_WANT_TORRENTZIP => i64::from(z.a.want_torrent),
            AFL_CREATE_OR_KEEP_FILE_FOR_EMPTY_ARCHIVE => i64::from(z.keep_empty),
            _ => 0,
        }))
    })
}

fn set_comment(z: &mut Zip, i: i64, comment: Vec<u8>) -> R {
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    if z.a.slots[i].deleted {
        return fail(z, archive::ER_DELETED);
    }
    if !modify(z) {
        return Ok(Value::Bool(false));
    }
    z.a.slots[i].cur.comment = comment;
    ok(z)
}

fn set_comment_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setCommentIndex", args)?;
    let (i, c) = (int(args, 0), bytes(args, 1));
    require_open(ctx, this)?;
    if c.len() > 65535 {
        return Err(value_error(ctx, "ZipArchive::setCommentIndex", 2, "comment", "must be less than 65535 bytes"));
    }
    with_zip(ctx, this, |z, _| set_comment(z, i, c))
}

fn set_comment_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setCommentName", args)?;
    let (name, c) = (bytes(args, 0), bytes(args, 1));
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::setCommentName", 1, "name"));
    }
    require_open(ctx, this)?;
    if c.len() > 65535 {
        return Err(value_error(ctx, "ZipArchive::setCommentName", 2, "comment", "must be less than 65535 bytes"));
    }
    with_zip(ctx, this, |z, _| match locate(z, &name, 0) {
        Some(i) => set_comment(z, i as i64, c),
        None => Ok(Value::Bool(false)),
    })
}

fn get_comment(z: &mut Zip, i: i64, flags: i64) -> R {
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    let s = &z.a.slots[i];
    let rec = if flags & FL_UNCHANGED != 0 { s.orig.as_ref() } else if s.deleted { None } else { Some(&s.cur) };
    match rec {
        Some(r) => Ok(string(r.comment.clone())),
        None => fail(z, archive::ER_INVAL),
    }
}

fn get_comment_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getCommentIndex", args)?;
    let (i, flags) = (int(args, 0), opt_int(args, 1, 0));
    with_zip(ctx, this, |z, _| get_comment(z, i, flags))
}

fn get_comment_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getCommentName", args)?;
    let (name, flags) = (bytes(args, 0), opt_int(args, 1, 0));
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::getCommentName", 1, "name"));
    }
    with_zip(ctx, this, |z, _| match locate(z, &name, flags) {
        Some(i) => get_comment(z, i as i64, flags),
        None => fail(z, archive::ER_NOENT),
    })
}

fn set_mtime(z: &mut Zip, i: i64, t: i64) -> R {
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    if z.a.slots[i].deleted {
        return fail(z, archive::ER_DELETED);
    }
    if !modify(z) {
        return Ok(Value::Bool(false));
    }
    z.a.slots[i].cur.mtime = t;
    ok(z)
}

fn set_mtime_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setMtimeIndex", args)?;
    let (i, t) = (int(args, 0), int(args, 1));
    with_zip(ctx, this, |z, _| set_mtime(z, i, t))
}

fn set_mtime_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setMtimeName", args)?;
    let (name, t) = (bytes(args, 0), int(args, 1));
    require_open(ctx, this)?;
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::setMtimeName", 1, "name"));
    }
    with_zip(ctx, this, |z, _| match locate(z, &name, 0) {
        Some(i) => set_mtime(z, i as i64, t),
        None => Ok(Value::Bool(false)),
    })
}

fn delete(z: &mut Zip, i: i64) -> R {
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    if z.a.slots[i].deleted {
        return fail(z, archive::ER_DELETED);
    }
    if !modify(z) {
        return Ok(Value::Bool(false));
    }
    z.a.slots[i].deleted = true;
    ok(z)
}

fn delete_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::deleteIndex", args)?;
    let i = int(args, 0);
    with_zip(ctx, this, |z, _| delete(z, i))
}

fn delete_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::deleteName", args)?;
    let name = bytes(args, 0);
    with_zip(ctx, this, |z, _| {
        if name.is_empty() {
            return Ok(Value::Bool(false));
        }
        match locate(z, &name, 0) {
            Some(i) => delete(z, i as i64),
            None => fail(z, archive::ER_NOENT),
        }
    })
}

// ---- stat, locate, names ----

fn stat_array(st: archive::Stat) -> Value {
    let mut a = PhpArray::new();
    let mut set = |k: &str, v: Value| a.insert(Key::from_string(k.into()), v);
    set("name", string(st.name));
    set("index", Value::Int(st.index as i64));
    set("crc", Value::Int(i64::from(st.crc)));
    set("size", Value::Int(st.size as i64));
    set("mtime", Value::Int(st.mtime));
    set("comp_size", Value::Int(st.comp_size));
    set("comp_method", Value::Int(i64::from(st.comp_method)));
    set("encryption_method", Value::Int(i64::from(st.encryption)));
    Value::Array(Rc::new(a))
}

fn stat(z: &mut Zip, i: i64, flags: i64) -> R {
    let unchanged = flags & FL_UNCHANGED != 0;
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    match z.a.stat(i, unchanged) {
        Ok(st) => Ok(stat_array(st)),
        Err(_) => fail(z, archive::ER_INVAL),
    }
}

fn stat_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::statIndex", args)?;
    let (i, flags) = (int(args, 0), opt_int(args, 1, 0));
    with_zip(ctx, this, |z, _| stat(z, i, flags))
}

fn stat_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::statName", args)?;
    let (name, flags) = (bytes(args, 0), opt_int(args, 1, 0));
    require_open(ctx, this)?;
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::statName", 1, "name"));
    }
    with_zip(ctx, this, |z, _| match locate(z, &name, flags) {
        Some(i) => stat(z, i as i64, flags),
        None => fail(z, archive::ER_NOENT),
    })
}

fn locate_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::locateName", args)?;
    let (name, flags) = (bytes(args, 0), opt_int(args, 1, 0));
    with_zip(ctx, this, |z, _| {
        if name.is_empty() {
            return Ok(Value::Bool(false));
        }
        match locate(z, &name, flags) {
            Some(i) => Ok(Value::Int(i as i64)),
            None => fail(z, archive::ER_NOENT),
        }
    })
}

fn get_name_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getNameIndex", args)?;
    let (i, flags) = (int(args, 0), opt_int(args, 1, 0));
    with_zip(ctx, this, |z, _| {
        let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
        match z.a.name(i, flags & FL_UNCHANGED != 0) {
            Ok(n) => Ok(string(n.to_vec())),
            Err(_) => fail(z, archive::ER_INVAL),
        }
    })
}

// ---- unchange ----

fn unchange_archive(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::unchangeArchive", args)?;
    with_zip(ctx, this, |z, _| {
        z.a.comment = z.a.orig_comment.clone();
        ok(z)
    })
}

fn unchange_all(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::unchangeAll", args)?;
    with_zip(ctx, this, |z, _| {
        for i in 0..z.a.count() {
            z.a.unchange(i);
        }
        z.a.comment = z.a.orig_comment.clone();
        ok(z)
    })
}

fn unchange(z: &mut Zip, i: i64) -> R {
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    // Undoing a rename must not clash with another entry's name.
    if let Some(o) = &z.a.slots[i].orig
        && let Some(j) = locate(z, &o.name.clone(), 0)
            && j != i {
                return fail(z, archive::ER_EXISTS);
            }
    z.a.unchange(i);
    ok(z)
}

fn unchange_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::unchangeIndex", args)?;
    let i = int(args, 0);
    require_open(ctx, this)?;
    if i < 0 {
        return Ok(Value::Bool(false));
    }
    with_zip(ctx, this, |z, _| unchange(z, i))
}

fn unchange_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::unchangeName", args)?;
    let name = bytes(args, 0);
    with_zip(ctx, this, |z, _| {
        if name.is_empty() {
            return Ok(Value::Bool(false));
        }
        match locate(z, &name, 0) {
            Some(i) => unchange(z, i as i64),
            None => fail(z, archive::ER_NOENT),
        }
    })
}

// ---- reading ----

/// Whether a failed read is zip_fopen's (silent) or zip_fread's (a warning).
fn silent_read_error(e: Error) -> bool {
    matches!(e.code, archive::ER_CHANGED | archive::ER_NOPASSWD | archive::ER_ENCRNOTSUPP | archive::ER_COMPNOTSUPP | archive::ER_INVAL | archive::ER_DELETED | archive::ER_NOENT)
}

fn read_entry(z: &mut Zip, i: usize, flags: i64, len: i64, warns: &mut Vec<String>, func: &str) -> R {
    let unchanged = flags & FL_UNCHANGED != 0;
    let r = if flags & FL_COMPRESSED != 0 { z.a.read_compressed(i, unchanged) } else { z.a.read(i, unchanged) };
    match r {
        Ok(mut data) => {
            if len > 0 && (len as usize) < data.len() {
                data.truncate(len as usize);
            }
            Ok(string(data))
        }
        Err(e) => {
            z.err = e;
            if !silent_read_error(e) {
                warns.push(format!("{func}(): Cannot read entry: {}", e.message()));
            }
            Ok(Value::Bool(false))
        }
    }
}

fn get_from_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getFromName", args)?;
    let (name, len, flags) = (bytes(args, 0), opt_int(args, 1, 0), opt_int(args, 2, 0));
    require_open(ctx, this)?;
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::getFromName", 1, "name"));
    }
    let mut warns = Vec::new();
    let r = with_zip(ctx, this, |z, _| match locate(z, &name, flags) {
        Some(i) => read_entry(z, i, flags, len, &mut warns, "ZipArchive::getFromName"),
        None => fail(z, archive::ER_NOENT),
    });
    warn_all(ctx, warns);
    r
}

fn get_from_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getFromIndex", args)?;
    let (i, len, flags) = (int(args, 0), opt_int(args, 1, 0), opt_int(args, 2, 0));
    let mut warns = Vec::new();
    let r = with_zip(ctx, this, |z, _| match index(z, i) {
        Some(i) => read_entry(z, i, flags, len, &mut warns, "ZipArchive::getFromIndex"),
        None => fail(z, archive::ER_INVAL),
    });
    warn_all(ctx, warns);
    r
}

fn entry_stream(ctx: &mut dyn Ctx, data: Vec<u8>, uri: &str, mode: &str) -> Value {
    let stream = Stream::new(Target::Memory(data), uri, mode);
    ctx.new_resource(Resource { kind: "stream", closed: Cell::new(false), stream: RefCell::new(stream) })
}

fn open_stream(ctx: &mut dyn Ctx, this: &Rc<Object>, found: Result<usize, i32>, flags: i64) -> R {
    let got = with_zip(ctx, this, |z, _| {
        let i = match found {
            Ok(i) => i,
            Err(code) => return fail(z, code),
        };
        let unchanged = flags & FL_UNCHANGED != 0;
        let name = z.a.name(i, unchanged).map(<[u8]>::to_vec).unwrap_or_default();
        match z.a.read(i, unchanged) {
            Ok(d) => Ok(Value::Array(Rc::new(PhpArray::from_list(vec![string(d), string(name)])))),
            Err(e) => {
                z.err = e;
                Ok(Value::Bool(false))
            }
        }
    })?;
    Ok(match got {
        Value::Array(a) => {
            let data = a.get_int(0).and_then(|v| v.to_php_string()).map(|s| s.as_bytes().to_vec()).unwrap_or_default();
            let name = a.get_int(1).and_then(|v| v.to_php_string()).map(|s| text(s.as_bytes())).unwrap_or_default();
            entry_stream(ctx, data, &name, "rb")
        }
        other => other,
    })
}

fn get_stream_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getStreamIndex", args)?;
    let (i, flags) = (int(args, 0), opt_int(args, 1, 0));
    require_open(ctx, this)?;
    let found = match &*handle(this).z.borrow() {
        Some(z) => index(z, i).ok_or(archive::ER_INVAL),
        None => Err(archive::ER_INVAL),
    };
    open_stream(ctx, this, found, flags)
}

fn get_stream_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getStreamName", args)?;
    let (name, flags) = (bytes(args, 0), opt_int(args, 1, 0));
    stream_by_name(ctx, this, "ZipArchive::getStreamName", name, flags)
}

fn get_stream(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getStream", args)?;
    let name = bytes(args, 0);
    stream_by_name(ctx, this, "ZipArchive::getStream", name, 0)
}

fn stream_by_name(ctx: &mut dyn Ctx, this: &Rc<Object>, func: &str, name: Vec<u8>, flags: i64) -> R {
    if name.is_empty() {
        return Err(not_empty(ctx, func, 1, "name"));
    }
    require_open(ctx, this)?;
    let found = match &*handle(this).z.borrow() {
        Some(z) => locate(z, &name, flags).ok_or(archive::ER_NOENT),
        None => Err(archive::ER_NOENT),
    };
    open_stream(ctx, this, found, flags)
}

/// fopen("zip:///path/archive.zip#entry"): the entry's bytes.
fn open_wrapper(ctx: &mut dyn Ctx, func: &str, url: &str, mode: &str) -> Option<Stream> {
    let fail = |ctx: &mut dyn Ctx| {
        ctx.warn(&format!("{func}({url}): Failed to open stream: operation failed"));
        None
    };
    let rest = &url[url.find("://").map_or(0, |i| i + 3)..];
    let Some((path, entry)) = rest.split_once('#') else { return fail(ctx) };
    if !mode.starts_with('r') {
        return fail(ctx);
    }
    let Ok(zip) = open_archive(ctx, func, path, RDONLY) else { return fail(ctx) };
    let Some(i) = zip.a.locate(entry.as_bytes(), false, false, false) else { return fail(ctx) };
    match zip.a.read(i, false) {
        Ok(d) => Some(Stream::new(Target::Memory(d), url, mode)),
        Err(_) => fail(ctx),
    }
}

// ---- extracting ----

/// php_zip_extract_file's path: the entry name made relative to the
/// destination (`..` that would climb out are dropped, a leading `/` too).
/// None: nothing to extract.
fn clean_entry_path(name: &[u8]) -> Option<String> {
    let name = text(name);
    let mut parts: Vec<&str> = Vec::new();
    for p in name.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut out = parts.join("/");
    if name.ends_with('/') {
        out.push('/');
    }
    Some(out)
}

fn extract_file(z: &mut Zip, dest: &str, name: &[u8], idx: Option<usize>, warns: &mut Vec<String>) -> bool {
    let Some(i) = idx.or_else(|| z.a.locate(name, false, false, false)) else { return false };
    let Some(cleaned) = clean_entry_path(name) else { return false };
    if cleaned.len() >= MAXPATHLEN || z.a.stat(i, false).is_err() {
        return false;
    }
    let dest = dest.trim_end_matches('/');
    if cleaned.len() > 1 && cleaned.ends_with('/') {
        let dir = format!("{dest}/{cleaned}");
        return std::path::Path::new(&dir).is_dir() || std::fs::create_dir_all(&dir).is_ok();
    }
    let (dir_part, base) = match cleaned.rfind('/') {
        Some(p) => (Some(&cleaned[..p]), &cleaned[p + 1..]),
        None => (None, &cleaned[..]),
    };
    let dir = match dir_part {
        Some(d) => format!("{dest}/{d}"),
        None => dest.to_string(),
    };
    if !std::path::Path::new(&dir).exists()
        && let Err(e) = std::fs::create_dir_all(&dir) {
            warns.push(format!("ZipArchive::extractTo(): {}", os_error(&e)));
            return false;
        }
    let full = format!("{dir}/{base}");
    if full.len() > MAXPATHLEN {
        warns.push(format!("ZipArchive::extractTo(): Full extraction path exceed MAXPATHLEN ({MAXPATHLEN})"));
        return false;
    }
    let data = match z.a.read(i, false) {
        Ok(d) => d,
        Err(e) => {
            z.err = e;
            if !silent_read_error(e) {
                warns.push(format!("ZipArchive::extractTo(): Cannot extract \"{}\": \"{}\"", text(name), e.message()));
            }
            return false;
        }
    };
    let mtime = z.a.stat(i, false).map(|s| s.mtime).unwrap_or(0);
    match std::fs::File::create(&full) {
        Ok(mut f) => {
            use std::io::Write;
            if f.write_all(&data).is_err() {
                return false;
            }
            if let Some(t) = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(mtime.max(0) as u64)) {
                let _ = f.set_modified(t);
            }
            true
        }
        Err(e) => {
            warns.push(format!("ZipArchive::extractTo({full}): Failed to open stream: {}", os_error(&e)));
            false
        }
    }
}

fn extract_to(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::extractTo", args)?;
    let dest = bytes(args, 0);
    let files = arg(args, 1);
    let mut warns = Vec::new();
    let r = with_zip(ctx, this, |z, _| {
        if dest.is_empty() {
            return Ok(Value::Bool(false));
        }
        let dest = text(&dest);
        if !std::path::Path::new(&dest).exists() && std::fs::create_dir_all(&dest).is_err() {
            return Ok(Value::Bool(false));
        }
        match &files {
            Value::Str(s) => {
                if !extract_file(z, &dest, s.as_bytes(), None, &mut warns) {
                    return Ok(Value::Bool(false));
                }
            }
            Value::Array(a) => {
                if a.is_empty() {
                    return Ok(Value::Bool(false));
                }
                for v in a.values() {
                    if let Value::Str(s) = v.deref()
                        && !extract_file(z, &dest, s.as_bytes(), None, &mut warns) {
                            return Ok(Value::Bool(false));
                        }
                }
            }
            _ => {
                for i in 0..z.a.count() {
                    let Ok(name) = z.a.name(i, true).map(<[u8]>::to_vec) else { return Ok(Value::Bool(false)) };
                    if !extract_file(z, &dest, &name, Some(i), &mut warns) {
                        return Ok(Value::Bool(false));
                    }
                }
            }
        }
        Ok(Value::Bool(true))
    });
    warn_all(ctx, warns);
    r
}

// ---- attributes and compression ----

fn set_ext_attr(z: &mut Zip, i: i64, flags: i64, opsys: i64, attr: i64) -> R {
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    if z.a.slots[i].deleted || flags != 0 {
        return fail(z, archive::ER_INVAL);
    }
    if !modify(z) {
        return Ok(Value::Bool(false));
    }
    let r = &mut z.a.slots[i].cur;
    r.opsys = (opsys & 0xff) as u8;
    r.attr = attr as u32;
    ok(z)
}

fn set_ext_attr_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setExternalAttributesIndex", args)?;
    let (i, o, a, f) = (int(args, 0), int(args, 1), int(args, 2), opt_int(args, 3, 0));
    with_zip(ctx, this, |z, _| match index(z, i) {
        Some(_) => set_ext_attr(z, i, f, o, a),
        None => fail(z, archive::ER_INVAL),
    })
}

fn set_ext_attr_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setExternalAttributesName", args)?;
    let (name, o, a, f) = (bytes(args, 0), int(args, 1), int(args, 2), opt_int(args, 3, 0));
    require_open(ctx, this)?;
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::setExternalAttributesName", 1, "name"));
    }
    with_zip(ctx, this, |z, _| match locate(z, &name, 0) {
        Some(i) => set_ext_attr(z, i as i64, f, o, a),
        None => Ok(Value::Bool(false)),
    })
}

/// Store through a by-reference argument.
fn set_ref(args: &[Value], i: usize, v: Value) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = v;
    }
}

fn get_ext_attr(z: &mut Zip, args: &[Value], i: i64, flags: i64) -> R {
    let unchanged = flags & FL_UNCHANGED != 0;
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    let s = &z.a.slots[i];
    let rec = if unchanged { s.orig.as_ref() } else if s.deleted { None } else { Some(&s.cur) };
    let Some(r) = rec else { return fail(z, archive::ER_INVAL) };
    set_ref(args, 1, Value::Int(i64::from(r.opsys)));
    set_ref(args, 2, Value::Int(i64::from(r.attr)));
    ok(z)
}

fn get_ext_attr_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getExternalAttributesIndex", args)?;
    let (i, flags) = (int(args, 0), opt_int(args, 3, 0));
    let args = &*args;
    with_zip(ctx, this, |z, _| get_ext_attr(z, args, i, flags))
}

fn get_ext_attr_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::getExternalAttributesName", args)?;
    let (name, flags) = (bytes(args, 0), opt_int(args, 3, 0));
    require_open(ctx, this)?;
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::getExternalAttributesName", 1, "name"));
    }
    let args = &*args;
    with_zip(ctx, this, |z, _| match locate(z, &name, 0) {
        Some(i) => get_ext_attr(z, args, i as i64, flags),
        None => Ok(Value::Bool(false)),
    })
}

fn set_compression(z: &mut Zip, i: i64, method: i64, level: i64) -> R {
    let Some(i) = index(z, i) else { return fail(z, archive::ER_INVAL) };
    if z.a.slots[i].deleted {
        return fail(z, archive::ER_INVAL);
    }
    if !matches!(method, -1 | 0 | 8) {
        let known = matches!(method, 1..=6 | 9 | 10 | 12 | 14 | 18 | 19 | 33 | 93 | 95 | 97 | 98);
        return fail(z, if known { archive::ER_COMPNOTSUPP } else { archive::ER_INVAL });
    }
    if !modify(z) {
        return Ok(Value::Bool(false));
    }
    let r = &mut z.a.slots[i].cur;
    let raw = matches!(r.source, Source::Raw(..));
    // Asking for what the stored bytes already are changes nothing.
    r.recompress = if raw { !(r.src_method == method as i32 && level == 0) } else { false };
    r.method = method as i32;
    r.level = level.clamp(0, 9) as u32;
    ok(z)
}

fn set_compression_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setCompressionIndex", args)?;
    let (i, m, l) = (int(args, 0), int(args, 1), opt_int(args, 2, 0));
    with_zip(ctx, this, |z, _| set_compression(z, i, m, l))
}

fn set_compression_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setCompressionName", args)?;
    let (name, m, l) = (bytes(args, 0), int(args, 1), opt_int(args, 2, 0));
    require_open(ctx, this)?;
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::setCompressionName", 1, "name"));
    }
    with_zip(ctx, this, |z, _| match locate(z, &name, 0) {
        Some(i) => set_compression(z, i as i64, m, l),
        None => Ok(Value::Bool(false)),
    })
}

fn set_encryption_index(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setEncryptionIndex", args)?;
    let (i, m) = (int(args, 0), int(args, 1));
    with_zip(ctx, this, |z, _| match index(z, i) {
        None => fail(z, archive::ER_INVAL),
        // No encryption is the one method that needs nothing.
        Some(_) if m == 0 => ok(z),
        Some(_) => fail(z, archive::ER_ENCRNOTSUPP),
    })
}

fn set_encryption_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::setEncryptionName", args)?;
    let (name, m) = (bytes(args, 0), int(args, 1));
    require_open(ctx, this)?;
    if name.is_empty() {
        return Err(not_empty(ctx, "ZipArchive::setEncryptionName", 1, "name"));
    }
    with_zip(ctx, this, |z, _| match locate(z, &name, 0) {
        None => Ok(Value::Bool(false)),
        Some(_) if m == 0 => ok(z),
        Some(_) => fail(z, archive::ER_ENCRNOTSUPP),
    })
}

fn register_progress_callback(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::registerProgressCallback", args)?;
    let (rate, cb) = (arg(args, 0).to_float(), arg(args, 1));
    with_zip(ctx, this, |z, _| {
        z.progress = Some((rate, cb));
        Ok(Value::Bool(true))
    })
}

fn register_cancel_callback(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::registerCancelCallback", args)?;
    let cb = arg(args, 0);
    with_zip(ctx, this, |z, _| {
        z.cancel = Some(cb);
        Ok(Value::Bool(true))
    })
}

fn is_compression_supported(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::isCompressionMethodSupported", args)?;
    Ok(Value::Bool(matches!(int(args, 0), 0 | 8)))
}

fn is_encryption_supported(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    prep(ctx, "ZipArchive::isEncryptionMethodSupported", args)?;
    Ok(Value::Bool(int(args, 0) == 0))
}

//! SplFileInfo and the directory iterators: DirectoryIterator,
//! FilesystemIterator, RecursiveDirectoryIterator and GlobIterator.
//!
//! An object holds a file name and its path (SplFileInfo), or a listing
//! and the entry it is on (the iterators). Directories list as p8's
//! opendir() does: "." and "..", then the names in the system's order. A glob lists
//! the matches' names, with the pattern's directory as the path.
//!
//! Reference: $PHP_SRC_PATH/ext/spl/spl_directory.c

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::{Class, Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use p8_runtime::os::MetadataExt;
use std::rc::Rc;

type R = Result<Value, Throw>;

const CURRENT_MODE_MASK: i64 = 0xF0;
const CURRENT_AS_PATHNAME: i64 = 0x20;
const CURRENT_AS_FILEINFO: i64 = 0;
const CURRENT_AS_SELF: i64 = 0x10;
const KEY_MODE_MASK: i64 = 0xF00;
const KEY_AS_PATHNAME: i64 = 0;
const KEY_AS_FILENAME: i64 = 0x100;
const FOLLOW_SYMLINKS: i64 = 0x4000;
const SKIP_DOTS: i64 = 0x1000;
const UNIX_PATHS: i64 = 0x2000;
const OTHER_MODE_MASK: i64 = 0x7000;

/// One object's state (spl_filesystem_object).
#[derive(Default)]
pub struct Fs(RefCell<State>);

#[derive(Default)]
struct State {
    /// The full name (an info's, or the directory entry's once asked for).
    file_name: Option<Vec<u8>>,
    path: Option<Vec<u8>>,
    dir: Option<Dir>,
    flags: i64,
    info_class: Option<Rc<Class>>,
}

/// A listing being iterated.
struct Dir {
    /// The directory as opened (a glob's pattern, for re-listing).
    source: Vec<u8>,
    glob: bool,
    entries: Vec<Vec<u8>>,
    /// The entry the iterator is on (entries.len(): none left).
    at: usize,
    /// key(): how many moves since the start.
    index: i64,
    sub_path: Option<Vec<u8>>,
}

pub fn make() -> Rc<dyn std::any::Any> {
    Rc::new(Fs::default())
}

fn state(o: &Object) -> &RefCell<State> {
    &o.native::<Fs>().expect("SplFileInfo state").0
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn bytes(v: &Value) -> Vec<u8> {
    v.to_php_string().map_or_else(Vec::new, |p| p.as_bytes().to_vec())
}

fn string(b: impl Into<Vec<u8>>) -> Value {
    Value::str(PhpStr::new(b.into()))
}

fn is_dot(name: &[u8]) -> bool {
    name == b"." || name == b".."
}

impl Dir {
    fn entry(&self) -> &[u8] {
        self.entries.get(self.at).map_or(&[][..], Vec::as_slice)
    }
}

impl State {
    /// The entry's name in a directory (empty when done or not one).
    fn entry(&self) -> Vec<u8> {
        self.dir.as_ref().map_or_else(Vec::new, |d| d.entry().to_vec())
    }

    /// spl_filesystem_object_get_file_name(): an entry's is its path and
    /// name (the name alone without a path).
    fn file_name(&mut self) -> Option<Vec<u8>> {
        if self.file_name.is_none()
            && let Some(d) = &self.dir
        {
            let name = d.entry().to_vec();
            self.file_name = Some(match &self.path {
                Some(p) if !p.is_empty() => [p.as_slice(), b"/", &name].concat(),
                _ => name,
            });
        }
        self.file_name.clone()
    }

    /// spl_filesystem_object_get_pathname(): none for a finished listing.
    fn pathname(&mut self) -> Option<Vec<u8>> {
        match &self.dir {
            Some(d) if d.entry().is_empty() => None,
            _ => self.file_name(),
        }
    }

    /// The name without the path (getFilename()).
    fn short_name(&mut self) -> Option<Vec<u8>> {
        let name = self.file_name.clone()?;
        Some(match &self.path {
            Some(p) if !p.is_empty() && p.len() < name.len() => name[p.len() + 1..].to_vec(),
            _ => name,
        })
    }

    /// Move on (spl_filesystem_dir_read), past dots when told to.
    fn read(&mut self) {
        let skip = self.flags & SKIP_DOTS != 0;
        self.file_name = None;
        if let Some(d) = &mut self.dir {
            loop {
                d.at = (d.at + 1).min(d.entries.len());
                if !(skip && is_dot(d.entry())) {
                    break;
                }
            }
        }
    }

    /// Back to the first entry (rewinddir: the directory listed again).
    fn rewind(&mut self) {
        let skip = self.flags & SKIP_DOTS != 0;
        self.file_name = None;
        if let Some(d) = &mut self.dir {
            if let Ok(entries) = list(&d.source, d.glob) {
                d.entries = entries;
            }
            d.index = 0;
            d.at = 0;
            while skip && is_dot(d.entry()) {
                d.at += 1;
            }
        }
    }
}

/// spl_filesystem_info_set_filename(): the name without trailing slashes,
/// the path up to its last slash.
fn set_filename(st: &mut State, path: &[u8]) {
    let mut len = path.len();
    let file_name = if len > 1 && path[len - 1] == b'/' {
        while len > 1 && path[len - 1] == b'/' {
            len -= 1;
        }
        path[..len].to_vec()
    } else {
        path.to_vec()
    };
    while len > 1 && path[len - 1] != b'/' {
        len -= 1;
    }
    len = len.saturating_sub(1);
    st.file_name = Some(file_name);
    st.path = Some(path[..len].to_vec());
}

/// A directory's entries, or a glob's matches' names.
fn list(source: &[u8], glob: bool) -> std::io::Result<Vec<Vec<u8>>> {
    if glob {
        let pattern = s(source);
        let pattern = pattern.strip_prefix("glob://").unwrap_or(&pattern);
        return Ok(crate::file::glob_paths(pattern, 0)
            .into_iter()
            .map(|p| p.rsplit('/').next().unwrap_or(&p).as_bytes().to_vec())
            .collect());
    }
    crate::file::entries(&s(source))
}

/// The directory a glob's matches are in (php_glob_stream_get_path): the
/// pattern up to its last slash.
fn glob_dir(pattern: &[u8]) -> Option<Vec<u8>> {
    let p = pattern.strip_prefix(b"glob://").unwrap_or(pattern);
    p.iter().rposition(|&c| c == b'/').map(|i| p[..i.max(1)].to_vec())
}

// ---- SplFileInfo ----

fn info_construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    set_filename(&mut state(this).borrow_mut(), &bytes(&arg(args, 0)));
    Ok(Value::NULL)
}

fn not_initialized(ctx: &mut dyn Ctx) -> Throw {
    ctx.throw("Error", "Object not initialized".into())
}

fn get_path(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let st = state(this).borrow();
    Ok(string(st.path.clone().unwrap_or_default()))
}

fn get_filename(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let mut st = state(this).borrow_mut();
    if st.dir.is_some() {
        return Ok(string(st.entry()));
    }
    match st.short_name() {
        Some(n) => Ok(string(n)),
        None => {
            drop(st);
            Err(not_initialized(ctx))
        }
    }
}

/// php_basename(): the last path component, less `suffix`.
fn basename(name: &[u8], suffix: &[u8]) -> Vec<u8> {
    let trimmed = {
        let mut end = name.len();
        while end > 0 && name[end - 1] == b'/' {
            end -= 1;
        }
        &name[..end]
    };
    let start = trimmed.iter().rposition(|&c| c == b'/').map_or(0, |i| i + 1);
    let base = &trimmed[start..];
    match base.strip_suffix(suffix) {
        Some(b) if !suffix.is_empty() && !b.is_empty() => b.to_vec(),
        _ => base.to_vec(),
    }
}

fn extension(name: &[u8]) -> Vec<u8> {
    let b = basename(name, b"");
    b.iter().rposition(|&c| c == b'.').map_or_else(Vec::new, |i| b[i + 1..].to_vec())
}

/// The name getExtension() and getBasename() work on.
fn short_or_entry(ctx: &mut dyn Ctx, this: &Object) -> Result<Vec<u8>, Throw> {
    let mut st = state(this).borrow_mut();
    if st.dir.is_some() {
        return Ok(st.entry());
    }
    match st.short_name() {
        Some(n) => Ok(n),
        None => {
            drop(st);
            Err(not_initialized(ctx))
        }
    }
}

fn get_extension(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(string(extension(&short_or_entry(ctx, this)?)))
}

fn get_basename(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let suffix = bytes(&arg(args, 0));
    Ok(string(basename(&short_or_entry(ctx, this)?, &suffix)))
}

fn get_pathname(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(string(state(this).borrow_mut().pathname().unwrap_or_default()))
}

/// The file name a stat is about, or "Object not initialized".
fn stat_name(ctx: &mut dyn Ctx, this: &Object) -> Result<String, Throw> {
    match state(this).borrow_mut().file_name() {
        Some(n) => Ok(s(&n)),
        None => Err(not_initialized(ctx)),
    }
}

/// A stat for one of the getters: its failure is a RuntimeException
/// (php_stat's warning, thrown).
fn stat_for(ctx: &mut dyn Ctx, this: &Object, method: &str, link: bool) -> Result<std::fs::Metadata, Throw> {
    let name = stat_name(ctx, this)?;
    let m = if link { std::fs::symlink_metadata(&name) } else { std::fs::metadata(&name) };
    m.map_err(|_| {
        let what = if link { "Lstat" } else { "stat" };
        ctx.throw("RuntimeException", format!("SplFileInfo::{method}(): {what} failed for {name}"))
    })
}

macro_rules! stat_getter {
    ($fn:ident, $name:literal, $link:expr, |$m:ident| $e:expr) => {
        fn $fn(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
            let $m = stat_for(ctx, this, $name, $link)?;
            Ok($e)
        }
    };
}

stat_getter!(get_perms, "getPerms", false, |m| Value::int(m.mode() as i64));
stat_getter!(get_inode, "getInode", false, |m| Value::int(m.ino() as i64));
stat_getter!(get_size, "getSize", false, |m| Value::int(m.len() as i64));
stat_getter!(get_owner, "getOwner", false, |m| Value::int(m.uid() as i64));
stat_getter!(get_group, "getGroup", false, |m| Value::int(m.gid() as i64));
stat_getter!(get_atime, "getATime", false, |m| Value::int(m.atime()));
stat_getter!(get_mtime, "getMTime", false, |m| Value::int(m.mtime()));
stat_getter!(get_ctime, "getCTime", false, |m| Value::int(m.ctime()));
stat_getter!(get_type, "getType", true, |m| Value::str(file_type(&m)));

fn file_type(m: &std::fs::Metadata) -> &'static str {
    use p8_runtime::os::FileTypeExt;
    let t = m.file_type();
    if t.is_symlink() {
        "link"
    } else if t.is_dir() {
        "dir"
    } else if t.is_file() {
        "file"
    } else if t.is_fifo() {
        "fifo"
    } else if t.is_char_device() {
        "char"
    } else if t.is_block_device() {
        "block"
    } else if t.is_socket() {
        "socket"
    } else {
        "unknown"
    }
}

/// The is*() checks: false (and no exception) for a file that is not there.
fn check(ctx: &mut dyn Ctx, this: &Object, f: impl Fn(&str) -> bool) -> R {
    let name = stat_name(ctx, this)?;
    Ok(Value::bool(f(&name)))
}

/// The access checks, as is_readable() and the like make them.
fn mode_has(name: &str, bits: u32) -> bool {
    crate::file::access(name, bits)
}

fn is_writable(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    check(ctx, this, |n| mode_has(n, 0o222))
}

fn is_readable(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    check(ctx, this, |n| mode_has(n, 0o444))
}

fn is_executable(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    check(ctx, this, |n| std::fs::metadata(n).is_ok_and(|m| m.is_file()) && mode_has(n, 0o111))
}

fn is_file(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    check(ctx, this, |n| std::fs::metadata(n).is_ok_and(|m| m.is_file()))
}

fn is_dir(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    check(ctx, this, |n| std::fs::metadata(n).is_ok_and(|m| m.is_dir()))
}

fn is_link(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    check(ctx, this, |n| std::fs::symlink_metadata(n).is_ok_and(|m| m.file_type().is_symlink()))
}

fn get_link_target(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let name = stat_name(ctx, this)?;
    match std::fs::read_link(&name) {
        Ok(t) => Ok(string(t.into_os_string().into_encoded_bytes())),
        Err(e) => Err(ctx.throw("RuntimeException", format!("Unable to read link {name}, error: {}", errno_text(&e)))),
    }
}

/// strerror() for an I/O error.
fn errno_text(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(22) => "Invalid argument".into(),
        _ => crate::file::os_error(e),
    }
}

fn get_real_path(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let name = state(this).borrow_mut().pathname();
    let name = match name {
        Some(n) => n,
        None => return Ok(Value::FALSE),
    };
    let name = if name.is_empty() { b".".to_vec() } else { name };
    Ok(match p8_runtime::os::canonicalize(s(&name)) {
        Ok(p) => string(p.into_os_string().into_encoded_bytes()),
        Err(_) => Value::FALSE,
    })
}

/// An info object of `class` (or the object's info class) for `path`: its
/// own constructor runs when it has one.
fn info_object(ctx: &mut dyn Ctx, class: Option<Rc<Class>>, path: Vec<u8>, dir_path: Option<Vec<u8>>) -> R {
    let class = match class {
        Some(c) => c,
        None => ctx.class(b"SplFileInfo").expect("SplFileInfo"),
    };
    let own_constructor = class.method(b"__construct").is_some_and(|m| !m.declaring_name.as_bytes().eq_ignore_ascii_case(b"SplFileInfo"));
    if own_constructor {
        return Ok(Value::object(ctx.new_instance(&class, &mut [string(path)], true)?));
    }
    let o = ctx.new_instance(&class, &mut [], false)?;
    {
        let mut st = state(&o).borrow_mut();
        match dir_path {
            Some(p) => {
                st.file_name = Some(path);
                st.path = Some(p);
            }
            None => set_filename(&mut st, &path),
        }
    }
    Ok(Value::object(o))
}

/// A class argument (getFileInfo, setInfoClass...): it must be SplFileInfo
/// or derive from it.
fn class_arg(ctx: &mut dyn Ctx, args: &[Value], method: &str) -> Result<Option<Rc<Class>>, Throw> {
    match arg(args, 0) {
        v if v.is_null() => Ok(None),
        v => {
            let name = bytes(&v);
            match ctx.class_load(&name)? {
                Some(c) if c.is_a(b"splfileinfo") => Ok(Some(c)),
                _ => Err(ctx.throw(
                    "TypeError",
                    format!("SplFileInfo::{method}(): Argument #1 ($class) must be a class name derived from SplFileInfo or null, {} given", s(&name)),
                )),
            }
        }
    }
}

fn get_file_info(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let class = class_arg(ctx, args, "getFileInfo")?;
    let (name, path, info_class, empty_dir) = {
        let mut st = state(this).borrow_mut();
        let empty = st.dir.as_ref().is_some_and(|d| d.entry().is_empty());
        (st.file_name(), st.path.clone(), st.info_class.clone(), empty)
    };
    if empty_dir {
        return Err(ctx.throw("RuntimeException", "Could not open file".into()));
    }
    let Some(name) = name else { return Err(not_initialized(ctx)) };
    info_object(ctx, class.or(info_class), name, Some(path.unwrap_or_default()))
}

fn get_path_info(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let class = class_arg(ctx, args, "getPathInfo")?;
    let (name, info_class) = {
        let mut st = state(this).borrow_mut();
        (st.pathname(), st.info_class.clone())
    };
    match name {
        Some(n) if !n.is_empty() => info_object(ctx, class.or(info_class), dirname(&n), None),
        _ => Ok(Value::NULL),
    }
}

/// zend_dirname().
fn dirname(p: &[u8]) -> Vec<u8> {
    let mut end = p.len();
    while end > 1 && p[end - 1] == b'/' {
        end -= 1;
    }
    match p[..end].iter().rposition(|&c| c == b'/') {
        None => b".".to_vec(),
        Some(0) => b"/".to_vec(),
        Some(i) => {
            let mut j = i;
            while j > 1 && p[j - 1] == b'/' {
                j -= 1;
            }
            p[..j].to_vec()
        }
    }
}

fn set_info_class(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let class = class_arg(ctx, args, "setInfoClass")?;
    state(this).borrow_mut().info_class = class;
    Ok(Value::NULL)
}

fn set_file_class(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    // The file class is for openFile(): SplFileObject, which p8 lacks yet.
    Ok(Value::NULL)
}

fn open_file(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Class \"SplFileObject\" not found".into()))
}

fn info_to_string(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    get_pathname(ctx, this, args)
}

fn debug_info(o: &Object) -> PhpArray {
    let mut out = PhpArray::new();
    for (k, v) in o.dynamic.borrow().iter() {
        out.insert(k, v.clone());
    }
    let private = |class: &str, name: &str| Key::from_string(PhpStr::from(format!("\0{class}\0{name}").as_str()));
    let mut st = state(o).borrow_mut();
    out.insert(private("SplFileInfo", "pathName"), string(st.pathname().unwrap_or_default()));
    if st.file_name.is_some() {
        let name = st.short_name().unwrap_or_default();
        out.insert(private("SplFileInfo", "fileName"), string(name));
    }
    if let Some(d) = &st.dir {
        out.insert(private("DirectoryIterator", "glob"), if d.glob { string(d.source.clone()) } else { Value::FALSE });
        out.insert(private("RecursiveDirectoryIterator", "subPathName"), string(d.sub_path.clone().unwrap_or_default()));
    }
    out
}

fn debug_info_method(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::array(debug_info(this)))
}

fn bad_state(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "The parent constructor was not called: the object is in an invalid state".into()))
}

// ---- DirectoryIterator and the rest ----

/// spl_filesystem_object_construct(): open `path` (a glob for
/// GlobIterator) with `flags`.
fn open(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &[Value], flags: i64, glob: bool, class: &str) -> R {
    let path = bytes(&arg(args, 0));
    if path.is_empty() {
        return Err(ctx.throw("ValueError", format!("{class}::__construct(): Argument #1 ($directory) must not be empty")));
    }
    if state(this).borrow().path.is_some() {
        return Err(ctx.throw("Error", "Directory object is already initialized".into()));
    }
    let source = if glob && !path.starts_with(b"glob://") { [b"glob://".as_slice(), &path].concat() } else { path.clone() };
    let is_glob = source.starts_with(b"glob://");
    let entries = match list(&source, is_glob) {
        Ok(e) => e,
        Err(e) => {
            return Err(ctx.throw("UnexpectedValueException", format!("{class}::__construct({}): Failed to open directory: {}", s(&path), crate::file::os_error(&e))));
        }
    };
    let dir_path = if is_glob {
        glob_dir(&source)
    } else if path.len() > 1 && path.ends_with(b"/") {
        Some(path[..path.len() - 1].to_vec())
    } else {
        Some(path)
    };
    let mut st = state(this).borrow_mut();
    st.flags = flags;
    st.path = Some(dir_path.unwrap_or_default());
    st.dir = Some(Dir { source, glob: is_glob, entries, at: 0, index: 0, sub_path: None });
    st.file_name = None;
    if flags & SKIP_DOTS != 0 {
        let d = st.dir.as_mut().expect("just set");
        while is_dot(d.entry()) {
            d.at += 1;
        }
    }
    Ok(Value::NULL)
}

fn constructor_flags(args: &[Value], default: i64) -> i64 {
    if args.len() > 1 { arg(args, 1).to_int() } else { default }
}

fn di_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    open(ctx, this, args, KEY_AS_PATHNAME | CURRENT_AS_SELF, false, "DirectoryIterator")
}

fn fsi_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let flags = constructor_flags(args, SKIP_DOTS | KEY_AS_PATHNAME | CURRENT_AS_FILEINFO);
    open(ctx, this, args, flags, false, "FilesystemIterator")
}

fn rdi_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let flags = constructor_flags(args, KEY_AS_PATHNAME | CURRENT_AS_FILEINFO);
    open(ctx, this, args, flags, false, "RecursiveDirectoryIterator")
}

fn glob_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let flags = constructor_flags(args, KEY_AS_PATHNAME | CURRENT_AS_FILEINFO);
    open(ctx, this, args, flags, true, "GlobIterator")
}

/// CHECK_DIRECTORY_ITERATOR_IS_INITIALIZED.
fn opened(ctx: &mut dyn Ctx, this: &Object) -> Result<(), Throw> {
    if state(this).borrow().dir.is_none() {
        return Err(not_initialized(ctx));
    }
    Ok(())
}

fn di_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    opened(ctx, this)?;
    let mut st = state(this).borrow_mut();
    // DirectoryIterator::rewind() reads the first entry as it comes.
    let flags = st.flags;
    st.flags &= !SKIP_DOTS;
    st.rewind();
    st.flags = flags;
    Ok(Value::NULL)
}

fn fsi_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    opened(ctx, this)?;
    state(this).borrow_mut().rewind();
    Ok(Value::NULL)
}

fn di_valid(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    opened(ctx, this)?;
    Ok(Value::bool(!state(this).borrow().entry().is_empty()))
}

fn di_key(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    opened(ctx, this)?;
    Ok(Value::int(state(this).borrow().dir.as_ref().map_or(0, |d| d.index)))
}

fn di_current(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    opened(ctx, this)?;
    Ok(Value::object(Rc::clone(this)))
}

fn di_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    opened(ctx, this)?;
    let mut st = state(this).borrow_mut();
    if let Some(d) = &mut st.dir {
        d.index += 1;
    }
    st.read();
    Ok(Value::NULL)
}

fn di_seek(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    opened(ctx, this)?;
    let pos = arg(args, 0).to_int();
    let index = |o: &Object| state(o).borrow().dir.as_ref().map_or(0, |d| d.index);
    if index(this) > pos {
        crate::spl::call_this(ctx, this, "rewind", &mut [])?;
    }
    while index(this) < pos {
        if !crate::spl::call_this(ctx, this, "valid", &mut [])?.to_bool() {
            return Err(ctx.throw("OutOfBoundsException", format!("Seek position {pos} is out of range")));
        }
        crate::spl::call_this(ctx, this, "next", &mut [])?;
    }
    Ok(Value::NULL)
}

fn di_is_dot(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    opened(ctx, this)?;
    Ok(Value::bool(is_dot(&state(this).borrow().entry())))
}

fn di_to_string(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    get_filename(ctx, this, args)
}

fn fsi_key(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let mut st = state(this).borrow_mut();
    if st.flags & KEY_AS_FILENAME != 0 {
        return Ok(string(st.entry()));
    }
    Ok(string(st.file_name().unwrap_or_default()))
}

fn fsi_current(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let (flags, name, path, info_class, empty) = {
        let mut st = state(this).borrow_mut();
        let empty = st.entry().is_empty();
        (st.flags, st.file_name(), st.path.clone(), st.info_class.clone(), empty)
    };
    if flags & CURRENT_AS_PATHNAME != 0 {
        return Ok(string(name.unwrap_or_default()));
    }
    if flags & CURRENT_AS_SELF == 0 {
        if empty {
            return Err(ctx.throw("RuntimeException", "Could not open file".into()));
        }
        return info_object(ctx, info_class, name.unwrap_or_default(), Some(path.unwrap_or_default()));
    }
    Ok(Value::object(Rc::clone(this)))
}

fn fsi_get_flags(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::int(state(this).borrow().flags & (KEY_MODE_MASK | CURRENT_MODE_MASK | OTHER_MODE_MASK)))
}

fn fsi_set_flags(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let mask = KEY_MODE_MASK | CURRENT_MODE_MASK | OTHER_MODE_MASK;
    let mut st = state(this).borrow_mut();
    st.flags = (st.flags & !mask) | (arg(args, 0).to_int() & mask);
    Ok(Value::NULL)
}

fn rdi_has_children(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let allow_links = arg(args, 0).to_bool();
    let mut st = state(this).borrow_mut();
    let entry = st.entry();
    if entry.is_empty() || is_dot(&entry) {
        return Ok(Value::FALSE);
    }
    let name = s(&st.file_name().unwrap_or_default());
    let Ok(m) = std::fs::symlink_metadata(&name) else { return Ok(Value::FALSE) };
    if !m.file_type().is_symlink() {
        return Ok(Value::bool(m.is_dir()));
    }
    if !allow_links && st.flags & FOLLOW_SYMLINKS == 0 {
        return Ok(Value::FALSE);
    }
    Ok(Value::bool(std::fs::metadata(&name).is_ok_and(|m| m.is_dir())))
}

fn rdi_get_children(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let (name, flags, entry, sub, info_class) = {
        let mut st = state(this).borrow_mut();
        let entry = st.entry();
        let sub = st.dir.as_ref().and_then(|d| d.sub_path.clone());
        (st.file_name().unwrap_or_default(), st.flags, entry, sub, st.info_class.clone())
    };
    let class = Rc::clone(&this.class);
    let child = ctx.new_instance(&class, &mut [string(name), Value::int(flags)], true)?;
    let sub_path = match sub {
        Some(p) if !p.is_empty() => [p.as_slice(), b"/", &entry].concat(),
        _ => entry,
    };
    if let Some(Fs(cell)) = child.native::<Fs>() {
        let mut st = cell.borrow_mut();
        if let Some(d) = &mut st.dir {
            d.sub_path = Some(sub_path);
        }
        st.info_class = info_class;
    }
    Ok(Value::object(child))
}

fn rdi_get_sub_path(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(string(state(this).borrow().dir.as_ref().and_then(|d| d.sub_path.clone()).unwrap_or_default()))
}

fn rdi_get_sub_pathname(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let st = state(this).borrow();
    let entry = st.entry();
    Ok(string(match st.dir.as_ref().and_then(|d| d.sub_path.clone()) {
        Some(p) => [p.as_slice(), b"/", &entry].concat(),
        None => entry,
    }))
}

fn glob_count(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::int(state(this).borrow().dir.as_ref().map_or(0, |d| d.entries.len() as i64)))
}

const fn m(f: p8_runtime::object::NativeMethod) -> Option<Native> {
    Some(Native::Method(f))
}

const fn spec(name: &'static str, parent: Option<&'static str>, interfaces: &'static [&'static str]) -> ClassSpec {
    ClassSpec { name, parent, interfaces, flags: 0, properties: &[], methods: &[], constants: &[], debug_info: Some(debug_info) }
}

pub static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        methods: &[
            ("__construct", m(info_construct)),
            ("getPath", m(get_path)),
            ("getFilename", m(get_filename)),
            ("getExtension", m(get_extension)),
            ("getBasename", m(get_basename)),
            ("getPathname", m(get_pathname)),
            ("getPerms", m(get_perms)),
            ("getInode", m(get_inode)),
            ("getSize", m(get_size)),
            ("getOwner", m(get_owner)),
            ("getGroup", m(get_group)),
            ("getATime", m(get_atime)),
            ("getMTime", m(get_mtime)),
            ("getCTime", m(get_ctime)),
            ("getType", m(get_type)),
            ("isWritable", m(is_writable)),
            ("isReadable", m(is_readable)),
            ("isExecutable", m(is_executable)),
            ("isFile", m(is_file)),
            ("isDir", m(is_dir)),
            ("isLink", m(is_link)),
            ("getLinkTarget", m(get_link_target)),
            ("getRealPath", m(get_real_path)),
            ("getFileInfo", m(get_file_info)),
            ("getPathInfo", m(get_path_info)),
            ("openFile", m(open_file)),
            ("setFileClass", m(set_file_class)),
            ("setInfoClass", m(set_info_class)),
            ("__toString", m(info_to_string)),
            ("__debugInfo", m(debug_info_method)),
            ("_bad_state_ex", m(bad_state)),
        ],
        ..spec("SplFileInfo", None, &["Stringable"])
    },
    ClassSpec {
        methods: &[
            ("__construct", m(di_construct)),
            ("getFilename", m(get_filename)),
            ("getExtension", m(get_extension)),
            ("getBasename", m(get_basename)),
            ("isDot", m(di_is_dot)),
            ("rewind", m(di_rewind)),
            ("valid", m(di_valid)),
            ("key", m(di_key)),
            ("current", m(di_current)),
            ("next", m(di_next)),
            ("seek", m(di_seek)),
            ("__toString", m(di_to_string)),
        ],
        ..spec("DirectoryIterator", Some("SplFileInfo"), &["SeekableIterator"])
    },
    ClassSpec {
        constants: &[
            ("CURRENT_MODE_MASK", Constant::Int(CURRENT_MODE_MASK)),
            ("CURRENT_AS_PATHNAME", Constant::Int(CURRENT_AS_PATHNAME)),
            ("CURRENT_AS_FILEINFO", Constant::Int(CURRENT_AS_FILEINFO)),
            ("CURRENT_AS_SELF", Constant::Int(CURRENT_AS_SELF)),
            ("KEY_MODE_MASK", Constant::Int(KEY_MODE_MASK)),
            ("KEY_AS_PATHNAME", Constant::Int(KEY_AS_PATHNAME)),
            ("FOLLOW_SYMLINKS", Constant::Int(FOLLOW_SYMLINKS)),
            ("KEY_AS_FILENAME", Constant::Int(KEY_AS_FILENAME)),
            ("NEW_CURRENT_AND_KEY", Constant::Int(KEY_AS_FILENAME | CURRENT_AS_FILEINFO)),
            ("OTHER_MODE_MASK", Constant::Int(OTHER_MODE_MASK)),
            ("SKIP_DOTS", Constant::Int(SKIP_DOTS)),
            ("UNIX_PATHS", Constant::Int(UNIX_PATHS)),
        ],
        methods: &[
            ("__construct", m(fsi_construct)),
            ("rewind", m(fsi_rewind)),
            ("key", m(fsi_key)),
            ("current", m(fsi_current)),
            ("getFlags", m(fsi_get_flags)),
            ("setFlags", m(fsi_set_flags)),
        ],
        ..spec("FilesystemIterator", Some("DirectoryIterator"), &[])
    },
    ClassSpec {
        methods: &[
            ("__construct", m(rdi_construct)),
            ("hasChildren", m(rdi_has_children)),
            ("getChildren", m(rdi_get_children)),
            ("getSubPath", m(rdi_get_sub_path)),
            ("getSubPathname", m(rdi_get_sub_pathname)),
        ],
        ..spec("RecursiveDirectoryIterator", Some("FilesystemIterator"), &["RecursiveIterator"])
    },
    ClassSpec {
        methods: &[("__construct", m(glob_construct)), ("count", m(glob_count))],
        ..spec("GlobIterator", Some("FilesystemIterator"), &["Countable"])
    },
];

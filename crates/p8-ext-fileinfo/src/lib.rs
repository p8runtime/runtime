#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's fileinfo extension: the finfo class, the finfo_* functions and
//! mime_content_type(), over a pure-Rust reading of libmagic's rules: no
//! libmagic, no C. What is observable follows libmagic 5.45 as php-src 8.4
//! bundles it: the description (FILEINFO_NONE), the MIME type, the encoding
//! and both together, the extension (FILEINFO_EXTENSION), FILEINFO_CONTINUE.
//!
//! `magic.rs` interprets the magic(5) rule language as softmagic.c does:
//! offsets (absolute, relative, indirect), string/search/regex and numeric
//! tests, continuation levels, `name`/`use`/`indirect`/`default`/`clear`,
//! rule strengths and the output rules for descriptions and MIME types.
//! `detect.rs` is file_buffer(): the order of the checks (tar, JSON, CSV, SIMH
//! tapes, the binary rules, the text rules) and how text is described.
//! `text.rs` is the character-set detection and the checks libmagic does in C.
//! `rules/` holds the rule files the web meets (images, fonts, audio and video,
//! documents, archives, markup and source code, executables): excerpts of
//! file(1) 5.45's Magdir with php-src's patch applied (see rules/LICENSE).
//!
//! A user-supplied magic file is read with the same interpreter and replaces
//! the built-in rules. Regular expressions run on p8-pcre.
//!
//! Not implemented: Microsoft Office compound documents are not opened (CDF), so
//! a .doc, .xls or .ppt is told apart only by the signatures of its rules;
//! libmagic's ELF notes and section reading; the rule files that are not in
//! `rules/`; and the float, GUID, DER and octal tests, the offset type and the
//! 64-bit varints of the rule language (a rule that uses one is left out).
//!
//! Reference: $PHP_SRC_PATH/ext/fileinfo/fileinfo.c, ext/fileinfo/libmagic

mod detect;
mod magic;
mod text;

use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::ops::Diag;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "fileinfo", register };

type R = Result<Value, Throw>;

/// libmagic reads at most this much of a file.
const BYTES_MAX: usize = 7 * 1024 * 1024;

const RULES: &[&[u8]] = &[
    include_bytes!("../rules/animation"),
    include_bytes!("../rules/apache"),
    include_bytes!("../rules/archive"),
    include_bytes!("../rules/assembler"),
    include_bytes!("../rules/audio"),
    include_bytes!("../rules/c-lang"),
    include_bytes!("../rules/cafebabe"),
    include_bytes!("../rules/clojure"),
    include_bytes!("../rules/commands"),
    include_bytes!("../rules/compress"),
    include_bytes!("../rules/ctags"),
    include_bytes!("../rules/cups"),
    include_bytes!("../rules/diff"),
    include_bytes!("../rules/elf"),
    include_bytes!("../rules/erlang"),
    include_bytes!("../rules/filesystems"),
    include_bytes!("../rules/fonts"),
    include_bytes!("../rules/forth"),
    include_bytes!("../rules/fortran"),
    include_bytes!("../rules/gentoo"),
    include_bytes!("../rules/gnu"),
    include_bytes!("../rules/iff"),
    include_bytes!("../rules/images"),
    include_bytes!("../rules/intel"),
    include_bytes!("../rules/ispell"),
    include_bytes!("../rules/java"),
    include_bytes!("../rules/javascript"),
    include_bytes!("../rules/jpeg"),
    include_bytes!("../rules/kml"),
    include_bytes!("../rules/lisp"),
    include_bytes!("../rules/lua"),
    include_bytes!("../rules/m4"),
    include_bytes!("../rules/magic"),
    include_bytes!("../rules/mail.news"),
    include_bytes!("../rules/make"),
    include_bytes!("../rules/mathematica"),
    include_bytes!("../rules/matroska"),
    include_bytes!("../rules/mime"),
    include_bytes!("../rules/misctools"),
    include_bytes!("../rules/mozilla"),
    include_bytes!("../rules/msdos"),
    include_bytes!("../rules/msooxml"),
    include_bytes!("../rules/netscape"),
    include_bytes!("../rules/news"),
    include_bytes!("../rules/nim-lang"),
    include_bytes!("../rules/ocaml"),
    include_bytes!("../rules/ole2compounddocs"),
    include_bytes!("../rules/pascal"),
    include_bytes!("../rules/pdf"),
    include_bytes!("../rules/perl"),
    include_bytes!("../rules/pgp"),
    include_bytes!("../rules/pgp-binary-keys"),
    include_bytes!("../rules/printer"),
    include_bytes!("../rules/python"),
    include_bytes!("../rules/riff"),
    include_bytes!("../rules/rpm"),
    include_bytes!("../rules/rst"),
    include_bytes!("../rules/rtf"),
    include_bytes!("../rules/ruby"),
    include_bytes!("../rules/rust"),
    include_bytes!("../rules/sc"),
    include_bytes!("../rules/sendmail"),
    include_bytes!("../rules/sgml"),
    include_bytes!("../rules/smalltalk"),
    include_bytes!("../rules/sql"),
    include_bytes!("../rules/ssh"),
    include_bytes!("../rules/ssl"),
    include_bytes!("../rules/subtitle"),
    include_bytes!("../rules/tcl"),
    include_bytes!("../rules/terminfo"),
    include_bytes!("../rules/tex"),
    include_bytes!("../rules/troff"),
    include_bytes!("../rules/unicode"),
    include_bytes!("../rules/uuencode"),
    include_bytes!("../rules/varied.script"),
    include_bytes!("../rules/vorbis"),
    include_bytes!("../rules/webassembly"),
    include_bytes!("../rules/windows"),
    include_bytes!("../rules/wsdl"),
    include_bytes!("../rules/zip"),
];

thread_local! {
    static PARSED: Rc<magic::Rules> = Rc::new(magic::Rules::parse(RULES).expect("the built-in rules parse").0);
}

/// A finfo's native state: its flags once constructed.
#[derive(Default)]
struct Handle {
    flags: Cell<Option<i64>>,
    /// The rules of a user-supplied magic file; None: the built-in ones.
    rules: RefCell<Option<Rc<magic::Rules>>>,
}

fn handle(o: &Object) -> &Handle {
    o.native::<Handle>().expect("finfo state")
}

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_data("finfo", || Rc::new(Handle::default()));
    for (name, value) in CONSTANTS {
        registry.add_constant(name.as_bytes(), Constant::Int(*value));
    }
    registry.mark_reenters(&["finfo_file", "mime_content_type"]);
    registry.add_module("fileinfo");
}

static CONSTANTS: &[(&str, i64)] = &[
    ("FILEINFO_NONE", 0),
    ("FILEINFO_SYMLINK", 2),
    ("FILEINFO_MIME", 1040),
    ("FILEINFO_MIME_TYPE", 16),
    ("FILEINFO_MIME_ENCODING", 1024),
    ("FILEINFO_DEVICES", 8),
    ("FILEINFO_CONTINUE", 32),
    ("FILEINFO_PRESERVE_ATIME", 128),
    ("FILEINFO_RAW", 256),
    ("FILEINFO_APPLE", 2048),
    ("FILEINFO_EXTENSION", 16777216),
];

macro_rules! b {
    ($name:literal, $f:ident) => {
        Builtin { name: $name, run: $f, by_ref: &[] }
    };
}

static BUILTINS: &[Builtin] = &[
    b!("finfo_open", finfo_open),
    b!("finfo_close", finfo_close),
    b!("finfo_set_flags", finfo_set_flags),
    b!("finfo_file", finfo_file),
    b!("finfo_buffer", finfo_buffer),
    b!("mime_content_type", mime_content_type),
];

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "finfo",
    parent: None,
    interfaces: &[],
    flags: CLASS_NOT_SERIALIZABLE | CLASS_UNCLONEABLE,
    properties: &[],
    methods: &[
        ("__construct", Some(Native::Method(m_construct))),
        ("file", Some(Native::Method(m_file))),
        ("buffer", Some(Native::Method(m_buffer))),
        ("set_flags", Some(Native::Method(m_set_flags))),
    ],
    constants: &[],
    debug_info: None,
}];

// ---- helpers ----

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    arg(args, i).to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// PHP's argument checks for the method `stub` ("finfo::file").
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

fn os_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        std::io::ErrorKind::IsADirectory => "Is a directory".into(),
        _ => {
            let s = e.to_string();
            match s.find(" (os error") {
                Some(at) => s[..at].to_string(),
                None => s,
            }
        }
    }
}

fn new_finfo(ctx: &mut dyn Ctx, opened: Opened) -> Value {
    let class = ctx.class(b"finfo").expect("finfo");
    let o = ctx.new_object(&class);
    let h = Handle::default();
    h.flags.set(Some(opened.flags));
    *h.rules.borrow_mut() = opened.rules;
    Value::Object(o.sibling(class, Some(Rc::new(h))))
}

struct Opened {
    flags: i64,
    rules: Option<Rc<magic::Rules>>,
}

/// The rules a magic file (or a directory of them) holds.
fn load_magic(path: &std::path::Path) -> Option<magic::Rules> {
    let mut sources: Vec<Vec<u8>> = Vec::new();
    if path.is_dir() {
        let mut names: Vec<_> = std::fs::read_dir(path).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
        names.sort();
        for n in names {
            if n.extension().is_some_and(|e| e == "mgc") || n.file_name().is_some_and(|f| f.to_string_lossy().starts_with('.')) {
                continue;
            }
            sources.push(std::fs::read(n).ok()?);
        }
    } else {
        sources.push(std::fs::read(path).ok()?);
    }
    let refs: Vec<&[u8]> = sources.iter().map(Vec::as_slice).collect();
    magic::Rules::parse(&refs).ok().map(|(r, _)| r)
}

/// finfo_open() / finfo::__construct(): the flags and rules (None when it
/// failed) and the warnings libmagic's loading gave.
fn open(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<(Option<Opened>, Vec<String>), Throw> {
    let flags = if args.is_empty() { 0 } else { arg(args, 0).to_int() };
    let db = match arg(args, 1) {
        Value::Null => Vec::new(),
        v => v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default(),
    };
    if db.contains(&0) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #2 ($magic_database) must not contain any null bytes")));
    }
    let mut warns = Vec::new();
    let mut rules = None;
    if !db.is_empty() {
        let shown = lossy(&db);
        let cwd = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
        let full_len = if shown.starts_with('/') { shown.len() } else { cwd.len() + 1 + shown.len() };
        if full_len >= 4096 {
            warns.push(format!("{func}(): File name is longer than the maximum allowed path length on this platform (4096): {shown}"));
            return Ok((None, warns));
        }
        match std::fs::canonicalize(&shown) {
            Ok(p) => {
                // libmagic only says so when MAGIC_CHECK (0x40) is among the flags
                if !p.is_dir() && flags & 0x40 != 0 {
                    warns.push(format!("{func}(): using regular magic file `{}'", p.display()));
                }
                match load_magic(&p) {
                    Some(r) => rules = Some(Rc::new(r)),
                    None => {
                        warns.push(format!("{func}(): Failed to load magic database at \"{}\"", p.display()));
                        return Ok((None, warns));
                    }
                }
            }
            Err(e) => {
                let full = if shown.starts_with('/') { shown.clone() } else { format!("{}/{shown}", std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default()) };
                warns.push(format!("{func}({full}): Failed to open stream: {}", os_error(&e)));
                warns.push(format!("{func}({full}): Failed to open stream: {}", os_error(&e)));
                warns.push(format!("{func}(): Failed to load magic database at \"{full}\""));
                return Ok((None, warns));
            }
        }
    }
    Ok((Some(Opened { flags, rules }), warns))
}

fn finfo_open(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (opened, warns) = open(ctx, "finfo_open", args)?;
    for w in warns {
        ctx.warn(&w);
    }
    match opened {
        Some(o) => Ok(new_finfo(ctx, o)),
        None => Ok(Value::Bool(false)),
    }
}

fn m_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "finfo::__construct", args)?;
    let (opened, warns) = open(ctx, "finfo::__construct", args)?;
    // The constructor runs with warnings turned into exceptions.
    if let Some(first) = warns.into_iter().next() {
        return Err(ctx.throw("Exception", first));
    }
    match opened {
        Some(o) => {
            handle(this).flags.set(Some(o.flags));
            *handle(this).rules.borrow_mut() = o.rules;
            Ok(Value::Null)
        }
        None => Err(ctx.throw("Exception", "Constructor failed".into())),
    }
}

fn flags_of(ctx: &mut dyn Ctx, o: &Rc<Object>) -> Result<i64, Throw> {
    match handle(o).flags.get() {
        Some(f) => Ok(f),
        None => Err(ctx.throw("Error", "Invalid finfo object".into())),
    }
}

fn this_arg(args: &[Value]) -> Rc<Object> {
    match arg(args, 0) {
        Value::Object(o) => o,
        _ => unreachable!("checked against the stub"),
    }
}

fn finfo_close(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::Bool(true))
}

fn set_flags(ctx: &mut dyn Ctx, o: &Rc<Object>, flags: i64) -> R {
    flags_of(ctx, o)?;
    handle(o).flags.set(Some(flags));
    Ok(Value::Bool(true))
}

fn finfo_set_flags(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args);
    set_flags(ctx, &o, arg(args, 1).to_int())
}

fn m_set_flags(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "finfo::set_flags", args)?;
    set_flags(ctx, this, arg(args, 0).to_int())
}

// ---- identifying ----

fn identify(ctx: &mut dyn Ctx, func: &str, flags: i64, data: &[u8], custom: Option<Rc<magic::Rules>>, env: magic::Env) -> R {
    let rules = custom.unwrap_or_else(|| PARSED.with(Rc::clone));
    match detect::identify(&rules, &data[..data.len().min(BYTES_MAX)], flags, env) {
        Ok(out) => Ok(Value::Str(PhpStr::new(printable(out)))),
        Err(e) => {
            ctx.warn(&format!("{func}(): Failed identify data 0:{e}"));
            Ok(Value::Bool(false))
        }
    }
}

/// file_getbuffer: bytes that are not printable are shown as octal escapes.
fn printable(out: Vec<u8>) -> Vec<u8> {
    if out.iter().all(|&c| (0x20..0x7f).contains(&c)) {
        return out;
    }
    let mut o = Vec::with_capacity(out.len());
    for c in out {
        if (0x20..0x7f).contains(&c) {
            o.push(c);
        } else {
            o.extend(format!("\\{c:03o}").bytes());
        }
    }
    o
}

fn do_buffer(ctx: &mut dyn Ctx, func: &str, o: &Rc<Object>, data: &[u8], flags: i64) -> R {
    let base = flags_of(ctx, o)?;
    let rules = handle(o).rules.borrow().clone();
    identify(ctx, func, if flags != 0 { flags } else { base }, data, rules, magic::Env::default())
}

fn finfo_buffer(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args);
    let data = bytes(args, 1);
    do_buffer(ctx, "finfo_buffer", &o, &data, arg(args, 2).to_int())
}

fn m_buffer(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "finfo::buffer", args)?;
    let data = bytes(args, 0);
    do_buffer(ctx, "finfo::buffer", this, &data, arg(args, 1).to_int())
}

/// The first BYTES_MAX bytes a filename names, `Ok(None)` after a warning,
/// or the string "directory".
enum Source {
    Dir,
    Data(Vec<u8>, magic::Env),
}

fn read_source(ctx: &mut dyn Ctx, func: &str, path: &[u8]) -> Result<Option<Source>, Throw> {
    let shown = lossy(path);
    if shown.contains("://") && !shown.starts_with("file://") || shown.starts_with("data:") {
        let r = ctx.call(&Value::str("file_get_contents"), &mut [Value::str(shown.as_str())])?;
        return Ok(match r {
            Value::Str(s) => Some(Source::Data(s.as_bytes().to_vec(), magic::Env::default())),
            _ => None,
        });
    }
    let p = shown.strip_prefix("file://").unwrap_or(&shown);
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => Ok(Some(Source::Dir)),
        _ => {
            use std::io::Read;
            match std::fs::File::open(p) {
                Ok(f) => {
                    let mut data = Vec::new();
                    let _ = f.take(BYTES_MAX as u64).read_to_end(&mut data);
                    // A plain file can be read back from its end, and has permission bits.
                    let mode = std::fs::metadata(p).map_or(0, |m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()));
                    Ok(Some(Source::Data(data, magic::Env { mode, tail: true })))
                }
                Err(e) => {
                    ctx.warn(&format!("{func}({shown}): Failed to open stream: {}", os_error(&e)));
                    Ok(None)
                }
            }
        }
    }
}

/// `n`: the argument number the filename has in the call; the method's
/// stub names that position `param` (php-src reports the function's number).
fn do_file(ctx: &mut dyn Ctx, func: &str, o: Option<&Rc<Object>>, path: &[u8], flags: i64, n: usize, param: &str) -> R {
    let base = match o {
        Some(o) => flags_of(ctx, o)?,
        None => 16,
    };
    if path.is_empty() {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{n} (${param}) must not be empty")));
    }
    if path.contains(&0) {
        return Err(ctx.throw("TypeError", format!("{func}(): Argument #{n} (${param}) must not contain any null bytes")));
    }
    let flags = if flags != 0 { flags } else { base };
    match read_source(ctx, func, path)? {
        None => Ok(Value::Bool(false)),
        Some(Source::Dir) => Ok(Value::str("directory")),
        Some(Source::Data(d, env)) => {
            let rules = o.and_then(|o| handle(o).rules.borrow().clone());
            identify(ctx, func, flags, &d, rules, env)
        }
    }
}

fn finfo_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = this_arg(args);
    let path = bytes(args, 1);
    do_file(ctx, "finfo_file", Some(&o), &path, arg(args, 2).to_int(), 2, "filename")
}

fn m_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prep(ctx, "finfo::file", args)?;
    let path = bytes(args, 0);
    // php-src numbers the filename argument 2 for the method as well, naming it by its position in the method
    do_file(ctx, "finfo::file", Some(this), &path, arg(args, 1).to_int(), 2, "flags")
}

fn mime_content_type(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let a = arg(args, 0);
    if let Some(r) = p8_runtime::stream::resource_of(&a) {
        let Some(res) = p8_runtime::stream::resource(&r) else { return Ok(Value::Bool(false)) };
        let data = {
            let mut s = res.stream.borrow_mut();
            let at = s.tell();
            s.seek(0, 0);
            let mut d = s.read(BYTES_MAX);
            d.truncate(BYTES_MAX);
            s.seek(at, 0);
            d
        };
        return identify(ctx, "mime_content_type", 16, &data, None, magic::Env::default());
    }
    match a {
        Value::Str(s) => do_file(ctx, "mime_content_type", None, s.as_bytes(), 0, 1, "filename"),
        other => Err(ctx.throw("TypeError", format!("mime_content_type(): Argument #1 ($filename) must be of type resource|string, {} given", other.type_name()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(data: &[u8], flags: i64) -> String {
        let rules = PARSED.with(Rc::clone);
        let out = detect::identify(&rules, data, flags, magic::Env::default()).expect("identified");
        String::from_utf8(printable(out)).expect("utf-8")
    }

    #[test]
    fn the_rules_load_with_few_left_out() {
        let (_, skipped) = magic::Rules::parse(RULES).expect("parse");
        // Only tests of a kind the interpreter lacks (floats, GUIDs, DER, offsets) are left out.
        assert!(skipped.len() < 100, "{} rule lines left out, e.g. {:?}", skipped.len(), &skipped[..5.min(skipped.len())]);
    }

    #[test]
    fn answers_for_the_common_formats() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x10\0\0\0\x10\x08\x06\0\0\0";
        assert_eq!(kind(png, 16), "image/png");
        assert_eq!(kind(png, 1040), "image/png; charset=binary");
        assert_eq!(kind(png, 0), "PNG image data, 16 x 16, 8-bit/color RGBA, non-interlaced");
        assert_eq!(kind(b"<?php echo 1;", 16), "text/x-php");
        assert_eq!(kind(b"{\"a\": [1, 2]}", 1040), "application/json; charset=us-ascii");
        assert_eq!(kind(b"", 16), "application/x-empty");
        assert_eq!(kind(b"hello", 1024), "us-ascii");
        assert_eq!(kind(b"\x01\x02\x03\x04\x05", 16), "application/octet-stream");
    }

    #[test]
    fn a_pattern_that_backtracks_without_end_stops_the_text_rules() {
        // libmagic's C++ class rule backtracks until pcre gives up; PHP then reports plain text,
        // not the C source its `main(` rule would find next.
        let src = r####"--TEST--
Bug #69084: Unclear error message when not implementing a renamed abstract trait function
--FILE--
<?php

trait Foo {
    abstract public function doStuff();

    public function main() {
        $this->doStuff();
    }
}

class Bar {
    use Foo {
        Foo::doStuff as doOtherStuff;
    }

    public function doStuff() {
        var_dump(__FUNCTION__);
    }
}

$b = new Bar();
$b->main();

?>
--EXPECTF--
Fatal error: Class Bar contains 1 abstract method and must therefore be declared abstract or implement the remaining methods (Bar::doOtherStuff) in %s on line %d
"####;
        assert_eq!(kind(src.as_bytes(), 16), "text/plain");
        assert_eq!(kind(src.as_bytes(), 0), "ASCII text");
    }
}

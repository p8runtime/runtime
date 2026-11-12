//! XMLWriter and the xmlwriter_* functions over the ported libxml2
//! writer (writer.rs): each function is its method with the writer as
//! the first argument. Output goes to memory, a file or a PHP stream,
//! flushed as libxml2 flushes (4000 bytes at a time, on flush, at the
//! end of the document, and when the writer goes away).
//!
//! Reference: $PHP_SRC_PATH/ext/xmlwriter/php_xmlwriter.c

use crate::writer::{Res, Writer, valid_name};
use p8_runtime::ext::{Builtin, ClassSpec, Ctx, Throw};
use p8_runtime::object::{Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

/// Where a writer's output goes.
enum Sink {
    /// openMemory: the buffer outputMemory returns.
    Memory(Vec<u8>),
    File(std::fs::File),
    /// toStream: the stream resource.
    Stream(Value),
}

struct Open {
    writer: Writer,
    sink: Sink,
}

/// An XMLWriter object's state: None until it is opened.
#[derive(Default)]
pub(crate) struct State(RefCell<Option<Open>>);

pub(crate) fn make_state() -> Rc<dyn std::any::Any> {
    Rc::new(State::default())
}

/// Move what the writer holds to its sink: the bytes moved, or None
/// when the sink fails (a closed stream: libxml2's -1).
fn flush(ctx: &mut dyn Ctx, open: &mut Open) -> Option<usize> {
    let bytes = open.writer.take_pending();
    match &mut open.sink {
        Sink::Memory(buf) => buf.extend_from_slice(&bytes),
        Sink::File(f) => f.write_all(&bytes).ok()?,
        Sink::Stream(v) => {
            let o = p8_runtime::stream::resource_of(v)?;
            let r = p8_runtime::stream::resource(&o).filter(|r| !r.closed.get())?;
            // (A user-space wrapper's exception has nowhere to go here.)
            r.stream.borrow_mut().write(ctx, &bytes).ok().flatten()?;
        }
    }
    Some(bytes.len())
}

/// The writer going away (a new open, the object's end): what it holds
/// is flushed, as xmlFreeTextWriter does.
pub(crate) fn destroy(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let state = this.native::<State>().expect("an XMLWriter");
    let old = state.0.borrow_mut().take();
    if let Some(mut open) = old {
        flush(ctx, &mut open);
    }
    Ok(Value::NULL)
}

/// A call: its writer, the arguments after it, and how it was called
/// (for messages).
struct Call<'a> {
    this: Rc<Object>,
    args: &'a [Value],
    /// "xmlwriter_start_element" or "XMLWriter::startElement".
    shown: &'static str,
    /// The method's parameter names (the function's are the writer, then these).
    params: &'static [&'static str],
    method: bool,
}

impl Call<'_> {
    fn arg(&self, i: usize) -> Value {
        self.args.get(i).map(Value::deref).unwrap_or(Value::NULL)
    }

    /// A string argument (C stops at a NUL, as libxml2 sees it).
    fn str(&self, i: usize) -> Vec<u8> {
        let s = self.arg(i).to_php_string().unwrap_or_else(PhpStr::empty);
        let b = s.as_bytes();
        b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())].to_vec()
    }

    /// A nullable string argument (missing: null).
    fn opt(&self, i: usize) -> Option<Vec<u8>> {
        if self.arg(i).is_null_or_undef() { None } else { Some(self.str(i)) }
    }

    fn bool(&self, i: usize, default: bool) -> bool {
        match self.args.get(i) {
            Some(v) => v.deref().to_bool(),
            None => default,
        }
    }

    /// PHP's ValueError for an invalid name at argument `n` (counted as
    /// the function counts, the writer first).
    fn check_name(&self, ctx: &mut dyn Ctx, n: usize, subject: &str, name: &[u8]) -> Result<(), Throw> {
        if valid_name(name) {
            return Ok(());
        }
        // The method form keeps the function's numbering (PHP's quirk),
        // naming the method's parameter at that position, if it has one.
        let param = match (self.method, n) {
            (true, n) => self.params.get(n - 1).copied(),
            (false, 1) => Some("writer"),
            (false, n) => self.params.get(n - 2).copied(),
        };
        let label = match param {
            Some(p) => format!(" (${p})"),
            None => String::new(),
        };
        Err(ctx.throw(
            "ValueError",
            format!("{}(): Argument #{n}{label} must be a valid {subject}, \"{}\" given", self.shown, String::from_utf8_lossy(name)),
        ))
    }

    /// Run `f` on the open writer; flush past libxml2's buffer size.
    fn with(&self, ctx: &mut dyn Ctx, f: impl FnOnce(&mut Writer) -> Res) -> Result<Value, Throw> {
        let state = self.this.native::<State>().expect("an XMLWriter");
        let mut guard = state.0.borrow_mut();
        let Some(open) = guard.as_mut() else {
            return Err(ctx.throw("Error", "Invalid or uninitialized XMLWriter object".into()));
        };
        let ok = f(&mut open.writer).is_ok();
        if !matches!(open.sink, Sink::Memory(_)) && open.writer.wants_flush() {
            flush(ctx, open);
        }
        Ok(Value::bool(ok))
    }
}

/// The operations, each shared by its function and its method.
type Op = fn(&mut dyn Ctx, &Call) -> Result<Value, Throw>;

fn set_indent(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let on = c.bool(0, false);
    c.with(ctx, |w| {
        w.set_indent(on);
        Ok(())
    })
}

fn set_indent_string(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let s = c.str(0);
    c.with(ctx, |w| {
        w.set_indent_string(&s);
        Ok(())
    })
}

fn start_attribute(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let name = c.str(0);
    c.check_name(ctx, 2, "attribute name", &name)?;
    c.with(ctx, |w| w.start_attribute(&name))
}

fn end_attribute(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_attribute)
}

fn start_attribute_ns(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (prefix, name, uri) = (c.opt(0), c.str(1), c.opt(2));
    c.check_name(ctx, 3, "attribute name", &name)?;
    c.with(ctx, |w| w.start_attribute_ns(prefix.as_deref(), &name, uri.as_deref()))
}

fn write_attribute(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (name, value) = (c.str(0), c.str(1));
    c.check_name(ctx, 2, "attribute name", &name)?;
    c.with(ctx, |w| w.write_attribute(&name, &value))
}

fn write_attribute_ns(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (prefix, name, uri, value) = (c.opt(0), c.str(1), c.opt(2), c.str(3));
    c.check_name(ctx, 3, "attribute name", &name)?;
    c.with(ctx, |w| w.write_attribute_ns(prefix.as_deref(), &name, uri.as_deref(), &value))
}

fn start_element(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let name = c.str(0);
    c.check_name(ctx, 2, "element name", &name)?;
    c.with(ctx, |w| w.start_element(&name))
}

fn start_element_ns(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (prefix, name, uri) = (c.opt(0), c.str(1), c.opt(2));
    c.check_name(ctx, 3, "element name", &name)?;
    c.with(ctx, |w| w.start_element_ns(prefix.as_deref(), &name, uri.as_deref()))
}

fn end_element(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_element)
}

fn full_end_element(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::full_end_element)
}

fn write_element(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (name, content) = (c.str(0), c.opt(1));
    c.check_name(ctx, 2, "element name", &name)?;
    c.with(ctx, |w| w.write_element(&name, content.as_deref()))
}

fn write_element_ns(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (prefix, name, uri, content) = (c.opt(0), c.str(1), c.opt(2), c.opt(3));
    c.check_name(ctx, 3, "element name", &name)?;
    c.with(ctx, |w| match &content {
        Some(content) => w.write_element_ns(prefix.as_deref(), &name, uri.as_deref(), content),
        None => {
            w.start_element_ns(prefix.as_deref(), &name, uri.as_deref())?;
            w.end_element()
        }
    })
}

fn start_pi(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let target = c.str(0);
    c.check_name(ctx, 2, "PI target", &target)?;
    c.with(ctx, |w| w.start_pi(&target))
}

fn end_pi(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_pi)
}

fn write_pi(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (target, content) = (c.str(0), c.str(1));
    c.check_name(ctx, 2, "PI target", &target)?;
    c.with(ctx, |w| w.write_pi(&target, &content))
}

fn start_cdata(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::start_cdata)
}

fn end_cdata(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_cdata)
}

fn write_cdata(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let content = c.str(0);
    c.with(ctx, |w| w.write_cdata(&content))
}

fn write_raw(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let content = c.str(0);
    c.with(ctx, |w| w.write_raw(&content))
}

fn text(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let content = c.str(0);
    c.with(ctx, |w| w.write_string(&content))
}

fn start_comment(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::start_comment)
}

fn end_comment(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_comment)
}

fn write_comment(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let content = c.str(0);
    c.with(ctx, |w| w.write_comment(&content))
}

fn start_document(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let version = match c.args.first() {
        None => Some(b"1.0".to_vec()),
        Some(_) => c.opt(0),
    };
    let (encoding, standalone) = (c.opt(1), c.opt(2));
    c.with(ctx, |w| w.start_document(version.as_deref(), encoding.as_deref(), standalone.as_deref()))
}

fn end_document(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let r = c.with(ctx, Writer::end_document)?;
    // xmlTextWriterEndDocument flushes.
    let state = c.this.native::<State>().expect("an XMLWriter");
    if let Some(open) = state.0.borrow_mut().as_mut()
        && !matches!(open.sink, Sink::Memory(_))
    {
        flush(ctx, open);
    }
    Ok(r)
}

fn start_dtd(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (name, pubid, sysid) = (c.str(0), c.opt(1), c.opt(2));
    c.with(ctx, |w| w.start_dtd(&name, pubid.as_deref(), sysid.as_deref()))
}

fn end_dtd(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_dtd)
}

fn write_dtd(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (name, pubid, sysid, subset) = (c.str(0), c.opt(1), c.opt(2), c.opt(3));
    c.with(ctx, |w| w.write_dtd(&name, pubid.as_deref(), sysid.as_deref(), subset.as_deref()))
}

fn start_dtd_element(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let name = c.str(0);
    c.check_name(ctx, 2, "element name", &name)?;
    c.with(ctx, |w| w.start_dtd_element(&name))
}

fn end_dtd_element(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_dtd_element)
}

fn write_dtd_element(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (name, content) = (c.str(0), c.str(1));
    c.check_name(ctx, 2, "element name", &name)?;
    c.with(ctx, |w| w.write_dtd_element(&name, &content))
}

fn start_dtd_attlist(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let name = c.str(0);
    c.check_name(ctx, 2, "element name", &name)?;
    c.with(ctx, |w| w.start_dtd_attlist(&name))
}

fn end_dtd_attlist(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_dtd_attlist)
}

fn write_dtd_attlist(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (name, content) = (c.str(0), c.str(1));
    c.check_name(ctx, 2, "element name", &name)?;
    c.with(ctx, |w| w.write_dtd_attlist(&name, &content))
}

fn start_dtd_entity(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (name, pe) = (c.str(0), c.bool(1, false));
    c.check_name(ctx, 2, "attribute name", &name)?;
    c.with(ctx, |w| w.start_dtd_entity(pe, &name))
}

fn end_dtd_entity(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    c.with(ctx, Writer::end_dtd_entity)
}

fn write_dtd_entity(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    let (name, content, pe) = (c.str(0), c.str(1), c.bool(2, false));
    let (pubid, sysid, ndata) = (c.opt(3), c.opt(4), c.opt(5));
    c.check_name(ctx, 2, "element name", &name)?;
    c.with(ctx, |w| w.write_dtd_entity(pe, &name, pubid.as_deref(), sysid.as_deref(), ndata.as_deref(), Some(&content)))
}

/// flush / outputMemory: the memory buffer (emptied unless told not
/// to), or the bytes a file or stream writer flushed.
fn flush_op(ctx: &mut dyn Ctx, c: &Call, force_string: bool) -> Result<Value, Throw> {
    let empty = c.bool(0, true);
    let state = c.this.native::<State>().expect("an XMLWriter");
    let mut guard = state.0.borrow_mut();
    let Some(open) = guard.as_mut() else {
        return Err(ctx.throw("Error", "Invalid or uninitialized XMLWriter object".into()));
    };
    if force_string && !matches!(open.sink, Sink::Memory(_)) {
        return Ok(Value::str(""));
    }
    let n = flush(ctx, open);
    Ok(match &mut open.sink {
        Sink::Memory(buf) => {
            // PHP copies the buffer as a C string (up to a NUL).
            let shown = buf[..buf.iter().position(|&b| b == 0).unwrap_or(buf.len())].to_vec();
            if empty {
                buf.clear();
            }
            Value::str(PhpStr::new(shown))
        }
        _ => Value::int(n.map_or(-1, |n| n as i64)),
    })
}

fn output_memory(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    flush_op(ctx, c, true)
}

fn flush_fn(ctx: &mut dyn Ctx, c: &Call) -> Result<Value, Throw> {
    flush_op(ctx, c, false)
}

/// Open `this` on `sink` (any writer it had goes away first).
fn open_on(ctx: &mut dyn Ctx, this: &Rc<Object>, sink: Sink) -> Result<(), Throw> {
    destroy(ctx, this, &mut [])?;
    let state = this.native::<State>().expect("an XMLWriter");
    *state.0.borrow_mut() = Some(Open { writer: Writer::new(), sink });
    Ok(())
}

/// _xmlwriter_get_valid_file_path: a file path (or file:// URI) whose
/// directory exists, made absolute against `cwd`, the request's working
/// directory.
fn valid_file_path(cwd: &std::path::Path, uri: &[u8]) -> Option<std::path::PathBuf> {
    let s = String::from_utf8_lossy(uri).into_owned();
    let lower = s.to_ascii_lowercase();
    let path = if lower.starts_with("file:///") {
        if s.len() == 8 {
            return None;
        }
        s[7..].to_string()
    } else if lower.starts_with("file://localhost/") {
        if s.len() == 17 {
            return None;
        }
        s[16..].to_string()
    } else if s.contains("://") {
        // Another scheme: opened as a PHP stream (open_file).
        return Some(std::path::PathBuf::from(s));
    } else {
        s
    };
    // realpath() and expand_filepath() fail at MAXPATHLEN.
    if path.len() >= p8_runtime::stream::MAXPATHLEN {
        return None;
    }
    let path = p8_runtime::cwd::absolute(cwd, std::path::Path::new(&path));
    let dir = path.parent()?;
    dir.is_dir().then_some(path)
}

/// Open the sink `uri` names (a file, truncated; a php:// stream, through
/// PHP's streams as libxml2 is given them), or the reason not.
fn open_file(ctx: &mut dyn Ctx, uri: &[u8]) -> Result<Sink, &'static str> {
    let path = valid_file_path(ctx.cwd(), uri).ok_or("resolve")?;
    let text = path.to_string_lossy().into_owned();
    if let Some(rest) = text.to_ascii_lowercase().strip_prefix("php://") {
        use p8_runtime::stream::{Resource, Stream, Target};
        let target = match rest.split('/').next().unwrap_or("") {
            "memory" | "temp" => Target::Memory(Vec::new()),
            "stdout" | "output" => Target::Stdout,
            "stderr" => Target::Stderr,
            _ => return Err("create"),
        };
        let stream = Stream::new(target, &text, "wb");
        let resource = Resource { kind: "stream", closed: std::cell::Cell::new(false), stream: RefCell::new(stream) };
        return Ok(Sink::Stream(ctx.new_resource(resource)));
    }
    if text.contains("://") {
        return Err("create");
    }
    std::fs::File::create(path).map(Sink::File).map_err(|_| "create")
}

fn open_uri_op(ctx: &mut dyn Ctx, this: &Rc<Object>, uri: &[u8], shown: &str) -> Result<Value, Throw> {
    if uri.is_empty() {
        return Err(ctx.throw("ValueError", format!("{shown}(): Argument #1 ($uri) must not be empty")));
    }
    match open_file(ctx, uri) {
        Ok(sink) => {
            open_on(ctx, this, sink)?;
            Ok(Value::TRUE)
        }
        Err("resolve") => {
            ctx.warn(&format!("{shown}(): Unable to resolve file path"));
            Ok(Value::FALSE)
        }
        Err(_) => Ok(Value::FALSE),
    }
}

/// A call's string argument `i`, PHP's "must not contain any null bytes"
/// checked (a path).
fn path_arg(ctx: &mut dyn Ctx, args: &[Value], i: usize, shown: &str, param: &str) -> Result<Vec<u8>, Throw> {
    let s = args.get(i).map(Value::deref).unwrap_or(Value::NULL).to_php_string().unwrap_or_else(PhpStr::empty);
    if s.as_bytes().contains(&0) {
        return Err(ctx.throw("ValueError", format!("{shown}(): Argument #{} (${param}) must not contain any null bytes", i + 1)));
    }
    Ok(s.as_bytes().to_vec())
}

fn new_writer(ctx: &mut dyn Ctx) -> Result<Rc<Object>, Throw> {
    let class = ctx.class(b"XMLWriter").expect("XMLWriter is declared");
    Ok(ctx.new_object(&class))
}

/// The class a static constructor was called on (a subclass makes its own).
fn called_writer(ctx: &mut dyn Ctx) -> Result<Rc<Object>, Throw> {
    match ctx.native_class() {
        Some(class) => ctx.new_instance(&class, &mut [], true),
        None => new_writer(ctx),
    }
}

fn xmlwriter_open_uri(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let uri = path_arg(ctx, args, 0, "xmlwriter_open_uri", "uri")?;
    let o = new_writer(ctx)?;
    let r = open_uri_op(ctx, &o, &uri, "xmlwriter_open_uri")?;
    Ok(if r.to_bool() { Value::object(o) } else { r })
}

fn xmlwriter_open_memory(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let o = new_writer(ctx)?;
    open_on(ctx, &o, Sink::Memory(Vec::new()))?;
    Ok(Value::object(o))
}

fn m_open_uri(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let uri = path_arg(ctx, args, 0, "XMLWriter::openUri", "uri")?;
    open_uri_op(ctx, this, &uri, "XMLWriter::openUri")
}

fn m_open_memory(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    open_on(ctx, this, Sink::Memory(Vec::new()))?;
    Ok(Value::TRUE)
}

fn s_to_memory(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let o = called_writer(ctx)?;
    open_on(ctx, &o, Sink::Memory(Vec::new()))?;
    Ok(Value::object(o))
}

fn s_to_uri(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let uri = path_arg(ctx, args, 0, "XMLWriter::toUri", "uri")?;
    if uri.is_empty() {
        return Err(ctx.throw("ValueError", "XMLWriter::toUri(): Argument #1 ($uri) must not be empty".into()));
    }
    let sink = match open_file(ctx, &uri) {
        Ok(f) => f,
        Err("resolve") => return Err(ctx.throw("ValueError", "XMLWriter::toUri(): Argument #1 ($uri) must resolve to a valid file path".into())),
        Err(_) => return Err(ctx.throw("Error", "Could not construct libxml writer".into())),
    };
    let o = called_writer(ctx)?;
    open_on(ctx, &o, sink)?;
    Ok(Value::object(o))
}

fn s_to_stream(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = args.first().map(Value::deref).unwrap_or(Value::NULL);
    let Some(res) = p8_runtime::stream::resource_of(&v) else {
        return Err(ctx.throw(
            "TypeError",
            format!("XMLWriter::toStream(): Argument #1 ($stream) must be of type resource, {} given", v.type_name()),
        ));
    };
    if p8_runtime::stream::resource(&res).is_none_or(|r| r.closed.get()) {
        return Err(ctx.throw("TypeError", "XMLWriter::toStream(): supplied resource is not a valid stream resource".into()));
    }
    let o = called_writer(ctx)?;
    open_on(ctx, &o, Sink::Stream(v))?;
    Ok(Value::object(o))
}

/// Run an operation as its function: the writer is argument 0.
fn as_function(ctx: &mut dyn Ctx, fname: &'static str, params: &'static [&'static str], op: Op, args: &[Value]) -> Result<Value, Throw> {
    let writer = args.first().map(Value::deref).unwrap_or(Value::NULL);
    let this = match writer.as_object() {
        Some(o) if o.native::<State>().is_some() => Rc::clone(o),
        Some(o) => {
            let given = String::from_utf8_lossy(o.class.name.as_bytes()).into_owned();
            return Err(ctx.throw("TypeError", format!("{fname}(): Argument #1 ($writer) must be of type XMLWriter, {given} given")));
        }
        None => return Err(ctx.throw("TypeError", format!("{fname}(): Argument #1 ($writer) must be of type XMLWriter, {} given", writer.type_name()))),
    };
    op(ctx, &Call { this, args: &args[1..], shown: fname, params, method: false })
}

/// Each operation as a function and a method, and the tables of both:
/// `fn_name method_name "function" "Class::method" [params] op;`
macro_rules! operations {
    ($($f:ident $m:ident $fname:literal $mname:literal $method:literal [$($p:literal),*] $op:ident;)*) => {
        $(
            fn $f(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
                as_function(ctx, $fname, &[$($p),*], $op, args)
            }
            fn $m(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
                $op(ctx, &Call { this: Rc::clone(this), args, shown: $method, params: &[$($p),*], method: true })
            }
        )*
        pub(crate) static BUILTINS: &[Builtin] = &[
            Builtin { name: "xmlwriter_open_uri", run: xmlwriter_open_uri, by_ref: &[] },
            Builtin { name: "xmlwriter_open_memory", run: xmlwriter_open_memory, by_ref: &[] },
            $(Builtin { name: $fname, run: $f, by_ref: &[] },)*
        ];
        static METHODS: &[(&str, Option<Native>)] = &[
            ("openUri", Some(Native::Method(m_open_uri))),
            ("openMemory", Some(Native::Method(m_open_memory))),
            ("toStream", Some(Native::Static(s_to_stream))),
            ("toUri", Some(Native::Static(s_to_uri))),
            ("toMemory", Some(Native::Static(s_to_memory))),
            $(($mname, Some(Native::Method($m))),)*
        ];
    };
}

#[rustfmt::skip]
operations! {
    f_set_indent m_set_indent "xmlwriter_set_indent" "setIndent" "XMLWriter::setIndent" ["enable"] set_indent;
    f_set_indent_string m_set_indent_string "xmlwriter_set_indent_string" "setIndentString" "XMLWriter::setIndentString" ["indentation"] set_indent_string;
    f_start_comment m_start_comment "xmlwriter_start_comment" "startComment" "XMLWriter::startComment" [] start_comment;
    f_end_comment m_end_comment "xmlwriter_end_comment" "endComment" "XMLWriter::endComment" [] end_comment;
    f_start_attribute m_start_attribute "xmlwriter_start_attribute" "startAttribute" "XMLWriter::startAttribute" ["name"] start_attribute;
    f_end_attribute m_end_attribute "xmlwriter_end_attribute" "endAttribute" "XMLWriter::endAttribute" [] end_attribute;
    f_write_attribute m_write_attribute "xmlwriter_write_attribute" "writeAttribute" "XMLWriter::writeAttribute" ["name", "value"] write_attribute;
    f_start_attribute_ns m_start_attribute_ns "xmlwriter_start_attribute_ns" "startAttributeNs" "XMLWriter::startAttributeNs" ["prefix", "name", "namespace"] start_attribute_ns;
    f_write_attribute_ns m_write_attribute_ns "xmlwriter_write_attribute_ns" "writeAttributeNs" "XMLWriter::writeAttributeNs" ["prefix", "name", "namespace", "value"] write_attribute_ns;
    f_start_element m_start_element "xmlwriter_start_element" "startElement" "XMLWriter::startElement" ["name"] start_element;
    f_end_element m_end_element "xmlwriter_end_element" "endElement" "XMLWriter::endElement" [] end_element;
    f_full_end_element m_full_end_element "xmlwriter_full_end_element" "fullEndElement" "XMLWriter::fullEndElement" [] full_end_element;
    f_start_element_ns m_start_element_ns "xmlwriter_start_element_ns" "startElementNs" "XMLWriter::startElementNs" ["prefix", "name", "namespace"] start_element_ns;
    f_write_element m_write_element "xmlwriter_write_element" "writeElement" "XMLWriter::writeElement" ["name", "content"] write_element;
    f_write_element_ns m_write_element_ns "xmlwriter_write_element_ns" "writeElementNs" "XMLWriter::writeElementNs" ["prefix", "name", "namespace", "content"] write_element_ns;
    f_start_pi m_start_pi "xmlwriter_start_pi" "startPi" "XMLWriter::startPi" ["target"] start_pi;
    f_end_pi m_end_pi "xmlwriter_end_pi" "endPi" "XMLWriter::endPi" [] end_pi;
    f_write_pi m_write_pi "xmlwriter_write_pi" "writePi" "XMLWriter::writePi" ["target", "content"] write_pi;
    f_start_cdata m_start_cdata "xmlwriter_start_cdata" "startCdata" "XMLWriter::startCdata" [] start_cdata;
    f_end_cdata m_end_cdata "xmlwriter_end_cdata" "endCdata" "XMLWriter::endCdata" [] end_cdata;
    f_write_cdata m_write_cdata "xmlwriter_write_cdata" "writeCdata" "XMLWriter::writeCdata" ["content"] write_cdata;
    f_text m_text "xmlwriter_text" "text" "XMLWriter::text" ["content"] text;
    f_write_raw m_write_raw "xmlwriter_write_raw" "writeRaw" "XMLWriter::writeRaw" ["content"] write_raw;
    f_start_document m_start_document "xmlwriter_start_document" "startDocument" "XMLWriter::startDocument" ["version", "encoding", "standalone"] start_document;
    f_end_document m_end_document "xmlwriter_end_document" "endDocument" "XMLWriter::endDocument" [] end_document;
    f_write_comment m_write_comment "xmlwriter_write_comment" "writeComment" "XMLWriter::writeComment" ["content"] write_comment;
    f_start_dtd m_start_dtd "xmlwriter_start_dtd" "startDtd" "XMLWriter::startDtd" ["qualifiedName", "publicId", "systemId"] start_dtd;
    f_end_dtd m_end_dtd "xmlwriter_end_dtd" "endDtd" "XMLWriter::endDtd" [] end_dtd;
    f_write_dtd m_write_dtd "xmlwriter_write_dtd" "writeDtd" "XMLWriter::writeDtd" ["name", "publicId", "systemId", "content"] write_dtd;
    f_start_dtd_element m_start_dtd_element "xmlwriter_start_dtd_element" "startDtdElement" "XMLWriter::startDtdElement" ["qualifiedName"] start_dtd_element;
    f_end_dtd_element m_end_dtd_element "xmlwriter_end_dtd_element" "endDtdElement" "XMLWriter::endDtdElement" [] end_dtd_element;
    f_write_dtd_element m_write_dtd_element "xmlwriter_write_dtd_element" "writeDtdElement" "XMLWriter::writeDtdElement" ["name", "content"] write_dtd_element;
    f_start_dtd_attlist m_start_dtd_attlist "xmlwriter_start_dtd_attlist" "startDtdAttlist" "XMLWriter::startDtdAttlist" ["name"] start_dtd_attlist;
    f_end_dtd_attlist m_end_dtd_attlist "xmlwriter_end_dtd_attlist" "endDtdAttlist" "XMLWriter::endDtdAttlist" [] end_dtd_attlist;
    f_write_dtd_attlist m_write_dtd_attlist "xmlwriter_write_dtd_attlist" "writeDtdAttlist" "XMLWriter::writeDtdAttlist" ["name", "content"] write_dtd_attlist;
    f_start_dtd_entity m_start_dtd_entity "xmlwriter_start_dtd_entity" "startDtdEntity" "XMLWriter::startDtdEntity" ["name", "isParam"] start_dtd_entity;
    f_end_dtd_entity m_end_dtd_entity "xmlwriter_end_dtd_entity" "endDtdEntity" "XMLWriter::endDtdEntity" [] end_dtd_entity;
    f_write_dtd_entity m_write_dtd_entity "xmlwriter_write_dtd_entity" "writeDtdEntity" "XMLWriter::writeDtdEntity" ["name", "content", "isParam", "publicId", "systemId", "notationData"] write_dtd_entity;
    f_output_memory m_output_memory "xmlwriter_output_memory" "outputMemory" "XMLWriter::outputMemory" ["flush"] output_memory;
    f_flush m_flush "xmlwriter_flush" "flush" "XMLWriter::flush" ["empty"] flush_fn;
}

pub(crate) static CLASSES: &[ClassSpec] =
    &[ClassSpec { name: "XMLWriter", parent: None, interfaces: &[], flags: p8_runtime::object::CLASS_UNCLONEABLE, properties: &[], methods: METHODS, constants: &[], debug_info: None }];

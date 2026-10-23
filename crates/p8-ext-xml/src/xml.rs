//! The xml extension: xml_parser_create and friends, an expat-style
//! event API over the parser, as PHP builds it on libxml2 (its compat
//! layer: OLDSAX, entities substituted, SAX1 unless namespaces are on,
//! and `wellFormed` cleared, so references reach PHP through getEntity).
//!
//! Reference: $PHP_SRC_PATH/ext/xml/xml.c, compat.c

use crate::parser::{Attr, Entity, NsBinding, EntityKind, Options, Parser, Sax, LEVEL_WARNING};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Ctx, Throw};
use p8_runtime::object::{CLASS_FINAL, CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

/// The handlers, in this order.
const START: usize = 0;
const END: usize = 1;
const CDATA: usize = 2;
const PI: usize = 3;
const DEFAULT: usize = 4;
const UNPARSED: usize = 5;
const NOTATION: usize = 6;
const EXTERNAL: usize = 7;
const START_NS: usize = 8;
const END_NS: usize = 9;

/// What xml_parse_into_struct builds.
struct Struct {
    /// The by-reference arguments it fills: once a handler puts something
    /// other than an array there, PHP stops writing to it.
    values_ref: Option<Rc<RefCell<Value>>>,
    index_ref: Option<Rc<RefCell<Value>>>,
    values: Vec<PhpArray>,
    index: Option<PhpArray>,
    /// The tag each open level has (decoded, case folded).
    ltags: Vec<Vec<u8>>,
    lastwasopen: bool,
    curtag: i64,
}

struct XmlParser {
    /// The PHP callables (None: none set), and whether each was given as
    /// a method name of the xml_set_object object.
    handlers: [Option<Value>; 10],
    by_name: [bool; 10],
    /// The function parsing ("xml_parse"), for its warnings.
    calling: &'static str,
    /// Whether the C-level handler is installed (xml_set_*_handler was
    /// called, even with null): it decides where events go.
    installed: [bool; 10],
    object: Option<Value>,
    /// xml_parser_create_ns's separator.
    separator: Option<u8>,
    target_encoding: &'static str,
    case_folding: bool,
    skip_white: bool,
    tag_start: usize,
    parse_huge: bool,
    parsing: bool,
    /// The input so far (each call parses it again, from the start).
    buffer: Vec<u8>,
    /// Events already handed to PHP by earlier, non-final calls.
    delivered: usize,
    /// The final chunk was parsed.
    done: bool,
    err_no: i32,
    line: i64,
    column: i64,
    byte_index: i64,
    level: i64,
    structure: Option<Struct>,
}

#[derive(Default)]
pub(crate) struct State(RefCell<Option<XmlParser>>);

pub(crate) fn make_state() -> Rc<dyn std::any::Any> {
    Rc::new(State::default())
}

fn with<R>(o: &Object, f: impl FnOnce(&mut XmlParser) -> R) -> R {
    let state = o.native::<State>().expect("an XMLParser");
    let mut guard = state.0.borrow_mut();
    f(guard.as_mut().expect("an XMLParser made by xml_parser_create"))
}

fn parser_arg(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<Rc<Object>, Throw> {
    match args.first().map(Value::deref) {
        Some(Value::Object(o)) if o.native::<State>().is_some_and(|s| s.0.borrow().is_some()) => Ok(o),
        v => {
            let given = match &v {
                Some(Value::Object(o)) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                Some(v) => v.type_name().into_owned(),
                None => "null".into(),
            };
            Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($parser) must be of type XMLParser, {given} given")))
        }
    }
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

fn bytes(v: &Value) -> Vec<u8> {
    v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()
}

/// xml_parser_create / xml_parser_create_ns.
fn create(ctx: &mut dyn Ctx, args: &[Value], ns: bool, func: &str) -> Result<Value, Throw> {
    let target_encoding = match args.first().map(Value::deref) {
        None | Some(Value::Null) => "UTF-8",
        Some(v) => {
            let e = bytes(&v);
            if e.is_empty() || e.eq_ignore_ascii_case(b"UTF-8") {
                "UTF-8"
            } else if e.eq_ignore_ascii_case(b"ISO-8859-1") {
                "ISO-8859-1"
            } else if e.eq_ignore_ascii_case(b"US-ASCII") {
                "US-ASCII"
            } else {
                return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($encoding) is not a supported source encoding")));
            }
        }
    };
    let separator = ns.then(|| match args.get(1).map(Value::deref) {
        Some(v) => bytes(&v).first().copied().unwrap_or(0),
        None => b':',
    });
    let class = ctx.class_load(b"XMLParser")?.expect("XMLParser is declared");
    let o = ctx.new_object(&class);
    let state = o.native::<State>().expect("an XMLParser");
    *state.0.borrow_mut() = Some(XmlParser {
        handlers: Default::default(),
        by_name: [false; 10],
        calling: "xml_parse",
        installed: [false; 10],
        object: None,
        separator,
        target_encoding,
        case_folding: true,
        skip_white: false,
        tag_start: 0,
        parse_huge: false,
        parsing: false,
        buffer: Vec::new(),
        delivered: 0,
        done: false,
        err_no: 0,
        line: 1,
        column: 1,
        byte_index: 0,
        level: 0,
        structure: None,
    });
    Ok(Value::Object(o))
}

fn xml_parser_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    create(ctx, args, false, "xml_parser_create")
}

fn xml_parser_create_ns(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    create(ctx, args, true, "xml_parser_create_ns")
}

/// A handler argument: a callable, null (none), or (deprecated) the name
/// of a method of the xml_set_object object: (handler, given by name).
fn handler_value(ctx: &mut dyn Ctx, o: &Object, func: &str, arg_num: usize, param: &str, v: Value) -> Result<(Option<Value>, bool), Throw> {
    match &v {
        Value::Null => Ok((None, false)),
        _ if ctx.is_callable(&v) => Ok((Some(v), false)),
        Value::Str(s) => {
            if s.is_empty() {
                return Ok((None, false));
            }
            let Some(object) = with(o, |p| p.object.clone()) else {
                return Err(ctx.throw("ValueError", format!("{func}(): Argument #{arg_num} (${param}) an object must be set via xml_set_object() to be able to lookup method")));
            };
            let Value::Object(obj) = object.deref() else { return Ok((None, false)) };
            if obj.class.method(&s.as_bytes().to_ascii_lowercase()).is_none() {
                return Err(ctx.throw(
                    "ValueError",
                    format!(
                        "{func}(): Argument #{arg_num} (${param}) method {}::{}() does not exist",
                        String::from_utf8_lossy(obj.class.name.as_bytes()),
                        String::from_utf8_lossy(s.as_bytes())
                    ),
                ));
            }
            Ok((Some(Value::Array(Rc::new(PhpArray::from_list(vec![object.clone(), v.clone()])))), true))
        }
        _ => Err(ctx.throw("TypeError", format!("{func}(): Argument #{arg_num} (${param}) must be of type callable|string|null"))),
    }
}

/// Whether a handler argument is a string that is not callable (PHP's
/// "Passing non-callable strings is deprecated").
fn non_callable_string(ctx: &mut dyn Ctx, v: &Value) -> bool {
    matches!(v, Value::Str(_)) && !ctx.is_callable(v)
}

fn set_handler(ctx: &mut dyn Ctx, args: &[Value], which: usize, func: &str) -> Result<Value, Throw> {
    let o = parser_arg(ctx, func, args)?;
    let v = arg(args, 1);
    if !matches!(v, Value::Null | Value::Str(_)) && !ctx.is_callable(&v) {
        return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($handler) must be of type callable|string|null")));
    }
    if non_callable_string(ctx, &v) {
        ctx.report(8192, &format!("{func}(): Passing non-callable strings is deprecated since 8.4"));
    }
    let (h, by_name) = handler_value(ctx, &o, func, 2, "handler", v)?;
    if with(&o, |p| p.parsing) {
        return Err(ctx.throw("Error", "Cannot change handlers while parsing".into()));
    }
    with(&o, |p| {
        p.handlers[which] = h;
        p.by_name[which] = by_name;
        p.installed[which] = true;
    });
    Ok(Value::Bool(true))
}

fn xml_set_element_handler(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let func = "xml_set_element_handler";
    let o = parser_arg(ctx, func, args)?;
    let (a, b) = (arg(args, 1), arg(args, 2));
    for (i, (v, param)) in [(&a, "start_handler"), (&b, "end_handler")].into_iter().enumerate() {
        if !matches!(v, Value::Null | Value::Str(_)) && !ctx.is_callable(v) {
            return Err(ctx.throw("TypeError", format!("{func}(): Argument #{} (${param}) must be of type callable|string|null", i + 2)));
        }
    }
    if non_callable_string(ctx, &a) || non_callable_string(ctx, &b) {
        ctx.report(8192, &format!("{func}(): Passing non-callable strings is deprecated since 8.4"));
    }
    let (start, start_named) = handler_value(ctx, &o, func, 2, "start_handler", a)?;
    let (end, end_named) = handler_value(ctx, &o, func, 3, "end_handler", b)?;
    if with(&o, |p| p.parsing) {
        return Err(ctx.throw("Error", "Cannot change handlers while parsing".into()));
    }
    with(&o, |p| {
        p.handlers[START] = start;
        p.handlers[END] = end;
        p.by_name[START] = start_named;
        p.by_name[END] = end_named;
        p.installed[START] = true;
        p.installed[END] = true;
    });
    Ok(Value::Bool(true))
}

macro_rules! setter {
    ($name:ident, $which:expr) => {
        fn $name(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
            set_handler(ctx, args, $which, stringify!($name))
        }
    };
}

setter!(xml_set_character_data_handler, CDATA);
setter!(xml_set_processing_instruction_handler, PI);
setter!(xml_set_default_handler, DEFAULT);
setter!(xml_set_unparsed_entity_decl_handler, UNPARSED);
setter!(xml_set_notation_decl_handler, NOTATION);
setter!(xml_set_external_entity_ref_handler, EXTERNAL);
setter!(xml_set_start_namespace_decl_handler, START_NS);
setter!(xml_set_end_namespace_decl_handler, END_NS);

/// The xml_set_*_handler function that sets each handler (for messages).
const SETTERS: [&str; 10] = [
    "xml_set_element_handler()",
    "xml_set_element_handler()",
    "xml_set_character_data_handler()",
    "xml_set_processing_instruction_handler()",
    "xml_set_default_handler()",
    "xml_set_unparsed_entity_decl_handler()",
    "xml_set_notation_decl_handler()",
    "xml_set_external_entity_ref_handler()",
    "xml_set_start_namespace_decl_handler()",
    "xml_set_end_namespace_decl_handler()",
];

/// xml_set_object: methods named by string are looked up on it (those
/// already set by name move to it).
fn xml_set_object(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_set_object", args)?;
    let target = arg(args, 1);
    let Value::Object(new) = &target else {
        return Err(ctx.throw("TypeError", format!("xml_set_object(): Argument #2 ($object) must be of type object, {} given", target.type_name())));
    };
    for (i, setter) in SETTERS.iter().enumerate() {
        let (handler, named) = with(&o, |p| (p.handlers[i].clone(), p.by_name[i]));
        let (Some(Value::Array(a)), true) = (handler, named) else { continue };
        let method = a.get(&Key::Int(1)).cloned().unwrap_or(Value::Null);
        let name = method.to_php_string().unwrap_or_else(PhpStr::empty);
        if new.class.method(&name.as_bytes().to_ascii_lowercase()).is_none() {
            return Err(ctx.throw(
                "ValueError",
                format!(
                    "xml_set_object(): Argument #2 ($object) cannot safely swap to object of class {} as method \"{}\" does not exist, which was set via {}",
                    String::from_utf8_lossy(new.class.name.as_bytes()),
                    String::from_utf8_lossy(name.as_bytes()),
                    setter
                ),
            ));
        }
        with(&o, |p| p.handlers[i] = Some(Value::Array(Rc::new(PhpArray::from_list(vec![target.clone(), method])))));
    }
    with(&o, |p| p.object = Some(target.clone()));
    Ok(Value::Bool(true))
}


/// PHP's target encoding of UTF-8 text (unrepresentable: '?').
fn decode(s: &[u8], target: &str) -> Vec<u8> {
    let limit = match target {
        "ISO-8859-1" => 0xFF,
        "US-ASCII" => 0x7F,
        _ => return s.to_vec(),
    };
    let mut out = Vec::with_capacity(s.len());
    let text = String::from_utf8_lossy(s);
    for ch in text.chars() {
        let c = u32::from(ch);
        out.push(if c == 0xFFFD || c > limit { b'?' } else { c as u8 });
    }
    out
}

/// The events, delivered to the parser object's PHP handlers.
struct Events<'a> {
    ctx: &'a mut dyn Ctx,
    this: Rc<Object>,
    /// An exception a handler threw: later handlers are not called.
    thrown: Option<Throw>,
    ns: Option<u8>,
    /// Events seen in this parse, and how many an earlier call delivered.
    seen: usize,
    skip: usize,
}

impl Events<'_> {
    /// Count an event: true when an earlier call already delivered it.
    fn replayed(&mut self) -> bool {
        self.seen += 1;
        self.seen <= self.skip
    }

    fn p<R>(&self, f: impl FnOnce(&mut XmlParser) -> R) -> R {
        with(&self.this, f)
    }

    fn sync_position(&mut self, parser: &Parser) {
        let (line, col, byte) = (parser.line(), parser.column(), parser.byte_index());
        self.p(|x| {
            x.line = i64::from(line);
            x.column = i64::from(col);
            x.byte_index = byte as i64;
        });
    }

    /// Call handler `which` with the parser and `args`.
    fn call(&mut self, which: usize, mut args: Vec<Value>) -> Option<Value> {
        if self.thrown.is_some() {
            return None;
        }
        let handler = self.p(|x| x.handlers[which].clone())?;
        args.insert(0, Value::Object(Rc::clone(&self.this)));
        match self.ctx.call(&handler, &mut args) {
            Ok(v) => Some(v),
            Err(t) => {
                self.thrown = Some(t);
                None
            }
        }
    }

    fn installed(&self, which: usize) -> bool {
        self.p(|x| x.installed[which])
    }

    fn text(&self, s: &[u8]) -> Value {
        let target = self.p(|x| x.target_encoding);
        Value::Str(PhpStr::new(decode(s, target)))
    }

    fn opt_text(&self, s: Option<&[u8]>) -> Value {
        match s {
            Some(s) => self.text(s),
            None => Value::Bool(false),
        }
    }

    /// xml_decode_tag: decoded, upper-cased when case folding.
    fn tag(&self, name: &[u8]) -> Vec<u8> {
        let (target, fold) = self.p(|x| (x.target_encoding, x.case_folding));
        let mut t = decode(name, target);
        if fold {
            t.make_ascii_uppercase();
        }
        t
    }

    /// SKIP_TAGSTART.
    fn skipped(&self, tag: &[u8]) -> Vec<u8> {
        let skip = self.p(|x| x.tag_start).min(tag.len());
        tag[skip..].to_vec()
    }

    fn default(&mut self, s: &[u8]) {
        let v = self.text(s);
        self.call(DEFAULT, vec![v]);
    }

    /// "uri<sep>name", or the name.
    fn qualify(&self, name: &[u8], uri: Option<&[u8]>) -> Vec<u8> {
        match (uri, self.ns) {
            (Some(u), Some(sep)) => [u, &[sep][..], name].concat(),
            _ => name.to_vec(),
        }
    }

    fn start_element(&mut self, name: &[u8], attrs: Vec<(Vec<u8>, Vec<u8>)>) {
        if self.thrown.is_some() {
            return;
        }
        let level = self.p(|x| {
            x.level += 1;
            x.level
        });
        let tag = self.tag(name);
        let mut attributes = PhpArray::new();
        for (k, v) in &attrs {
            let key = self.tag(k);
            attributes.insert(Key::from_string(PhpStr::new(key)), self.text(v));
        }
        if self.p(|x| x.handlers[START].is_some()) {
            let skipped = Value::Str(PhpStr::new(self.skipped(&tag)));
            self.call(START, vec![skipped, Value::Array(Rc::new(attributes.clone()))]);
        }
        if self.thrown.is_some() {
            return;
        }
        let skipped = self.skipped(&tag);
        let warn = self.p(|x| {
            let Some(s) = x.structure.as_mut().filter(|s| s.writable()) else { return false };
            if level > 255 {
                return level == 256;
            }
            add_to_info(s, &skipped);
            let mut entry = PhpArray::new();
            entry.insert(Key::from_string(PhpStr::from("tag")), Value::Str(PhpStr::new(skipped.clone())));
            entry.insert(Key::from_string(PhpStr::from("type")), Value::str("open"));
            entry.insert(Key::from_string(PhpStr::from("level")), Value::Int(level));
            if !attributes.is_empty() {
                entry.insert(Key::from_string(PhpStr::from("attributes")), Value::Array(Rc::new(attributes)));
            }
            s.ltags.truncate(level as usize - 1);
            s.ltags.push(tag.clone());
            s.lastwasopen = true;
            s.values.push(entry);
            false
        });
        if warn {
            let func = self.p(|x| x.calling);
            self.ctx.warn(&format!("{func}(): Maximum depth exceeded - Results truncated"));
        }
    }

    fn end_element(&mut self, name: &[u8]) {
        if self.thrown.is_some() {
            return;
        }
        let tag = self.tag(name);
        if self.p(|x| x.handlers[END].is_some()) {
            let skipped = Value::Str(PhpStr::new(self.skipped(&tag)));
            self.call(END, vec![skipped]);
        }
        if self.thrown.is_some() {
            return;
        }
        let skipped = self.skipped(&tag);
        self.p(|x| {
            let level = x.level;
            if let Some(s) = x.structure.as_mut().filter(|s| s.writable()) {
                if s.lastwasopen {
                    if let Some(last) = s.values.last_mut() {
                        last.insert(Key::from_string(PhpStr::from("type")), Value::str("complete"));
                    }
                } else {
                    add_to_info(s, &skipped);
                    let mut entry = PhpArray::new();
                    entry.insert(Key::from_string(PhpStr::from("tag")), Value::Str(PhpStr::new(skipped.clone())));
                    entry.insert(Key::from_string(PhpStr::from("type")), Value::str("close"));
                    entry.insert(Key::from_string(PhpStr::from("level")), Value::Int(level));
                    s.values.push(entry);
                }
                s.lastwasopen = false;
            }
            x.level -= 1;
        });
    }

    fn character_data(&mut self, s: &[u8]) {
        if self.thrown.is_some() {
            return;
        }
        if self.p(|x| x.handlers[CDATA].is_some()) {
            let v = self.text(s);
            self.call(CDATA, vec![v]);
        }
        if self.thrown.is_some() {
            return;
        }
        let decoded = decode(s, self.p(|x| x.target_encoding));
        let warn = self.p(|x| {
            let skip_white = x.skip_white;
            let level = x.level;
            let tag_start = x.tag_start;
            let Some(st) = x.structure.as_mut().filter(|s| s.writable()) else { return false };
            let printable = decoded.iter().any(|c| !matches!(c, b' ' | b'\t' | b'\n'));
            let value_key = Key::from_string(PhpStr::from("value"));
            if st.lastwasopen {
                let Some(ctag) = st.values.last_mut() else { return false };
                match ctag.get(&value_key) {
                    Some(Value::Str(old)) => {
                        let joined = [old.as_bytes(), &decoded].concat();
                        ctag.insert(value_key, Value::Str(PhpStr::new(joined)));
                    }
                    _ => {
                        if printable || !skip_white {
                            ctag.insert(value_key, Value::Str(PhpStr::new(decoded)));
                        }
                    }
                }
                return false;
            }
            if let Some(last) = st.values.last_mut()
                && matches!(last.get(&Key::from_string(PhpStr::from("type"))), Some(Value::Str(t)) if t.as_bytes() == b"cdata")
                && let Some(Value::Str(old)) = last.get(&value_key)
            {
                let joined = [old.as_bytes(), &decoded].concat();
                last.insert(value_key, Value::Str(PhpStr::new(joined)));
                return false;
            }
            if level > 0 && level <= 255 && (printable || !skip_white) {
                let tag = st.ltags.get(level as usize - 1).cloned().unwrap_or_default();
                let tag = tag[tag_start.min(tag.len())..].to_vec();
                add_to_info(st, &tag);
                let mut entry = PhpArray::new();
                entry.insert(Key::from_string(PhpStr::from("tag")), Value::Str(PhpStr::new(tag)));
                entry.insert(value_key, Value::Str(PhpStr::new(decoded)));
                entry.insert(Key::from_string(PhpStr::from("type")), Value::str("cdata"));
                entry.insert(Key::from_string(PhpStr::from("level")), Value::Int(level));
                st.values.push(entry);
                return false;
            }
            level == 256
        });
        if warn {
            let func = self.p(|x| x.calling);
            self.ctx.warn(&format!("{func}(): Maximum depth exceeded - Results truncated"));
        }
    }
}

impl Struct {
    /// The values argument still holds an array.
    fn writable(&self) -> bool {
        self.values_ref.as_ref().is_none_or(|c| matches!(*c.borrow(), Value::Array(_)))
    }
}

/// xml_add_to_info: the index of each tag's entries.
fn add_to_info(s: &mut Struct, name: &[u8]) {
    let curtag = s.curtag;
    if let Some(index) = s.index.as_mut() {
        let key = Key::from_string(PhpStr::new(name.to_vec()));
        let mut list = match index.get(&key) {
            Some(Value::Array(a)) => (**a).clone(),
            _ => PhpArray::new(),
        };
        let _ = list.push(Value::Int(curtag));
        index.insert(key, Value::Array(Rc::new(list)));
    }
    s.curtag += 1;
}

impl Sax for Events<'_> {
    fn start_element_ns(&mut self, p: &mut Parser, local: &[u8], _prefix: Option<&[u8]>, uri: Option<&[u8]>, namespaces: &[NsBinding], attrs: &[Attr]) {
        self.sync_position(p);
        if self.replayed() {
            return;
        }
        if self.ns.is_some() && self.installed(START_NS) {
            for (prefix, uri) in namespaces {
                let (a, b) = (self.opt_text(prefix.as_deref()), self.text(uri));
                self.call(START_NS, vec![a, b]);
            }
        }
        if !self.installed(START) {
            if self.installed(DEFAULT) {
                let mut raw = p.start_tag_text();
                if raw.last() == Some(&b'/') {
                    *raw.last_mut().expect("a byte") = b'>';
                }
                self.default(&raw);
            }
            return;
        }
        let name = self.qualify(local, uri);
        let attrs: Vec<(Vec<u8>, Vec<u8>)> = attrs
            .iter()
            .map(|a| {
                let n = if a.prefix.is_some() { self.qualify(&a.local, a.uri.as_deref()) } else { a.local.clone() };
                (n, a.value.clone())
            })
            .collect();
        self.start_element(&name, attrs);
    }

    fn end_element_ns(&mut self, p: &mut Parser, local: &[u8], prefix: Option<&[u8]>, uri: Option<&[u8]>) {
        self.sync_position(p);
        if self.replayed() {
            return;
        }
        if !self.installed(END) {
            if self.installed(DEFAULT) {
                let tag = match (prefix, self.ns) {
                    (Some(pr), Some(_)) => [b"</", pr, b":", local, b">"].concat(),
                    _ => [b"</", local, b">"].concat(),
                };
                self.default(&tag);
            }
            return;
        }
        let name = self.qualify(local, uri);
        self.end_element(&name);
    }

    fn characters(&mut self, p: &mut Parser, text: &[u8]) {
        self.sync_position(p);
        if self.replayed() {
            return;
        }
        if !self.installed(CDATA) {
            if self.installed(DEFAULT) {
                self.default(text);
            }
            return;
        }
        self.character_data(text);
    }

    fn processing_instruction(&mut self, p: &mut Parser, target: &[u8], data: Option<&[u8]>) {
        self.sync_position(p);
        if self.replayed() {
            return;
        }
        if !self.installed(PI) {
            if self.installed(DEFAULT) {
                let data = data.map_or(b"(null)".to_vec(), <[u8]>::to_vec);
                self.default(&[b"<?", target, b" ", &data[..], b"?>"].concat());
            }
            return;
        }
        let (t, d) = (self.text(target), self.opt_text(data));
        self.call(PI, vec![t, d]);
    }

    fn comment(&mut self, p: &mut Parser, text: &[u8]) {
        self.sync_position(p);
        if self.replayed() {
            return;
        }
        if self.installed(DEFAULT) {
            self.default(&[b"<!--", text, b"-->"].concat());
        }
    }

    fn unparsed_entity_decl(&mut self, p: &mut Parser, name: &[u8], public_id: Option<&[u8]>, system_id: Option<&[u8]>, notation: &[u8]) {
        self.sync_position(p);
        if self.replayed() {
            return;
        }
        if self.installed(UNPARSED) {
            let args = vec![self.text(name), Value::Bool(false), self.opt_text(system_id), self.opt_text(public_id), self.text(notation)];
            self.call(UNPARSED, args);
        }
    }

    fn notation_decl(&mut self, p: &mut Parser, name: &[u8], public_id: Option<&[u8]>, system_id: Option<&[u8]>) {
        self.sync_position(p);
        if self.replayed() {
            return;
        }
        if self.installed(NOTATION) {
            let args = vec![self.text(name), Value::Bool(false), self.opt_text(system_id), self.opt_text(public_id)];
            self.call(NOTATION, args);
        }
    }

    /// compat.c's get_entity: references reach PHP from here (the default
    /// handler gets `&name;`, else the character data handler the
    /// replacement text), and external entities go to their handler.
    fn get_entity(&mut self, p: &mut Parser, name: &[u8]) -> Option<Entity> {
        if p.in_subset != 0 {
            return None;
        }
        self.sync_position(p);
        let predefined: Option<&[u8]> = match name {
            b"lt" => Some(b"<"),
            b"gt" => Some(b">"),
            b"amp" => Some(b"&"),
            b"apos" => Some(b"'"),
            b"quot" => Some(b"\""),
            _ => None,
        };
        let ret = match predefined {
            Some(t) => Some(Entity { name: name.to_vec(), kind: EntityKind::Predefined, external_id: None, system_id: None, content: Some(t.to_vec()), notation: None, checked: 0 }),
            None => p.entities.get(name).cloned(),
        };
        if (ret.is_none() || p.instate == crate::parser::State::Content) && !self.replayed() {
            let internal = ret.as_ref().is_none_or(|e| matches!(e.kind, EntityKind::InternalGeneral | EntityKind::InternalParameter | EntityKind::Predefined));
            if internal {
                let is_predefined = ret.as_ref().is_some_and(|e| e.kind == EntityKind::Predefined);
                if self.installed(DEFAULT) && !(is_predefined && self.installed(CDATA)) {
                    self.default(&[b"&", name, b";"].concat());
                } else if self.installed(CDATA)
                    && let Some(e) = &ret
                {
                    let content = e.content.clone().unwrap_or_default();
                    self.character_data(&content);
                }
            } else if let Some(e) = &ret
                && e.kind == EntityKind::ExternalGeneralParsed
                && self.installed(EXTERNAL)
            {
                let args = vec![self.text(name), Value::str(""), self.opt_text(e.system_id.as_deref()), self.opt_text(e.external_id.as_deref())];
                let ok = self.call(EXTERNAL, args).is_some_and(|v| v.to_int() != 0);
                if !ok {
                    p.halt();
                    p.err_no = 21;
                }
            }
        }
        ret
    }
}

/// Parse the input so far (the whole of it, when `is_final`): 1 or 0.
fn run(ctx: &mut dyn Ctx, o: &Rc<Object>, is_final: bool) -> Result<i64, Throw> {
    let (data, ns, huge, skip) = with(o, |p| (p.buffer.clone(), p.separator, p.parse_huge, p.delivered));
    let options = Options { replace_entities: true, oldsax: true, push: true, sax1: ns.is_none(), huge, ..Options::default() };
    let mut parser = Parser::new(data, None, options);
    parser.partial = !is_final;
    // PHP's compat layer starts with wellFormed cleared.
    parser.well_formed = false;
    let mut events = Events { ctx, this: Rc::clone(o), thrown: None, ns, seen: 0, skip };
    with(o, |p| p.parsing = true);
    let code = parser.parse_push_document(&mut events);
    with(o, |p| p.parsing = false);
    events.sync_position(&parser);
    let thrown = events.thrown.take();
    let failed = code != 0 || parser.last_error.as_ref().is_some_and(|e| e.level > LEVEL_WARNING);
    let seen = events.seen;
    with(o, |p| {
        p.err_no = parser.err_no;
        p.delivered = seen.max(p.delivered);
        // After the final chunk, or a fatal error, it parses no more.
        p.done = is_final || (parser.err_no != 0 && parser.disable_sax);
    });
    if let Some(t) = thrown {
        return Err(t);
    }
    Ok(i64::from(!failed))
}

fn xml_parse(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_parse", args)?;
    if with(&o, |p| p.parsing) {
        return Err(ctx.throw("Error", "Parser must not be called recursively".into()));
    }
    let data = bytes(&arg(args, 1));
    let is_final = args.len() > 2 && arg(args, 2).to_bool();
    if with(&o, |p| p.done) {
        return Ok(Value::Int(0));
    }
    with(&o, |p| {
        p.buffer.extend_from_slice(&data);
        p.calling = "xml_parse";
    });
    Ok(Value::Int(run(ctx, &o, is_final)?))
}

/// By-reference argument `i`'s cell.
fn ref_cell(args: &[Value], i: usize) -> Option<Rc<RefCell<Value>>> {
    match args.get(i) {
        Some(Value::Ref(cell)) => Some(Rc::clone(cell)),
        _ => None,
    }
}

/// Write `v` into by-reference argument `i`.
fn set_ref(args: &mut [Value], i: usize, v: Value) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = v;
    }
}

fn xml_parse_into_struct(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_parse_into_struct", args)?;
    if with(&o, |p| p.parsing) {
        ctx.warn("xml_parse_into_struct(): Parser must not be called recursively");
        return Ok(Value::Bool(false));
    }
    let with_index = args.len() > 3;
    if with_index {
        set_ref(args, 3, Value::Array(Rc::new(PhpArray::new())));
    }
    set_ref(args, 2, Value::Array(Rc::new(PhpArray::new())));
    let data = bytes(&arg(args, 1));
    with(&o, |p| {
        p.buffer.extend_from_slice(&data);
        p.calling = "xml_parse_into_struct";
        p.level = 0;
        p.installed[START] = true;
        p.installed[END] = true;
        p.installed[CDATA] = true;
        p.structure = Some(Struct {
            values_ref: ref_cell(args, 2),
            index_ref: if with_index { ref_cell(args, 3) } else { None },
            values: Vec::new(),
            index: with_index.then(PhpArray::new),
            ltags: Vec::new(),
            lastwasopen: false,
            curtag: 0,
        });
    });
    let r = if with(&o, |p| p.done) { Ok(0) } else { run(ctx, &o, true) };
    let s = with(&o, |p| p.structure.take()).expect("the structure");
    let still_array = |c: &Option<Rc<RefCell<Value>>>| c.as_ref().is_none_or(|c| matches!(*c.borrow(), Value::Array(_)));
    if still_array(&s.values_ref) {
        let values = PhpArray::from_list(s.values.into_iter().map(|a| Value::Array(Rc::new(a))).collect());
        set_ref(args, 2, Value::Array(Rc::new(values)));
    }
    if let Some(index) = s.index
        && still_array(&s.index_ref)
    {
        set_ref(args, 3, Value::Array(Rc::new(index)));
    }
    Ok(Value::Int(r?))
}

fn xml_get_error_code(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_get_error_code", args)?;
    Ok(Value::Int(i64::from(with(&o, |p| p.err_no))))
}

/// compat.c's error_mapping, by libxml2 error code.
#[rustfmt::skip]
static ERROR_STRINGS: &[&str] = &[
    "No error", "No memory", "Invalid document start", "Empty document", "Not well-formed (invalid token)",
    "Invalid document end", "Invalid hexadecimal character reference", "Invalid decimal character reference",
    "Invalid character reference", "Invalid character", "XML_ERR_CHARREF_AT_EOF", "XML_ERR_CHARREF_IN_PROLOG",
    "XML_ERR_CHARREF_IN_EPILOG", "XML_ERR_CHARREF_IN_DTD", "XML_ERR_ENTITYREF_AT_EOF", "XML_ERR_ENTITYREF_IN_PROLOG",
    "XML_ERR_ENTITYREF_IN_EPILOG", "XML_ERR_ENTITYREF_IN_DTD", "PEReference at end of document", "PEReference in prolog",
    "PEReference in epilog", "PEReference: forbidden within markup decl in internal subset", "XML_ERR_ENTITYREF_NO_NAME",
    "EntityRef: expecting ';'", "PEReference: no name", "PEReference: expecting ';'", "Undeclared entity error",
    "Undeclared entity warning", "Unparsed Entity", "XML_ERR_ENTITY_IS_EXTERNAL", "XML_ERR_ENTITY_IS_PARAMETER",
    "Unknown encoding", "Unsupported encoding", "String not started expecting ' or \"", "String not closed expecting \" or '",
    "Namespace declaration error", "EntityValue: \" or ' expected", "EntityValue: \" or ' expected", "< in attribute",
    "Attribute not started", "Attribute not finished", "Attribute without value", "Attribute redefined",
    "SystemLiteral \" or ' expected", "SystemLiteral \" or ' expected", "Comment not finished",
    "Processing Instruction not started", "Processing Instruction not finished", "NOTATION: Name expected here",
    "'>' required to close NOTATION declaration", "'(' required to start ATTLIST enumeration",
    "'(' required to start ATTLIST enumeration", "MixedContentDecl : '|' or ')*' expected", "XML_ERR_MIXED_NOT_FINISHED",
    "ELEMENT in DTD not started", "ELEMENT in DTD not finished", "XML declaration not started", "XML declaration not finished",
    "XML_ERR_CONDSEC_NOT_STARTED", "XML conditional section not closed", "Content error in the external subset",
    "DOCTYPE not finished", "Sequence ']]>' not allowed in content", "CDATA not finished", "Reserved XML Name",
    "Space required", "XML_ERR_SEPARATOR_REQUIRED", "NmToken expected in ATTLIST enumeration", "XML_ERR_NAME_REQUIRED",
    "MixedContentDecl : '#PCDATA' expected", "SYSTEM or PUBLIC, the URI is missing", "PUBLIC, the Public Identifier is missing",
    "< required", "> required", "</ required", "= required", "Mismatched tag", "Tag not finished",
    "standalone accepts only 'yes' or 'no'", "Invalid XML encoding name", "Comment must not contain '--' (double-hyphen)",
    "Invalid encoding", "external parsed entities cannot be standalone", "XML conditional section '[' expected",
    "Entity value required", "chunk is not well balanced", "extra content at the end of well balanced chunk",
    "XML_ERR_ENTITY_CHAR_ERROR", "PEReferences forbidden in internal subset", "Detected an entity reference loop",
    "XML_ERR_ENTITY_BOUNDARY", "Invalid URI", "Fragment not allowed", "XML_WAR_CATALOG_PI", "XML_ERR_NO_DTD",
    "conditional section INCLUDE or IGNORE keyword expected", "Version in XML Declaration missing",
    "XML_WAR_UNKNOWN_VERSION", "XML_WAR_LANG_VALUE", "XML_WAR_NS_URI", "XML_WAR_NS_URI_RELATIVE",
    "Missing encoding in text declaration",
];

fn xml_error_string(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let code = arg(args, 0).to_int();
    let s = usize::try_from(code).ok().and_then(|c| ERROR_STRINGS.get(c)).copied().unwrap_or("Unknown");
    Ok(Value::str(s))
}

fn xml_get_current_line_number(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_get_current_line_number", args)?;
    Ok(Value::Int(with(&o, |p| p.line)))
}

fn xml_get_current_column_number(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_get_current_column_number", args)?;
    Ok(Value::Int(with(&o, |p| p.column)))
}

fn xml_get_current_byte_index(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_get_current_byte_index", args)?;
    Ok(Value::Int(with(&o, |p| p.byte_index)))
}

fn xml_parser_free(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_parser_free", args)?;
    if with(&o, |p| p.parsing) {
        ctx.warn("xml_parser_free(): Parser cannot be freed while it is parsing");
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn xml_parser_set_option(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_parser_set_option", args)?;
    let opt = arg(args, 1).to_int();
    let value = arg(args, 2);
    if !matches!(value, Value::Bool(_) | Value::Int(_) | Value::Str(_)) {
        ctx.warn(&format!("xml_parser_set_option(): Argument #3 ($value) must be of type string|int|bool, {} given", value.type_name()));
    }
    match opt {
        1 => with(&o, |p| p.case_folding = value.to_bool()),
        4 => with(&o, |p| p.skip_white = value.to_bool()),
        5 => {
            if with(&o, |p| p.parsing) {
                return Err(ctx.throw("Error", "Cannot change option XML_OPTION_PARSE_HUGE while parsing".into()));
            }
            with(&o, |p| p.parse_huge = value.to_bool());
        }
        3 => {
            if let Value::Object(obj) = &value {
                ctx.warn(&format!("Object of class {} could not be converted to int", String::from_utf8_lossy(obj.class.name.as_bytes())));
            }
            let n = if matches!(value, Value::Object(_)) { 1 } else { value.to_int() };
            if !(0..=i64::from(i32::MAX)).contains(&n) {
                ctx.warn(&format!("xml_parser_set_option(): Argument #3 ($value) must be between 0 and {} for option XML_OPTION_SKIP_TAGSTART", i32::MAX));
                return Ok(Value::Bool(false));
            }
            with(&o, |p| p.tag_start = n as usize);
        }
        2 => {
            match &value {
                Value::Array(_) => ctx.warn("Array to string conversion"),
                Value::Object(obj) if obj.class.method(b"__tostring").is_none() => {
                    return Err(ctx.throw("Error", format!("Object of class {} could not be converted to string", String::from_utf8_lossy(obj.class.name.as_bytes()))));
                }
                _ => {}
            }
            let e = if matches!(value, Value::Array(_)) { b"Array".to_vec() } else { bytes(&value) };
            let enc = if e.eq_ignore_ascii_case(b"ISO-8859-1") {
                "ISO-8859-1"
            } else if e.eq_ignore_ascii_case(b"UTF-8") {
                "UTF-8"
            } else if e.eq_ignore_ascii_case(b"US-ASCII") {
                "US-ASCII"
            } else {
                return Err(ctx.throw("ValueError", "xml_parser_set_option(): Argument #3 ($value) is not a supported target encoding".into()));
            };
            with(&o, |p| p.target_encoding = enc);
        }
        _ => return Err(ctx.throw("ValueError", "xml_parser_set_option(): Argument #2 ($option) must be a XML_OPTION_* constant".into())),
    }
    Ok(Value::Bool(true))
}

fn xml_parser_get_option(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = parser_arg(ctx, "xml_parser_get_option", args)?;
    Ok(match arg(args, 1).to_int() {
        1 => Value::Bool(with(&o, |p| p.case_folding)),
        3 => Value::Int(with(&o, |p| p.tag_start as i64)),
        4 => Value::Bool(with(&o, |p| p.skip_white)),
        5 => Value::Bool(with(&o, |p| p.parse_huge)),
        2 => Value::str(with(&o, |p| p.target_encoding)),
        _ => return Err(ctx.throw("ValueError", "xml_parser_get_option(): Argument #2 ($option) must be a XML_OPTION_* constant".into())),
    })
}

fn construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(ctx.throw("Error", "Cannot directly construct XMLParser, use xml_parser_create() or xml_parser_create_ns() instead".into()))
}

pub(crate) static BUILTINS: &[Builtin] = &[
    Builtin { name: "xml_parser_create", run: xml_parser_create, by_ref: &[] },
    Builtin { name: "xml_parser_create_ns", run: xml_parser_create_ns, by_ref: &[] },
    Builtin { name: "xml_set_object", run: xml_set_object, by_ref: &[] },
    Builtin { name: "xml_set_element_handler", run: xml_set_element_handler, by_ref: &[] },
    Builtin { name: "xml_set_character_data_handler", run: xml_set_character_data_handler, by_ref: &[] },
    Builtin { name: "xml_set_processing_instruction_handler", run: xml_set_processing_instruction_handler, by_ref: &[] },
    Builtin { name: "xml_set_default_handler", run: xml_set_default_handler, by_ref: &[] },
    Builtin { name: "xml_set_unparsed_entity_decl_handler", run: xml_set_unparsed_entity_decl_handler, by_ref: &[] },
    Builtin { name: "xml_set_notation_decl_handler", run: xml_set_notation_decl_handler, by_ref: &[] },
    Builtin { name: "xml_set_external_entity_ref_handler", run: xml_set_external_entity_ref_handler, by_ref: &[] },
    Builtin { name: "xml_set_start_namespace_decl_handler", run: xml_set_start_namespace_decl_handler, by_ref: &[] },
    Builtin { name: "xml_set_end_namespace_decl_handler", run: xml_set_end_namespace_decl_handler, by_ref: &[] },
    Builtin { name: "xml_parse", run: xml_parse, by_ref: &[] },
    Builtin { name: "xml_parse_into_struct", run: xml_parse_into_struct, by_ref: &[2, 3] },
    Builtin { name: "xml_get_error_code", run: xml_get_error_code, by_ref: &[] },
    Builtin { name: "xml_error_string", run: xml_error_string, by_ref: &[] },
    Builtin { name: "xml_get_current_line_number", run: xml_get_current_line_number, by_ref: &[] },
    Builtin { name: "xml_get_current_column_number", run: xml_get_current_column_number, by_ref: &[] },
    Builtin { name: "xml_get_current_byte_index", run: xml_get_current_byte_index, by_ref: &[] },
    Builtin { name: "xml_parser_free", run: xml_parser_free, by_ref: &[] },
    Builtin { name: "xml_parser_set_option", run: xml_parser_set_option, by_ref: &[] },
    Builtin { name: "xml_parser_get_option", run: xml_parser_get_option, by_ref: &[] },
];

/// The functions that call PHP handlers (the engine lends them its registers).
pub(crate) const REENTER: &[&str] = &["xml_parse", "xml_parse_into_struct"];

pub(crate) static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "XMLParser",
    parent: None,
    interfaces: &[],
    flags: CLASS_FINAL | CLASS_UNCLONEABLE | p8_runtime::object::CLASS_NOT_SERIALIZABLE,
    properties: &[],
    methods: &[("__construct", Some(Native::Method(construct)))],
    constants: &[],
    debug_info: None,
}];

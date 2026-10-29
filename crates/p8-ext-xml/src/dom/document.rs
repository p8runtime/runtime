//! DOMDocument, DOMElement, DOMAttr, DOMCharacterData and the other node
//! classes' own properties and methods.
//!
//! Reference: php-src ext/dom/document.c, element.c, attr.c,
//! characterdata.c, text.c

use super::build;
use super::node::*;
use super::html_out::{self, HtmlOut};
use super::serialize::Out;
use super::tree::{DOC, Doc, Id, Kind, Node};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

fn new_doc_for(this: &Rc<Object>) -> Rc<RefCell<Doc>> {
    Rc::new(RefCell::new(Doc::new(this.class.name.as_bytes().to_vec())))
}

/// The document `this` is (made empty on first use, for an object made
/// without its constructor running).
fn this_doc(this: &Rc<Object>) -> NodeRef {
    if let Some(r) = data(this).node.borrow().clone() {
        return r;
    }
    let r = NodeRef { doc: new_doc_for(this), id: DOC };
    bind(this, &r);
    r
}

// ---- DOMDocument ----

pub fn document_construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = NodeRef { doc: new_doc_for(this), id: DOC };
    {
        let mut d = r.doc.borrow_mut();
        let version = arg(args, 0);
        if !matches!(version, Value::Null) {
            d.settings.version = Some(bytes(&version));
        }
        let enc = bytes(&arg(args, 1));
        if !enc.is_empty() {
            d.settings.encoding = Some(enc);
        }
    }
    bind(this, &r);
    Ok(Value::Null)
}

/// The libxml2 options a load uses: the document's settings and `flags`.
fn load_flags(d: &Doc, flags: i64) -> i64 {
    let s = &d.settings;
    let mut f = flags;
    if s.substitute_entities {
        f |= build::NOENT;
    }
    if s.resolve_externals {
        f |= build::DTDLOAD;
    }
    if !s.preserve_white_space {
        f |= build::NOBLANKS;
    }
    if s.recover {
        f |= build::RECOVER;
    }
    f
}

/// Parses into a new tree for `this` document: true, or false when the
/// document is not well-formed (recover off). Each libxml2 error is a
/// warning, as PHP's libxml error handler gives it.
fn load(ctx: &mut dyn Ctx, this: &Rc<Object>, data: Vec<u8>, filename: Option<String>, flags: i64, func: &str) -> R {
    let old = this_doc(this);
    let settings = old.doc.borrow().settings.clone();
    let mut doc = Doc::new(this.class.name.as_bytes().to_vec());
    doc.settings = settings;
    let flags = load_flags(&old.doc.borrow(), flags);
    let parser = build::parse(&mut doc, data, filename.clone(), flags);
    let where_ = filename.as_deref().unwrap_or("Entity");
    for e in &parser.errors {
        if flags & (32 | 64) != 0 {
            break;
        }
        let msg = e.message.trim_end_matches('\n');
        crate::libxml::report(ctx, e, |ctx| ctx.warn(&format!("{func}(): {msg} in {where_}, line: {}", e.line)));
    }
    if !parser.well_formed && flags & build::RECOVER == 0 {
        return Ok(Value::Bool(false));
    }
    if let Some(f) = filename {
        doc.settings.url = Some(f.into_bytes());
    }
    let r = NodeRef { doc: Rc::new(RefCell::new(doc)), id: DOC };
    bind(this, &r);
    Ok(Value::Bool(true))
}

pub fn load_xml(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let source = bytes(&arg(args, 0));
    if source.is_empty() {
        return Err(ctx.throw("ValueError", "DOMDocument::loadXML(): Argument #1 ($source) must not be empty".into()));
    }
    load(ctx, this, source, None, arg(args, 1).to_int(), "DOMDocument::loadXML")
}

pub fn load_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if name.is_empty() {
        return Err(ctx.throw("ValueError", "DOMDocument::load(): Argument #1 ($filename) must not be empty".into()));
    }
    let path = String::from_utf8_lossy(&name).into_owned();
    match std::fs::read(&path) {
        Ok(data) => {
            let full = std::path::absolute(&path).map(|p| p.to_string_lossy().into_owned()).unwrap_or(path);
            load(ctx, this, data, Some(full), arg(args, 1).to_int(), "DOMDocument::load")
        }
        Err(_) => {
            ctx.warn(&format!("DOMDocument::load(): I/O warning : failed to load external entity \"{path}\""));
            Ok(Value::Bool(false))
        }
    }
}

/// Parses HTML into a new tree for `this` document. The parse is never
/// refused: errors only become warnings (or entries in libxml's list).
fn load_html_data(ctx: &mut dyn Ctx, this: &Rc<Object>, data: Vec<u8>, filename: Option<String>, flags: i64, func: &str) -> R {
    let old = this_doc(this);
    let settings = old.doc.borrow().settings.clone();
    let mut doc = Doc::new(this.class.name.as_bytes().to_vec());
    doc.settings = settings;
    doc.settings.encoding = None;
    doc.settings.url = None;
    let errors = crate::html::parse(&mut doc, data, filename.clone(), flags);
    let where_ = filename.as_deref().unwrap_or("Entity");
    for e in &errors {
        // LIBXML_NOERROR drops the warnings, not the errors libxml_get_errors lists.
        let msg = e.message.trim_end_matches('\n');
        crate::libxml::report(ctx, e, |ctx| {
            if flags & 32 == 0 {
                ctx.warn(&format!("{func}(): {msg} in {where_}, line: {}", e.line));
            }
        });
    }
    if let Some(f) = filename {
        doc.settings.url = Some(f.into_bytes());
    }
    let r = NodeRef { doc: Rc::new(RefCell::new(doc)), id: DOC };
    bind(this, &r);
    Ok(Value::Bool(true))
}

pub fn load_html(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let source = bytes(&arg(args, 0));
    if source.is_empty() {
        return Err(ctx.throw("ValueError", "DOMDocument::loadHTML(): Argument #1 ($source) must not be empty".into()));
    }
    load_html_data(ctx, this, source, None, arg(args, 1).to_int(), "DOMDocument::loadHTML")
}

pub fn load_html_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if name.is_empty() {
        return Err(ctx.throw("ValueError", "DOMDocument::loadHTMLFile(): Argument #1 ($filename) must not be empty".into()));
    }
    if name.contains(&0) {
        return Err(ctx.throw("ValueError", "DOMDocument::loadHTMLFile(): Argument #1 ($filename) must not contain any null bytes".into()));
    }
    let path = String::from_utf8_lossy(&name).into_owned();
    match std::fs::read(&path) {
        Ok(data) => {
            load_html_data(ctx, this, data, Some(path), arg(args, 1).to_int(), "DOMDocument::loadHTMLFile")
        }
        Err(_) => {
            ctx.warn(&format!("DOMDocument::loadHTMLFile(): I/O warning : failed to load external entity \"{path}\""));
            Ok(Value::Bool(false))
        }
    }
}

/// The warning text of a libxml2 error that has no PHP error handler:
/// parser-level ones print as xmlReportError does (several lines).
fn schema_warnings(e: &crate::parser::XmlError) -> Vec<String> {
    match e.domain {
        crate::parser::FROM_PARSER | crate::parser::FROM_NAMESPACE | crate::parser::FROM_DTD => e.report_lines(),
        crate::xsd::FROM_IO => vec![format!("I/O {} : {}", if e.level == crate::parser::LEVEL_WARNING { "warning" } else { "error" }, e.message.trim_end_matches('\n'))],
        _ => vec![e.message.trim_end_matches('\n').to_string()],
    }
}

/// DOMDocument::schemaValidate[Source]: compiles the schema (read from
/// `file`, or `source`) and validates the document against it.
fn schema_run(ctx: &mut dyn Ctx, this: &Rc<Object>, schema: Result<crate::xsd::Schema, Vec<crate::parser::XmlError>>, flags: i64, func: &str) -> R {
    let report_all = |ctx: &mut dyn Ctx, errors: &[crate::parser::XmlError]| {
        for e in errors {
            crate::libxml::report(ctx, e, |ctx| {
                for line in schema_warnings(e) {
                    ctx.warn(&format!("{func}(): {line}"));
                }
            });
        }
    };
    let schema = match schema {
        Ok(s) => s,
        Err(errors) => {
            report_all(ctx, &errors);
            ctx.warn(&format!("{func}(): Invalid Schema"));
            return Ok(Value::Bool(false));
        }
    };
    let r = this_doc(this);
    let errors = {
        let mut d = r.doc.borrow_mut();
        crate::xsd::validate(&schema, &mut d, flags & 1 != 0)
    };
    report_all(ctx, &errors);
    Ok(Value::Bool(errors.is_empty()))
}

pub fn schema_validate(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if name.is_empty() {
        return Err(ctx.throw("ValueError", "DOMDocument::schemaValidate(): Argument #1 ($filename) must not be empty".into()));
    }
    if name.contains(&0) {
        return Err(ctx.throw("ValueError", "DOMDocument::schemaValidate(): Argument #1 ($filename) must not contain any null bytes".into()));
    }
    let path = String::from_utf8_lossy(&name).into_owned();
    if name.len() > p8_runtime::stream::MAXPATHLEN {
        ctx.warn("DOMDocument::schemaValidate(): Invalid Schema file source");
        return Ok(Value::Bool(false));
    }
    let full = std::path::absolute(&path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.clone());
    let schema = match std::fs::read(&path) {
        Ok(data) => crate::xsd::compile(data, Some(full)),
        Err(_) => Err(crate::xsd::load_errors(&path)),
    };
    schema_run(ctx, this, schema, arg(args, 1).to_int(), "DOMDocument::schemaValidate")
}

pub fn schema_validate_source(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let source = bytes(&arg(args, 0));
    if source.is_empty() {
        return Err(ctx.throw("ValueError", "DOMDocument::schemaValidateSource(): Argument #1 ($source) must not be empty".into()));
    }
    let schema = crate::xsd::compile(source, None);
    schema_run(ctx, this, schema, arg(args, 1).to_int(), "DOMDocument::schemaValidateSource")
}

/// LIBXML_NOEMPTYTAG for saveXML.
const NOEMPTYTAG: i64 = 4;

/// The document, or one of its nodes, as XML.
fn save(ctx: &mut dyn Ctx, this: &Rc<Object>, node: Option<NodeRef>, options: i64) -> Result<Option<Vec<u8>>, Throw> {
    let r = this_doc(this);
    let d = r.doc.borrow();
    let format = d.settings.format_output;
    match node {
        Some(n) => {
            if !n.same_doc(&r) {
                drop(d);
                return Err(dom_error(ctx, WRONG_DOCUMENT));
            }
            let mut out = Out::new(&d, format, false);
            out.no_empty = options & NOEMPTYTAG != 0;
            out.node(n.id, 0);
            Ok(Some(out.buf))
        }
        None => {
            let mut out = Out::new(&d, format, d.settings.encoding.is_none());
            out.no_empty = options & NOEMPTYTAG != 0;
            out.document();
            Ok(Some(out.buf))
        }
    }
}

pub fn save_xml(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let node = arg_node(args, 0);
    Ok(match save(ctx, this, node, arg(args, 1).to_int())? {
        Some(b) => str(&b),
        None => Value::Bool(false),
    })
}

pub fn save_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let path = String::from_utf8_lossy(&bytes(&arg(args, 0))).into_owned();
    let Some(b) = save(ctx, this, None, arg(args, 1).to_int())? else { return Ok(Value::Bool(false)) };
    match std::fs::write(&path, &b) {
        Ok(()) => Ok(Value::Int(b.len() as i64)),
        Err(_) => Ok(Value::Bool(false)),
    }
}

/// saveHTML: the document (through the output encoding libxml2 picks from
/// its meta element) or one node (as it is).
pub fn save_html(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let node = arg_node(args, 0);
    let r = this_doc(this);
    let d = r.doc.borrow();
    let mut out = HtmlOut::new(&d, d.settings.format_output);
    match node {
        Some(n) => {
            if !n.same_doc(&r) {
                drop(d);
                return Err(dom_error(ctx, WRONG_DOCUMENT));
            }
            if d.node(n.id).kind == Kind::Fragment {
                for &c in &d.node(n.id).children {
                    out.dump(c);
                }
            } else {
                out.dump(n.id);
            }
            Ok(str(&out.buf))
        }
        None => {
            out.dump(DOC);
            let encoding = html_out::meta_encoding(&d);
            let (bytes, failed) = html_out::encode_output(out.buf, encoding.as_deref());
            drop(d);
            if failed {
                // libxml2's output buffer reports the failed conversion.
                {
                    let e = crate::parser::XmlError {
                        domain: 8,
                        code: 1544,
                        level: 2,
                        message: "encoder error".into(),
                        line: 0,
                        column: 0,
                        file: None,
                        context: Vec::new(),
                        context_col: 0,
                    };
                    crate::libxml::report(ctx, &e, |ctx| ctx.warn("DOMDocument::saveHTML(): encoder error"));
                }
            }
            Ok(if bytes.is_empty() { Value::Bool(false) } else { str(&bytes) })
        }
    }
}

pub fn save_html_file(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if name.is_empty() {
        return Err(ctx.throw("ValueError", "DOMDocument::saveHTMLFile(): Argument #1 ($filename) must not be empty".into()));
    }
    if name.contains(&0) {
        return Err(ctx.throw("ValueError", "DOMDocument::saveHTMLFile(): Argument #1 ($filename) must not contain any null bytes".into()));
    }
    let path = String::from_utf8_lossy(&name).into_owned();
    let r = this_doc(this);
    let mut d = r.doc.borrow_mut();
    // htmlSaveFileFormat records the encoding in the document first.
    let encoding = html_out::meta_encoding(&d);
    html_out::set_meta_encoding(&mut d, Some(encoding.as_deref().unwrap_or(b"UTF-8")));
    let format = d.settings.format_output;
    let mut out = HtmlOut::new(&d, format);
    out.dump(DOC);
    let (bytes, _) = html_out::encode_output(out.buf, encoding.as_deref());
    Ok(match std::fs::write(&path, &bytes) {
        Ok(()) => Value::Int(bytes.len() as i64),
        Err(e) => {
            let why = e.to_string();
            let why = why.split(" (os error").next().unwrap_or(&why);
            ctx.warn(&format!("DOMDocument::saveHTMLFile({path}): Failed to open stream: {why}"));
            Value::Int(0)
        }
    })
}

/// A new node in `this` document.
fn create(ctx: &mut dyn Ctx, this: &Rc<Object>, node: Node) -> R {
    let r = this_doc(this);
    let id = r.doc.borrow_mut().add(node);
    wrap(ctx, &r.at(id))
}

pub fn create_element(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if !valid_name(&name) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    let r = this_doc(this);
    let id = {
        let mut d = r.doc.borrow_mut();
        let id = d.add(Node::new(Kind::Element, name));
        set_content_parsed(ctx, "DOMDocument::createElement", &mut d, id, &bytes(&arg(args, 1)));
        id
    };
    wrap(ctx, &r.at(id))
}

/// An element in a namespace: it declares the namespace itself, as
/// libxml2's xmlNewNs on the new node does.
fn element_ns(ctx: &mut dyn Ctx, d: &mut Doc, ns: Option<Vec<u8>>, qname: &[u8]) -> Result<Id, Throw> {
    let (prefix, local) = split_qname(qname).map_err(|code| dom_error(ctx, code))?;
    let ns = ns.filter(|n| !n.is_empty());
    if prefix.is_some() && ns.is_none() {
        return Err(dom_error(ctx, NAMESPACE));
    }
    if prefix.as_deref() == Some(b"xml") && ns.as_deref() != Some(crate::parser::XML_NS) {
        return Err(dom_error(ctx, NAMESPACE));
    }
    let mut node = Node::new(Kind::Element, local);
    node.prefix = prefix.clone();
    if let Some(uri) = &ns {
        node.ns = Some(uri.clone());
        if prefix.as_deref() != Some(b"xml") {
            node.ns_defs.push((prefix, uri.clone()));
        }
    }
    Ok(d.add(node))
}

pub fn create_element_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_doc(this);
    let ns = opt_bytes(&arg(args, 0));
    let qname = bytes(&arg(args, 1));
    let id = {
        let mut d = r.doc.borrow_mut();
        let id = element_ns(ctx, &mut d, ns, &qname)?;
        set_content_parsed(ctx, "DOMDocument::createElementNS", &mut d, id, &bytes(&arg(args, 2)));
        id
    };
    wrap(ctx, &r.at(id))
}

fn valued(kind: Kind, name: &[u8], value: &[u8]) -> Node {
    let mut n = Node::new(kind, name.to_vec());
    n.value = value.to_vec();
    n
}

pub fn create_text_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    create(ctx, this, valued(Kind::Text, b"", &bytes(&arg(args, 0))))
}

pub fn create_comment(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    create(ctx, this, valued(Kind::Comment, b"", &bytes(&arg(args, 0))))
}

pub fn create_cdata_section(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    create(ctx, this, valued(Kind::CData, b"", &bytes(&arg(args, 0))))
}

pub fn create_document_fragment(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    create(ctx, this, Node::new(Kind::Fragment, Vec::new()))
}

pub fn create_processing_instruction(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let target = bytes(&arg(args, 0));
    if !valid_name(&target) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    create(ctx, this, valued(Kind::Pi, &target, &bytes(&arg(args, 1))))
}

pub fn create_entity_reference(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if !valid_name(&name) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    create(ctx, this, Node::new(Kind::EntityRef, name))
}

pub fn create_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if !valid_name(&name) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    create(ctx, this, Node::new(Kind::Attribute, name))
}

pub fn create_attribute_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let ns = opt_bytes(&arg(args, 0)).filter(|n| !n.is_empty());
    let qname = bytes(&arg(args, 1));
    let (prefix, local) = split_qname(&qname).map_err(|code| dom_error(ctx, code))?;
    if prefix.is_some() && ns.is_none() {
        return Err(dom_error(ctx, NAMESPACE));
    }
    let mut n = Node::new(Kind::Attribute, local);
    n.prefix = prefix;
    n.ns = ns;
    create(ctx, this, n)
}

pub fn get_element_by_id(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_doc(this);
    let id = bytes(&arg(args, 0));
    let found = {
        let d = r.doc.borrow();
        let mut all = Vec::new();
        d.descendants(DOC, &mut all);
        // Only xml:id counts without a DTD declaring ID attributes; in an
        // HTML document id does, and name on a (xmlIsID).
        all.into_iter().find(|&e| {
            if d.html {
                d.node(e).attrs.iter().any(|&a| is_html_id(&d, e, a) && d.node(a).value == id)
            } else {
                d.attr_ns(e, Some(crate::parser::XML_NS), b"id").is_some_and(|a| d.node(a).value == id)
            }
        })
    };
    wrap_opt(ctx, &r, found)
}

pub fn import_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_doc(this);
    let Some(n) = arg_node(args, 0) else { return Ok(Value::Bool(false)) };
    if matches!(n.kind(), Kind::Document | Kind::DocType) {
        return Err(dom_error(ctx, NOT_SUPPORTED));
    }
    let deep = arg(args, 1).to_bool();
    let id = if n.same_doc(&r) {
        let src = r.doc.borrow().clone_tree();
        r.doc.borrow_mut().copy_from(&src, n.id, deep)
    } else {
        let src = n.doc.borrow();
        r.doc.borrow_mut().copy_from(&src, n.id, deep)
    };
    wrap(ctx, &r.at(id))
}

pub fn adopt_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_doc(this);
    let Some(n) = arg_node(args, 0) else { return Ok(Value::Bool(false)) };
    if matches!(n.kind(), Kind::Document | Kind::DocType) {
        return Err(dom_error(ctx, NOT_SUPPORTED));
    }
    if n.same_doc(&r) {
        r.doc.borrow_mut().detach(n.id);
    } else {
        move_into(&n, &r.doc);
    }
    Ok(arg(args, 0))
}

pub fn normalize_document(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    normalize(ctx, this, args)
}

/// Substitutes the document's XIncludes: their count, false for none, -1 on
/// an error.
pub fn xinclude(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let flags = arg(args, 0).to_int();
    if flags > i64::from(i32::MAX) || flags < i64::from(i32::MIN) {
        ctx.warn("DOMDocument::xinclude(): Invalid flags");
        return Ok(Value::Bool(false));
    }
    let r = this_doc(this);
    let (done, diags) = super::xinclude::run(&mut r.doc.borrow_mut(), flags);
    for diag in &diags {
        crate::libxml::report(ctx, &diag.error, |ctx| {
            for line in &diag.warnings {
                ctx.warn(&format!("DOMDocument::xinclude(): {line}"));
            }
        });
    }
    Ok(if done == 0 { Value::Bool(false) } else { Value::Int(done) })
}

getter!(document_element_get, |ctx, r| {
    let root = r.doc.borrow().root();
    wrap_opt(ctx, &r, root)
});
getter!(doctype_get, |ctx, r| {
    let dt = r.doc.borrow().node(DOC).children.iter().copied().find(|&c| r.doc.borrow().node(c).kind == Kind::DocType);
    wrap_opt(ctx, &r, dt)
});
getter!(encoding_get, |ctx, r| Ok(opt_str(r.doc.borrow().settings.encoding.as_deref())));
getter!(version_get, |ctx, r| Ok(opt_str(r.doc.borrow().settings.version.as_deref())));
getter!(standalone_get, |ctx, r| Ok(Value::Bool(r.doc.borrow().settings.standalone == 1)));
getter!(document_uri_get, |ctx, r| Ok(opt_str(r.doc.borrow().settings.url.as_deref())));
getter!(config_get, |ctx, r| {
    let _ = r;
    Ok(Value::Null)
});

pub fn implementation_get(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    let class = ctx.class(b"DOMImplementation").expect("declared");
    Ok(Value::Object(ctx.new_object(&class)))
}

pub fn encoding_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let v = bytes(&arg(args, 0));
    if !["UTF-8", "UTF8", "ISO-8859-1", "ASCII", "US-ASCII", "UTF-16", "ISO-8859-15", "WINDOWS-1252"].iter().any(|e| e.as_bytes().eq_ignore_ascii_case(&v)) {
        return Err(ctx.throw("ValueError", "Invalid document encoding".into()));
    }
    r.doc.borrow_mut().settings.encoding = Some(v);
    Ok(Value::Null)
}

pub fn version_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    r.doc.borrow_mut().settings.version = Some(bytes(&arg(args, 0)));
    Ok(Value::Null)
}

pub fn standalone_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    r.doc.borrow_mut().settings.standalone = i32::from(arg(args, 0).to_bool());
    Ok(Value::Null)
}

pub fn document_uri_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    r.doc.borrow_mut().settings.url = Some(bytes(&arg(args, 0)));
    Ok(Value::Null)
}

/// The document's boolean settings, each a get/set pair.
macro_rules! flag {
    ($get:ident, $set:ident, $field:ident) => {
        pub fn $get(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
            Ok(Value::Bool(this_doc(this).doc.borrow().settings.$field))
        }
        pub fn $set(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
            this_doc(this).doc.borrow_mut().settings.$field = arg(args, 0).to_bool();
            Ok(Value::Null)
        }
    };
}
flag!(format_output_get, format_output_set, format_output);
flag!(validate_on_parse_get, validate_on_parse_set, validate_on_parse);
flag!(resolve_externals_get, resolve_externals_set, resolve_externals);
flag!(preserve_white_space_get, preserve_white_space_set, preserve_white_space);
flag!(recover_get, recover_set, recover);
flag!(substitute_entities_get, substitute_entities_set, substitute_entities);
flag!(strict_error_checking_get, strict_error_checking_set, strict_error_checking);

// ---- Nodes made with `new`: each in its own (orphan) document ----

fn orphan(this: &Rc<Object>, node: Node) -> NodeRef {
    let mut d = Doc::new(Vec::new());
    d.orphan = true;
    let id = d.add(node);
    let r = NodeRef { doc: Rc::new(RefCell::new(d)), id };
    bind(this, &r);
    r
}

pub fn element_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    let ns = opt_bytes(&arg(args, 2)).filter(|n| !n.is_empty());
    let mut d = Doc::new(Vec::new());
    d.orphan = true;
    let id = match ns {
        Some(ns) => element_ns(ctx, &mut d, Some(ns), &name)?,
        None => {
            if !valid_name(&name) {
                return Err(dom_error(ctx, INVALID_CHARACTER));
            }
            d.add(Node::new(Kind::Element, name))
        }
    };
    set_content_parsed(ctx, "DOMElement::__construct", &mut d, id, &bytes(&arg(args, 1)));
    bind(this, &NodeRef { doc: Rc::new(RefCell::new(d)), id });
    Ok(Value::Null)
}

pub fn text_construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    orphan(this, valued(Kind::Text, b"", &bytes(&arg(args, 0))));
    Ok(Value::Null)
}

pub fn comment_construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    orphan(this, valued(Kind::Comment, b"", &bytes(&arg(args, 0))));
    Ok(Value::Null)
}

pub fn cdata_construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    orphan(this, valued(Kind::CData, b"", &bytes(&arg(args, 0))));
    Ok(Value::Null)
}

pub fn fragment_construct(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    orphan(this, Node::new(Kind::Fragment, Vec::new()));
    Ok(Value::Null)
}

pub fn pi_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let target = bytes(&arg(args, 0));
    if !valid_name(&target) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    orphan(this, valued(Kind::Pi, &target, &bytes(&arg(args, 1))));
    Ok(Value::Null)
}

pub fn entity_ref_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if !valid_name(&name) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    orphan(this, Node::new(Kind::EntityRef, name));
    Ok(Value::Null)
}

pub fn attr_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if !valid_name(&name) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    orphan(this, valued(Kind::Attribute, &name, &bytes(&arg(args, 1))));
    Ok(Value::Null)
}

// ---- DOMElement ----

getter!(tag_name_get, |ctx, r| Ok(str(&r.doc.borrow().node(r.id).qname())));

/// An attribute value by qualified name; `xmlns` and `xmlns:p` read the
/// element's namespace declarations.
fn attr_value(d: &Doc, el: Id, name: &[u8]) -> Option<Vec<u8>> {
    if let Some(a) = d.attr(el, name) {
        return Some(d.node(a).value.clone());
    }
    let n = d.node(el);
    if name == b"xmlns" {
        return n.ns_defs.iter().find(|(p, _)| p.is_none()).map(|(_, u)| u.clone());
    }
    let p = name.strip_prefix(b"xmlns:")?;
    n.ns_defs.iter().find(|(q, _)| q.as_deref() == Some(p)).map(|(_, u)| u.clone())
}

pub fn get_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let name = bytes(&arg(args, 0));
    Ok(match attr_value(&r.doc.borrow(), r.id, &name) {
        Some(v) => str(&v),
        None => str(b""),
    })
}

pub fn has_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let name = bytes(&arg(args, 0));
    Ok(Value::Bool(attr_value(&r.doc.borrow(), r.id, &name).is_some()))
}

pub fn set_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let name = bytes(&arg(args, 0));
    let value = bytes(&arg(args, 1));
    if !valid_name(&name) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    let id = {
        let mut d = r.doc.borrow_mut();
        if name == b"xmlns" || name.starts_with(b"xmlns:") {
            let prefix = name.strip_prefix(b"xmlns:").map(<[u8]>::to_vec);
            let defs = &mut d.node_mut(r.id).ns_defs;
            match defs.iter_mut().find(|(p, _)| *p == prefix) {
                Some(def) => def.1 = value,
                None => defs.push((prefix, value)),
            }
            return Ok(Value::Bool(true));
        }
        match d.attr(r.id, &name) {
            Some(a) => {
                d.node_mut(a).value = value;
                a
            }
            None => {
                let a = d.add(valued(Kind::Attribute, &name, &value));
                d.set_attr(r.id, a);
                a
            }
        }
    };
    wrap(ctx, &r.at(id))
}

pub fn remove_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let name = bytes(&arg(args, 0));
    let mut d = r.doc.borrow_mut();
    match d.attr(r.id, &name) {
        Some(a) => {
            d.detach(a);
            Ok(Value::Bool(true))
        }
        None => {
            let prefix = if name == b"xmlns" { Some(None) } else { name.strip_prefix(b"xmlns:").map(|p| Some(p.to_vec())) };
            match prefix {
                Some(p) => {
                    let defs = &mut d.node_mut(r.id).ns_defs;
                    let before = defs.len();
                    defs.retain(|(q, _)| *q != p);
                    Ok(Value::Bool(defs.len() != before))
                }
                None => Ok(Value::Bool(false)),
            }
        }
    }
}

pub fn toggle_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let name = bytes(&arg(args, 0));
    if !valid_name(&name) {
        return Err(dom_error(ctx, INVALID_CHARACTER));
    }
    let force = arg(args, 1);
    let mut d = r.doc.borrow_mut();
    // An element of an HTML document without a namespace takes a lower-case name.
    let name = if d.html && d.node(r.id).ns.is_none() { name.to_ascii_lowercase() } else { name };
    let existing = d.attr(r.id, &name);
    match (existing, force) {
        (Some(_), Value::Bool(true)) => Ok(Value::Bool(true)),
        (Some(a), _) => {
            d.detach(a);
            Ok(Value::Bool(false))
        }
        (None, Value::Bool(false)) => Ok(Value::Bool(false)),
        (None, _) => {
            let a = d.add(Node::new(Kind::Attribute, name));
            d.set_attr(r.id, a);
            Ok(Value::Bool(true))
        }
    }
}

pub fn get_attribute_names(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let d = r.doc.borrow();
    let n = d.node(r.id);
    let mut names: Vec<Value> = Vec::new();
    for (p, _) in &n.ns_defs {
        names.push(match p {
            Some(p) => str(&[b"xmlns:", p.as_slice()].concat()),
            None => str(b"xmlns"),
        });
    }
    names.extend(n.attrs.iter().map(|&a| str(&d.node(a).qname())));
    Ok(Value::Array(Rc::new(p8_runtime::array::PhpArray::from_list(names))))
}

pub fn get_attribute_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let name = bytes(&arg(args, 0));
    let a = r.doc.borrow().attr(r.id, &name);
    match a {
        Some(a) => wrap(ctx, &r.at(a)),
        None => Ok(Value::Bool(false)),
    }
}

pub fn set_attribute_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let Some(a) = arg_node(args, 0) else { return Ok(Value::Bool(false)) };
    if a.kind() != Kind::Attribute {
        return Ok(Value::Bool(false));
    }
    if a.same_doc(&r) && a.doc.borrow().parent(a.id).is_some_and(|p| p != r.id) {
        return Err(dom_error(ctx, INUSE_ATTRIBUTE));
    }
    let id = if a.same_doc(&r) {
        a.id
    } else if a.doc.borrow().orphan {
        move_into(&a, &r.doc)
    } else {
        return Err(dom_error(ctx, WRONG_DOCUMENT));
    };
    let mut d = r.doc.borrow_mut();
    let qname = d.node(id).qname();
    let old = d.attr(r.id, &qname).filter(|&o| o != id);
    if let Some(o) = old {
        d.detach(o);
    }
    d.set_attr(r.id, id);
    drop(d);
    match old {
        Some(o) => wrap(ctx, &r.at(o)),
        None => Ok(Value::Null),
    }
}

pub fn remove_attribute_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let Some(a) = arg_node(args, 0) else { return Ok(Value::Bool(false)) };
    if !a.same_doc(&r) || a.doc.borrow().parent(a.id) != Some(r.id) || a.kind() != Kind::Attribute {
        return Err(dom_error(ctx, NOT_FOUND));
    }
    r.doc.borrow_mut().detach(a.id);
    Ok(arg(args, 0))
}

pub fn get_attribute_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let ns = opt_bytes(&arg(args, 0)).filter(|n| !n.is_empty());
    let local = bytes(&arg(args, 1));
    let d = r.doc.borrow();
    Ok(match d.attr_ns(r.id, ns.as_deref(), &local) {
        Some(a) => str(&d.node(a).value),
        None => str(b""),
    })
}

pub fn has_attribute_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let ns = opt_bytes(&arg(args, 0)).filter(|n| !n.is_empty());
    let local = bytes(&arg(args, 1));
    Ok(Value::Bool(r.doc.borrow().attr_ns(r.id, ns.as_deref(), &local).is_some()))
}

pub fn get_attribute_node_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let ns = opt_bytes(&arg(args, 0)).filter(|n| !n.is_empty());
    let local = bytes(&arg(args, 1));
    let a = r.doc.borrow().attr_ns(r.id, ns.as_deref(), &local);
    wrap_opt(ctx, &r, a)
}

pub fn set_attribute_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let ns = opt_bytes(&arg(args, 0)).filter(|n| !n.is_empty());
    let qname = bytes(&arg(args, 1));
    let value = bytes(&arg(args, 2));
    let (prefix, local) = split_qname(&qname).map_err(|code| dom_error(ctx, code))?;
    if prefix.is_some() && ns.is_none() {
        return Err(dom_error(ctx, NAMESPACE));
    }
    let mut d = r.doc.borrow_mut();
    // xmlns="..." and xmlns:p="..." in the XMLNS namespace declare one.
    if ns.as_deref() == Some(b"http://www.w3.org/2000/xmlns/") {
        let p = if prefix.is_none() { None } else { Some(local.clone()) };
        let defs = &mut d.node_mut(r.id).ns_defs;
        match defs.iter_mut().find(|(q, _)| *q == p) {
            Some(def) => def.1 = value,
            None => defs.push((p, value)),
        }
        return Ok(Value::Null);
    }
    match d.attr_ns(r.id, ns.as_deref(), &local) {
        Some(a) => {
            d.node_mut(a).value = value;
            d.node_mut(a).prefix = prefix;
        }
        None => {
            if let (Some(p), Some(uri)) = (&prefix, &ns)
                && d.lookup_ns(r.id, Some(p)).as_deref() != Some(uri.as_slice())
            {
                d.node_mut(r.id).ns_defs.push((Some(p.clone()), uri.clone()));
            }
            let mut a = valued(Kind::Attribute, &local, &value);
            a.prefix = prefix;
            a.ns = ns;
            let a = d.add(a);
            d.set_attr(r.id, a);
        }
    }
    Ok(Value::Null)
}

pub fn remove_attribute_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let ns = opt_bytes(&arg(args, 0)).filter(|n| !n.is_empty());
    let local = bytes(&arg(args, 1));
    let mut d = r.doc.borrow_mut();
    if let Some(a) = d.attr_ns(r.id, ns.as_deref(), &local) {
        d.detach(a);
    }
    Ok(Value::Null)
}

pub fn id_get(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    Ok(str(&attr_value(&r.doc.borrow(), r.id, b"id").unwrap_or_default()))
}

pub fn id_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let mut a = vec![str(b"id"), arg(args, 0)];
    set_attribute(ctx, this, &mut a).map(|_| Value::Null)
}

pub fn class_name_get(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    Ok(str(&attr_value(&r.doc.borrow(), r.id, b"class").unwrap_or_default()))
}

pub fn class_name_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let mut a = vec![str(b"class"), arg(args, 0)];
    set_attribute(ctx, this, &mut a).map(|_| Value::Null)
}

pub fn schema_type_info_get(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Null)
}

// ---- DOMAttr ----

getter!(attr_name_get, |ctx, r| Ok(str(&r.doc.borrow().node(r.id).qname())));
getter!(attr_value_get, |ctx, r| Ok(str(&r.doc.borrow().node(r.id).value)));
getter!(owner_element_get, |ctx, r| {
    let p = r.doc.borrow().parent(r.id);
    wrap_opt(ctx, &r, p)
});
getter!(specified_get, |ctx, r| {
    let _ = r;
    Ok(Value::Bool(true))
});

pub fn attr_value_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    r.doc.borrow_mut().node_mut(r.id).value = bytes(&arg(args, 0));
    Ok(Value::Null)
}

/// xmlIsID for an HTML document: id, and name on an a element.
fn is_html_id(d: &Doc, el: Id, attr: Id) -> bool {
    let a = d.node(attr);
    a.ns.is_none() && (a.name == b"id" || (a.name == b"name" && d.node(el).name == b"a"))
}

pub fn is_id(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let d = r.doc.borrow();
    Ok(Value::Bool(d.html && d.parent(r.id).is_some_and(|el| is_html_id(&d, el, r.id))))
}

// ---- DOMCharacterData, DOMText, DOMProcessingInstruction ----

getter!(data_get, |ctx, r| Ok(str(&r.doc.borrow().node(r.id).value)));

pub fn data_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    r.doc.borrow_mut().node_mut(r.id).value = bytes(&arg(args, 0));
    Ok(Value::Null)
}

/// Characters, as DOM counts them in character data (UTF-8 characters).
fn chars(v: &[u8]) -> Vec<&[u8]> {
    let s = String::from_utf8_lossy(v);
    if matches!(s, std::borrow::Cow::Owned(_)) {
        return v.chunks(1).collect();
    }
    let mut out = Vec::new();
    let mut i = 0;
    for c in std::str::from_utf8(v).expect("checked").chars() {
        out.push(&v[i..i + c.len_utf8()]);
        i += c.len_utf8();
    }
    out
}

getter!(length_get, |ctx, r| Ok(Value::Int(chars(&r.doc.borrow().node(r.id).value).len() as i64)));

fn edit(ctx: &mut dyn Ctx, this: &Rc<Object>, offset: i64, count: Option<i64>, insert: Option<&[u8]>, keep_rest: bool) -> R {
    let r = this_node(ctx, this)?;
    let v = r.doc.borrow().node(r.id).value.clone();
    let cs = chars(&v);
    let Ok(off) = usize::try_from(offset) else { return Err(dom_error(ctx, INDEX_SIZE)) };
    if off > cs.len() || count.is_some_and(|c| c < 0) {
        return Err(dom_error(ctx, INDEX_SIZE));
    }
    let end = count.map_or(cs.len(), |c| (off + c as usize).min(cs.len()));
    let mut out: Vec<u8> = cs[..off].concat();
    if let Some(ins) = insert {
        out.extend_from_slice(ins);
    }
    if keep_rest {
        out.extend(cs[end..].concat());
    }
    r.doc.borrow_mut().node_mut(r.id).value = out;
    Ok(Value::Null)
}

pub fn append_data(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let add = bytes(&arg(args, 0));
    r.doc.borrow_mut().node_mut(r.id).value.extend_from_slice(&add);
    Ok(Value::Bool(true))
}

pub fn insert_data(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let data = bytes(&arg(args, 1));
    edit(ctx, this, arg(args, 0).to_int(), Some(0), Some(&data), true).map(|_| Value::Bool(true))
}

pub fn delete_data(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    edit(ctx, this, arg(args, 0).to_int(), Some(arg(args, 1).to_int()), None, true).map(|_| Value::Bool(true))
}

pub fn replace_data(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let data = bytes(&arg(args, 2));
    edit(ctx, this, arg(args, 0).to_int(), Some(arg(args, 1).to_int()), Some(&data), true).map(|_| Value::Bool(true))
}

pub fn substring_data(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let v = r.doc.borrow().node(r.id).value.clone();
    let cs = chars(&v);
    let (offset, count) = (arg(args, 0).to_int(), arg(args, 1).to_int());
    let Ok(off) = usize::try_from(offset) else { return Err(dom_error(ctx, INDEX_SIZE)) };
    if off > cs.len() || count < 0 {
        return Err(dom_error(ctx, INDEX_SIZE));
    }
    let end = (off + count as usize).min(cs.len());
    Ok(str(&cs[off..end].concat()))
}

getter!(whole_text_get, |ctx, r| {
    let d = r.doc.borrow();
    // The text of this node and its adjacent text siblings.
    let mut first = r.id;
    while let Some(p) = d.prev_sibling(first)
        && matches!(d.node(p).kind, Kind::Text | Kind::CData)
    {
        first = p;
    }
    let mut out = Vec::new();
    let mut at = Some(first);
    while let Some(a) = at
        && matches!(d.node(a).kind, Kind::Text | Kind::CData)
    {
        out.extend_from_slice(&d.node(a).value);
        at = d.next_sibling(a);
    }
    Ok(str(&out))
});

pub fn split_text(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let v = r.doc.borrow().node(r.id).value.clone();
    let cs = chars(&v);
    let Ok(off) = usize::try_from(arg(args, 0).to_int()) else { return Err(dom_error(ctx, INDEX_SIZE)) };
    if off > cs.len() {
        return Err(dom_error(ctx, INDEX_SIZE));
    }
    let id = {
        let mut d = r.doc.borrow_mut();
        let kind = d.node(r.id).kind;
        d.node_mut(r.id).value = cs[..off].concat();
        let new = d.add(valued(kind, b"", &cs[off..].concat()));
        if let Some(p) = d.parent(r.id) {
            let next = d.next_sibling(r.id);
            d.insert(p, new, next);
        }
        new
    };
    wrap(ctx, &r.at(id))
}

pub fn is_whitespace_in_element_content(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    Ok(Value::Bool(r.doc.borrow().node(r.id).value.iter().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))))
}

getter!(target_get, |ctx, r| Ok(str(&r.doc.borrow().node(r.id).name)));

// ---- DOMDocumentType, DOMEntity ----

getter!(doctype_name_get, |ctx, r| Ok(str(&r.doc.borrow().node(r.id).name)));
getter!(public_id_get, |ctx, r| Ok(str(r.doc.borrow().node(r.id).public_id.as_deref().unwrap_or_default())));
getter!(system_id_get, |ctx, r| Ok(str(r.doc.borrow().node(r.id).system_id.as_deref().unwrap_or_default())));
getter!(entities_get, |ctx, r| super::lists::named_map(ctx, super::lists::Src::Decls(r, Kind::Entity)));
getter!(notations_get, |ctx, r| super::lists::named_map(ctx, super::lists::Src::Decls(r, Kind::Notation)));
getter!(internal_subset_get, |ctx, r| {
    let d = r.doc.borrow();
    if d.node(r.id).children.is_empty() {
        return Ok(Value::Null);
    }
    let mut out = Out::new(&d, false, false);
    for &c in &d.node(r.id).children {
        out.node(c, 0);
    }
    Ok(str(&out.buf))
});

// ---- DOMDocumentFragment ----

pub fn append_xml(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let xml = bytes(&arg(args, 0));
    if xml.is_empty() {
        return Err(ctx.throw("ValueError", "DOMDocumentFragment::appendXML(): Argument #1 ($data) must not be empty".into()));
    }
    // Parsed as the content of an element, then moved in.
    let mut tmp = Doc::new(Vec::new());
    let wrapped = [b"<w>".as_slice(), &xml, b"</w>"].concat();
    let parser = build::parse(&mut tmp, wrapped, None, 0);
    for e in &parser.errors {
        crate::libxml::report(ctx, e, |ctx| ctx.warn(&format!("DOMDocumentFragment::appendXML(): {} in Entity, line: {}", e.message.trim_end_matches('\n'), e.line)));
    }
    if !parser.well_formed {
        return Ok(Value::Bool(false));
    }
    let Some(w) = tmp.root() else { return Ok(Value::Bool(false)) };
    for c in tmp.node(w).children.clone() {
        let id = r.doc.borrow_mut().copy_from(&tmp, c, true);
        r.doc.borrow_mut().insert(r.id, id, None);
    }
    Ok(Value::Bool(true))
}

pub fn implementation_create_document(ctx: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> R {
    let class = ctx.class(b"DOMDocument").expect("declared");
    let o = ctx.new_instance(&class, &mut [], true)?;
    let r = this_doc(&o);
    let ns = opt_bytes(&arg(args, 0));
    let qname = bytes(&arg(args, 1));
    if let Some(dt) = arg_node(args, 2) {
        let id = move_into(&dt, &r.doc);
        r.doc.borrow_mut().insert(DOC, id, None);
    }
    if !qname.is_empty() {
        let id = {
            let mut d = r.doc.borrow_mut();
            element_ns(ctx, &mut d, ns, &qname)?
        };
        r.doc.borrow_mut().insert(DOC, id, None);
    }
    Ok(Value::Object(o))
}

pub fn implementation_create_document_type(ctx: &mut dyn Ctx, _: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    if name.is_empty() {
        return Err(ctx.throw("ValueError", "DOMImplementation::createDocumentType(): Argument #1 ($qualifiedName) must not be empty".into()));
    }
    let class = ctx.class(b"DOMDocumentType").expect("declared");
    let o = ctx.new_object(&class);
    let mut n = Node::new(Kind::DocType, name);
    n.public_id = Some(bytes(&arg(args, 1))).filter(|v| !v.is_empty());
    n.system_id = Some(bytes(&arg(args, 2))).filter(|v| !v.is_empty());
    orphan(&o, n);
    Ok(Value::Object(o))
}

pub fn implementation_has_feature(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Bool(true))
}

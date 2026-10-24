//! DOMNode and what every DOM class shares: the PHP object of a node
//! (one per node while it lives, as PHP's DOM keeps), DOMException, and
//! the node properties and methods.
//!
//! Reference: php-src ext/dom/node.c, ext/dom/php_dom.c

use super::tree::{DOC, Doc, Id, Kind, Node};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub type R = Result<Value, Throw>;

/// A node: its document and its id there.
#[derive(Clone)]
pub struct NodeRef {
    pub doc: Rc<RefCell<Doc>>,
    pub id: Id,
}

impl NodeRef {
    pub fn kind(&self) -> Kind {
        self.doc.borrow().node(self.id).kind
    }

    pub fn same_doc(&self, other: &NodeRef) -> bool {
        Rc::ptr_eq(&self.doc, &other.doc)
    }

    pub fn at(&self, id: Id) -> NodeRef {
        NodeRef { doc: Rc::clone(&self.doc), id }
    }
}

/// What a DOM node's PHP object holds.
#[derive(Default)]
pub struct DomObj {
    pub node: RefCell<Option<NodeRef>>,
}

pub fn make_node() -> Rc<dyn std::any::Any> {
    Rc::new(DomObj::default())
}

pub fn data(o: &Object) -> &DomObj {
    o.native::<DomObj>().expect("a DOM node object")
}

pub fn bind(o: &Rc<Object>, r: &NodeRef) {
    *data(o).node.borrow_mut() = Some(r.clone());
    r.doc.borrow_mut().set_object(r.id, o);
}

/// The node `this` stands for ("Couldn't fetch" before it has one).
pub fn this_node(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<NodeRef, Throw> {
    match &*data(this).node.borrow() {
        Some(r) => Ok(r.clone()),
        None => Err(ctx.throw("Error", format!("Couldn't fetch {}", String::from_utf8_lossy(this.class.name.as_bytes())))),
    }
}

/// The node object argument `i` is (the stubs checked its class).
pub fn arg_node(args: &[Value], i: usize) -> Option<NodeRef> {
    match args.get(i).map(Value::deref) {
        Some(Value::Object(o)) => o.native::<DomObj>().and_then(|d| d.node.borrow().clone()),
        _ => None,
    }
}

pub fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

pub fn bytes(v: &Value) -> Vec<u8> {
    v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()
}

pub fn opt_bytes(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Null => None,
        v => Some(bytes(v)),
    }
}

pub fn str(b: &[u8]) -> Value {
    Value::Str(PhpStr::new(b.to_vec()))
}

pub fn opt_str(b: Option<&[u8]>) -> Value {
    b.map_or(Value::Null, str)
}

/// The class a node's object is made with.
fn class_for(doc: &Doc, kind: Kind) -> Vec<u8> {
    let name: &[u8] = match kind {
        Kind::Element => b"DOMElement",
        Kind::Attribute => b"DOMAttr",
        Kind::Text => b"DOMText",
        Kind::CData => b"DOMCdataSection",
        Kind::EntityRef => b"DOMEntityReference",
        Kind::Entity => b"DOMEntity",
        Kind::Pi => b"DOMProcessingInstruction",
        Kind::Comment => b"DOMComment",
        Kind::Document => return doc.class.clone(),
        Kind::DocType => b"DOMDocumentType",
        Kind::Fragment => b"DOMDocumentFragment",
        Kind::Notation => b"DOMNotation",
    };
    name.to_vec()
}

/// The PHP object for a node: the one it has, or a new one.
pub fn wrap(ctx: &mut dyn Ctx, r: &NodeRef) -> R {
    if let Some(o) = r.doc.borrow().object(r.id) {
        return Ok(Value::Object(o));
    }
    let name = class_for(&r.doc.borrow(), r.kind());
    let class = ctx.class(&name).ok_or_else(|| ctx.throw("Error", format!("Class \"{}\" not found", String::from_utf8_lossy(&name))))?;
    let o = ctx.new_object(&class);
    bind(&o, r);
    Ok(Value::Object(o))
}

pub fn wrap_opt(ctx: &mut dyn Ctx, r: &NodeRef, id: Option<Id>) -> R {
    match id {
        Some(id) => wrap(ctx, &r.at(id)),
        None => Ok(Value::Null),
    }
}

/// DOMException's codes (DOM_*_ERR) and messages.
pub const INDEX_SIZE: i64 = 1;
pub const HIERARCHY_REQUEST: i64 = 3;
pub const WRONG_DOCUMENT: i64 = 4;
pub const INVALID_CHARACTER: i64 = 5;
pub const NO_MODIFICATION_ALLOWED: i64 = 7;
pub const NOT_FOUND: i64 = 8;
pub const NOT_SUPPORTED: i64 = 9;
pub const INUSE_ATTRIBUTE: i64 = 10;
pub const INVALID_STATE: i64 = 11;
pub const SYNTAX: i64 = 12;
pub const NAMESPACE: i64 = 14;

fn message(code: i64) -> &'static str {
    match code {
        INDEX_SIZE => "Index Size Error",
        2 => "DOM String Size Error",
        HIERARCHY_REQUEST => "Hierarchy Request Error",
        WRONG_DOCUMENT => "Wrong Document Error",
        INVALID_CHARACTER => "Invalid Character Error",
        6 => "No Data Allowed Error",
        NO_MODIFICATION_ALLOWED => "No Modification Allowed Error",
        NOT_FOUND => "Not Found Error",
        NOT_SUPPORTED => "Not Supported Error",
        INUSE_ATTRIBUTE => "Inuse Attribute Error",
        INVALID_STATE => "Invalid State Error",
        SYNTAX => "Syntax Error",
        13 => "Invalid Modification Error",
        NAMESPACE => "Namespace Error",
        15 => "Invalid Access Error",
        16 => "Validation Error",
        _ => "Unhandled Error",
    }
}

/// A DOMException with `code` (php_dom_throw_error).
pub fn dom_error(ctx: &mut dyn Ctx, code: i64) -> Throw {
    dom_error_msg(ctx, code, message(code))
}

pub fn dom_error_msg(ctx: &mut dyn Ctx, code: i64, msg: &str) -> Throw {
    let Some(class) = ctx.class(b"DOMException") else { return ctx.throw("Exception", msg.to_owned()) };
    let mut args = [str(msg.as_bytes()), Value::Int(code)];
    match ctx.new_instance(&class, &mut args, true) {
        Ok(o) => Throw::Object(o),
        Err(t) => t,
    }
}

/// An error where the document's strictErrorChecking decides: a
/// DOMException when on, a warning (and `false` back) when off.
pub fn dom_fail(ctx: &mut dyn Ctx, func: &str, r: &NodeRef, code: i64) -> R {
    if r.doc.borrow().settings.strict_error_checking {
        Err(dom_error(ctx, code))
    } else {
        ctx.warn(&format!("{func}(): {}", message(code)));
        Ok(Value::Bool(false))
    }
}

/// XML's Name production (xmlValidateName), for createElement and friends.
pub fn valid_name(name: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(name) else { return false };
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    crate::parser::is_name_start_char(first as u32) && chars.all(|c| crate::parser::is_name_char(c as u32))
}

/// A qualified name split at its colon: (prefix, local). Errors as
/// php_dom_check_qname does: an empty part is a Namespace Error.
pub fn split_qname(qname: &[u8]) -> Result<(Option<Vec<u8>>, Vec<u8>), i64> {
    if !valid_name(qname) {
        return Err(INVALID_CHARACTER);
    }
    match qname.iter().position(|&b| b == b':') {
        None => Ok((None, qname.to_vec())),
        Some(i) => {
            let (p, l) = (&qname[..i], &qname[i + 1..]);
            if p.is_empty() || l.is_empty() || l.contains(&b':') {
                return Err(NAMESPACE);
            }
            Ok((Some(p.to_vec()), l.to_vec()))
        }
    }
}

/// Brings `child` into `parent`'s document, for an insert: a node made
/// with `new` moves over (its live objects follow); a node of another
/// real document is a Wrong Document Error. The child's id there.
pub fn adopt_for_insert(ctx: &mut dyn Ctx, parent: &NodeRef, child: &NodeRef) -> Result<Id, Throw> {
    if child.same_doc(parent) {
        return Ok(child.id);
    }
    if !child.doc.borrow().orphan {
        return Err(dom_error(ctx, WRONG_DOCUMENT));
    }
    Ok(move_into(child, &parent.doc))
}

/// Moves a subtree to another document, rebinding the objects made for
/// its nodes: its new id.
pub fn move_into(child: &NodeRef, to: &Rc<RefCell<Doc>>) -> Id {
    let mut map = Vec::new();
    let new = {
        let src = child.doc.borrow();
        to.borrow_mut().copy_mapped(&src, child.id, true, &mut map)
    };
    child.doc.borrow_mut().detach(child.id);
    for (old, id) in map {
        let o = child.doc.borrow().object(old);
        if let Some(o) = o {
            bind(&o, &NodeRef { doc: Rc::clone(to), id });
        }
    }
    new
}

/// The checks before `child` goes under `parent` (before `before`):
/// php_dom_pre_insert's hierarchy rules.
pub fn check_insert(ctx: &mut dyn Ctx, parent: &NodeRef, child: &NodeRef) -> Result<(), Throw> {
    let pk = parent.kind();
    if !matches!(pk, Kind::Element | Kind::Document | Kind::Fragment | Kind::EntityRef) {
        return Err(dom_error(ctx, HIERARCHY_REQUEST));
    }
    let ck = child.kind();
    if matches!(ck, Kind::Document | Kind::Attribute | Kind::Entity | Kind::Notation) {
        return Err(dom_error(ctx, HIERARCHY_REQUEST));
    }
    if child.same_doc(parent) && child.doc.borrow().is_ancestor_or_self(child.id, parent.id) {
        return Err(dom_error(ctx, HIERARCHY_REQUEST));
    }
    if pk == Kind::Document && ck == Kind::Element && parent.doc.borrow().root().is_some_and(|r| !(child.same_doc(parent) && r == child.id)) {
        return Err(dom_error_msg(ctx, HIERARCHY_REQUEST, "Cannot have more than one element child in a document"));
    }
    Ok(())
}

/// Inserts `child` under `parent` before `before` (None: at the end),
/// after the checks: `child`'s id in the parent's document.
pub fn insert_node(ctx: &mut dyn Ctx, parent: &NodeRef, child: &NodeRef, before: Option<Id>) -> Result<Id, Throw> {
    check_insert(ctx, parent, child)?;
    let id = adopt_for_insert(ctx, parent, child)?;
    parent.doc.borrow_mut().insert(parent.id, id, before);
    Ok(id)
}

/// DOM nodeName.
pub fn node_name(doc: &Doc, id: Id) -> Vec<u8> {
    let n = doc.node(id);
    match n.kind {
        Kind::Element | Kind::Attribute => n.qname(),
        Kind::Text => b"#text".to_vec(),
        Kind::CData => b"#cdata-section".to_vec(),
        Kind::Comment => b"#comment".to_vec(),
        Kind::Document => b"#document".to_vec(),
        Kind::Fragment => b"#document-fragment".to_vec(),
        Kind::Pi | Kind::EntityRef | Kind::DocType | Kind::Entity | Kind::Notation => n.name.clone(),
    }
}

/// Replaces a node's children with one text node holding `text` (none
/// for empty text): textContent and nodeValue writes.
pub fn set_text(doc: &mut Doc, id: Id, text: &[u8]) {
    match doc.node(id).kind {
        Kind::Element | Kind::Fragment | Kind::Attribute | Kind::EntityRef => {
            if doc.node(id).kind == Kind::Attribute {
                doc.node_mut(id).value = text.to_vec();
                return;
            }
            for c in doc.node(id).children.clone() {
                doc.detach(c);
            }
            if !text.is_empty() {
                let mut t = Node::new(Kind::Text, Vec::new());
                t.value = text.to_vec();
                let t = doc.add(t);
                doc.insert(id, t, None);
            }
        }
        Kind::Text | Kind::CData | Kind::Comment | Kind::Pi => doc.node_mut(id).value = text.to_vec(),
        _ => {}
    }
}

/// Content given as markup text (xmlStringGetNodeList): character and
/// predefined entity references are resolved, other `&name;` become
/// entity reference nodes. An `&` with no `;` after it warns and the
/// content is dropped, as libxml2 drops it. Replaces `id`'s children.
pub fn set_content_parsed(ctx: &mut dyn Ctx, func: &str, doc: &mut Doc, id: Id, value: &[u8]) {
    for c in doc.node(id).children.clone() {
        doc.detach(c);
    }
    let mut parts: Vec<(Kind, Vec<u8>)> = Vec::new();
    let mut text = Vec::new();
    let mut i = 0;
    while i < value.len() {
        if value[i] != b'&' {
            text.push(value[i]);
            i += 1;
            continue;
        }
        let rest = &value[i + 1..];
        let Some(end) = rest.iter().position(|&b| b == b';') else {
            let shown = String::from_utf8_lossy(rest);
            ctx.warn(&format!("{func}(): unterminated entity reference {shown:>15}"));
            return;
        };
        let name = &rest[..end];
        if let Some(num) = name.strip_prefix(b"#") {
            let n = match num.strip_prefix(b"x") {
                Some(hex) => u32::from_str_radix(std::str::from_utf8(hex).unwrap_or(""), 16).ok(),
                None => std::str::from_utf8(num).ok().and_then(|d| d.parse().ok()),
            };
            if let Some(c) = n.and_then(char::from_u32) {
                let mut b = [0; 4];
                text.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        } else {
            match name {
                b"amp" => text.push(b'&'),
                b"lt" => text.push(b'<'),
                b"gt" => text.push(b'>'),
                b"quot" => text.push(b'"'),
                b"apos" => text.push(b'\''),
                _ => {
                    if !text.is_empty() {
                        parts.push((Kind::Text, std::mem::take(&mut text)));
                    }
                    parts.push((Kind::EntityRef, name.to_vec()));
                }
            }
        }
        i += end + 2;
    }
    if !text.is_empty() {
        parts.push((Kind::Text, text));
    }
    for (kind, v) in parts {
        let node = if kind == Kind::Text {
            let mut t = Node::new(Kind::Text, Vec::new());
            t.value = v;
            t
        } else {
            Node::new(Kind::EntityRef, v)
        };
        let c = doc.add(node);
        doc.insert(id, c, None);
    }
}

// ---- DOMNode's properties ----

macro_rules! getter {
    ($name:ident, |$ctx:ident, $r:ident| $body:expr) => {
        pub fn $name($ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
            let $r = this_node($ctx, this)?;
            $body
        }
    };
}
pub(crate) use getter;

getter!(node_name_get, |ctx, r| Ok(str(&node_name(&r.doc.borrow(), r.id))));
getter!(node_type_get, |ctx, r| Ok(Value::Int(r.kind() as i64)));
getter!(node_value_get, |ctx, r| {
    let d = r.doc.borrow();
    let n = d.node(r.id);
    Ok(match n.kind {
        Kind::Text | Kind::CData | Kind::Comment | Kind::Pi | Kind::Attribute => str(&n.value),
        _ => Value::Null,
    })
});

pub fn node_value_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let v = bytes(&arg(args, 0));
    match r.kind() {
        Kind::Element => set_content_parsed(ctx, "DOMNode::$nodeValue", &mut r.doc.borrow_mut(), r.id, &v),
        Kind::Attribute | Kind::Text | Kind::CData | Kind::Comment | Kind::Pi => set_text(&mut r.doc.borrow_mut(), r.id, &v),
        _ => {}
    }
    Ok(Value::Null)
}

getter!(parent_node_get, |ctx, r| {
    let p = r.doc.borrow().parent(r.id).filter(|_| r.kind() != Kind::Attribute);
    wrap_opt(ctx, &r, p)
});
getter!(parent_element_get, |ctx, r| {
    let p = r.doc.borrow().parent(r.id).filter(|&p| r.kind() != Kind::Attribute && r.doc.borrow().node(p).kind == Kind::Element);
    wrap_opt(ctx, &r, p)
});
getter!(child_nodes_get, |ctx, r| super::lists::node_list(ctx, super::lists::Src::Children(r)));
getter!(first_child_get, |ctx, r| {
    let c = r.doc.borrow().node(r.id).children.first().copied();
    wrap_opt(ctx, &r, c)
});
getter!(last_child_get, |ctx, r| {
    let c = r.doc.borrow().node(r.id).children.last().copied();
    wrap_opt(ctx, &r, c)
});
getter!(previous_sibling_get, |ctx, r| {
    let c = r.doc.borrow().prev_sibling(r.id);
    wrap_opt(ctx, &r, c)
});
getter!(next_sibling_get, |ctx, r| {
    let c = r.doc.borrow().next_sibling(r.id);
    wrap_opt(ctx, &r, c)
});
getter!(attributes_get, |ctx, r| {
    if r.kind() == Kind::Element {
        super::lists::named_map(ctx, super::lists::Src::Attrs(r))
    } else {
        Ok(Value::Null)
    }
});
getter!(is_connected_get, |ctx, r| {
    let d = r.doc.borrow();
    let mut at = r.id;
    while let Some(p) = d.parent(at) {
        at = p;
    }
    Ok(Value::Bool(at == DOC && !d.orphan))
});
getter!(owner_document_get, |ctx, r| {
    if r.kind() == Kind::Document || r.doc.borrow().orphan {
        return Ok(Value::Null);
    }
    wrap(ctx, &r.at(DOC))
});
getter!(namespace_uri_get, |ctx, r| {
    let d = r.doc.borrow();
    let n = d.node(r.id);
    Ok(match n.kind {
        Kind::Element | Kind::Attribute => opt_str(n.ns.as_deref()),
        _ => Value::Null,
    })
});
getter!(prefix_get, |ctx, r| {
    let d = r.doc.borrow();
    let n = d.node(r.id);
    Ok(match n.kind {
        Kind::Element | Kind::Attribute => str(n.prefix.as_deref().unwrap_or_default()),
        _ => str(b""),
    })
});
getter!(local_name_get, |ctx, r| {
    let d = r.doc.borrow();
    let n = d.node(r.id);
    Ok(match n.kind {
        Kind::Element | Kind::Attribute => str(&n.name),
        _ => Value::Null,
    })
});
getter!(base_uri_get, |ctx, r| Ok(opt_str(r.doc.borrow().settings.url.as_deref())));
getter!(text_content_get, |ctx, r| Ok(str(&r.doc.borrow().text_content(r.id))));

pub fn text_content_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let v = bytes(&arg(args, 0));
    set_text(&mut r.doc.borrow_mut(), r.id, &v);
    Ok(Value::Null)
}

// ---- DOMNode's methods ----

pub fn append_child(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let parent = this_node(ctx, this)?;
    let child = arg_node(args, 0).ok_or_else(|| ctx.throw("TypeError", "DOMNode::appendChild(): Argument #1 ($node) must be of type DOMNode".into()))?;
    let is_fragment = child.kind() == Kind::Fragment;
    let id = insert_node(ctx, &parent, &child, None)?;
    if is_fragment {
        return Ok(arg(args, 0));
    }
    wrap(ctx, &parent.at(id))
}

pub fn insert_before(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let parent = this_node(ctx, this)?;
    let child = arg_node(args, 0).ok_or_else(|| ctx.throw("TypeError", "DOMNode::insertBefore(): Argument #1 ($node) must be of type DOMNode".into()))?;
    let before = match arg_node(args, 1) {
        Some(b) => {
            if !b.same_doc(&parent) || b.doc.borrow().parent(b.id) != Some(parent.id) || b.kind() == Kind::Attribute {
                return Err(dom_error(ctx, NOT_FOUND));
            }
            Some(b.id)
        }
        None => None,
    };
    let is_fragment = child.kind() == Kind::Fragment;
    let id = insert_node(ctx, &parent, &child, before)?;
    if is_fragment {
        return Ok(arg(args, 0));
    }
    wrap(ctx, &parent.at(id))
}

pub fn remove_child(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let parent = this_node(ctx, this)?;
    let Some(child) = arg_node(args, 0) else { return Ok(Value::Bool(false)) };
    if !child.same_doc(&parent) || child.doc.borrow().parent(child.id) != Some(parent.id) || child.kind() == Kind::Attribute {
        return dom_fail(ctx, "DOMNode::removeChild", &parent, NOT_FOUND);
    }
    parent.doc.borrow_mut().detach(child.id);
    Ok(arg(args, 0))
}

pub fn replace_child(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let parent = this_node(ctx, this)?;
    let (Some(new), Some(old)) = (arg_node(args, 0), arg_node(args, 1)) else { return Ok(Value::Bool(false)) };
    if !old.same_doc(&parent) || old.doc.borrow().parent(old.id) != Some(parent.id) {
        return dom_fail(ctx, "DOMNode::replaceChild", &parent, NOT_FOUND);
    }
    if new.same_doc(&old) && new.id == old.id {
        return Ok(arg(args, 1));
    }
    check_insert(ctx, &parent, &new)?;
    let id = adopt_for_insert(ctx, &parent, &new)?;
    let mut d = parent.doc.borrow_mut();
    d.insert(parent.id, id, Some(old.id));
    d.detach(old.id);
    Ok(arg(args, 1))
}

pub fn has_child_nodes(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    Ok(Value::Bool(!r.doc.borrow().node(r.id).children.is_empty()))
}

pub fn has_attributes(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let d = r.doc.borrow();
    let n = d.node(r.id);
    Ok(Value::Bool(n.kind == Kind::Element && !n.attrs.is_empty()))
}

pub fn clone_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let deep = arg(args, 0).to_bool();
    let id = {
        let src = r.doc.borrow().clone_tree();
        r.doc.borrow_mut().copy_from(&src, r.id, deep)
    };
    wrap(ctx, &r.at(id))
}

pub fn is_same_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    Ok(Value::Bool(arg_node(args, 0).is_some_and(|o| o.same_doc(&r) && o.id == r.id)))
}

pub fn contains(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    Ok(Value::Bool(arg_node(args, 0).is_some_and(|o| o.same_doc(&r) && r.doc.borrow().is_ancestor_or_self(r.id, o.id))))
}

pub fn get_root_node(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let mut at = r.id;
    while let Some(p) = r.doc.borrow().parent(at) {
        at = p;
    }
    wrap(ctx, &r.at(at))
}

pub fn get_line_no(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    Ok(Value::Int(r.doc.borrow().node(r.id).line as i64))
}

pub fn normalize(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    normalize_at(&mut r.doc.borrow_mut(), r.id);
    Ok(Value::Null)
}

/// Merges adjacent text nodes and drops empty ones, all the way down.
fn normalize_at(d: &mut Doc, id: Id) {
    let kids = d.node(id).children.clone();
    let mut last_text: Option<Id> = None;
    for c in kids {
        match d.node(c).kind {
            Kind::Text => {
                if d.node(c).value.is_empty() {
                    d.detach(c);
                    continue;
                }
                match last_text {
                    Some(t) => {
                        let v = d.node(c).value.clone();
                        d.node_mut(t).value.extend_from_slice(&v);
                        d.detach(c);
                    }
                    None => last_text = Some(c),
                }
            }
            Kind::Element => {
                last_text = None;
                normalize_at(d, c);
            }
            _ => last_text = None,
        }
    }
}

pub fn lookup_namespace_uri(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let prefix = opt_bytes(&arg(args, 0)).filter(|p| !p.is_empty());
    let at = element_for_lookup(&r);
    let Some(at) = at else { return Ok(Value::Null) };
    Ok(opt_str(r.doc.borrow().lookup_ns(at, prefix.as_deref()).as_deref()))
}

pub fn lookup_prefix(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let uri = bytes(&arg(args, 0));
    let Some(at) = element_for_lookup(&r) else { return Ok(Value::Null) };
    if uri.is_empty() {
        return Ok(Value::Null);
    }
    Ok(match r.doc.borrow().lookup_prefix(at, &uri) {
        Some(Some(p)) => str(&p),
        _ => Value::Null,
    })
}

pub fn is_default_namespace(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let uri = bytes(&arg(args, 0));
    let Some(at) = element_for_lookup(&r) else { return Ok(Value::Bool(false)) };
    let default = r.doc.borrow().lookup_ns(at, None);
    Ok(Value::Bool(!uri.is_empty() && default.as_deref() == Some(&uri[..])))
}

/// The element namespace lookups start from (the document's element, an
/// attribute's owner).
fn element_for_lookup(r: &NodeRef) -> Option<Id> {
    let d = r.doc.borrow();
    match d.node(r.id).kind {
        Kind::Document => d.root(),
        Kind::Attribute => d.parent(r.id),
        Kind::Element => Some(r.id),
        _ => d.parent(r.id),
    }
}

pub fn is_equal_node(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let Some(o) = arg_node(args, 0) else { return Ok(Value::Bool(false)) };
    let a = r.doc.borrow();
    if o.same_doc(&r) {
        return Ok(Value::Bool(equal(&a, r.id, &a, o.id)));
    }
    let b = o.doc.borrow();
    Ok(Value::Bool(equal(&a, r.id, &b, o.id)))
}

/// DOM's node equality: same kind, names, values, attributes (any order)
/// and children.
fn equal(da: &Doc, a: Id, db: &Doc, b: Id) -> bool {
    let (x, y) = (da.node(a), db.node(b));
    if x.kind != y.kind || x.name != y.name || x.prefix != y.prefix || x.ns != y.ns || x.value != y.value {
        return false;
    }
    if x.kind == Kind::DocType && (x.public_id != y.public_id || x.system_id != y.system_id) {
        return false;
    }
    if x.attrs.len() != y.attrs.len() || x.children.len() != y.children.len() {
        return false;
    }
    let attrs_match = x.attrs.iter().all(|&xa| {
        let n = da.node(xa);
        y.attrs.iter().any(|&ya| equal(da, xa, db, ya) && db.node(ya).name == n.name)
    });
    attrs_match && x.children.iter().zip(&y.children).all(|(&c, &d)| equal(da, c, db, d))
}

/// The nodes and strings given to append()/before()/...: each string
/// becomes a text node in `r`'s document.
pub fn nodes_from_args(ctx: &mut dyn Ctx, r: &NodeRef, args: &[Value]) -> Result<Vec<NodeRef>, Throw> {
    let mut out = Vec::new();
    for v in args {
        match v.deref() {
            Value::Object(o) if o.native::<DomObj>().is_some() => {
                let n = o.native::<DomObj>().and_then(|d| d.node.borrow().clone()).ok_or_else(|| ctx.throw("Error", "Couldn't fetch DOMNode".into()))?;
                out.push(n);
            }
            v => {
                let mut t = Node::new(Kind::Text, Vec::new());
                t.value = bytes(&v);
                let id = r.doc.borrow_mut().add(t);
                out.push(r.at(id));
            }
        }
    }
    Ok(out)
}

/// Inserts each of `nodes` under `parent` before `before`, in order.
pub fn insert_all(ctx: &mut dyn Ctx, parent: &NodeRef, nodes: &[NodeRef], before: Option<Id>) -> Result<(), Throw> {
    for n in nodes {
        check_insert(ctx, parent, n)?;
    }
    for n in nodes {
        let id = adopt_for_insert(ctx, parent, n)?;
        parent.doc.borrow_mut().insert(parent.id, id, before);
    }
    Ok(())
}

// ChildNode
pub fn remove(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    r.doc.borrow_mut().detach(r.id);
    Ok(Value::Null)
}

pub fn before(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let Some(parent) = r.doc.borrow().parent(r.id) else { return Ok(Value::Null) };
    let nodes = nodes_from_args(ctx, &r, args)?;
    // The first sibling before this one that is not being moved.
    let mut anchor = Some(r.id);
    while let Some(a) = anchor
        && nodes.iter().any(|n| n.same_doc(&r) && n.id == a)
    {
        anchor = r.doc.borrow().next_sibling(a);
    }
    insert_all(ctx, &r.at(parent), &nodes, anchor)?;
    Ok(Value::Null)
}

pub fn after(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let Some(parent) = r.doc.borrow().parent(r.id) else { return Ok(Value::Null) };
    let nodes = nodes_from_args(ctx, &r, args)?;
    let mut anchor = r.doc.borrow().next_sibling(r.id);
    while let Some(a) = anchor
        && nodes.iter().any(|n| n.same_doc(&r) && n.id == a)
    {
        anchor = r.doc.borrow().next_sibling(a);
    }
    insert_all(ctx, &r.at(parent), &nodes, anchor)?;
    Ok(Value::Null)
}

pub fn replace_with(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let Some(parent) = r.doc.borrow().parent(r.id) else { return Ok(Value::Null) };
    let nodes = nodes_from_args(ctx, &r, args)?;
    let mut anchor = r.doc.borrow().next_sibling(r.id);
    while let Some(a) = anchor
        && nodes.iter().any(|n| n.same_doc(&r) && n.id == a)
    {
        anchor = r.doc.borrow().next_sibling(a);
    }
    let keep = nodes.iter().any(|n| n.same_doc(&r) && n.id == r.id);
    if !keep {
        r.doc.borrow_mut().detach(r.id);
    }
    insert_all(ctx, &r.at(parent), &nodes, anchor)?;
    Ok(Value::Null)
}

// ParentNode
pub fn append(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let nodes = nodes_from_args(ctx, &r, args)?;
    insert_all(ctx, &r, &nodes, None)?;
    Ok(Value::Null)
}

pub fn prepend(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let nodes = nodes_from_args(ctx, &r, args)?;
    let first = r.doc.borrow().node(r.id).children.iter().copied().find(|&c| !nodes.iter().any(|n| n.same_doc(&r) && n.id == c));
    insert_all(ctx, &r, &nodes, first)?;
    Ok(Value::Null)
}

pub fn replace_children(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let r = this_node(ctx, this)?;
    let nodes = nodes_from_args(ctx, &r, args)?;
    for n in &nodes {
        check_insert(ctx, &r, n)?;
    }
    let old = r.doc.borrow().node(r.id).children.clone();
    for c in old {
        r.doc.borrow_mut().detach(c);
    }
    insert_all(ctx, &r, &nodes, None)?;
    Ok(Value::Null)
}

/// Element children counts and neighbours (ParentNode/ChildNode props).
pub fn element_children(d: &Doc, id: Id) -> Vec<Id> {
    d.node(id).children.iter().copied().filter(|&c| d.node(c).kind == Kind::Element).collect()
}

getter!(first_element_child_get, |ctx, r| {
    let c = element_children(&r.doc.borrow(), r.id).first().copied();
    wrap_opt(ctx, &r, c)
});
getter!(last_element_child_get, |ctx, r| {
    let c = element_children(&r.doc.borrow(), r.id).last().copied();
    wrap_opt(ctx, &r, c)
});
getter!(child_element_count_get, |ctx, r| Ok(Value::Int(element_children(&r.doc.borrow(), r.id).len() as i64)));
getter!(previous_element_sibling_get, |ctx, r| {
    let mut at = r.doc.borrow().prev_sibling(r.id);
    while let Some(a) = at
        && r.doc.borrow().node(a).kind != Kind::Element
    {
        at = r.doc.borrow().prev_sibling(a);
    }
    wrap_opt(ctx, &r, at)
});
getter!(next_element_sibling_get, |ctx, r| {
    let mut at = r.doc.borrow().next_sibling(r.id);
    while let Some(a) = at
        && r.doc.borrow().node(a).kind != Kind::Element
    {
        at = r.doc.borrow().next_sibling(a);
    }
    wrap_opt(ctx, &r, at)
});

impl Doc {
    /// A snapshot to copy from (a node copied within its own document).
    pub fn clone_tree(&self) -> Doc {
        Doc { nodes: self.nodes.clone(), objects: Vec::new(), settings: self.settings.clone(), class: self.class.clone(), orphan: self.orphan }
    }
}

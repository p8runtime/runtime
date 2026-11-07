//! SimpleXML (ext/simplexml) on the DOM tree: SimpleXMLElement,
//! SimpleXMLIterator, simplexml_load_string/file, simplexml_import_dom.
//!
//! An object is a node plus how it iterates (php_sxe_object's `iter`):
//! `$x->item` is $x's node iterating its children named "item", `$x['a']`
//! an attribute, `$x->attributes()` the node iterating its attributes.
//! The walking below follows simplexml.c function for function, on the
//! tree's sibling order (libxml2's `next` / `children` / `properties`).
//!
//! Reference: php-src ext/simplexml/simplexml.c

use crate::dom::node::{DomObj, NodeRef, arg, bytes, opt_bytes, str};
use crate::dom::serialize::Out;
use crate::dom::tree::{DOC, Doc, Id, Kind, Node};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassHooks, ClassSpec, Ctx, Throw};
use p8_runtime::object::{Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

type R = Result<Value, Throw>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum IterKind {
    #[default]
    None,
    Element,
    Child,
    AttrList,
}

#[derive(Clone, Default)]
struct Iter {
    kind: IterKind,
    name: Option<Vec<u8>>,
    ns: Option<Vec<u8>>,
    is_prefix: bool,
}

/// What a SimpleXMLElement holds (php_sxe_object).
#[derive(Default)]
pub struct Sxe {
    node: RefCell<Option<NodeRef>>,
    iter: RefCell<Iter>,
    /// The iterator's current item.
    data: RefCell<Option<Rc<Object>>>,
    /// registerXPathNamespace's prefixes.
    xpath_ns: RefCell<Vec<(Vec<u8>, Vec<u8>)>>,
}

pub fn make_sxe() -> Rc<dyn std::any::Any> {
    Rc::new(Sxe::default())
}

fn sxe(o: &Object) -> &Sxe {
    o.native::<Sxe>().expect("a SimpleXMLElement")
}

/// GET_NODE: the object's node ("not properly initialized" without one).
fn get_node(ctx: Option<&mut dyn Ctx>, o: &Object) -> Result<Option<NodeRef>, Throw> {
    match sxe(o).node.borrow().clone() {
        Some(n) => Ok(Some(n)),
        None => match ctx {
            Some(ctx) => Err(ctx.throw("Error", "SimpleXMLElement is not properly initialized".into())),
            None => Ok(None),
        },
    }
}

// ---- libxml2's view of the tree ----

fn first_child(d: &Doc, id: Id) -> Option<Id> {
    d.node(id).children.first().copied()
}

fn first_attr(d: &Doc, id: Id) -> Option<Id> {
    if d.node(id).kind == Kind::Element { d.node(id).attrs.first().copied() } else { None }
}

/// The next sibling (the next attribute, for an attribute).
fn next(d: &Doc, id: Id) -> Option<Id> {
    let p = d.parent(id)?;
    let list = if d.node(id).kind == Kind::Attribute { &d.node(p).attrs } else { &d.node(p).children };
    let i = list.iter().position(|&c| c == id)?;
    list.get(i + 1).copied()
}

fn prev(d: &Doc, id: Id) -> Option<Id> {
    let p = d.parent(id)?;
    let list = if d.node(id).kind == Kind::Attribute { &d.node(p).attrs } else { &d.node(p).children };
    let i = list.iter().position(|&c| c == id)?;
    i.checked_sub(1).map(|j| list[j])
}

/// match_ns: no filter matches a node without a prefixed namespace;
/// otherwise the namespace's prefix (or URI) must equal it.
fn match_ns(d: &Doc, id: Id, ns: Option<&[u8]>, is_prefix: bool) -> bool {
    let n = d.node(id);
    if ns.is_none() && (n.ns.is_none() || n.prefix.is_none()) {
        return true;
    }
    match &n.ns {
        Some(uri) => {
            let have = if is_prefix { n.prefix.as_deref() } else { Some(uri.as_slice()) };
            have == ns
        }
        None => false,
    }
}

/// xmlNodeListGetString(doc, list, 1): the text of `list` and its
/// following siblings; entity references give their replacement text.
fn list_string(d: &Doc, first: Option<Id>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = first;
    while let Some(id) = at {
        let n = d.node(id);
        match n.kind {
            Kind::Text | Kind::CData | Kind::Attribute => out.extend_from_slice(&n.value),
            Kind::EntityRef => match entity_value(d, &n.name) {
                Some(v) => out.extend_from_slice(&v),
                None => {
                    out.push(b'&');
                    out.extend_from_slice(&n.name);
                    out.push(b';');
                }
            },
            _ => {}
        }
        at = next(d, id);
    }
    out
}

fn entity_value(d: &Doc, name: &[u8]) -> Option<Vec<u8>> {
    let dt = d.node(DOC).children.iter().copied().find(|&c| d.node(c).kind == Kind::DocType)?;
    d.node(dt).children.iter().copied().find(|&e| d.node(e).kind == Kind::Entity && d.node(e).name == name).map(|e| d.node(e).value.clone())
}

/// A node's text children as one string (a node's "children" list; an
/// attribute is its own value).
fn node_text(d: &Doc, id: Id) -> Vec<u8> {
    if d.node(id).kind == Kind::Attribute {
        return d.node(id).value.clone();
    }
    list_string(d, first_child(d, id))
}

/// A node's libxml2 name: comments are "comment", CDATA has none.
fn xml_name(d: &Doc, id: Id) -> Option<Vec<u8>> {
    let n = d.node(id);
    match n.kind {
        Kind::Comment => Some(b"comment".to_vec()),
        Kind::Text => Some(b"text".to_vec()),
        Kind::CData | Kind::Document | Kind::Fragment => None,
        _ => Some(n.name.clone()).filter(|v| !v.is_empty()),
    }
}

fn is_blank(d: &Doc, id: Id) -> bool {
    let n = d.node(id);
    matches!(n.kind, Kind::Text | Kind::CData) && n.value.iter().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
}

// ---- iteration (php_sxe_iterator_fetch / reset) ----

fn iter_fetch(d: &Doc, it: &Iter, mut at: Option<Id>) -> Option<Id> {
    let ns = it.ns.as_deref();
    while let Some(id) = at {
        let n = d.node(id);
        let hit = match it.kind {
            IterKind::AttrList => n.kind == Kind::Attribute && it.name.as_ref().is_none_or(|name| n.name == *name) && match_ns(d, id, ns, it.is_prefix),
            IterKind::Element if it.name.is_some() => n.kind == Kind::Element && it.name.as_deref() == Some(&n.name[..]) && match_ns(d, id, ns, it.is_prefix),
            _ => n.kind == Kind::Element && match_ns(d, id, ns, it.is_prefix),
        };
        if hit {
            return Some(id);
        }
        at = next(d, id);
    }
    None
}

/// php_sxe_reset_iterator_no_clear_iter_data without use_data: the first
/// node the object iterates.
fn first_iterated(d: &Doc, node: Id, it: &Iter) -> Option<Id> {
    let start = match it.kind {
        IterKind::AttrList => first_attr(d, node),
        _ => first_child(d, node),
    };
    iter_fetch(d, it, start)
}

/// php_sxe_get_first_node_non_destructive.
fn first_node(d: &Doc, node: Id, it: &Iter) -> Option<Id> {
    if it.kind != IterKind::None { first_iterated(d, node, it) } else { Some(node) }
}

/// node_as_zval: a new object of `this`'s class for `id`.
fn wrap(this: &Object, r: &NodeRef, id: Id, kind: IterKind, name: Option<Vec<u8>>, ns: Option<Vec<u8>>, is_prefix: bool) -> Value {
    let data = Sxe {
        node: RefCell::new(Some(r.at(id))),
        iter: RefCell::new(Iter { kind, name, ns: ns.filter(|n| !n.is_empty()), is_prefix }),
        data: RefCell::new(None),
        xpath_ns: RefCell::new(Vec::new()),
    };
    Value::object(this.sibling(Rc::clone(&this.class), Some(Rc::new(data))))
}

fn element_by_offset(d: &Doc, it: &Iter, offset: i64, mut at: Option<Id>, cnt: &mut i64) -> Option<Id> {
    if it.kind == IterKind::None {
        *cnt = 0;
        return if offset == 0 { at } else { None };
    }
    let mut ndx = 0;
    while let Some(id) = at {
        if offset >= 0 && ndx > offset {
            break;
        }
        let n = d.node(id);
        if n.kind == Kind::Element && match_ns(d, id, it.ns.as_deref(), it.is_prefix) && (it.kind == IterKind::Child || (it.kind == IterKind::Element && it.name.as_deref() == Some(&n.name[..]))) {
            if ndx == offset {
                *cnt = ndx;
                return Some(id);
            }
            ndx += 1;
        }
        at = next(d, id);
    }
    *cnt = ndx;
    None
}

fn find_element_by_name(d: &Doc, it: &Iter, mut at: Option<Id>, name: &[u8]) -> Option<Id> {
    while let Some(id) = at {
        if d.node(id).kind == Kind::Element && match_ns(d, id, it.ns.as_deref(), it.is_prefix) && d.node(id).name == name {
            return Some(id);
        }
        at = next(d, id);
    }
    None
}

/// What a property or dimension access names.
enum Member {
    None,
    Index(i64),
    Name(Vec<u8>),
}

fn member(v: &Value) -> Member {
    let v = v.deref();
    if v.is_null_or_undef() {
        Member::None
    } else if let Some(i) = v.as_int() {
        Member::Index(i)
    } else {
        Member::Name(bytes(&v))
    }
}

// ---- reads (sxe_prop_dim_read) ----

fn prop_dim_read(ctx: &mut dyn Ctx, this: &Rc<Object>, m: Member, mut elements: bool, mut attribs: bool) -> R {
    let it = sxe(this).iter.borrow().clone();
    if matches!(m, Member::None) && it.kind == IterKind::AttrList {
        return Err(ctx.throw("Error", "Cannot create unnamed attribute".into()));
    }
    if matches!(m, Member::None | Member::Index(_)) && it.kind != IterKind::AttrList {
        attribs = false;
        elements = true;
    }
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(Value::NULL) };
    let d = r.doc.borrow();
    let mut node = Some(r.id);
    let mut attr: Option<Id> = None;
    let mut test = false;
    if it.kind == IterKind::AttrList {
        attribs = true;
        elements = false;
        node = first_node(&d, r.id, &it);
        attr = node;
        test = it.name.is_some();
    } else if it.kind != IterKind::Child {
        node = first_node(&d, r.id, &it);
        attr = node.and_then(|n| first_attr(&d, n));
    }
    let Some(node) = node else { return Ok(Value::NULL) };
    if attribs && (!matches!(m, Member::Index(_)) || it.kind == IterKind::AttrList) {
        let mut ndx = 0;
        while let Some(a) = attr {
            let an = d.node(a);
            let ok = (!test || it.name.as_deref() == Some(&an.name[..])) && match_ns(&d, a, it.ns.as_deref(), it.is_prefix);
            match &m {
                Member::Index(i) if ok => {
                    if ndx == *i {
                        return Ok(wrap(this, &r, a, IterKind::None, None, it.ns.clone(), it.is_prefix));
                    }
                    ndx += 1;
                }
                Member::Name(name) if ok && an.name == *name => return Ok(wrap(this, &r, a, IterKind::None, None, it.ns.clone(), it.is_prefix)),
                _ => {}
            }
            attr = next(&d, a);
        }
    }
    if elements {
        match &m {
            Member::None | Member::Index(_) => {
                let mut node = Some(node);
                if it.kind == IterKind::Child {
                    node = first_node(&d, r.id, &it);
                }
                let found = match (&m, it.kind) {
                    (Member::Index(i), IterKind::None) => node.filter(|_| *i == 0),
                    (Member::Index(i), _) => element_by_offset(&d, &it, *i, node, &mut 0),
                    _ => None,
                };
                if let Some(n) = found {
                    return Ok(wrap(this, &r, n, IterKind::None, None, it.ns.clone(), it.is_prefix));
                }
            }
            Member::Name(name) => return Ok(wrap(this, &r, node, IterKind::Element, Some(name.clone()), it.ns.clone(), it.is_prefix)),
        }
    }
    Ok(Value::NULL)
}

fn prop_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prop_dim_read(ctx, this, Member::Name(bytes(&arg(args, 0))), true, false)
}

fn dim_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prop_dim_read(ctx, this, member(&arg(args, 0)), false, true)
}

// ---- isset (sxe_prop_dim_exists) ----

fn prop_dim_exists(ctx: &mut dyn Ctx, this: &Rc<Object>, m: Member, mut elements: bool, mut attribs: bool) -> Result<bool, Throw> {
    let it = sxe(this).iter.borrow().clone();
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(false) };
    let d = r.doc.borrow();
    let mut node = Some(r.id);
    if let Member::Index(_) = m
        && it.kind != IterKind::AttrList
    {
        attribs = false;
        elements = true;
        if it.kind == IterKind::Child {
            node = first_node(&d, r.id, &it);
        }
    }
    let mut attr = None;
    let mut test = false;
    if it.kind == IterKind::AttrList {
        attribs = true;
        elements = false;
        node = first_node(&d, r.id, &it);
        attr = node;
        test = it.name.is_some();
    } else if it.kind != IterKind::Child {
        node = first_node(&d, r.id, &it);
        attr = node.and_then(|n| first_attr(&d, n));
    }
    let Some(node) = node else { return Ok(false) };
    if attribs {
        let mut ndx = 0;
        while let Some(a) = attr {
            let an = d.node(a);
            let ok = (!test || it.name.as_deref() == Some(&an.name[..])) && match_ns(&d, a, it.ns.as_deref(), it.is_prefix);
            match &m {
                Member::Index(i) if ok => {
                    if ndx == *i {
                        return Ok(true);
                    }
                    ndx += 1;
                }
                Member::Name(name) if ok && an.name == *name => return Ok(true),
                _ => {}
            }
            attr = next(&d, a);
        }
    }
    if elements {
        let found = match &m {
            Member::Index(i) => {
                let start = if it.kind == IterKind::Child { first_node(&d, r.id, &it) } else { Some(node) };
                element_by_offset(&d, &it, *i, start, &mut 0)
            }
            Member::Name(name) => find_element_by_name(&d, &it, first_child(&d, node), name),
            Member::None => None,
        };
        return Ok(found.is_some());
    }
    Ok(false)
}

fn prop_isset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    Ok(Value::bool(prop_dim_exists(ctx, this, Member::Name(bytes(&arg(args, 0))), true, false)?))
}

fn dim_has(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    Ok(Value::bool(prop_dim_exists(ctx, this, member(&arg(args, 0)), false, true)?))
}

// ---- writes (sxe_prop_dim_write) ----

/// The string a written value becomes ("complex type" error otherwise).
fn value_string(ctx: &mut dyn Ctx, v: &Value, attribs: bool) -> Result<Vec<u8>, Throw> {
    let v = v.deref();
    match v.as_object() {
        Some(o) if o.native::<Sxe>().is_some() => Ok(cast_string(o).as_bytes().to_vec()),
        _ if v.is_array() || v.is_object() => {
            let what = if attribs { "attributes" } else { "properties" };
            Err(ctx.throw("TypeError", format!("It's not possible to assign a complex type to {what}, {} given", p8_runtime::args::value_name(&v.deref()))))
        }
        _ => Ok(bytes(&v)),
    }
}

fn new_text_child(d: &mut Doc, parent: Id, ns: (Option<Vec<u8>>, Option<Vec<u8>>), name: &[u8], content: Option<&[u8]>) -> Id {
    let mut el = Node::new(Kind::Element, name.to_vec());
    el.prefix = ns.0;
    el.ns = ns.1;
    let id = d.add(el);
    d.insert(parent, id, None);
    if let Some(c) = content.filter(|c| !c.is_empty()) {
        let mut t = Node::new(Kind::Text, Vec::new());
        t.value = c.to_vec();
        let t = d.add(t);
        d.insert(id, t, None);
    }
    id
}

fn set_content(d: &mut Doc, id: Id, value: &[u8]) {
    if d.node(id).kind == Kind::Attribute {
        d.node_mut(id).value = value.to_vec();
        return;
    }
    for c in d.node(id).children.clone() {
        d.detach(c);
    }
    if !value.is_empty() {
        let mut t = Node::new(Kind::Text, Vec::new());
        t.value = value.to_vec();
        let t = d.add(t);
        d.insert(id, t, None);
    }
}

fn prop_dim_write(ctx: &mut dyn Ctx, func: &str, this: &Rc<Object>, m: Member, value: &Value, mut elements: bool, mut attribs: bool) -> Result<(), Throw> {
    let it = sxe(this).iter.borrow().clone();
    if matches!(m, Member::None) && it.kind == IterKind::AttrList {
        return Err(ctx.throw("Error", "Cannot append to an attribute list".into()));
    }
    if matches!(m, Member::None | Member::Index(_)) && it.kind != IterKind::AttrList {
        attribs = false;
        elements = true;
    }
    if let Member::Name(n) = &m
        && n.is_empty()
    {
        let what = if attribs { "attribute" } else { "element" };
        return Err(ctx.throw("ValueError", format!("Cannot create {what} with an empty name")));
    }
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(()) };
    let value = value_string(ctx, value, attribs)?;
    let mut d = r.doc.borrow_mut();
    let mut node = Some(r.id);
    let mut attr = None;
    let mut test = false;
    if it.kind == IterKind::AttrList {
        attribs = true;
        elements = false;
        node = first_node(&d, r.id, &it);
        attr = node;
        test = it.name.is_some();
    } else if it.kind != IterKind::Child {
        let mynode = r.id;
        node = first_node(&d, r.id, &it);
        attr = node.and_then(|n| first_attr(&d, n));
        if matches!(m, Member::None) && node.is_some_and(|n| d.parent(n) == Some(DOC)) {
            drop(d);
            return Err(ctx.throw("ValueError", "Cannot append to an attribute list".into()));
        }
        if attribs && node.is_none() && it.kind == IterKind::Element {
            let ns = (d.node(mynode).prefix.clone(), d.node(mynode).ns.clone());
            let n = new_text_child(&mut d, mynode, ns, it.name.as_deref().unwrap_or_default(), None);
            node = Some(n);
            attr = first_attr(&d, n);
        }
    }
    let Some(node) = node else { return Ok(()) };
    let mynode = node;
    let mut counter = 0;
    let mut newnode = None;
    let mut is_attr = false;
    let mut ndx = 0i64;
    let mut cnt = 0i64;
    if attribs {
        while let Some(a) = attr {
            let an = d.node(a);
            let ok = (!test || it.name.as_deref() == Some(&an.name[..])) && match_ns(&d, a, it.ns.as_deref(), it.is_prefix);
            let hit = match &m {
                Member::Index(i) if ok => {
                    if ndx == *i {
                        true
                    } else {
                        ndx += 1;
                        false
                    }
                }
                Member::Name(name) => ok && an.name == *name,
                _ => false,
            };
            if hit {
                is_attr = true;
                counter += 1;
                newnode = Some(a);
                break;
            }
            attr = next(&d, a);
        }
    }
    if elements {
        match &m {
            Member::None | Member::Index(_) => {
                if d.node(node).kind == Kind::Attribute {
                    drop(d);
                    return Err(ctx.throw("Error", "Cannot create duplicate attribute".into()));
                }
                if it.kind == IterKind::None {
                    if let Member::Index(i) = m
                        && i != 0
                    {
                        let name = String::from_utf8_lossy(&d.node(mynode).name).into_owned();
                        drop(d);
                        ctx.warn(&format!("{func}: Cannot add element {name} number {i} when only 0 such elements exist"));
                        return Ok(());
                    }
                    newnode = Some(node);
                    counter += 1;
                } else if let Member::Index(i) = m {
                    newnode = element_by_offset(&d, &it, i, Some(node), &mut cnt);
                    if newnode.is_some() {
                        counter += 1;
                    }
                }
            }
            Member::Name(name) => {
                let mut at = first_child(&d, node);
                while let Some(c) = at {
                    if d.node(c).kind != Kind::Text && d.node(c).name == *name && match_ns(&d, c, it.ns.as_deref(), it.is_prefix) {
                        newnode = Some(c);
                        counter += 1;
                    }
                    at = next(&d, c);
                }
            }
        }
    }
    if counter == 1 {
        let target = newnode.expect("counted");
        set_content(&mut d, target, &value);
        let _ = is_attr;
    } else if counter > 1 {
        drop(d);
        ctx.warn(&format!("{func}: Cannot assign to an array of nodes (duplicate subnodes or attr detected)"));
    } else if elements {
        match &m {
            Member::Name(name) => {
                // No namespace given: an element child takes its parent's.
                let ns = if d.node(mynode).kind == Kind::Element { (d.node(mynode).prefix.clone(), d.node(mynode).ns.clone()) } else { (None, None) };
                new_text_child(&mut d, mynode, ns, name, Some(&value));
            }
            Member::None | Member::Index(_) => {
                let i = if let Member::Index(i) = m { Some(i) } else { None };
                if let Some(i) = i
                    && (i < 0 || cnt < i)
                {
                    let name = String::from_utf8_lossy(&d.node(mynode).name).into_owned();
                    let msg = format!("{func}: Cannot add element {name} number {i} when only {cnt} such elements exist");
                    drop(d);
                    ctx.warn(&msg);
                    if i < 0 {
                        return Ok(());
                    }
                    d = r.doc.borrow_mut();
                }
                if let Some(parent) = d.parent(mynode) {
                    let ns = (d.node(mynode).prefix.clone(), d.node(mynode).ns.clone());
                    let name = d.node(mynode).name.clone();
                    new_text_child(&mut d, parent, ns, &name, Some(&value));
                }
            }
        }
    } else if attribs {
        match &m {
            Member::Index(i) => {
                drop(d);
                ctx.warn(&format!("{func}: Cannot change attribute number {i} when only {ndx} attributes exist"));
            }
            Member::Name(name) => {
                let mut a = Node::new(Kind::Attribute, name.clone());
                a.value = value;
                let a = d.add(a);
                d.set_attr(node, a);
            }
            Member::None => {}
        }
    }
    Ok(())
}

fn prop_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let v = arg(args, 1);
    prop_dim_write(ctx, "SimpleXMLElement::__set()", this, Member::Name(bytes(&arg(args, 0))), &v, true, false)?;
    Ok(Value::NULL)
}

fn dim_set(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let v = arg(args, 1);
    let m = member(&arg(args, 0));
    prop_dim_write(ctx, "SimpleXMLElement::offsetSet()", this, m, &v, false, true)?;
    Ok(Value::NULL)
}

// ---- unset (sxe_prop_dim_delete) ----

fn prop_dim_delete(ctx: &mut dyn Ctx, this: &Rc<Object>, m: Member, mut elements: bool, mut attribs: bool) -> Result<(), Throw> {
    let it = sxe(this).iter.borrow().clone();
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(()) };
    let mut d = r.doc.borrow_mut();
    let mut node = Some(r.id);
    if let Member::Index(_) = m
        && it.kind != IterKind::AttrList
    {
        attribs = false;
        elements = true;
        if it.kind == IterKind::Child {
            node = first_node(&d, r.id, &it);
        }
    }
    let mut attr = None;
    let mut test = false;
    if it.kind == IterKind::AttrList {
        attribs = true;
        elements = false;
        node = first_node(&d, r.id, &it);
        attr = node;
        test = it.name.is_some();
    } else if it.kind != IterKind::Child {
        node = first_node(&d, r.id, &it);
        attr = node.and_then(|n| first_attr(&d, n));
    }
    let Some(node) = node else { return Ok(()) };
    if attribs {
        let mut ndx = 0;
        while let Some(a) = attr {
            let nexta = next(&d, a);
            let an = d.node(a);
            let ok = (!test || it.name.as_deref() == Some(&an.name[..])) && match_ns(&d, a, it.ns.as_deref(), it.is_prefix);
            match &m {
                Member::Index(i) if ok => {
                    if ndx == *i {
                        d.detach(a);
                        break;
                    }
                    ndx += 1;
                }
                Member::Name(name) if ok && an.name == *name => {
                    d.detach(a);
                    break;
                }
                _ => {}
            }
            attr = nexta;
        }
    }
    if elements {
        match &m {
            Member::Index(i) => {
                let start = if it.kind == IterKind::Child { first_node(&d, r.id, &it) } else { Some(node) };
                if let Some(n) = element_by_offset(&d, &it, *i, start, &mut 0) {
                    d.detach(n);
                }
            }
            Member::Name(name) => {
                let mut at = first_child(&d, node);
                while let Some(c) = at {
                    let nextc = next(&d, c);
                    if d.node(c).kind != Kind::Text && d.node(c).name == *name && match_ns(&d, c, it.ns.as_deref(), it.is_prefix) {
                        d.detach(c);
                    }
                    at = nextc;
                }
            }
            Member::None => {}
        }
    }
    Ok(())
}

fn prop_unset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prop_dim_delete(ctx, this, Member::Name(bytes(&arg(args, 0))), true, false)?;
    Ok(Value::NULL)
}

fn dim_unset(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    prop_dim_delete(ctx, this, member(&arg(args, 0)), false, true)?;
    Ok(Value::NULL)
}

// ---- casts, properties ----

/// sxe_object_cast_ex to string.
fn cast_string(o: &Object) -> PhpStr {
    let Some(r) = sxe(o).node.borrow().clone() else { return PhpStr::empty() };
    let it = sxe(o).iter.borrow().clone();
    let d = r.doc.borrow();
    let s = if it.kind != IterKind::None {
        match first_iterated(&d, r.id, &it) {
            Some(n) => node_text(&d, n),
            None => Vec::new(),
        }
    } else {
        let n = d.node(r.id);
        if n.kind == Kind::Attribute {
            n.value.clone()
        } else if !n.children.is_empty() {
            node_text(&d, r.id)
        } else if matches!(n.kind, Kind::Comment | Kind::Pi) {
            n.value.clone()
        } else {
            Vec::new()
        }
    };
    PhpStr::new(s)
}

fn cast_bool(o: &Object) -> bool {
    let Some(r) = sxe(o).node.borrow().clone() else { return false };
    let it = sxe(o).iter.borrow().clone();
    if it.kind != IterKind::None && first_iterated(&r.doc.borrow(), r.id, &it).is_some() {
        return true;
    }
    !prop_is_empty(o)
}

/// sxe_prop_is_empty.
fn prop_is_empty(o: &Object) -> bool {
    let Some(r) = sxe(o).node.borrow().clone() else { return true };
    let it = sxe(o).iter.borrow().clone();
    let d = r.doc.borrow();
    let mut node = Some(r.id);
    if it.kind == IterKind::Element {
        node = first_node(&d, r.id, &it);
    }
    if let Some(n) = node
        && d.node(n).kind != Kind::Entity
    {
        let test = it.name.is_some() && it.kind == IterKind::AttrList;
        let mut attr = first_attr(&d, n);
        while let Some(a) = attr {
            if (!test || it.name.as_deref() == Some(&d.node(a).name[..])) && match_ns(&d, a, it.ns.as_deref(), it.is_prefix) {
                return false;
            }
            attr = next(&d, a);
        }
    }
    let node = first_node(&d, r.id, &it);
    let Some(node) = node else { return true };
    if it.kind == IterKind::AttrList {
        return true;
    }
    if d.node(node).kind == Kind::Attribute {
        return false;
    }
    let mut use_iter = false;
    let mut at = if it.kind != IterKind::Child {
        let n = d.node(node);
        let single_text_child = n.children.len() == 1 && d.node(n.children[0]).children.is_empty();
        let parent_has_one = d.parent(node).is_none_or(|p| d.node(p).children.len() == 1);
        if it.kind == IterKind::None || n.children.is_empty() || d.parent(node).is_none() || !single_text_child || parent_has_one {
            first_child(&d, node)
        } else {
            use_iter = true;
            first_iterated(&d, r.id, &it)
        }
    } else {
        Some(node)
    };
    while let Some(id) = at {
        let n = d.node(id);
        let skip_text = !n.children.is_empty() || prev(&d, id).is_some() || next(&d, id).is_some();
        let advance = if skip_text {
            n.kind == Kind::Text
        } else if n.kind == Kind::Text {
            if !n.value.is_empty() {
                return false;
            }
            true
        } else {
            false
        };
        if !advance && !(n.kind == Kind::Element && !match_ns(&d, id, it.ns.as_deref(), it.is_prefix)) && xml_name(&d, id).is_some() {
            return false;
        }
        at = if use_iter { iter_fetch(&d, &it, next(&d, id)) } else { next(&d, id) };
    }
    true
}

/// sxe_properties_add: a second value under a name makes it a list.
fn add_prop(rv: &mut PhpArray, name: &[u8], v: Value) {
    let key = Key::from_string(PhpStr::new(name.to_vec()));
    match rv.get(&key).cloned() {
        Some(old) => match old.into_array() {
            Ok(a) => {
                let mut a = (*a).clone();
                let _ = a.push(v);
                rv.insert(key, Value::array(a));
            }
            Err(old) => rv.insert(key, Value::array(PhpArray::from_list(vec![old, v]))),
        },
        None => rv.insert(key, v),
    }
}

/// sxe_get_prop_hash.
fn prop_hash(o: &Object, is_debug: bool) -> PhpArray {
    let mut rv = PhpArray::new();
    let Some(r) = sxe(o).node.borrow().clone() else { return rv };
    let it = sxe(o).iter.borrow().clone();
    let d = r.doc.borrow();
    if is_debug || it.kind != IterKind::Child {
        let node = if it.kind == IterKind::Element { first_node(&d, r.id, &it) } else { Some(r.id) };
        if let Some(n) = node
            && d.node(n).kind != Kind::Entity
        {
            let test = it.name.is_some() && it.kind == IterKind::AttrList;
            let mut attrs = PhpArray::new();
            let mut any = false;
            let mut attr = first_attr(&d, n);
            while let Some(a) = attr {
                let an = d.node(a);
                if (!test || it.name.as_deref() == Some(&an.name[..])) && match_ns(&d, a, it.ns.as_deref(), it.is_prefix) {
                    attrs.insert(Key::from_string(PhpStr::new(an.name.clone())), str(&an.value));
                    any = true;
                }
                attr = next(&d, a);
            }
            if any {
                add_prop(&mut rv, b"@attributes", Value::array(attrs));
            }
        }
    }
    let Some(node) = first_node(&d, r.id, &it) else { return rv };
    if it.kind == IterKind::AttrList {
        return rv;
    }
    if d.node(node).kind == Kind::Attribute {
        let _ = rv.push(str(&d.node(node).value));
        return rv;
    }
    let mut use_iter = false;
    let mut at = if it.kind != IterKind::Child {
        let n = d.node(node);
        let single_text_child = n.children.len() == 1 && d.node(n.children[0]).children.is_empty();
        let parent_has_one = d.parent(node).is_none_or(|p| d.node(p).children.len() == 1);
        if it.kind == IterKind::None || n.children.is_empty() || d.parent(node).is_none() || next(&d, node).is_none() || !single_text_child || parent_has_one {
            first_child(&d, node)
        } else {
            use_iter = true;
            first_iterated(&d, r.id, &it)
        }
    } else {
        Some(node)
    };
    while let Some(id) = at {
        let n = d.node(id);
        let next_id = if use_iter { iter_fetch(&d, &it, next(&d, id)) } else { next(&d, id) };
        let complex = !n.children.is_empty() || prev(&d, id).is_some() || next(&d, id).is_some() || is_blank(&d, id);
        if complex {
            if n.kind == Kind::Text {
                at = next_id;
                continue;
            }
        } else if n.kind == Kind::Text {
            if !n.value.is_empty() {
                let _ = rv.push(str(&n.value));
            }
            at = next_id;
            continue;
        }
        let Some(name) = xml_name(&d, id).filter(|_| !(n.kind == Kind::Element && !match_ns(&d, id, it.ns.as_deref(), it.is_prefix))) else {
            at = next_id;
            continue;
        };
        // get_base_node_value: an element with only text is its string.
        let value = match n.children.first() {
            Some(&c) if d.node(c).kind == Kind::Text && !is_blank(&d, c) => str(&list_string(&d, Some(c))),
            _ => wrap(o, &r, id, IterKind::None, None, it.ns.clone(), it.is_prefix),
        };
        if use_iter {
            let _ = rv.push(value);
        } else {
            add_prop(&mut rv, &name, value);
        }
        if n.kind == Kind::Entity {
            break;
        }
        at = next_id;
    }
    rv
}

fn get_properties(o: &Object) -> PhpArray {
    prop_hash(o, false)
}

fn debug_info(o: &Object) -> PhpArray {
    prop_hash(o, true)
}

// ---- methods ----

fn this_first(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<Option<(NodeRef, Id, Iter)>, Throw> {
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(None) };
    let it = sxe(this).iter.borrow().clone();
    let first = first_node(&r.doc.borrow(), r.id, &it);
    Ok(first.map(|f| (r, f, it)))
}

fn to_string(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::str(cast_string(this)))
}

fn count(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(Value::int(0)) };
    let it = sxe(this).iter.borrow().clone();
    let d = r.doc.borrow();
    let mut n = 0;
    let mut at = first_iterated(&d, r.id, &it);
    while let Some(id) = at {
        n += 1;
        at = iter_fetch(&d, &it, next(&d, id));
    }
    Ok(Value::int(n))
}

fn get_name(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(match this_first(ctx, this)? {
        Some((r, f, _)) => str(&r.doc.borrow().node(f).name),
        None => str(b""),
    })
}

fn children(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    if sxe(this).iter.borrow().kind == IterKind::AttrList {
        return Ok(Value::NULL);
    }
    let Some((r, f, _)) = this_first(ctx, this)? else { return Ok(Value::NULL) };
    Ok(wrap(this, &r, f, IterKind::Child, None, opt_bytes(&arg(args, 0)), arg(args, 1).to_bool()))
}

fn attributes(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let Some((r, f, it)) = this_first(ctx, this)? else { return Ok(Value::NULL) };
    if it.kind == IterKind::AttrList {
        return Ok(Value::NULL);
    }
    Ok(wrap(this, &r, f, IterKind::AttrList, None, opt_bytes(&arg(args, 0)), arg(args, 1).to_bool()))
}

fn as_xml(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let Some((r, f, _)) = this_first(ctx, this)? else { return Ok(Value::FALSE) };
    let d = r.doc.borrow();
    let whole = d.parent(f) == Some(DOC);
    let mut out = if whole { Out::new(&d, false, d.settings.encoding.is_none()) } else { Out::new(&d, false, false) };
    if whole {
        out.document();
    } else {
        out.node(f, 0);
    }
    let buf = out.buf;
    match opt_bytes(&arg(args, 0)) {
        Some(file) => {
            let path = String::from_utf8_lossy(&file).into_owned();
            Ok(Value::bool(std::fs::write(path, &buf).is_ok()))
        }
        None => Ok(str(&buf)),
    }
}

fn add_child(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let qname = bytes(&arg(args, 0));
    if qname.is_empty() {
        return Err(ctx.throw("ValueError", "SimpleXMLElement::addChild(): Argument #1 ($qualifiedName) must not be empty".into()));
    }
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(Value::NULL) };
    let it = sxe(this).iter.borrow().clone();
    if it.kind == IterKind::AttrList {
        ctx.warn("SimpleXMLElement::addChild(): Cannot add element to attributes");
        return Ok(Value::NULL);
    }
    let Some(node) = first_node(&r.doc.borrow(), r.id, &it) else {
        ctx.warn("SimpleXMLElement::addChild(): Cannot add child. Parent is not a permanent member of the XML tree");
        return Ok(Value::NULL);
    };
    let (prefix, local) = match qname.iter().position(|&b| b == b':') {
        Some(i) if i > 0 && i + 1 < qname.len() => (Some(qname[..i].to_vec()), qname[i + 1..].to_vec()),
        _ => (None, qname.clone()),
    };
    let value = opt_bytes(&arg(args, 1));
    let nsuri = opt_bytes(&arg(args, 2));
    let id = {
        let mut d = r.doc.borrow_mut();
        // xmlNewChild with no namespace: the parent's.
        let mut el = Node::new(Kind::Element, local.clone());
        if d.node(node).kind == Kind::Element {
            el.prefix = d.node(node).prefix.clone();
            el.ns = d.node(node).ns.clone();
        }
        let id = d.add(el);
        d.insert(node, id, None);
        if let Some(v) = &value {
            crate::dom::node::set_content_parsed(ctx, "SimpleXMLElement::addChild", &mut d, id, v);
        }
        if let Some(uri) = nsuri {
            if uri.is_empty() {
                d.node_mut(id).ns = None;
                d.node_mut(id).prefix = None;
                d.node_mut(id).ns_defs.push((prefix.clone(), uri));
            } else {
                let bound = d.lookup_prefix(node, &uri);
                match bound {
                    Some(p) => {
                        d.node_mut(id).prefix = p;
                        d.node_mut(id).ns = Some(uri);
                    }
                    None => {
                        d.node_mut(id).ns_defs.push((prefix.clone(), uri.clone()));
                        d.node_mut(id).prefix = prefix.clone();
                        d.node_mut(id).ns = Some(uri);
                    }
                }
            }
        }
        id
    };
    let retprefix = {
        let d = r.doc.borrow();
        if (prefix.is_some() || opt_bytes(&arg(args, 2)).is_some()) && d.node(id).ns.is_some() { d.node(id).prefix.clone() } else { None }
    };
    Ok(wrap(this, &r, id, IterKind::None, Some(local), retprefix, true))
}

fn add_attribute(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let qname = bytes(&arg(args, 0));
    if qname.is_empty() {
        return Err(ctx.throw("ValueError", "SimpleXMLElement::addAttribute(): Argument #1 ($qualifiedName) must not be empty".into()));
    }
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(Value::NULL) };
    let it = sxe(this).iter.borrow().clone();
    let mut node = first_node(&r.doc.borrow(), r.id, &it);
    if let Some(n) = node
        && r.doc.borrow().node(n).kind != Kind::Element
    {
        node = r.doc.borrow().parent(n);
    }
    let Some(node) = node else {
        ctx.warn("SimpleXMLElement::addAttribute(): Unable to locate parent Element");
        return Ok(Value::NULL);
    };
    let value = bytes(&arg(args, 1));
    let nsuri = opt_bytes(&arg(args, 2));
    let (prefix, local) = match qname.iter().position(|&b| b == b':') {
        Some(i) if i > 0 && i + 1 < qname.len() => (Some(qname[..i].to_vec()), qname[i + 1..].to_vec()),
        _ => {
            if nsuri.as_ref().is_some_and(|u| !u.is_empty()) {
                ctx.warn("SimpleXMLElement::addAttribute(): Attribute requires prefix for namespace");
                return Ok(Value::NULL);
            }
            (None, qname.clone())
        }
    };
    let mut d = r.doc.borrow_mut();
    if d.attr_ns(node, nsuri.as_deref().filter(|u| !u.is_empty()), &local).is_some() {
        drop(d);
        ctx.warn("SimpleXMLElement::addAttribute(): Attribute already exists");
        return Ok(Value::NULL);
    }
    let mut a = Node::new(Kind::Attribute, local);
    a.value = value;
    if let Some(uri) = nsuri.filter(|u| !u.is_empty()) {
        match d.lookup_prefix(node, &uri) {
            Some(p) => a.prefix = p,
            None => {
                d.node_mut(node).ns_defs.push((prefix.clone(), uri.clone()));
                a.prefix = prefix;
            }
        }
        a.ns = Some(uri);
    }
    let a = d.add(a);
    d.set_attr(node, a);
    Ok(Value::NULL)
}

/// Namespaces used by `id` (and, recursively, below it): prefix => URI.
fn used_namespaces(d: &Doc, id: Id, recursive: bool, out: &mut PhpArray) {
    let n = d.node(id);
    if n.kind != Kind::Element {
        return;
    }
    let mut add = |prefix: &Option<Vec<u8>>, uri: &Vec<u8>| {
        let key = Key::from_string(PhpStr::new(prefix.clone().unwrap_or_default()));
        if out.get(&key).is_none() {
            out.insert(key, str(uri));
        }
    };
    if let Some(uri) = &n.ns {
        add(&n.prefix, uri);
    }
    for &a in &n.attrs {
        if let Some(uri) = &d.node(a).ns {
            add(&d.node(a).prefix, uri);
        }
    }
    if recursive {
        for &c in &n.children {
            used_namespaces(d, c, true, out);
        }
    }
}

fn get_namespaces(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let recursive = arg(args, 0).to_bool();
    let mut out = PhpArray::new();
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(Value::array(out)) };
    let it = sxe(this).iter.borrow().clone();
    let d = r.doc.borrow();
    let mut at = first_node(&d, r.id, &it);
    while let Some(id) = at {
        match d.node(id).kind {
            Kind::Element => used_namespaces(&d, id, recursive, &mut out),
            Kind::Attribute => {
                if let Some(uri) = &d.node(id).ns {
                    let key = Key::from_string(PhpStr::new(d.node(id).prefix.clone().unwrap_or_default()));
                    if out.get(&key).is_none() {
                        out.insert(key, str(uri));
                    }
                }
            }
            _ => {}
        }
        at = if it.kind == IterKind::None { None } else { iter_fetch(&d, &it, next(&d, id)) };
    }
    Ok(Value::array(out))
}

fn declared_namespaces(d: &Doc, id: Id, recursive: bool, out: &mut PhpArray) {
    let n = d.node(id);
    if n.kind != Kind::Element {
        return;
    }
    for (prefix, uri) in &n.ns_defs {
        let key = Key::from_string(PhpStr::new(prefix.clone().unwrap_or_default()));
        if out.get(&key).is_none() {
            out.insert(key, str(uri));
        }
    }
    if recursive {
        for &c in &n.children {
            declared_namespaces(d, c, true, out);
        }
    }
}

fn get_doc_namespaces(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let recursive = arg(args, 0).to_bool();
    let from_root = args.len() < 2 || arg(args, 1).to_bool();
    let mut out = PhpArray::new();
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(Value::array(out)) };
    let d = r.doc.borrow();
    let start = if from_root { d.root() } else { Some(r.id) };
    if let Some(s) = start {
        declared_namespaces(&d, s, recursive, &mut out);
    }
    Ok(Value::array(out))
}

// Iteration (RecursiveIterator)
fn rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    *sxe(this).data.borrow_mut() = None;
    let Some(r) = get_node(Some(ctx), this)? else { return Ok(Value::NULL) };
    let it = sxe(this).iter.borrow().clone();
    let first = first_iterated(&r.doc.borrow(), r.id, &it);
    if let Some(f) = first
        && let Ok(o) = wrap(this, &r, f, IterKind::None, None, it.ns.clone(), it.is_prefix).into_object()
    {
        *sxe(this).data.borrow_mut() = Some(o);
    }
    Ok(Value::NULL)
}

fn valid(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::bool(sxe(this).data.borrow().is_some()))
}

fn current(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    match sxe(this).data.borrow().clone() {
        Some(o) => Ok(Value::object(o)),
        None => Err(ctx.throw("Error", "Iterator not initialized or already consumed".into())),
    }
}

fn key(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let cur = sxe(this).data.borrow().clone();
    let Some(cur) = cur else { return Err(ctx.throw("Error", "Iterator not initialized or already consumed".into())) };
    let r = sxe(&cur).node.borrow().clone();
    match r {
        Some(r) => Ok(str(&r.doc.borrow().node(r.id).name)),
        None => Err(ctx.throw("Error", "Iterator not initialized or already consumed".into())),
    }
}

fn next_item(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let cur = sxe(this).data.borrow_mut().take();
    let Some(cur) = cur else { return Ok(Value::NULL) };
    let Some(r) = sxe(&cur).node.borrow().clone() else { return Ok(Value::NULL) };
    let it = sxe(this).iter.borrow().clone();
    let found = {
        let d = r.doc.borrow();
        iter_fetch(&d, &it, next(&d, r.id))
    };
    if let Some(f) = found
        && let Ok(o) = wrap(this, &r, f, IterKind::None, None, it.ns.clone(), it.is_prefix).into_object()
    {
        *sxe(this).data.borrow_mut() = Some(o);
    }
    Ok(Value::NULL)
}

fn has_children(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    if sxe(this).iter.borrow().kind == IterKind::AttrList {
        return Ok(Value::FALSE);
    }
    let cur = sxe(this).data.borrow().clone();
    let Some(cur) = cur else { return Ok(Value::FALSE) };
    let Some(r) = sxe(&cur).node.borrow().clone() else { return Ok(Value::FALSE) };
    let d = r.doc.borrow();
    Ok(Value::bool(d.node(r.id).children.iter().any(|&c| d.node(c).kind == Kind::Element)))
}

fn get_children(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    if sxe(this).iter.borrow().kind == IterKind::AttrList {
        return Ok(Value::NULL);
    }
    Ok(sxe(this).data.borrow().clone().map_or(Value::NULL, Value::object))
}

/// SimpleXMLElement::xpath: the matching nodes as SimpleXMLElements (a
/// text node as its parent, an attribute as its element's attribute list
/// of that name), false for an invalid expression.
fn xpath(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    if sxe(this).iter.borrow().kind == IterKind::AttrList {
        return Ok(Value::NULL);
    }
    let Some((r, node, _)) = this_first(ctx, this)? else { return Ok(Value::NULL) };
    let expr = bytes(&arg(args, 0));
    let registered = sxe(this).xpath_ns.borrow().clone();
    let v = match crate::dom::xpath::evaluate_at(&r, Some(r.at(node)), &expr, &registered, true) {
        Ok(v) => v,
        Err(e) => {
            ctx.warn(&format!("SimpleXMLElement::xpath(): {}", e.message()));
            return Ok(Value::FALSE);
        }
    };
    let mut out = PhpArray::new();
    if let crate::xpath::XValue::Nodes(ids) = v {
        let d = r.doc.borrow();
        for id in ids {
            let n = d.node(id);
            let item = match n.kind {
                Kind::Text => d.parent(id).map(|p| wrap(this, &r, p, IterKind::None, None, None, false)),
                Kind::Attribute => d.parent(id).map(|p| wrap(this, &r, p, IterKind::AttrList, Some(n.name.clone()), n.ns.clone(), false)),
                Kind::Element | Kind::Pi | Kind::Comment => Some(wrap(this, &r, id, IterKind::None, None, None, false)),
                _ => None,
            };
            if let Some(item) = item {
                let _ = out.push(item);
            }
        }
    }
    Ok(Value::array(out))
}

fn register_xpath_namespace(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let (prefix, uri) = (bytes(&arg(args, 0)), bytes(&arg(args, 1)));
    let mut ns = sxe(this).xpath_ns.borrow_mut();
    match ns.iter_mut().find(|(p, _)| *p == prefix) {
        Some(e) => e.1 = uri,
        None => ns.push((prefix, uri)),
    }
    Ok(Value::TRUE)
}

fn debug_info_method(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::array(debug_info(this)))
}

// ---- loading ----

/// LIBXML_* option bits SimpleXML passes to the parser.
fn parse_into(ctx: &mut dyn Ctx, func: &str, data: Vec<u8>, filename: Option<String>, options: i64) -> Option<Doc> {
    let mut doc = Doc::new(b"DOMDocument".to_vec());
    let parser = crate::dom::build::parse(&mut doc, data, filename, options);
    if options & (32 | 64) == 0 {
        for e in &parser.errors {
            crate::libxml::report(ctx, e, |ctx| {
                for line in e.report_lines() {
                    ctx.warn(&format!("{func}(): {line}"));
                }
            });
        }
    }
    if !parser.well_formed && options & crate::dom::build::RECOVER == 0 {
        return None;
    }
    Some(doc)
}

/// The class to make (simplexml_load_*'s $class_name), checked.
fn element_class(ctx: &mut dyn Ctx, func: &str, v: &Value) -> Result<Rc<p8_runtime::object::Class>, Throw> {
    let name = if v.is_null_or_undef() { b"SimpleXMLElement".to_vec() } else { bytes(v) };
    match ctx.class_load(&name)? {
        Some(c) if c.is_a(b"simplexmlelement") => Ok(c),
        _ => Err(ctx.throw(
            "TypeError",
            format!("{func}(): Argument #2 ($class_name) must be a class name derived from SimpleXMLElement, {} given", String::from_utf8_lossy(&name)),
        )),
    }
}

fn from_doc(ctx: &mut dyn Ctx, class: &Rc<p8_runtime::object::Class>, doc: Doc, ns: Option<Vec<u8>>, is_prefix: bool) -> R {
    let root = doc.root();
    let doc = Rc::new(RefCell::new(doc));
    let o = ctx.new_object(class);
    if let Some(root) = root {
        *sxe(&o).node.borrow_mut() = Some(NodeRef { doc, id: root });
    }
    *sxe(&o).iter.borrow_mut() = Iter { kind: IterKind::None, name: None, ns: ns.filter(|n| !n.is_empty()), is_prefix };
    Ok(Value::object(o))
}

fn load_string(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let data = bytes(&arg(args, 0));
    let class = element_class(ctx, "simplexml_load_string", &arg(args, 1))?;
    let Some(doc) = parse_into(ctx, "simplexml_load_string", data, None, arg(args, 2).to_int()) else { return Ok(Value::FALSE) };
    from_doc(ctx, &class, doc, opt_bytes(&arg(args, 3)), arg(args, 4).to_bool())
}

fn load_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let name = String::from_utf8_lossy(&bytes(&arg(args, 0))).into_owned();
    let class = element_class(ctx, "simplexml_load_file", &arg(args, 1))?;
    let Ok(data) = std::fs::read(&name) else {
        ctx.warn(&format!("simplexml_load_file(): I/O warning : failed to load external entity \"{name}\""));
        return Ok(Value::FALSE);
    };
    let Some(doc) = parse_into(ctx, "simplexml_load_file", data, Some(name), arg(args, 2).to_int()) else { return Ok(Value::FALSE) };
    from_doc(ctx, &class, doc, opt_bytes(&arg(args, 3)), arg(args, 4).to_bool())
}

fn construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let data = bytes(&arg(args, 0));
    let options = arg(args, 1).to_int();
    let is_url = arg(args, 2).to_bool();
    let (data, filename) = if is_url {
        let name = String::from_utf8_lossy(&data).into_owned();
        match std::fs::read(&name) {
            Ok(d) => (d, Some(name)),
            Err(_) => {
                ctx.warn(&format!("SimpleXMLElement::__construct(): I/O warning : failed to load external entity \"{name}\""));
                return Err(ctx.throw("Exception", "String could not be parsed as XML".into()));
            }
        }
    } else {
        (data, None)
    };
    let Some(doc) = parse_into(ctx, "SimpleXMLElement::__construct", data, filename, options) else {
        return Err(ctx.throw("Exception", "String could not be parsed as XML".into()));
    };
    let root = doc.root();
    *sxe(this).node.borrow_mut() = root.map(|id| NodeRef { doc: Rc::new(RefCell::new(doc)), id });
    *sxe(this).iter.borrow_mut() = Iter { kind: IterKind::None, name: None, ns: opt_bytes(&arg(args, 3)).filter(|n| !n.is_empty()), is_prefix: arg(args, 4).to_bool() };
    Ok(Value::NULL)
}

fn import_dom(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let class = element_class(ctx, "simplexml_import_dom", &arg(args, 1))?;
    let Ok(o) = arg(args, 0).into_object() else { return Ok(Value::NULL) };
    // A SimpleXMLElement given: its node.
    let node = match o.native::<DomObj>() {
        Some(d) => d.node.borrow().clone(),
        None => o.native::<Sxe>().and_then(|s| s.node.borrow().clone()),
    };
    let Some(mut r) = node else { return Err(ctx.throw("TypeError", "simplexml_import_dom(): Argument #1 ($node) must be of type SimpleXMLElement|DOMNode".into())) };
    if r.kind() == Kind::Document {
        match r.doc.borrow().root() {
            Some(root) => r.id = root,
            None => return Ok(Value::NULL),
        }
    }
    if !matches!(r.kind(), Kind::Element | Kind::Attribute) {
        return Err(ctx.throw("TypeError", "simplexml_import_dom(): Argument #1 ($node) must be a valid XML node".into()));
    }
    let o = ctx.new_object(&class);
    *sxe(&o).node.borrow_mut() = Some(r);
    Ok(Value::object(o))
}

/// dom_import_simplexml: the DOM node a SimpleXMLElement stands on.
pub fn dom_import(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(o) = arg(args, 0).into_object() else { return Ok(Value::NULL) };
    let Some(s) = o.native::<Sxe>() else { return Ok(arg(args, 0)) };
    let r = s.node.borrow().clone();
    let it = s.iter.borrow().clone();
    let Some(r) = r else { return Err(ctx.throw("TypeError", "dom_import_simplexml(): Argument #1 ($node) must be a valid XML node".into())) };
    let first = first_node(&r.doc.borrow(), r.id, &it);
    let Some(f) = first else { return Err(ctx.throw("TypeError", "dom_import_simplexml(): Argument #1 ($node) must be a valid XML node".into())) };
    crate::dom::node::wrap(ctx, &r.at(f))
}

macro_rules! m {
    ($name:literal, $f:path) => {
        ($name, Some(Native::Method($f)))
    };
}

const METHODS: &[(&str, Option<Native>)] = &[
    m!("__construct", construct),
    m!("$*::get", prop_get),
    m!("$*::set", prop_set),
    m!("$*::isset", prop_isset),
    m!("$*::unset", prop_unset),
    m!("$[]::get", dim_get),
    m!("$[]::set", dim_set),
    m!("$[]::has", dim_has),
    m!("$[]::unset", dim_unset),
    m!("__toString", to_string),
    m!("count", count),
    m!("getName", get_name),
    m!("children", children),
    m!("attributes", attributes),
    m!("asXML", as_xml),
    m!("saveXML", as_xml),
    m!("addChild", add_child),
    m!("addAttribute", add_attribute),
    m!("getNamespaces", get_namespaces),
    m!("getDocNamespaces", get_doc_namespaces),
    m!("rewind", rewind),
    m!("valid", valid),
    m!("current", current),
    m!("key", key),
    m!("next", next_item),
    m!("hasChildren", has_children),
    m!("getChildren", get_children),
    m!("__debugInfo", debug_info_method),
    m!("xpath", xpath),
    m!("registerXPathNamespace", register_xpath_namespace),
];

pub const CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "SimpleXMLElement",
        parent: None,
        interfaces: &["Stringable", "Countable", "RecursiveIterator"],
        flags: 0,
        properties: &[],
        methods: METHODS,
        constants: &[],
        debug_info: Some(debug_info),
    },
    ClassSpec { name: "SimpleXMLIterator", parent: Some("SimpleXMLElement"), interfaces: &[], flags: 0, properties: &[], methods: &[], constants: &[], debug_info: Some(debug_info) },
];

pub const BUILTINS: &[Builtin] = &[
    Builtin { name: "simplexml_load_string", run: load_string, by_ref: &[] },
    Builtin { name: "simplexml_load_file", run: load_file, by_ref: &[] },
    Builtin { name: "simplexml_import_dom", run: import_dom, by_ref: &[] },
    Builtin { name: "dom_import_simplexml", run: dom_import, by_ref: &[] },
];

/// The functions that autoload their $class_name (the engine lends them
/// its registers).
pub const REENTER: &[&str] = &["simplexml_load_string", "simplexml_load_file", "simplexml_import_dom"];

pub const HOOKS: ClassHooks = ClassHooks { get_properties: Some(get_properties), cast_bool: Some(cast_bool), cast_string: Some(cast_string), cast_int: None, clone_data: None, forward_calls: None };

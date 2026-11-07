//! DOMNodeList and DOMNamedNodeMap: live views of a node's children,
//! the elements under it with a tag name, an element's attributes, or a
//! fixed list.
//!
//! Reference: php-src ext/dom/nodelist.c, ext/dom/namednodemap.c

use super::node::{NodeRef, R, arg, bytes, opt_bytes, this_node, wrap};
use super::tree::{Id, Kind};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

/// What a list shows.
#[derive(Clone)]
pub enum Src {
    Children(NodeRef),
    /// Elements under `root` named `name` ("*": any); with `ns`, matched by
    /// namespace (Some(None): no namespace, "*": any) and local name.
    Tag { root: NodeRef, ns: Option<Option<Vec<u8>>>, name: Vec<u8> },
    Attrs(NodeRef),
    /// A doctype's entities or notations.
    Decls(NodeRef, Kind),
    /// A fixed list (an XPath query's nodes).
    Fixed(Vec<NodeRef>),
}

#[derive(Default)]
pub struct ListObj {
    pub src: RefCell<Option<Src>>,
}

pub fn make_list() -> Rc<dyn std::any::Any> {
    Rc::new(ListObj::default())
}

fn new_list(ctx: &mut dyn Ctx, class: &[u8], src: Src) -> R {
    let class = ctx.class(class).expect("DOM list classes are declared");
    let o = ctx.new_object(&class);
    *o.native::<ListObj>().expect("a DOM list").src.borrow_mut() = Some(src);
    Ok(Value::object(o))
}

pub fn node_list(ctx: &mut dyn Ctx, src: Src) -> R {
    new_list(ctx, b"DOMNodeList", src)
}

pub fn named_map(ctx: &mut dyn Ctx, src: Src) -> R {
    new_list(ctx, b"DOMNamedNodeMap", src)
}

/// The list's nodes right now.
pub fn items(src: &Src) -> Vec<NodeRef> {
    match src {
        Src::Children(r) => r.doc.borrow().node(r.id).children.iter().map(|&c| r.at(c)).collect(),
        Src::Attrs(r) => r.doc.borrow().node(r.id).attrs.iter().map(|&c| r.at(c)).collect(),
        Src::Fixed(v) => v.clone(),
        Src::Decls(r, kind) => {
            let d = r.doc.borrow();
            d.node(r.id).children.iter().copied().filter(|&c| d.node(c).kind == *kind).map(|c| r.at(c)).collect()
        }
        Src::Tag { root, ns, name } => {
            let d = root.doc.borrow();
            let mut all: Vec<Id> = Vec::new();
            d.descendants(root.id, &mut all);
            all.into_iter()
                .filter(|&e| {
                    let n = d.node(e);
                    match ns {
                        None => name == b"*" || n.qname() == *name,
                        Some(ns) => {
                            let ns_ok = matches!(ns.as_deref(), Some(b"*")) || n.ns.as_deref() == ns.as_deref().filter(|s| !s.is_empty());
                            ns_ok && (name == b"*" || n.name == *name)
                        }
                    }
                })
                .map(|e| root.at(e))
                .collect()
        }
    }
}

fn this_src(this: &Rc<Object>) -> Option<Src> {
    this.native::<ListObj>().and_then(|l| l.src.borrow().clone())
}

/// The nodes `this` list shows (none for a list made with `new`).
fn this_items(_: &mut dyn Ctx, this: &Rc<Object>) -> Result<Vec<NodeRef>, Throw> {
    Ok(this_src(this).map(|s| items(&s)).unwrap_or_default())
}

pub fn length_get(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::int(this_items(ctx, this)?.len() as i64))
}

pub fn count(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    length_get(ctx, this, &mut [])
}

pub fn item(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let i = arg(args, 0).to_int();
    let list = this_items(ctx, this)?;
    match usize::try_from(i).ok().and_then(|i| list.get(i)) {
        Some(n) => wrap(ctx, n),
        None => Ok(Value::NULL),
    }
}

/// foreach: an ArrayIterator over the nodes (keyed by name for a map).
pub fn get_iterator(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let list = this_items(ctx, this)?;
    let named = matches!(this_src(this), Some(Src::Attrs(_) | Src::Decls(..)));
    let mut arr = PhpArray::new();
    for n in &list {
        let v = wrap(ctx, n)?;
        if named {
            let name = super::node::node_name(&n.doc.borrow(), n.id);
            arr.insert(Key::from_string(PhpStr::new(name)), v);
        } else {
            let _ = arr.push(v);
        }
    }
    let class = ctx.class(b"ArrayIterator").expect("SPL is loaded");
    let it = ctx.new_instance(&class, &mut [Value::array(arr)], true)?;
    Ok(Value::object(it))
}

/// `$list[k]`: an index, or (a map) a name.
pub fn dim_get(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let key = arg(args, 0);
    let named = matches!(this_src(this), Some(Src::Attrs(_) | Src::Decls(..)));
    // A string key that isn't an integer names an item.
    let by_name = named && key.is_str() && !matches!(Key::from_value(&key), Ok((Key::Int(_), _)));
    if by_name {
        return get_named_item(ctx, this, args);
    }
    item(ctx, this, &mut [Value::int(key.to_int())])
}

/// isset($list[k])
pub fn dim_has(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    Ok(Value::bool(!dim_get(ctx, this, args)?.is_null()))
}

// DOMNamedNodeMap
pub fn get_named_item(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let name = bytes(&arg(args, 0));
    for n in this_items(ctx, this)? {
        if super::node::node_name(&n.doc.borrow(), n.id) == name {
            return wrap(ctx, &n);
        }
    }
    Ok(Value::NULL)
}

pub fn get_named_item_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let ns = opt_bytes(&arg(args, 0)).filter(|s| !s.is_empty());
    let local = bytes(&arg(args, 1));
    for n in this_items(ctx, this)? {
        let hit = {
            let d = n.doc.borrow();
            let node = d.node(n.id);
            node.name == local && node.ns == ns
        };
        if hit {
            return wrap(ctx, &n);
        }
    }
    Ok(Value::NULL)
}

/// Element::getElementsByTagName(NS) and Document's.
pub fn by_tag_name(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let root = this_node(ctx, this)?;
    let name = bytes(&arg(args, 0));
    node_list(ctx, Src::Tag { root, ns: None, name })
}

pub fn by_tag_name_ns(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let root = this_node(ctx, this)?;
    let ns = Some(opt_bytes(&arg(args, 0)));
    let name = bytes(&arg(args, 1));
    node_list(ctx, Src::Tag { root, ns, name })
}

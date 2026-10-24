//! DOMXPath: XPath 1.0 (crate::xpath) over a DOMDocument.
//!
//! Reference: php-src ext/dom/xpath.c

use super::node::{NodeRef, R, arg, arg_node, bytes, str, wrap};
use super::tree::{Doc, Id, Kind};
use crate::xpath::{Eval, XError, XValue, compile};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::value::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
pub struct XPathObj {
    doc: RefCell<Option<NodeRef>>,
    /// registerNamespace's prefixes, in order.
    ns: RefCell<Vec<(Vec<u8>, Vec<u8>)>>,
    register_node_ns: Cell<bool>,
}

pub fn make_xpath() -> Rc<dyn std::any::Any> {
    Rc::new(XPathObj::default())
}

fn obj(o: &Object) -> &XPathObj {
    o.native::<XPathObj>().expect("a DOMXPath")
}

pub fn construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let Some(doc) = arg_node(args, 0) else { return Err(ctx.throw("TypeError", "DOMXPath::__construct(): Argument #1 ($document) must be of type DOMDocument".into())) };
    *obj(this).doc.borrow_mut() = Some(doc.at(super::tree::DOC));
    obj(this).register_node_ns.set(args.len() < 2 || arg(args, 1).to_bool());
    Ok(Value::Null)
}

fn this_doc(ctx: &mut dyn Ctx, this: &Rc<Object>) -> Result<NodeRef, Throw> {
    obj(this).doc.borrow().clone().ok_or_else(|| ctx.throw("Error", "Invalid XPath Context".into()))
}

pub fn document_get(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    let d = this_doc(ctx, this)?;
    wrap(ctx, &d)
}

pub fn register_node_ns_get(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> R {
    Ok(Value::Bool(obj(this).register_node_ns.get()))
}

pub fn register_node_ns_set(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    obj(this).register_node_ns.set(arg(args, 0).to_bool());
    Ok(Value::Null)
}

pub fn register_namespace(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    let (prefix, uri) = (bytes(&arg(args, 0)), bytes(&arg(args, 1)));
    let mut ns = obj(this).ns.borrow_mut();
    match ns.iter_mut().find(|(p, _)| *p == prefix) {
        Some(e) => e.1 = uri,
        None => ns.push((prefix, uri)),
    }
    Ok(Value::Bool(true))
}

/// The namespaces in scope at `id`, nearest first (xmlGetNsList; the
/// default namespace has no prefix XPath could use).
pub fn in_scope(d: &Doc, id: Id) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut at = Some(id);
    while let Some(n) = at {
        if d.node(n).kind == Kind::Element {
            for (p, u) in &d.node(n).ns_defs {
                if let Some(p) = p
                    && !out.iter().any(|(q, _)| q == p)
                {
                    out.push((p.clone(), u.clone()));
                }
            }
        }
        at = d.parent(n);
    }
    out
}

/// Evaluates `expr` at `context` (None: the document element): the value,
/// or the warning PHP gives (libxml2's message).
pub fn evaluate_at(doc: &NodeRef, context: Option<NodeRef>, expr: &[u8], registered: &[(Vec<u8>, Vec<u8>)], node_ns: bool) -> Result<XValue, XError> {
    let d = doc.doc.borrow();
    let ctx_id = match context {
        Some(c) => c.id,
        None => d.root().unwrap_or(super::tree::DOC),
    };
    let mut ns = if node_ns { in_scope(&d, ctx_id) } else { Vec::new() };
    ns.extend(registered.iter().cloned());
    let compiled = compile(&String::from_utf8_lossy(expr))?;
    Eval::new(&d, &ns, None).run(&compiled, ctx_id)
}

fn eval_method(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], query: bool) -> R {
    let func = if query { "DOMXPath::query" } else { "DOMXPath::evaluate" };
    let doc = this_doc(ctx, this)?;
    let expr = bytes(&arg(args, 0));
    let context = arg_node(args, 1);
    if let Some(c) = &context
        && !c.same_doc(&doc)
    {
        return Err(ctx.throw("Error", "Node from wrong document".into()));
    }
    let node_ns = if args.len() > 2 { arg(args, 2).to_bool() } else { obj(this).register_node_ns.get() };
    let registered = obj(this).ns.borrow().clone();
    let v = match evaluate_at(&doc, context, &expr, &registered, node_ns) {
        Ok(v) => v,
        Err(e) => {
            ctx.warn(&format!("{func}(): {}", e.message()));
            return Ok(Value::Bool(false));
        }
    };
    match v {
        XValue::Nodes(ids) => super::lists::node_list(ctx, super::lists::Src::Fixed(ids.into_iter().map(|id| doc.at(id)).collect())),
        _ if query => super::lists::node_list(ctx, super::lists::Src::Fixed(Vec::new())),
        XValue::Bool(b) => Ok(Value::Bool(b)),
        XValue::Num(n) => Ok(Value::Float(n)),
        XValue::Str(s) => Ok(str(&s)),
    }
}

pub fn evaluate(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    eval_method(ctx, this, args, false)
}

pub fn query(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> R {
    eval_method(ctx, this, args, true)
}

/// DOMXPath::quote: `$str` as an XPath string literal.
pub fn quote(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = bytes(&arg(args, 0));
    let out = if !s.contains(&b'\'') {
        [b"'".as_slice(), &s, b"'"].concat()
    } else if !s.contains(&b'"') {
        [b"\"".as_slice(), &s, b"\""].concat()
    } else {
        // php-src's concat() trick: each run up to the farther of the next
        // two quotes, quoted with the quote it doesn't contain.
        let mut out = b"concat(".to_vec();
        let mut at = 0;
        while at < s.len() {
            let rest = &s[at..];
            let single = rest.iter().position(|&b| b == b'\'').unwrap_or(rest.len());
            let double = rest.iter().position(|&b| b == b'"').unwrap_or(rest.len());
            let run = single.max(double);
            let q = if single > double { b'\'' } else { b'"' };
            out.push(q);
            out.extend_from_slice(&rest[..run]);
            out.push(q);
            out.push(b',');
            at += run;
        }
        *out.last_mut().expect("at least one run") = b')';
        out
    };
    Ok(str(&out))
}

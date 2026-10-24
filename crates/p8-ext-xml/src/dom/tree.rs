//! A DOM document: its nodes in one arena, as libxml2's xmlDoc holds its
//! xmlNode tree. A node is an index; a removed node stays in the arena
//! (a PHP object may still hold it) with no parent.

use std::rc::{Rc, Weak};

use p8_runtime::object::Object;

pub type Id = usize;

/// The document node's id.
pub const DOC: Id = 0;

/// libxml2's xmlElementType: PHP's nodeType values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Element = 1,
    Attribute = 2,
    Text = 3,
    CData = 4,
    EntityRef = 5,
    Entity = 6,
    Pi = 7,
    Comment = 8,
    Document = 9,
    DocType = 10,
    Fragment = 11,
    Notation = 12,
}

/// A namespace binding: (prefix, URI).
pub type Ns = (Option<Vec<u8>>, Vec<u8>);

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Kind,
    /// Local name (element, attribute), target (PI), name (entity
    /// reference, doctype); empty otherwise.
    pub name: Vec<u8>,
    pub prefix: Option<Vec<u8>>,
    /// Namespace URI (element, attribute).
    pub ns: Option<Vec<u8>>,
    /// Text, comment, CDATA, PI data, attribute value.
    pub value: Vec<u8>,
    pub parent: Option<Id>,
    pub children: Vec<Id>,
    /// An element's attributes, in order.
    pub attrs: Vec<Id>,
    /// Namespaces an element declares (xmlns, xmlns:p).
    pub ns_defs: Vec<Ns>,
    /// The line it started on when parsed.
    pub line: u32,
    /// Doctype: public and system ids, internal subset text.
    pub public_id: Option<Vec<u8>>,
    pub system_id: Option<Vec<u8>>,
}

impl Node {
    pub fn new(kind: Kind, name: Vec<u8>) -> Node {
        Node {
            kind,
            name,
            prefix: None,
            ns: None,
            value: Vec::new(),
            parent: None,
            children: Vec::new(),
            attrs: Vec::new(),
            ns_defs: Vec::new(),
            line: 0,
            public_id: None,
            system_id: None,
        }
    }

    /// prefix:name, or name.
    pub fn qname(&self) -> Vec<u8> {
        match &self.prefix {
            Some(p) if !p.is_empty() => [p.as_slice(), b":", &self.name].concat(),
            _ => self.name.clone(),
        }
    }
}

/// The document's own settings (DOMDocument's properties).
#[derive(Clone, Debug)]
pub struct Settings {
    pub version: Option<Vec<u8>>,
    pub encoding: Option<Vec<u8>>,
    /// -1 unset, 0 no, 1 yes.
    pub standalone: i32,
    pub url: Option<Vec<u8>>,
    pub format_output: bool,
    pub validate_on_parse: bool,
    pub resolve_externals: bool,
    pub preserve_white_space: bool,
    pub recover: bool,
    pub substitute_entities: bool,
    pub strict_error_checking: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            version: Some(b"1.0".to_vec()),
            encoding: None,
            standalone: -1,
            url: None,
            format_output: false,
            validate_on_parse: false,
            resolve_externals: false,
            preserve_white_space: true,
            recover: false,
            substitute_entities: false,
            strict_error_checking: true,
        }
    }
}

pub struct Doc {
    pub nodes: Vec<Node>,
    /// The PHP object made for each node, while one is alive.
    pub objects: Vec<Weak<Object>>,
    pub settings: Settings,
    /// The class the document's own object has (DOMDocument or a subclass).
    pub class: Vec<u8>,
    /// No DOMDocument: the home of a node made with `new` (DOMElement,
    /// DOMText, ...) until it is inserted into a document.
    pub orphan: bool,
}

impl Doc {
    pub fn new(class: Vec<u8>) -> Doc {
        Doc { nodes: vec![Node::new(Kind::Document, Vec::new())], objects: Vec::new(), settings: Settings::default(), class, orphan: false }
    }

    pub fn add(&mut self, node: Node) -> Id {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    pub fn node(&self, id: Id) -> &Node {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: Id) -> &mut Node {
        &mut self.nodes[id]
    }

    pub fn object(&self, id: Id) -> Option<Rc<Object>> {
        self.objects.get(id).and_then(Weak::upgrade)
    }

    pub fn set_object(&mut self, id: Id, o: &Rc<Object>) {
        if self.objects.len() <= id {
            self.objects.resize(id + 1, Weak::new());
        }
        self.objects[id] = Rc::downgrade(o);
    }

    /// The document element: the first element child of the document.
    pub fn root(&self) -> Option<Id> {
        self.nodes[DOC].children.iter().copied().find(|&c| self.nodes[c].kind == Kind::Element)
    }

    pub fn parent(&self, id: Id) -> Option<Id> {
        self.nodes[id].parent
    }

    /// Where `id` is among its parent's children.
    pub fn index(&self, id: Id) -> Option<usize> {
        let p = self.nodes[id].parent?;
        if self.nodes[id].kind == Kind::Attribute {
            return None;
        }
        self.nodes[p].children.iter().position(|&c| c == id)
    }

    pub fn prev_sibling(&self, id: Id) -> Option<Id> {
        let i = self.index(id)?;
        let p = self.nodes[id].parent?;
        i.checked_sub(1).map(|j| self.nodes[p].children[j])
    }

    pub fn next_sibling(&self, id: Id) -> Option<Id> {
        let i = self.index(id)?;
        let p = self.nodes[id].parent?;
        self.nodes[p].children.get(i + 1).copied()
    }

    /// Whether `a` is `b` or one of its ancestors.
    pub fn is_ancestor_or_self(&self, a: Id, b: Id) -> bool {
        let mut at = Some(b);
        while let Some(n) = at {
            if n == a {
                return true;
            }
            at = self.nodes[n].parent;
        }
        false
    }

    /// Takes `id` out of its parent (or its element, for an attribute).
    pub fn detach(&mut self, id: Id) {
        let Some(p) = self.nodes[id].parent.take() else { return };
        if self.nodes[id].kind == Kind::Attribute {
            self.nodes[p].attrs.retain(|&a| a != id);
        } else {
            self.nodes[p].children.retain(|&c| c != id);
        }
    }

    /// Puts `id` under `parent`, before `before` (at the end: None). A
    /// fragment's children are moved instead of the fragment. Adjacent
    /// text is not merged (DOM keeps separate text nodes).
    pub fn insert(&mut self, parent: Id, id: Id, before: Option<Id>) {
        if self.nodes[id].kind == Kind::Fragment {
            let kids = std::mem::take(&mut self.nodes[id].children);
            for k in kids {
                self.nodes[k].parent = None;
                self.insert(parent, k, before);
            }
            return;
        }
        self.detach(id);
        let at = before.and_then(|b| self.nodes[parent].children.iter().position(|&c| c == b)).unwrap_or(self.nodes[parent].children.len());
        self.nodes[parent].children.insert(at, id);
        self.nodes[id].parent = Some(parent);
    }

    /// An element's attribute by qualified name.
    pub fn attr(&self, el: Id, qname: &[u8]) -> Option<Id> {
        self.nodes[el].attrs.iter().copied().find(|&a| self.nodes[a].qname() == qname)
    }

    /// An element's attribute by namespace and local name.
    pub fn attr_ns(&self, el: Id, ns: Option<&[u8]>, local: &[u8]) -> Option<Id> {
        self.nodes[el].attrs.iter().copied().find(|&a| self.nodes[a].name == local && self.nodes[a].ns.as_deref() == ns)
    }

    pub fn set_attr(&mut self, el: Id, attr: Id) {
        self.detach(attr);
        self.nodes[attr].parent = Some(el);
        self.nodes[el].attrs.push(attr);
    }

    /// A node's text content (DOM textContent): an element's or
    /// fragment's text descendants in order, a text-like node's own text.
    pub fn text_content(&self, id: Id) -> Vec<u8> {
        let n = &self.nodes[id];
        match n.kind {
            Kind::Text | Kind::CData | Kind::Comment | Kind::Pi | Kind::Attribute => n.value.clone(),
            Kind::Element | Kind::Fragment | Kind::Document | Kind::EntityRef => {
                let mut out = Vec::new();
                self.collect_text(id, &mut out);
                out
            }
            _ => Vec::new(),
        }
    }

    fn collect_text(&self, id: Id, out: &mut Vec<u8>) {
        for &c in &self.nodes[id].children {
            match self.nodes[c].kind {
                Kind::Text | Kind::CData => out.extend_from_slice(&self.nodes[c].value),
                Kind::Element | Kind::EntityRef => self.collect_text(c, out),
                _ => {}
            }
        }
    }

    /// Descendant elements of `id` in document order.
    pub fn descendants(&self, id: Id, out: &mut Vec<Id>) {
        for &c in &self.nodes[id].children {
            if self.nodes[c].kind == Kind::Element {
                out.push(c);
            }
            self.descendants(c, out);
        }
    }

    /// The namespace URI `prefix` is bound to at `id` (None prefix: the
    /// default namespace), looking up through its ancestors.
    pub fn lookup_ns(&self, id: Id, prefix: Option<&[u8]>) -> Option<Vec<u8>> {
        if prefix == Some(b"xml") {
            return Some(crate::parser::XML_NS.to_vec());
        }
        let mut at = Some(id);
        while let Some(n) = at {
            let node = &self.nodes[n];
            if node.kind == Kind::Element {
                for (p, uri) in &node.ns_defs {
                    if p.as_deref() == prefix {
                        return if uri.is_empty() { None } else { Some(uri.clone()) };
                    }
                }
            }
            at = node.parent;
        }
        None
    }

    /// A prefix bound to `uri` at `id`, if any.
    pub fn lookup_prefix(&self, id: Id, uri: &[u8]) -> Option<Option<Vec<u8>>> {
        let mut at = Some(id);
        while let Some(n) = at {
            let node = &self.nodes[n];
            if node.kind == Kind::Element {
                for (p, u) in &node.ns_defs {
                    if u == uri && self.lookup_ns(id, p.as_deref()).as_deref() == Some(uri) {
                        return Some(p.clone());
                    }
                }
            }
            at = node.parent;
        }
        None
    }

    /// A deep (or shallow) copy of `id` from `src` into this document,
    /// unattached: its new id.
    pub fn copy_from(&mut self, src: &Doc, id: Id, deep: bool) -> Id {
        self.copy_mapped(src, id, deep, &mut Vec::new())
    }

    /// As copy_from, recording each (old id, new id).
    pub fn copy_mapped(&mut self, src: &Doc, id: Id, deep: bool, map: &mut Vec<(Id, Id)>) -> Id {
        let mut node = src.nodes[id].clone();
        node.parent = None;
        node.children = Vec::new();
        node.attrs = Vec::new();
        let new = self.add(node);
        map.push((id, new));
        if src.nodes[id].kind == Kind::Element {
            for &a in &src.nodes[id].attrs {
                let copy = self.copy_mapped(src, a, true, map);
                self.set_attr(new, copy);
            }
        }
        if deep || src.nodes[id].kind == Kind::Attribute {
            for &c in &src.nodes[id].children {
                let copy = self.copy_mapped(src, c, true, map);
                self.insert(new, copy, None);
            }
        }
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(doc: &mut Doc, name: &str) -> Id {
        doc.add(Node::new(Kind::Element, name.as_bytes().to_vec()))
    }

    fn text(doc: &mut Doc, t: &str) -> Id {
        let mut n = Node::new(Kind::Text, Vec::new());
        n.value = t.as_bytes().to_vec();
        doc.add(n)
    }

    #[test]
    fn insert_moves_and_orders_children() {
        let mut d = Doc::new(b"DOMDocument".to_vec());
        let root = el(&mut d, "root");
        d.insert(DOC, root, None);
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        d.insert(root, b, None);
        d.insert(root, a, Some(b));
        assert_eq!(d.node(root).children, [a, b]);
        assert_eq!(d.next_sibling(a), Some(b));
        assert_eq!(d.prev_sibling(b), Some(a));
        // Moving a node takes it from where it was.
        d.insert(a, b, None);
        assert_eq!(d.node(root).children, [a]);
        assert_eq!(d.parent(b), Some(a));
        assert!(d.is_ancestor_or_self(root, b));
        assert_eq!(d.root(), Some(root));
    }

    #[test]
    fn a_fragment_inserts_its_children() {
        let mut d = Doc::new(b"DOMDocument".to_vec());
        let root = el(&mut d, "root");
        let frag = d.add(Node::new(Kind::Fragment, Vec::new()));
        let x = el(&mut d, "x");
        let t = text(&mut d, "hi");
        d.insert(frag, x, None);
        d.insert(frag, t, None);
        d.insert(root, frag, None);
        assert_eq!(d.node(root).children, [x, t]);
        assert!(d.node(frag).children.is_empty());
        assert_eq!(d.text_content(root), b"hi");
    }

    #[test]
    fn namespaces_resolve_through_ancestors() {
        let mut d = Doc::new(b"DOMDocument".to_vec());
        let root = el(&mut d, "root");
        d.node_mut(root).ns_defs.push((Some(b"p".to_vec()), b"urn:p".to_vec()));
        d.node_mut(root).ns_defs.push((None, b"urn:default".to_vec()));
        let child = el(&mut d, "c");
        d.insert(root, child, None);
        assert_eq!(d.lookup_ns(child, Some(b"p")), Some(b"urn:p".to_vec()));
        assert_eq!(d.lookup_ns(child, None), Some(b"urn:default".to_vec()));
        assert_eq!(d.lookup_ns(child, Some(b"q")), None);
        assert_eq!(d.lookup_prefix(child, b"urn:p"), Some(Some(b"p".to_vec())));
    }

    #[test]
    fn copies_are_deep_and_unattached() {
        let mut src = Doc::new(b"DOMDocument".to_vec());
        let root = el(&mut src, "root");
        let t = text(&mut src, "x");
        src.insert(root, t, None);
        let mut a = Node::new(Kind::Attribute, b"k".to_vec());
        a.value = b"v".to_vec();
        let a = src.add(a);
        src.set_attr(root, a);
        let mut dst = Doc::new(b"DOMDocument".to_vec());
        let copy = dst.copy_from(&src, root, true);
        assert_eq!(dst.parent(copy), None);
        assert_eq!(dst.text_content(copy), b"x");
        assert_eq!(dst.node(dst.attr(copy, b"k").unwrap()).value, b"v");
        let shallow = dst.copy_from(&src, root, false);
        assert!(dst.node(shallow).children.is_empty());
        assert_eq!(dst.node(shallow).attrs.len(), 1);
    }
}

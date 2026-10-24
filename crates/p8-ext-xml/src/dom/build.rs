//! Builds a DOM tree from the parser's events, as libxml2's SAX2 handlers
//! (xmlSAX2StartElementNs, xmlSAX2Characters, ...) build an xmlDoc.
//!
//! Reference: libxml2 SAX2.c

use super::tree::{DOC, Doc, Id, Kind, Node};
use crate::parser::{Attr, Entity, NsBinding, Options, Parser, Sax};

pub struct Builder<'d> {
    pub doc: &'d mut Doc,
    /// The node new children go under.
    at: Id,
    /// Keep ATTLIST defaults (LIBXML_DTDATTR).
    dtd_attrs: bool,
}

impl<'d> Builder<'d> {
    pub fn new(doc: &'d mut Doc, dtd_attrs: bool) -> Builder<'d> {
        Builder { doc, at: DOC, dtd_attrs }
    }

    fn append(&mut self, node: Node) -> Id {
        let id = self.doc.add(node);
        self.doc.insert(self.at, id, None);
        id
    }

    /// Text goes onto the last child when that is text too (xmlSAX2Text).
    fn text(&mut self, kind: Kind, text: &[u8]) {
        if kind == Kind::Text
            && let Some(&last) = self.doc.node(self.at).children.last()
            && self.doc.node(last).kind == Kind::Text
        {
            self.doc.node_mut(last).value.extend_from_slice(text);
            return;
        }
        let mut node = Node::new(kind, Vec::new());
        node.value = text.to_vec();
        self.append(node);
    }

    /// The doctype node, made when the internal subset starts.
    fn doctype(&self) -> Option<Id> {
        self.doc.node(DOC).children.iter().copied().find(|&c| self.doc.node(c).kind == Kind::DocType)
    }
}

impl Sax for Builder<'_> {
    fn internal_subset(&mut self, _: &mut Parser, name: &[u8], external_id: Option<&[u8]>, system_id: Option<&[u8]>) {
        let mut node = Node::new(Kind::DocType, name.to_vec());
        node.public_id = external_id.map(<[u8]>::to_vec);
        node.system_id = system_id.map(<[u8]>::to_vec);
        self.append(node);
    }

    fn entity_decl(&mut self, _: &mut Parser, entity: &Entity) {
        let Some(dt) = self.doctype() else { return };
        let mut node = Node::new(Kind::Entity, entity.name.clone());
        node.value = entity.content.clone().unwrap_or_default();
        node.public_id = entity.external_id.clone();
        node.system_id = entity.system_id.clone();
        let id = self.doc.add(node);
        self.doc.insert(dt, id, None);
    }

    fn start_element_ns(&mut self, p: &mut Parser, local: &[u8], prefix: Option<&[u8]>, uri: Option<&[u8]>, namespaces: &[NsBinding], attrs: &[Attr]) {
        let mut node = Node::new(Kind::Element, local.to_vec());
        node.prefix = prefix.map(<[u8]>::to_vec);
        node.ns = uri.map(<[u8]>::to_vec);
        node.ns_defs = namespaces.to_vec();
        node.line = p.line().max(0) as u32;
        let id = self.append(node);
        for a in attrs {
            if a.defaulted && !self.dtd_attrs {
                continue;
            }
            let mut attr = Node::new(Kind::Attribute, a.local.clone());
            attr.prefix = a.prefix.clone();
            attr.ns = a.uri.clone();
            attr.value = a.value.clone();
            let aid = self.doc.add(attr);
            self.doc.set_attr(id, aid);
        }
        self.at = id;
    }

    fn end_element_ns(&mut self, _: &mut Parser, _: &[u8], _: Option<&[u8]>, _: Option<&[u8]>) {
        self.at = self.doc.parent(self.at).unwrap_or(DOC);
    }

    fn characters(&mut self, _: &mut Parser, text: &[u8]) {
        if self.at != DOC {
            self.text(Kind::Text, text);
        }
    }

    fn ignorable_whitespace(&mut self, _: &mut Parser, _: &[u8]) {}

    fn cdata_block(&mut self, _: &mut Parser, text: &[u8]) {
        self.text(Kind::CData, text);
    }

    fn processing_instruction(&mut self, _: &mut Parser, target: &[u8], data: Option<&[u8]>) {
        let mut node = Node::new(Kind::Pi, target.to_vec());
        node.value = data.unwrap_or_default().to_vec();
        self.append(node);
    }

    fn comment(&mut self, _: &mut Parser, text: &[u8]) {
        let mut node = Node::new(Kind::Comment, Vec::new());
        node.value = text.to_vec();
        self.append(node);
    }

    fn reference(&mut self, _: &mut Parser, name: &[u8]) {
        self.append(Node::new(Kind::EntityRef, name.to_vec()));
    }
}

/// libxml2 parser options DOM passes (LIBXML_* values).
pub const NOENT: i64 = 2;
pub const DTDLOAD: i64 = 4;
pub const DTDATTR: i64 = 8;
pub const RECOVER: i64 = 1;
pub const NOBLANKS: i64 = 256;
pub const NOCDATA: i64 = 16384;
pub const NSCLEAN: i64 = 8192;
pub const HUGE: i64 = 1 << 19;

/// Parses `data` into `doc` (which must be empty): the parser, for its
/// errors and the declaration it read.
pub fn parse(doc: &mut Doc, data: Vec<u8>, filename: Option<String>, flags: i64) -> Parser {
    let options = Options {
        replace_entities: flags & NOENT != 0,
        recover: flags & RECOVER != 0,
        no_cdata: flags & NOCDATA != 0,
        ns_clean: flags & NSCLEAN != 0,
        huge: flags & HUGE != 0,
        ..Options::default()
    };
    let mut parser = Parser::new(data, filename, options);
    parser.keep_blanks = flags & NOBLANKS == 0;
    let mut builder = Builder::new(doc, flags & DTDATTR != 0);
    parser.parse_document(&mut builder);
    doc.settings.version = parser.version.clone();
    doc.settings.encoding = parser.encoding.clone();
    doc.settings.standalone = if parser.standalone == -2 { -1 } else { parser.standalone };
    parser
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(xml: &str, flags: i64) -> Doc {
        let mut doc = Doc::new(b"DOMDocument".to_vec());
        parse(&mut doc, xml.as_bytes().to_vec(), None, flags);
        doc
    }

    #[test]
    fn elements_attributes_and_text_become_nodes() {
        let d = load("<?xml version='1.0' encoding='UTF-8'?><r a='1'>hi<c/>there</r>", 0);
        let r = d.root().unwrap();
        assert_eq!(d.node(r).name, b"r");
        assert_eq!(d.node(d.attr(r, b"a").unwrap()).value, b"1");
        let kids: Vec<Kind> = d.node(r).children.iter().map(|&c| d.node(c).kind).collect();
        assert_eq!(kids, [Kind::Text, Kind::Element, Kind::Text]);
        assert_eq!(d.text_content(r), b"hithere");
        assert_eq!(d.settings.encoding.as_deref(), Some(&b"UTF-8"[..]));
    }

    #[test]
    fn namespaces_are_bound_on_the_element() {
        let d = load("<p:r xmlns:p='urn:p' xmlns='urn:d'><c/></p:r>", 0);
        let r = d.root().unwrap();
        assert_eq!(d.node(r).prefix.as_deref(), Some(&b"p"[..]));
        assert_eq!(d.node(r).ns.as_deref(), Some(&b"urn:p"[..]));
        assert_eq!(d.node(r).ns_defs.len(), 2);
        assert!(d.node(r).attrs.is_empty());
        let c = d.node(r).children[0];
        assert_eq!(d.node(c).ns.as_deref(), Some(&b"urn:d"[..]));
    }

    #[test]
    fn noblanks_drops_whitespace_between_elements() {
        let keep = load("<r>\n  <c/>\n</r>", 0);
        assert_eq!(keep.node(keep.root().unwrap()).children.len(), 3);
        let drop = load("<r>\n  <c/>\n</r>", NOBLANKS);
        assert_eq!(drop.node(drop.root().unwrap()).children.len(), 1);
    }

    #[test]
    fn comments_pis_and_cdata() {
        let d = load("<!--top--><r><?pi data?><![CDATA[<x>]]></r>", 0);
        let top = d.node(DOC).children[0];
        assert_eq!(d.node(top).kind, Kind::Comment);
        let r = d.root().unwrap();
        let kids = &d.node(r).children;
        assert_eq!((d.node(kids[0]).kind, d.node(kids[0]).name.as_slice(), d.node(kids[0]).value.as_slice()), (Kind::Pi, &b"pi"[..], &b"data"[..]));
        assert_eq!((d.node(kids[1]).kind, d.node(kids[1]).value.as_slice()), (Kind::CData, &b"<x>"[..]));
    }
}

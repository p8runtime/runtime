//! Writes a DOM tree out as XML, as libxml2's xmlsave.c does
//! (xmlDocContentDumpOutput, xmlNodeDumpOutputInternal).
//!
//! Reference: libxml2 xmlsave.c

use super::tree::{DOC, Doc, Id, Kind};

pub struct Out<'d> {
    doc: &'d Doc,
    pub buf: Vec<u8>,
    /// Indent element content (formatOutput).
    format: bool,
    /// No output encoding: characters past ASCII become `&#xHH;`
    /// (libxml2's xmlEscapeEntities), as when the document declares none.
    escape_non_ascii: bool,
    /// `<a></a>` for an empty element, not `<a/>` (LIBXML_NOEMPTYTAG).
    pub no_empty: bool,
}

impl<'d> Out<'d> {
    pub fn new(doc: &'d Doc, format: bool, escape_non_ascii: bool) -> Out<'d> {
        Out { doc, buf: Vec::new(), format, escape_non_ascii, no_empty: false }
    }

    /// The whole document: the XML declaration, then each top-level node
    /// on its own line (xmlDocContentDumpOutput).
    pub fn document(&mut self) {
        let s = &self.doc.settings;
        self.buf.extend_from_slice(b"<?xml version=\"");
        self.buf.extend_from_slice(s.version.as_deref().unwrap_or(b"1.0"));
        self.buf.push(b'"');
        if let Some(enc) = &s.encoding {
            self.buf.extend_from_slice(b" encoding=\"");
            self.buf.extend_from_slice(enc);
            self.buf.push(b'"');
        }
        match s.standalone {
            0 => self.buf.extend_from_slice(b" standalone=\"no\""),
            1 => self.buf.extend_from_slice(b" standalone=\"yes\""),
            _ => {}
        }
        self.buf.extend_from_slice(b"?>\n");
        for &c in &self.doc.node(DOC).children {
            self.node(c, 0);
            self.buf.push(b'\n');
        }
    }

    /// One node and what's under it (xmlNodeDumpOutputInternal).
    pub fn node(&mut self, id: Id, level: usize) {
        let n = self.doc.node(id);
        match n.kind {
            Kind::Document => {
                for &c in &n.children {
                    self.node(c, level);
                    self.buf.push(b'\n');
                }
            }
            Kind::Fragment => {
                for &c in &n.children {
                    self.node(c, level);
                }
            }
            Kind::Element => self.element(id, level),
            Kind::Text => {
                let v = n.value.clone();
                self.escape_text(&v);
            }
            Kind::CData => {
                self.buf.extend_from_slice(b"<![CDATA[");
                // "]]>" can't be in one section: split it.
                let v = &n.value;
                let mut start = 0;
                while let Some(at) = v[start..].windows(3).position(|w| w == b"]]>") {
                    let end = start + at + 2;
                    self.buf.extend_from_slice(&v[start..end]);
                    self.buf.extend_from_slice(b"]]><![CDATA[");
                    start = end;
                }
                self.buf.extend_from_slice(&v[start..]);
                self.buf.extend_from_slice(b"]]>");
            }
            Kind::Comment => {
                self.buf.extend_from_slice(b"<!--");
                self.buf.extend_from_slice(&n.value);
                self.buf.extend_from_slice(b"-->");
            }
            Kind::Pi => {
                self.buf.extend_from_slice(b"<?");
                self.buf.extend_from_slice(&n.name);
                if !n.value.is_empty() {
                    self.buf.push(b' ');
                    self.buf.extend_from_slice(&n.value);
                }
                self.buf.extend_from_slice(b"?>");
            }
            Kind::EntityRef => {
                self.buf.push(b'&');
                self.buf.extend_from_slice(&n.name);
                self.buf.push(b';');
            }
            Kind::Attribute => {
                self.buf.push(b' ');
                self.attribute(id);
            }
            Kind::DocType => self.doctype(id),
            Kind::Entity => self.entity_decl(id),
            Kind::Notation => {}
        }
    }

    fn doctype(&mut self, id: Id) {
        let n = self.doc.node(id);
        self.buf.extend_from_slice(b"<!DOCTYPE ");
        self.buf.extend_from_slice(&n.name);
        match (&n.public_id, &n.system_id) {
            (Some(p), s) => {
                self.buf.extend_from_slice(b" PUBLIC ");
                quote(&mut self.buf, p);
                if let Some(s) = s {
                    self.buf.push(b' ');
                    quote(&mut self.buf, s);
                }
            }
            (None, Some(s)) => {
                self.buf.extend_from_slice(b" SYSTEM ");
                quote(&mut self.buf, s);
            }
            (None, None) => {}
        }
        if !n.children.is_empty() {
            self.buf.extend_from_slice(b" [\n");
            for &c in &n.children {
                self.node(c, 0);
            }
            self.buf.push(b']');
        }
        self.buf.push(b'>');
    }

    /// `<!ENTITY name "value">` (xmlDumpEntityDecl), one per line.
    fn entity_decl(&mut self, id: Id) {
        let n = self.doc.node(id);
        self.buf.extend_from_slice(b"<!ENTITY ");
        self.buf.extend_from_slice(&n.name);
        match (&n.public_id, &n.system_id) {
            (Some(p), Some(s)) => {
                self.buf.extend_from_slice(b" PUBLIC ");
                quote(&mut self.buf, p);
                self.buf.push(b' ');
                quote(&mut self.buf, s);
            }
            (None, Some(s)) => {
                self.buf.extend_from_slice(b" SYSTEM ");
                quote(&mut self.buf, s);
            }
            _ => {
                self.buf.push(b' ');
                quote(&mut self.buf, &n.value);
            }
        }
        self.buf.extend_from_slice(b">\n");
    }

    fn element(&mut self, id: Id, level: usize) {
        let n = self.doc.node(id);
        let name = n.qname();
        self.buf.push(b'<');
        self.buf.extend_from_slice(&name);
        for (prefix, uri) in &n.ns_defs {
            self.buf.extend_from_slice(b" xmlns");
            if let Some(p) = prefix {
                self.buf.push(b':');
                self.buf.extend_from_slice(p);
            }
            self.buf.extend_from_slice(b"=\"");
            let uri = uri.clone();
            self.escape_attr(&uri);
            self.buf.push(b'"');
        }
        for &a in &n.attrs.clone() {
            self.buf.push(b' ');
            self.attribute(a);
        }
        let n = self.doc.node(id);
        if n.children.is_empty() {
            if self.no_empty {
                self.buf.extend_from_slice(b"></");
                self.buf.extend_from_slice(&name);
                self.buf.push(b'>');
            } else {
                self.buf.extend_from_slice(b"/>");
            }
            return;
        }
        self.buf.push(b'>');
        // Indent only when no child is text or an entity reference (libxml2
        // turns formatting off under such an element).
        let format = self.format && n.children.iter().all(|&c| !matches!(self.doc.node(c).kind, Kind::Text | Kind::CData | Kind::EntityRef));
        if format {
            self.buf.push(b'\n');
        }
        let saved = self.format;
        self.format = format;
        for &c in &n.children.clone() {
            if format {
                self.indent(level + 1);
            }
            self.node(c, level + 1);
            if format {
                self.buf.push(b'\n');
            }
        }
        self.format = saved;
        if format {
            self.indent(level);
        }
        self.buf.extend_from_slice(b"</");
        self.buf.extend_from_slice(&name);
        self.buf.push(b'>');
    }

    fn attribute(&mut self, id: Id) {
        let n = self.doc.node(id);
        self.buf.extend_from_slice(&n.qname());
        self.buf.extend_from_slice(b"=\"");
        let v = n.value.clone();
        self.escape_attr(&v);
        self.buf.push(b'"');
    }

    fn indent(&mut self, level: usize) {
        for _ in 0..level {
            self.buf.extend_from_slice(b"  ");
        }
    }

    /// Text content: `< > &` and CR escaped (xmlEscapeContent).
    fn escape_text(&mut self, v: &[u8]) {
        let mut i = 0;
        while i < v.len() {
            match v[i] {
                b'<' => self.buf.extend_from_slice(b"&lt;"),
                b'>' => self.buf.extend_from_slice(b"&gt;"),
                b'&' => self.buf.extend_from_slice(b"&amp;"),
                b'\r' => self.buf.extend_from_slice(b"&#13;"),
                c if c >= 0x80 && self.escape_non_ascii => {
                    i += self.char_ref(&v[i..]);
                    continue;
                }
                c => self.buf.push(c),
            }
            i += 1;
        }
    }

    /// An attribute value: also quotes and whitespace that isn't a space
    /// (xmlBufAttrSerializeTxtContent).
    fn escape_attr(&mut self, v: &[u8]) {
        let mut i = 0;
        while i < v.len() {
            match v[i] {
                b'<' => self.buf.extend_from_slice(b"&lt;"),
                b'>' => self.buf.extend_from_slice(b"&gt;"),
                b'&' => self.buf.extend_from_slice(b"&amp;"),
                b'"' => self.buf.extend_from_slice(b"&quot;"),
                b'\n' => self.buf.extend_from_slice(b"&#10;"),
                b'\r' => self.buf.extend_from_slice(b"&#13;"),
                b'\t' => self.buf.extend_from_slice(b"&#9;"),
                c if c >= 0x80 && self.escape_non_ascii => {
                    i += self.char_ref(&v[i..]);
                    continue;
                }
                c => self.buf.push(c),
            }
            i += 1;
        }
    }

    /// The UTF-8 character at the start of `v` as `&#xHH;`: the bytes read.
    fn char_ref(&mut self, v: &[u8]) -> usize {
        let len = match v[0] {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        let s = std::str::from_utf8(&v[..len.min(v.len())]).ok().and_then(|s| s.chars().next());
        match s {
            Some(c) => {
                self.buf.extend_from_slice(format!("&#x{:X};", c as u32).as_bytes());
                len
            }
            None => {
                self.buf.push(v[0]);
                1
            }
        }
    }
}

fn quote(buf: &mut Vec<u8>, v: &[u8]) {
    let q = if v.contains(&b'"') { b'\'' } else { b'"' };
    buf.push(q);
    buf.extend_from_slice(v);
    buf.push(q);
}

#[cfg(test)]
mod tests {
    use super::super::build::{NOBLANKS, parse};
    use super::*;

    fn roundtrip(xml: &str, format: bool) -> String {
        let mut doc = Doc::new(b"DOMDocument".to_vec());
        parse(&mut doc, xml.as_bytes().to_vec(), None, if format { NOBLANKS } else { 0 });
        let mut out = Out::new(&doc, format, doc.settings.encoding.is_none());
        out.document();
        String::from_utf8(out.buf).unwrap()
    }

    #[test]
    fn a_document_has_its_declaration_and_a_newline() {
        assert_eq!(roundtrip("<r a='1'><c/>t</r>", false), "<?xml version=\"1.0\"?>\n<r a=\"1\"><c/>t</r>\n");
        assert_eq!(
            roundtrip("<?xml version='1.0' encoding='UTF-8' standalone='yes'?><r/>", false),
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<r/>\n"
        );
    }

    #[test]
    fn escaping_follows_libxml2() {
        assert_eq!(roundtrip("<r a='&lt;&quot;&#10;'>&lt;&amp;&gt;</r>", false), "<?xml version=\"1.0\"?>\n<r a=\"&lt;&quot;&#10;\">&lt;&amp;&gt;</r>\n");
        // No declared encoding: non-ASCII as character references.
        assert_eq!(roundtrip("<r>é</r>", false), "<?xml version=\"1.0\"?>\n<r>&#xE9;</r>\n");
        assert_eq!(roundtrip("<?xml version='1.0' encoding='UTF-8'?><r>é</r>", false), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<r>é</r>\n");
    }

    #[test]
    fn format_output_indents_element_only_content() {
        assert_eq!(
            roundtrip("<r><a><b/></a><c>text<d/></c></r>", true),
            "<?xml version=\"1.0\"?>\n<r>\n  <a>\n    <b/>\n  </a>\n  <c>text<d/></c>\n</r>\n"
        );
    }

    #[test]
    fn namespaces_comments_pis_cdata() {
        assert_eq!(
            roundtrip("<p:r xmlns:p='urn:p'><!--c--><?t d?><![CDATA[a]]>b&gt;</p:r>", false),
            "<?xml version=\"1.0\"?>\n<p:r xmlns:p=\"urn:p\"><!--c--><?t d?><![CDATA[a]]>b&gt;</p:r>\n"
        );
    }
}

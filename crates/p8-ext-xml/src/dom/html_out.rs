//! Writes a DOM tree out as HTML, as libxml2's HTMLtree.c does for
//! DOMDocument::saveHTML() and saveHTMLFile(): no end tag for void
//! elements, bare boolean attributes, URI escaping of href/src/action, the
//! DOCTYPE line, and the entity conversion of the output encoding.
//!
//! Reference: libxml2 2.9.14 HTMLtree.c, entities.c (xmlEncodeEntities)

use super::tree::{DOC, Doc, Id, Kind, Node};
use crate::html_tables::{entity_name, is_boolean_attr, tag_lookup};

pub struct HtmlOut<'d> {
    doc: &'d Doc,
    pub buf: Vec<u8>,
    format: bool,
}

/// xmlBufWriteQuotedString
fn quoted(out: &mut Vec<u8>, v: &[u8]) {
    if v.contains(&b'"') {
        if v.contains(&b'\'') {
            out.push(b'"');
            for &c in v {
                if c == b'"' {
                    out.extend_from_slice(b"&quot;");
                } else {
                    out.push(c);
                }
            }
            out.push(b'"');
        } else {
            out.push(b'\'');
            out.extend_from_slice(v);
            out.push(b'\'');
        }
    } else {
        out.push(b'"');
        out.extend_from_slice(v);
        out.push(b'"');
    }
}

/// xmlEncodeEntitiesInternal. `html`: the document is an HTML document;
/// `attr`: the text is an attribute value; `has_encoding`: the document
/// names an encoding.
pub fn encode_entities(v: &[u8], html: bool, attr: bool, has_encoding: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() + 16);
    let mut i = 0;
    while i < v.len() {
        let c = v[i];
        match c {
            b'<' => {
                if html
                    && attr
                    && v[i..].starts_with(b"<!--")
                    && let Some(end) = v[i..].windows(3).position(|w| w == b"-->")
                {
                    out.extend_from_slice(&v[i..i + end + 3]);
                    i += end + 3;
                    continue;
                }
                out.extend_from_slice(b"&lt;");
            }
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => {
                if html && attr && v.get(i + 1) == Some(&b'{') && v[i..].contains(&b'}') {
                    let end = v[i..].iter().position(|&x| x == b'}').unwrap_or(0);
                    out.extend_from_slice(&v[i..=i + end]);
                    i += end + 1;
                    continue;
                }
                out.extend_from_slice(b"&amp;");
            }
            0x20..=0x7F | b'\n' | b'\t' => out.push(c),
            b'\r' if html => out.push(c),
            0x80..=0xFF => {
                if has_encoding || html {
                    out.push(c);
                } else {
                    // UTF-8 as a character reference.
                    let len = match c {
                        0xC0..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        0xF0..=0xF7 => 4,
                        _ => 1,
                    };
                    let seq = v.get(i..i + len);
                    let cp = seq.filter(|s| s.len() == len && len > 1 && s[1..].iter().all(|b| b & 0xC0 == 0x80)).and_then(|s| std::str::from_utf8(s).ok()).and_then(|s| s.chars().next());
                    match cp {
                        Some(ch) => {
                            out.extend_from_slice(format!("&#x{:X};", ch as u32).as_bytes());
                            i += len;
                            continue;
                        }
                        None => out.extend_from_slice(format!("&#{c};").as_bytes()),
                    }
                }
            }
            // IS_BYTE_CHAR: only a carriage return is left to write.
            0x0D => out.extend_from_slice(b"&#13;"),
            0x00..=0x1F => {}
        }
        i += 1;
    }
    out
}

/// xmlURIEscapeStr with the characters HTML attributes keep.
fn uri_escape(v: &[u8]) -> Vec<u8> {
    const KEEP: &[u8] = b"@/:=?;#%&,+<>";
    let mut out = Vec::with_capacity(v.len());
    for &c in v {
        if c.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&c) || KEEP.contains(&c) {
            out.push(c);
        } else {
            out.extend_from_slice(format!("%{c:02X}").as_bytes());
        }
    }
    out
}

fn is_text_like(n: &Node) -> bool {
    matches!(n.kind, Kind::Text | Kind::EntityRef)
}

impl<'d> HtmlOut<'d> {
    pub fn new(doc: &'d Doc, format: bool) -> HtmlOut<'d> {
        HtmlOut { doc, buf: Vec::new(), format }
    }

    fn put(&mut self, s: &[u8]) {
        self.buf.extend_from_slice(s);
    }

    /// htmlNodeDumpFormatOutput: `id` and what is under it.
    pub fn dump(&mut self, id: Id) {
        let parent = self.doc.parent(id);
        let next = self.doc.next_sibling(id);
        self.walk(id, parent, next);
    }

    fn walk(&mut self, id: Id, parent: Option<Id>, next: Option<Id>) {
        let doc = self.doc;
        let n = doc.node(id);
        match n.kind {
            Kind::Document => {
                if let Some(&dtd) = n.children.iter().find(|&&c| doc.node(c).kind == Kind::DocType) {
                    self.dtd(dtd);
                }
                if n.children.is_empty() {
                    self.put(b"\n");
                } else {
                    self.children(id);
                    self.put(b"\n");
                }
            }
            Kind::Element => self.element(id, parent, next),
            Kind::Attribute => self.attribute(id),
            Kind::Text => {
                let raw = parent.is_some_and(|p| {
                    let pn = doc.node(p);
                    pn.kind == Kind::Element && (pn.name.eq_ignore_ascii_case(b"script") || pn.name.eq_ignore_ascii_case(b"style"))
                });
                if raw {
                    self.put(&n.value);
                } else {
                    let enc = encode_entities(&n.value, doc.html, false, doc.settings.encoding.is_some());
                    self.put(&enc);
                }
            }
            Kind::Comment => {
                self.put(b"<!--");
                self.put(&n.value);
                self.put(b"-->");
            }
            Kind::Pi => {
                self.put(b"<?");
                self.put(&n.name);
                if !n.value.is_empty() {
                    self.put(b" ");
                    self.put(&n.value);
                }
                self.put(b">");
            }
            Kind::EntityRef => {
                self.put(b"&");
                self.put(&n.name);
                self.put(b";");
            }
            // HTML_PRESERVE_NODE: written as it is.
            Kind::CData => self.put(&n.value),
            Kind::DocType | Kind::Entity | Kind::Notation | Kind::Fragment => {}
        }
    }

    fn children(&mut self, id: Id) {
        let kids = &self.doc.node(id).children;
        for (i, &c) in kids.iter().enumerate() {
            self.walk(c, Some(id), kids.get(i + 1).copied());
        }
    }

    /// htmlDtdDumpOutput
    fn dtd(&mut self, id: Id) {
        let n = self.doc.node(id);
        self.put(b"<!DOCTYPE ");
        self.put(&n.name);
        if let Some(ext) = &n.public_id {
            self.put(b" PUBLIC ");
            quoted(&mut self.buf, ext);
            if let Some(sys) = &n.system_id {
                self.put(b" ");
                quoted(&mut self.buf, sys);
            }
        } else if let Some(sys) = &n.system_id
            && sys.as_slice() != b"about:legacy-compat"
        {
            self.put(b" SYSTEM ");
            quoted(&mut self.buf, sys);
        }
        self.put(b">\n");
    }

    /// htmlAttrDumpOutput
    fn attribute(&mut self, id: Id) {
        let doc = self.doc;
        let n = doc.node(id);
        self.put(b" ");
        if n.ns.is_some()
            && let Some(p) = n.prefix.as_deref().filter(|p| !p.is_empty())
        {
            self.put(p);
            self.put(b":");
        }
        self.put(&n.name);
        if (n.no_value && n.value.is_empty()) || is_boolean_attr(&n.name) {
            return;
        }
        let value = encode_entities(&n.value, doc.html, true, doc.settings.encoding.is_some());
        self.put(b"=");
        let parent = n.parent.map(|p| doc.node(p));
        let uri = n.ns.is_none()
            && parent.is_some_and(|p| {
                p.ns.is_none()
                    && (n.name.eq_ignore_ascii_case(b"href")
                        || n.name.eq_ignore_ascii_case(b"action")
                        || n.name.eq_ignore_ascii_case(b"src")
                        || (n.name.eq_ignore_ascii_case(b"name") && p.name.eq_ignore_ascii_case(b"a")))
            });
        if uri {
            let start = value.iter().position(|&c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r')).unwrap_or(value.len());
            let escaped = uri_escape(&value[start..]);
            quoted(&mut self.buf, &escaped);
        } else {
            quoted(&mut self.buf, &value);
        }
    }

    fn prefixed(&mut self, n: &Node) {
        if n.ns.is_some()
            && let Some(p) = n.prefix.as_deref().filter(|p| !p.is_empty())
        {
            self.put(p);
            self.put(b":");
        }
    }

    fn element(&mut self, id: Id, parent: Option<Id>, next: Option<Id>) {
        let doc = self.doc;
        let n = doc.node(id);
        let info = if n.ns.is_none() { tag_lookup(&n.name) } else { None };
        self.put(b"<");
        self.prefixed(n);
        self.put(&n.name);
        for (prefix, uri) in &n.ns_defs {
            if prefix.as_deref() == Some(b"xml") {
                continue;
            }
            self.put(b" xmlns");
            if let Some(p) = prefix {
                self.put(b":");
                self.put(p);
            }
            self.put(b"=");
            quoted(&mut self.buf, uri);
        }
        for &a in &n.attrs {
            self.attribute(a);
        }
        if info.as_ref().is_some_and(|i| i.empty) {
            self.put(b">");
        } else if n.children.is_empty() {
            if info.as_ref().is_some_and(|i| i.save_end_tag != 0 && i.name != "html" && i.name != "body") {
                self.put(b">");
            } else {
                self.put(b"></");
                self.prefixed(n);
                self.put(&n.name);
                self.put(b">");
            }
        } else {
            self.put(b">");
            let first = doc.node(n.children[0]);
            if self.format
                && info.as_ref().is_some_and(|i| !i.inline)
                && !is_text_like(first)
                && n.children.len() > 1
                && n.name.first() != Some(&b'p')
            {
                self.put(b"\n");
            }
            self.children(id);
            let last = doc.node(*n.children.last().unwrap_or(&id));
            if self.format
                && info.as_ref().is_some_and(|i| !i.inline)
                && !is_text_like(last)
                && n.children.len() > 1
                && n.name.first() != Some(&b'p')
            {
                self.put(b"\n");
            }
            self.put(b"</");
            self.prefixed(n);
            self.put(&n.name);
            self.put(b">");
        }
        if self.format
            && let Some(nx) = next
            && info.as_ref().is_some_and(|i| !i.inline)
            && !is_text_like(doc.node(nx))
            && let Some(p) = parent.map(|p| doc.node(p))
            && p.kind == Kind::Element
            && p.name.first() != Some(&b'p')
        {
            self.put(b"\n");
        }
    }
}

/// Where the head's meta elements are looked for: the head if the document
/// has one (or the html element, when a meta is found there directly), and
/// the sibling run to search. None: no html, head or meta at the top.
fn meta_scope(doc: &Doc, ci: bool) -> Option<(Option<Id>, Vec<Id>)> {
    let is_el = |id: Id, name: &[u8]| {
        let n = doc.node(id);
        n.kind == Kind::Element && if ci { n.name.eq_ignore_ascii_case(name) } else { n.name == name }
    };
    let top = &doc.node(DOC).children;
    for (i, &c) in top.iter().enumerate() {
        if doc.node(c).kind != Kind::Element {
            continue;
        }
        if is_el(c, b"html") {
            let kids = &doc.node(c).children;
            for (j, &k) in kids.iter().enumerate() {
                if doc.node(k).kind != Kind::Element {
                    continue;
                }
                if is_el(k, b"head") {
                    return Some((Some(k), doc.node(k).children.clone()));
                }
                if is_el(k, b"meta") {
                    return Some((Some(c), kids[j..].to_vec()));
                }
            }
            return None;
        }
        if is_el(c, b"head") {
            return Some((Some(c), doc.node(c).children.clone()));
        }
        if is_el(c, b"meta") {
            return Some((None, top[i..].to_vec()));
        }
    }
    None
}

/// htmlGetMetaEncoding: the charset a Content-Type meta element names.
pub fn meta_encoding(doc: &Doc) -> Option<Vec<u8>> {
    let (_, metas) = meta_scope(doc, false)?;
    for m in metas {
        let n = doc.node(m);
        if n.kind != Kind::Element || n.name != b"meta" {
            continue;
        }
        let mut http = false;
        let mut content: Option<&[u8]> = None;
        for &a in &n.attrs {
            let attr = doc.node(a);
            let value = attr.value.as_slice();
            if attr.name.eq_ignore_ascii_case(b"http-equiv") && value.eq_ignore_ascii_case(b"Content-Type") {
                http = true;
            } else if attr.name.eq_ignore_ascii_case(b"content") {
                content = Some(value);
            }
            if http && let Some(c) = content {
                return charset_in(c);
            }
        }
    }
    None
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn charset_in(content: &[u8]) -> Option<Vec<u8>> {
    let enc = [(&b"charset="[..], 8), (b"Charset=", 8), (b"CHARSET=", 8), (b"charset =", 9), (b"Charset =", 9), (b"CHARSET =", 9)]
        .iter()
        .find_map(|(pat, skip)| find(content, pat).map(|i| i + skip))?;
    let rest = &content[enc..];
    let start = rest.iter().position(|&c| c != b' ' && c != b'\t').unwrap_or(rest.len());
    Some(rest[start..].to_vec())
}

/// Output conversion (xmlOutputBuffer with the encoder saveHTML picks):
/// UTF-8 is written as it is, other encodings convert (what they cannot
/// hold becomes `&#N;`), and with no encoding named the HTML encoder
/// writes non-ASCII characters as entities. The flag says the conversion
/// failed (the output stops there).
pub fn encode_output(bytes: Vec<u8>, encoding: Option<&[u8]>) -> (Vec<u8>, bool) {
    match encoding {
        None => to_html_entities(&bytes),
        Some(e) => {
            let upper = e.to_ascii_uppercase();
            let out = match upper.as_slice() {
                b"UTF-8" | b"UTF8" => bytes,
                b"ISO-8859-1" | b"ISO-LATIN-1" | b"ISO LATIN 1" | b"LATIN1" | b"ISO8859-1" | b"ISO_8859-1" | b"L1" => {
                    String::from_utf8_lossy(&bytes).chars().flat_map(|c| if (c as u32) < 0x100 { vec![c as u32 as u8] } else { format!("&#{};", c as u32).into_bytes() }).collect()
                }
                b"US-ASCII" | b"ASCII" => {
                    String::from_utf8_lossy(&bytes).chars().flat_map(|c| if c.is_ascii() { vec![c as u8] } else { format!("&#{};", c as u32).into_bytes() }).collect()
                }
                _ => match encoding_rs::Encoding::for_label_no_replacement(e) {
                    Some(enc) if enc != encoding_rs::UTF_8 => {
                        let text = String::from_utf8_lossy(&bytes);
                        enc.encode(&text).0.into_owned()
                    }
                    _ => bytes,
                },
            };
            (out, false)
        }
    }
}

/// UTF8ToHtml, including how it reads a truncated sequence: the bytes it
/// takes for the missing continuation are lost.
fn to_html_entities(v: &[u8]) -> (Vec<u8>, bool) {
    let mut out = Vec::with_capacity(v.len());
    let mut i = 0;
    while i < v.len() {
        let d = v[i];
        i += 1;
        let (mut c, mut trailing) = match d {
            0..=0x7F => (d as u32, 0),
            0x80..=0xBF => return (out, true),
            0xC0..=0xDF => ((d & 0x1F) as u32, 1),
            0xE0..=0xEF => ((d & 0x0F) as u32, 2),
            0xF0..=0xF7 => ((d & 0x07) as u32, 3),
            _ => return (out, true),
        };
        if v.len() - i < trailing {
            break;
        }
        while trailing > 0 {
            if i >= v.len() {
                break;
            }
            let t = v[i];
            i += 1;
            if t & 0xC0 != 0x80 {
                break;
            }
            c = (c << 6) | (t & 0x3F) as u32;
            trailing -= 1;
        }
        if c < 0x80 {
            out.push(c as u8);
        } else {
            out.push(b'&');
            match entity_name(c) {
                Some(name) => out.extend_from_slice(name.as_bytes()),
                None => out.extend_from_slice(format!("#{c}").as_bytes()),
            }
            out.push(b';');
        }
    }
    (out, false)
}

/// htmlSetMetaEncoding: records `encoding` in the Content-Type meta element
/// of the head (making one when there is none).
pub fn set_meta_encoding(doc: &mut Doc, encoding: Option<&[u8]>) -> i32 {
    if encoding.is_some_and(|e| e.eq_ignore_ascii_case(b"html")) {
        return -1;
    }
    let newcontent = encoding.map(|e| [b"text/html; charset=".as_slice(), e].concat());
    let Some((head, candidates)) = meta_scope(doc, true) else { return -1 };
    let is_el = |d: &Doc, id: Id, name: &[u8]| d.node(id).kind == Kind::Element && d.node(id).name.eq_ignore_ascii_case(name);
    let mut meta: Option<Id> = None;
    for &c in &candidates {
        if !is_el(doc, c, b"meta") {
            continue;
        }
        let mut http = false;
        let mut content = false;
        for &a in &doc.node(c).attrs {
            let attr = doc.node(a);
            if attr.name.eq_ignore_ascii_case(b"http-equiv") && attr.value.eq_ignore_ascii_case(b"Content-Type") {
                http = true;
            } else if attr.name.eq_ignore_ascii_case(b"content") {
                content = true;
            }
            if http && content {
                break;
            }
        }
        if http && content {
            meta = Some(c);
            break;
        }
    }
    match (meta, newcontent) {
        (None, Some(content)) => {
            if let Some(head) = head {
                let m = doc.add(Node::new(Kind::Element, b"meta".to_vec()));
                let first = doc.node(head).children.first().copied();
                doc.insert(head, m, first);
                for (name, value) in [(&b"http-equiv"[..], &b"Content-Type"[..]), (b"content", &content)] {
                    let mut attr = Node::new(Kind::Attribute, name.to_vec());
                    attr.value = value.to_vec();
                    let aid = doc.add(attr);
                    doc.set_attr(m, aid);
                }
            }
        }
        (Some(m), None) => doc.detach(m),
        (Some(m), Some(content)) => {
            let current = doc.node(m).attrs.iter().copied().find(|&a| doc.node(a).name.eq_ignore_ascii_case(b"content"));
            if let Some(a) = current {
                let old = doc.node(a).value.clone();
                let needle = encoding.unwrap_or_default().to_ascii_lowercase();
                if !old.to_ascii_lowercase().windows(needle.len().max(1)).any(|w| w == needle.as_slice()) {
                    doc.node_mut(a).value = content;
                }
            }
        }
        (None, None) => {}
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(html: &[u8], format: bool) -> String {
        let mut doc = Doc::new(b"DOMDocument".to_vec());
        crate::html::parse(&mut doc, html.to_vec(), None, 0);
        let mut out = HtmlOut::new(&doc, format);
        out.dump(DOC);
        String::from_utf8(encode_output(out.buf, meta_encoding(&doc).as_deref()).0).unwrap()
    }

    #[test]
    fn void_elements_and_boolean_attributes() {
        let s = save(b"<p>a<br><input checked disabled=disabled value=x></p>", false);
        assert!(s.ends_with("<html><body><p>a<br><input checked disabled value=\"x\"></p></body></html>\n"), "{s}");
        assert!(s.starts_with("<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\""), "{s}");
    }

    #[test]
    fn non_ascii_text_becomes_entities_and_uris_are_escaped() {
        let s = save("<meta charset=utf-8><a href=\"x y\u{e9}\">\u{e9}\u{20ac}</a>".as_bytes(), false);
        assert!(s.contains("<a href=\"x%20y%C3%A9\">&eacute;&euro;</a>"), "{s}");
    }

    #[test]
    fn format_output_breaks_lines_between_block_elements() {
        let s = save(b"<ul><li>a<li>b</ul>", true);
        assert!(s.ends_with("<ul>\n<li>a</li>\n<li>b</li>\n</ul></body></html>\n"), "{s}");
    }

    #[test]
    fn latin1_output_for_a_content_type_meta() {
        let s = encode_output("<p>\u{e9}\u{20ac}</p>".as_bytes().to_vec(), Some(b"ISO-8859-1")).0;
        assert_eq!(s, b"<p>\xe9&#8364;</p>");
        let (out, failed) = encode_output(b"a\x80b".to_vec(), None);
        assert!(failed && out == b"a");
    }
}

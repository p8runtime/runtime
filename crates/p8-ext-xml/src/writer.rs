//! libxml2's xmlTextWriter, ported: the state machine XMLWriter drives,
//! its escaping, indenting and namespace declarations, byte for byte as
//! libxml2 2.9.14 (the version PHP 8.4's reference build links) writes
//! them. libxml2 is MIT licensed, Copyright (C) Daniel Veillard.
//!
//! Reference: libxml2 xmlwriter.c, entities.c (xmlEncodeSpecialChars),
//! xmlsave.c (xmlBufAttrSerializeTxtContent, xmlSerializeHexCharRef),
//! encoding.c (xmlFindCharEncodingHandler)

/// Where an open element (or other construct) is: libxml2's
/// xmlTextWriterState.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Name,
    Attribute,
    Text,
    Pi,
    PiText,
    Cdata,
    Dtd,
    DtdText,
    DtdElem,
    DtdElemText,
    DtdAttl,
    DtdAttlText,
    DtdEnty,
    DtdEntyText,
    DtdPent,
    Comment,
}

struct Node {
    name: Vec<u8>,
    state: St,
    /// Which node this is (a namespace declaration belongs to one).
    id: u64,
}

/// A namespace declaration waiting for its element's start tag to close.
struct Ns {
    /// "xmlns" or "xmlns:prefix".
    prefix: Vec<u8>,
    uri: Vec<u8>,
    /// The node open when it was declared.
    elem: Option<u64>,
}

/// The output encoding startDocument chose.
#[derive(Clone, Copy)]
enum Encoder {
    Utf8,
    Latin1,
    Ascii,
    Other(&'static encoding_rs::Encoding),
}

/// The failure libxml2 reports as -1 (PHP: false).
pub(crate) type Res = Result<(), ()>;

/// libxml2's output buffer flushes to its sink once this much is pending.
const MINLEN: usize = 4000;

pub(crate) struct Writer {
    nodes: Vec<Node>,
    ns: Vec<Ns>,
    next_id: u64,
    indent: bool,
    doindent: bool,
    ichar: Vec<u8>,
    qchar: u8,
    encoder: Option<Encoder>,
    /// The writer's document has an encoding (startDocument gave one):
    /// attribute values then keep their non-ASCII characters.
    doc_encoding: bool,
    /// Written, not yet flushed to the sink.
    pending: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Writer {
        Writer {
            nodes: Vec::new(),
            ns: Vec::new(),
            next_id: 0,
            indent: false,
            doindent: false,
            ichar: b" ".to_vec(),
            qchar: b'"',
            encoder: None,
            doc_encoding: false,
            pending: Vec::new(),
        }
    }

    /// What is written and not yet flushed.
    pub(crate) fn take_pending(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }

    /// Whether libxml2 would hand its buffer to the sink now.
    pub(crate) fn wants_flush(&self) -> bool {
        self.pending.len() >= MINLEN
    }

    fn out(&mut self, bytes: &[u8]) {
        match self.encoder {
            None | Some(Encoder::Utf8) => self.pending.extend_from_slice(bytes),
            Some(enc) => encode(enc, bytes, &mut self.pending),
        }
    }

    fn q(&mut self) {
        let q = self.qchar;
        self.out(&[q]);
    }

    fn top_state(&self) -> Option<St> {
        self.nodes.last().map(|n| n.state)
    }

    fn set_top(&mut self, state: St) {
        if let Some(n) = self.nodes.last_mut() {
            n.state = state;
        }
    }

    fn push(&mut self, name: &[u8], state: St) {
        self.next_id += 1;
        self.nodes.push(Node { name: name.to_vec(), state, id: self.next_id });
    }

    fn write_indent(&mut self) {
        for _ in 1..self.nodes.len() {
            let ichar = self.ichar.clone();
            self.out(&ichar);
        }
    }

    /// The pending namespace declarations, as attributes (last declared first).
    fn output_ns_decl(&mut self) -> Res {
        while let Some(ns) = self.ns.pop() {
            self.write_attribute(&ns.prefix, &ns.uri)?;
        }
        Ok(())
    }

    /// Close what must be closed before content: a start tag, a PI's
    /// target, a DTD's or declaration's name.
    fn handle_state_dependencies(&mut self) -> Res {
        let extra: &[u8] = match self.top_state() {
            Some(St::Name) => {
                self.output_ns_decl()?;
                self.set_top(St::Text);
                b">"
            }
            Some(St::Pi) => {
                self.set_top(St::PiText);
                b" "
            }
            Some(St::Dtd) => {
                self.set_top(St::DtdText);
                b" ["
            }
            Some(St::DtdElem) => {
                self.set_top(St::DtdElemText);
                b" "
            }
            Some(St::DtdAttl) => {
                self.set_top(St::DtdAttlText);
                b" "
            }
            Some(St::DtdEnty | St::DtdPent) => {
                self.set_top(St::DtdEntyText);
                let q = self.qchar;
                self.out(&[b' ', q]);
                return Ok(());
            }
            _ => return Ok(()),
        };
        self.out(extra);
        Ok(())
    }

    /// Close an open start tag (">") before something that goes inside it.
    fn close_start_tag(&mut self, newline: bool) -> Res {
        self.output_ns_decl()?;
        self.out(b">");
        if newline && self.indent {
            self.out(b"\n");
        }
        self.set_top(St::Text);
        Ok(())
    }

    pub(crate) fn start_document(&mut self, version: Option<&[u8]>, encoding: Option<&[u8]>, standalone: Option<&[u8]>) -> Res {
        if !self.nodes.is_empty() {
            return Err(());
        }
        let mut name = None;
        if let Some(e) = encoding {
            let (enc, shown) = find_encoder(e).ok_or(())?;
            self.encoder = Some(enc);
            name = Some(shown);
            self.doc_encoding = true;
        } else {
            self.encoder = None;
        }
        self.out(b"<?xml version=");
        self.q();
        self.out(version.unwrap_or(b"1.0"));
        self.q();
        if let Some(name) = name {
            self.out(b" encoding=");
            self.q();
            self.out(&name);
            self.q();
        }
        if let Some(s) = standalone {
            self.out(b" standalone=");
            self.q();
            self.out(s);
            self.q();
        }
        self.out(b"?>\n");
        Ok(())
    }

    pub(crate) fn end_document(&mut self) -> Res {
        while let Some(state) = self.top_state() {
            match state {
                St::Name | St::Attribute | St::Text => self.end_element()?,
                St::Pi | St::PiText => self.end_pi()?,
                St::Cdata => self.end_cdata()?,
                St::Comment => self.end_comment()?,
                _ => self.end_dtd()?,
            }
        }
        if !self.indent {
            self.out(b"\n");
        }
        Ok(())
    }

    pub(crate) fn start_comment(&mut self) -> Res {
        match self.top_state() {
            None | Some(St::Text) => {}
            Some(St::Name) => self.close_start_tag(true)?,
            Some(_) => return Err(()),
        }
        self.push(b"", St::Comment);
        if self.indent {
            self.write_indent();
        }
        self.out(b"<!--");
        Ok(())
    }

    pub(crate) fn end_comment(&mut self) -> Res {
        match self.top_state() {
            Some(St::Comment) => self.out(b"-->"),
            _ => return Err(()),
        }
        if self.indent {
            self.out(b"\n");
        }
        self.nodes.pop();
        Ok(())
    }

    pub(crate) fn write_comment(&mut self, content: &[u8]) -> Res {
        self.start_comment()?;
        self.write_string(content)?;
        self.end_comment()
    }

    pub(crate) fn start_element(&mut self, name: &[u8]) -> Res {
        if name.is_empty() {
            return Err(());
        }
        match self.top_state() {
            Some(St::Attribute) => {
                self.end_attribute()?;
                self.close_start_tag(true)?;
            }
            Some(St::Name) => self.close_start_tag(true)?,
            _ => {}
        }
        self.push(name, St::Name);
        if self.indent {
            self.write_indent();
        }
        self.out(b"<");
        self.out(name);
        Ok(())
    }

    pub(crate) fn start_element_ns(&mut self, prefix: Option<&[u8]>, name: &[u8], uri: Option<&[u8]>) -> Res {
        if name.is_empty() {
            return Err(());
        }
        self.start_element(&qualified(prefix, name))?;
        if let Some(uri) = uri {
            let elem = self.nodes.last().map(|n| n.id);
            self.ns.push(Ns { prefix: xmlns(prefix), uri: uri.to_vec(), elem });
        }
        Ok(())
    }

    pub(crate) fn end_element(&mut self) -> Res {
        let Some(state) = self.top_state() else {
            self.ns.clear();
            return Err(());
        };
        match state {
            St::Attribute | St::Name => {
                if state == St::Attribute {
                    self.end_attribute()?;
                }
                self.output_ns_decl()?;
                if self.indent {
                    self.doindent = true;
                }
                self.out(b"/>");
            }
            St::Text => self.close_tag(),
            _ => return Err(()),
        }
        if self.indent {
            self.out(b"\n");
        }
        self.nodes.pop();
        Ok(())
    }

    /// "</name>", indented when the element held only elements.
    fn close_tag(&mut self) {
        if self.indent && self.doindent {
            self.write_indent();
        }
        self.doindent = true;
        let name = self.nodes.last().map(|n| n.name.clone()).unwrap_or_default();
        self.out(b"</");
        self.out(&name);
        self.out(b">");
    }

    pub(crate) fn full_end_element(&mut self) -> Res {
        let Some(state) = self.top_state() else { return Err(()) };
        match state {
            St::Attribute | St::Name => {
                if state == St::Attribute {
                    self.end_attribute()?;
                }
                self.output_ns_decl()?;
                self.out(b">");
                if self.indent {
                    self.doindent = false;
                }
                self.close_tag();
            }
            St::Text => self.close_tag(),
            _ => return Err(()),
        }
        if self.indent {
            self.out(b"\n");
        }
        self.nodes.pop();
        Ok(())
    }

    pub(crate) fn start_attribute(&mut self, name: &[u8]) -> Res {
        if name.is_empty() {
            return Err(());
        }
        match self.top_state() {
            Some(St::Attribute) => self.end_attribute()?,
            Some(St::Name) => {}
            _ => return Err(()),
        }
        self.out(b" ");
        self.out(name);
        self.out(b"=");
        self.q();
        self.set_top(St::Attribute);
        Ok(())
    }

    pub(crate) fn start_attribute_ns(&mut self, prefix: Option<&[u8]>, name: &[u8], uri: Option<&[u8]>) -> Res {
        if name.is_empty() {
            return Err(());
        }
        if let Some(uri) = uri {
            let decl = xmlns(prefix);
            let elem = self.nodes.last().map(|n| n.id);
            match self.ns.iter().find(|n| n.prefix == decl && n.elem == elem) {
                Some(existing) if existing.uri == uri => {}
                Some(_) => return Err(()),
                None => self.ns.push(Ns { prefix: decl, uri: uri.to_vec(), elem }),
            }
        }
        self.start_attribute(&qualified(prefix, name))
    }

    pub(crate) fn end_attribute(&mut self) -> Res {
        match self.top_state() {
            Some(St::Attribute) => {
                self.set_top(St::Name);
                self.q();
                Ok(())
            }
            _ => Err(()),
        }
    }

    pub(crate) fn write_attribute(&mut self, name: &[u8], content: &[u8]) -> Res {
        self.start_attribute(name)?;
        self.write_string(content)?;
        self.end_attribute()
    }

    pub(crate) fn write_attribute_ns(&mut self, prefix: Option<&[u8]>, name: &[u8], uri: Option<&[u8]>, content: &[u8]) -> Res {
        if name.is_empty() {
            return Err(());
        }
        self.start_attribute_ns(prefix, name, uri)?;
        self.write_string(content)?;
        self.end_attribute()
    }

    pub(crate) fn write_element(&mut self, name: &[u8], content: Option<&[u8]>) -> Res {
        self.start_element(name)?;
        if let Some(c) = content {
            self.write_string(c)?;
        }
        self.end_element()
    }

    pub(crate) fn write_element_ns(&mut self, prefix: Option<&[u8]>, name: &[u8], uri: Option<&[u8]>, content: &[u8]) -> Res {
        if name.is_empty() {
            return Err(());
        }
        self.start_element_ns(prefix, name, uri)?;
        self.write_string(content)?;
        self.end_element()
    }

    /// Text: escaped in an element, attribute-escaped in an attribute,
    /// as it is elsewhere (a comment, PI, CDATA section, DTD).
    pub(crate) fn write_string(&mut self, content: &[u8]) -> Res {
        match self.top_state() {
            Some(St::Name | St::Text) => {
                let escaped = encode_special_chars(content);
                self.write_raw(&escaped)
            }
            Some(St::Attribute) => {
                let escaped = attribute_text(content, self.doc_encoding);
                self.out(&escaped);
                Ok(())
            }
            _ => self.write_raw(content),
        }
    }

    pub(crate) fn write_raw(&mut self, content: &[u8]) -> Res {
        if !self.nodes.is_empty() {
            self.handle_state_dependencies()?;
        }
        if self.indent {
            self.doindent = false;
        }
        self.out(content);
        Ok(())
    }

    pub(crate) fn start_pi(&mut self, target: &[u8]) -> Res {
        if target.is_empty() || target.eq_ignore_ascii_case(b"xml") {
            return Err(());
        }
        match self.top_state() {
            Some(St::Attribute) => {
                self.end_attribute()?;
                self.close_start_tag(false)?;
            }
            Some(St::Name) => self.close_start_tag(false)?,
            None | Some(St::Text | St::Dtd) => {}
            Some(_) => return Err(()),
        }
        self.push(target, St::Pi);
        self.out(b"<?");
        self.out(target);
        Ok(())
    }

    pub(crate) fn end_pi(&mut self) -> Res {
        match self.top_state() {
            None => return Ok(()),
            Some(St::Pi | St::PiText) => self.out(b"?>"),
            Some(_) => return Err(()),
        }
        if self.indent {
            self.out(b"\n");
        }
        self.nodes.pop();
        Ok(())
    }

    pub(crate) fn write_pi(&mut self, target: &[u8], content: &[u8]) -> Res {
        self.start_pi(target)?;
        self.write_string(content)?;
        self.end_pi()
    }

    pub(crate) fn start_cdata(&mut self) -> Res {
        match self.top_state() {
            None | Some(St::Text | St::Pi | St::PiText) => {}
            Some(St::Attribute) => {
                self.end_attribute()?;
                self.close_start_tag(false)?;
            }
            Some(St::Name) => self.close_start_tag(false)?,
            Some(_) => return Err(()),
        }
        self.push(b"", St::Cdata);
        self.out(b"<![CDATA[");
        Ok(())
    }

    pub(crate) fn end_cdata(&mut self) -> Res {
        match self.top_state() {
            Some(St::Cdata) => self.out(b"]]>"),
            _ => return Err(()),
        }
        self.nodes.pop();
        Ok(())
    }

    pub(crate) fn write_cdata(&mut self, content: &[u8]) -> Res {
        self.start_cdata()?;
        self.write_string(content)?;
        self.end_cdata()
    }

    pub(crate) fn start_dtd(&mut self, name: &[u8], pubid: Option<&[u8]>, sysid: Option<&[u8]>) -> Res {
        if name.is_empty() || !self.nodes.is_empty() {
            return Err(());
        }
        self.push(name, St::Dtd);
        self.out(b"<!DOCTYPE ");
        self.out(name);
        if let Some(pubid) = pubid {
            if sysid.is_none() {
                return Err(());
            }
            self.out(if self.indent { b"\n" } else { b" " });
            self.out(b"PUBLIC ");
            self.q();
            self.out(pubid);
            self.q();
        }
        if let Some(sysid) = sysid {
            if pubid.is_none() {
                self.out(if self.indent { b"\n" } else { b" " });
                self.out(b"SYSTEM ");
            } else {
                self.out(if self.indent { b"\n       " } else { b" " });
            }
            self.q();
            self.out(sysid);
            self.q();
        }
        Ok(())
    }

    pub(crate) fn end_dtd(&mut self) -> Res {
        while let Some(state) = self.top_state() {
            match state {
                St::DtdText | St::Dtd => {
                    if state == St::DtdText {
                        self.out(b"]");
                    }
                    self.out(b">");
                    if self.indent {
                        self.out(b"\n");
                    }
                    self.nodes.pop();
                }
                St::DtdElem | St::DtdElemText => self.end_dtd_element()?,
                St::DtdAttl | St::DtdAttlText => self.end_dtd_attlist()?,
                St::DtdEnty | St::DtdPent | St::DtdEntyText => self.end_dtd_entity()?,
                St::Comment => self.end_comment()?,
                _ => break,
            }
        }
        Ok(())
    }

    pub(crate) fn write_dtd(&mut self, name: &[u8], pubid: Option<&[u8]>, sysid: Option<&[u8]>, subset: Option<&[u8]>) -> Res {
        self.start_dtd(name, pubid, sysid)?;
        if let Some(s) = subset {
            self.write_string(s)?;
        }
        self.end_dtd()
    }

    /// Open the DTD's internal subset (" [") for a declaration in it
    /// (with nothing open, the declaration stands alone: libxml2's lists
    /// have a sentinel, so its "nothing open" check never fires).
    fn open_subset(&mut self) -> Res {
        match self.top_state() {
            None => Ok(()),
            Some(St::Dtd) => {
                self.out(b" [");
                if self.indent {
                    self.out(b"\n");
                }
                self.set_top(St::DtdText);
                Ok(())
            }
            Some(St::DtdText) => Ok(()),
            _ => Err(()),
        }
    }

    pub(crate) fn start_dtd_element(&mut self, name: &[u8]) -> Res {
        if name.is_empty() {
            return Err(());
        }
        self.open_subset()?;
        self.push(name, St::DtdElem);
        if self.indent {
            self.write_indent();
        }
        self.out(b"<!ELEMENT ");
        self.out(name);
        Ok(())
    }

    pub(crate) fn end_dtd_element(&mut self) -> Res {
        match self.top_state() {
            Some(St::DtdElem | St::DtdElemText) => self.out(b">"),
            _ => return Err(()),
        }
        if self.indent {
            self.out(b"\n");
        }
        self.nodes.pop();
        Ok(())
    }

    pub(crate) fn write_dtd_element(&mut self, name: &[u8], content: &[u8]) -> Res {
        self.start_dtd_element(name)?;
        self.write_string(content)?;
        self.end_dtd_element()
    }

    pub(crate) fn start_dtd_attlist(&mut self, name: &[u8]) -> Res {
        if name.is_empty() {
            return Err(());
        }
        self.open_subset()?;
        self.push(name, St::DtdAttl);
        if self.indent {
            self.write_indent();
        }
        self.out(b"<!ATTLIST ");
        self.out(name);
        Ok(())
    }

    pub(crate) fn end_dtd_attlist(&mut self) -> Res {
        match self.top_state() {
            Some(St::DtdAttl | St::DtdAttlText) => self.out(b">"),
            _ => return Err(()),
        }
        if self.indent {
            self.out(b"\n");
        }
        self.nodes.pop();
        Ok(())
    }

    pub(crate) fn write_dtd_attlist(&mut self, name: &[u8], content: &[u8]) -> Res {
        self.start_dtd_attlist(name)?;
        self.write_string(content)?;
        self.end_dtd_attlist()
    }

    pub(crate) fn start_dtd_entity(&mut self, pe: bool, name: &[u8]) -> Res {
        if name.is_empty() {
            return Err(());
        }
        self.open_subset()?;
        self.push(name, if pe { St::DtdPent } else { St::DtdEnty });
        if self.indent {
            self.write_indent();
        }
        self.out(b"<!ENTITY ");
        if pe {
            self.out(b"% ");
        }
        self.out(name);
        Ok(())
    }

    pub(crate) fn end_dtd_entity(&mut self) -> Res {
        match self.top_state() {
            Some(St::DtdEntyText) => {
                self.q();
                self.out(b">");
            }
            Some(St::DtdEnty | St::DtdPent) => self.out(b">"),
            _ => return Err(()),
        }
        if self.indent {
            self.out(b"\n");
        }
        self.nodes.pop();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // libxml2's xmlTextWriterWriteDTDEntity takes these six
    pub(crate) fn write_dtd_entity(
        &mut self,
        pe: bool,
        name: &[u8],
        pubid: Option<&[u8]>,
        sysid: Option<&[u8]>,
        ndataid: Option<&[u8]>,
        content: Option<&[u8]>,
    ) -> Res {
        if (content.is_none() && pubid.is_none() && sysid.is_none()) || (pe && ndataid.is_some()) {
            return Err(());
        }
        if pubid.is_none() && sysid.is_none() {
            let content = content.ok_or(())?;
            if name.is_empty() {
                return Err(());
            }
            self.start_dtd_entity(pe, name)?;
            self.write_string(content)?;
            return self.end_dtd_entity();
        }
        self.start_dtd_entity(pe, name)?;
        match self.top_state() {
            Some(St::DtdEnty) => {}
            Some(St::DtdPent) if ndataid.is_none() => {}
            _ => return Err(()),
        }
        if let Some(pubid) = pubid {
            if sysid.is_none() {
                return Err(());
            }
            self.out(b" PUBLIC ");
            self.q();
            self.out(pubid);
            self.q();
        }
        if let Some(sysid) = sysid {
            if pubid.is_none() {
                self.out(b" SYSTEM");
            }
            self.out(b" ");
            self.q();
            self.out(sysid);
            self.q();
        }
        if let Some(n) = ndataid {
            self.out(b" NDATA ");
            self.out(n);
        }
        self.end_dtd_entity()
    }

    pub(crate) fn set_indent(&mut self, indent: bool) {
        self.indent = indent;
        self.doindent = true;
    }

    pub(crate) fn set_indent_string(&mut self, s: &[u8]) {
        self.ichar = s.to_vec();
    }
}

/// "prefix:name", or "name".
fn qualified(prefix: Option<&[u8]>, name: &[u8]) -> Vec<u8> {
    match prefix {
        Some(p) => [p, b":", name].concat(),
        None => name.to_vec(),
    }
}

/// "xmlns:prefix", or "xmlns".
fn xmlns(prefix: Option<&[u8]>) -> Vec<u8> {
    match prefix {
        Some(p) => [b"xmlns:", p].concat(),
        None => b"xmlns".to_vec(),
    }
}

/// xmlEncodeSpecialChars: element content's escaping.
fn encode_special_chars(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        match c {
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\r' => out.extend_from_slice(b"&#13;"),
            c => out.push(c),
        }
    }
    out
}

/// "&#xHEX;" (uppercase), as xmlSerializeHexCharRef writes it.
fn hex_char_ref(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(format!("&#x{v:X};").as_bytes());
}

/// XML's Char production.
fn is_char(v: u32) -> bool {
    matches!(v, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// xmlBufAttrSerializeTxtContent: an attribute value's escaping. With no
/// document encoding, non-ASCII characters become hex references (a lone
/// lead byte at the very end is kept as it is).
fn attribute_text(s: &[u8], doc_encoding: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match c {
            b'\n' => out.extend_from_slice(b"&#10;"),
            b'\r' => out.extend_from_slice(b"&#13;"),
            b'\t' => out.extend_from_slice(b"&#9;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            0x80.. if at(i + 1) != 0 && !doc_encoding => {
                let (val, len) = if c < 0xC0 {
                    (0, 1)
                } else if c < 0xE0 {
                    ((u32::from(c) & 0x1F) << 6 | (u32::from(at(i + 1)) & 0x3F), 2)
                } else if c < 0xF0 && at(i + 2) != 0 {
                    ((u32::from(c) & 0x0F) << 12 | (u32::from(at(i + 1)) & 0x3F) << 6 | (u32::from(at(i + 2)) & 0x3F), 3)
                } else if c < 0xF8 && at(i + 2) != 0 && at(i + 3) != 0 {
                    let v = (u32::from(c) & 0x07) << 18 | (u32::from(at(i + 1)) & 0x3F) << 12 | (u32::from(at(i + 2)) & 0x3F) << 6 | (u32::from(at(i + 3)) & 0x3F);
                    (v, 4)
                } else {
                    (0, 1)
                };
                if len == 1 || !is_char(val) {
                    hex_char_ref(&mut out, u32::from(c));
                    i += 1;
                } else {
                    hex_char_ref(&mut out, val);
                    i += len;
                }
                continue;
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// xmlFindCharEncodingHandler: the encoder for `name`, and the name the
/// XML declaration then shows (libxml2's own handlers' names, else the
/// name as given, which iconv took).
fn find_encoder(name: &[u8]) -> Option<(Encoder, Vec<u8>)> {
    let upper = name.to_ascii_uppercase();
    let builtin = |enc: Encoder, shown: &[u8]| Some((enc, shown.to_vec()));
    match upper.as_slice() {
        b"UTF-8" => return builtin(Encoder::Utf8, b"UTF-8"),
        b"ISO-8859-1" => return builtin(Encoder::Latin1, b"ISO-8859-1"),
        b"ASCII" => return builtin(Encoder::Ascii, b"ASCII"),
        b"US-ASCII" => return builtin(Encoder::Ascii, b"US-ASCII"),
        b"UTF8" => return Some((Encoder::Utf8, name.to_vec())),
        b"LATIN1" | b"ISO8859-1" | b"ISO_8859-1" | b"L1" => return Some((Encoder::Latin1, name.to_vec())),
        _ => {}
    }
    let enc = encoding_rs::Encoding::for_label_no_replacement(name)?;
    Some((Encoder::Other(enc), name.to_vec()))
}

/// Convert UTF-8 output; a character the encoding lacks becomes "&#N;"
/// (libxml2's xmlCharEncOutput).
fn encode(enc: Encoder, bytes: &[u8], out: &mut Vec<u8>) {
    let text = String::from_utf8_lossy(bytes);
    let limit = match enc {
        Encoder::Latin1 => 0xFF,
        Encoder::Ascii => 0x7F,
        Encoder::Utf8 => {
            out.extend_from_slice(bytes);
            return;
        }
        Encoder::Other(e) => {
            let (encoded, _, _) = e.encode(&text);
            out.extend_from_slice(&encoded);
            return;
        }
    };
    for ch in text.chars() {
        let v = u32::from(ch);
        if v <= limit {
            out.push(v as u8);
        } else {
            out.extend_from_slice(format!("&#{v};").as_bytes());
        }
    }
}

/// xmlValidateName (no surrounding spaces): an XML Name.
pub(crate) fn valid_name(name: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(name) else { return false };
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    let start = |c: char| c.is_alphabetic() || c == '_' || c == ':';
    start(first) && chars.all(|c| start(c) || c.is_alphanumeric() || matches!(c, '.' | '-' | '\u{B7}') || is_combining(c))
}

/// Combining characters and extenders XML names may contain.
fn is_combining(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{0483}'..='\u{0486}' | '\u{0591}'..='\u{05C4}' | '\u{064B}'..='\u{0652}' | '\u{3005}' | '\u{3031}'..='\u{3035}' | '\u{309D}'..='\u{309E}' | '\u{30FC}'..='\u{30FE}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: impl FnOnce(&mut Writer) -> Res) -> String {
        let mut w = Writer::new();
        f(&mut w).expect("ok");
        String::from_utf8(w.take_pending()).expect("utf-8")
    }

    #[test]
    fn nested_elements_indent_like_libxml() {
        let out = run(|w| {
            w.set_indent(true);
            w.start_element(b"a")?;
            w.write_attribute(b"x", b"1")?;
            w.start_element(b"b")?;
            w.write_string(b"t")?;
            w.end_element()?;
            w.write_element(b"c", None)?;
            w.end_element()
        });
        assert_eq!(out, "<a x=\"1\">\n <b>t</b>\n <c/>\n</a>\n");
    }

    #[test]
    fn namespaces_close_with_the_start_tag() {
        let out = run(|w| {
            w.start_element_ns(Some(b"p"), b"r", Some(b"urn:a"))?;
            w.write_attribute_ns(Some(b"q"), b"at", Some(b"urn:b"), b"v")?;
            w.end_element()
        });
        assert_eq!(out, "<p:r q:at=\"v\" xmlns:q=\"urn:b\" xmlns:p=\"urn:a\"/>");
    }

    #[test]
    fn attribute_escaping_without_a_document_encoding() {
        assert_eq!(attribute_text("é\t<\"".as_bytes(), false), b"&#xE9;&#9;&lt;&quot;");
        assert_eq!(attribute_text("é".as_bytes(), true), "é".as_bytes());
    }
}

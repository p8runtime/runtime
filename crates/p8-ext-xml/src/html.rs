//! The HTML parser DOM's loadHTML() uses: libxml2's HTML 4 parser, with its
//! error recovery (implied html/head/body/p elements, auto-closing of
//! elements, end-tag priorities), its warnings, and the encoding guessing
//! it does on the way. It builds the DOM tree directly, as libxml2's SAX2
//! handlers do for an HTML document.
//!
//! Reference: libxml2 2.9.14 HTMLparser.c (htmlParseDocument and what it
//! calls), SAX2.c (xmlSAX2StartElement, xmlSAX2Characters, ...)

use crate::dom::tree::{DOC, Doc, Id, Kind, Node};
use crate::encoding::decoder;
use crate::html_tables::{ALLOW_PCDATA, check_auto_close, end_priority, entity_lookup, is_boolean_attr, tag_lookup};
use crate::parser::XmlError;

/// libxml2's htmlParserOption values.
pub const PARSE_RECOVER: i64 = 1;
pub const PARSE_NODEFDTD: i64 = 4;
pub const PARSE_NOBLANKS: i64 = 256;
pub const PARSE_NOIMPLIED: i64 = 8192;
pub const PARSE_IGNORE_ENC: i64 = 1 << 21;

const XML_FROM_HTML: i32 = 5;
const ERR_ERROR: i32 = 2;

const ERR_DOCUMENT_EMPTY: i32 = 4;
const ERR_INVALID_HEX_CHARREF: i32 = 6;
const ERR_INVALID_DEC_CHARREF: i32 = 7;
const ERR_INVALID_CHARREF: i32 = 8;
const ERR_INVALID_CHAR: i32 = 9;
const ERR_ENTITYREF_SEMICOL_MISSING: i32 = 23;
const ERR_UNSUPPORTED_ENCODING: i32 = 32;
const ERR_ATTRIBUTE_NOT_FINISHED: i32 = 40;
const ERR_ATTRIBUTE_REDEFINED: i32 = 42;
const ERR_LITERAL_NOT_STARTED: i32 = 43;
const ERR_LITERAL_NOT_FINISHED: i32 = 44;
const ERR_COMMENT_NOT_FINISHED: i32 = 45;
const ERR_PI_NOT_STARTED: i32 = 46;
const ERR_PI_NOT_FINISHED: i32 = 47;
const ERR_DOCTYPE_NOT_FINISHED: i32 = 61;
const ERR_SPACE_REQUIRED: i32 = 65;
const ERR_NAME_REQUIRED: i32 = 68;
const ERR_URI_REQUIRED: i32 = 70;
const ERR_PUBID_REQUIRED: i32 = 71;
const ERR_GT_REQUIRED: i32 = 73;
const ERR_LTSLASH_REQUIRED: i32 = 74;
const ERR_TAG_NAME_MISMATCH: i32 = 76;
const ERR_INVALID_ENCODING: i32 = 81;
const ERR_HTML_STRUCTURE: i32 = 800;
const ERR_HTML_UNKNOWN_TAG: i32 = 801;

/// An attribute as the tag parser read it: the name, and the value if
/// there was one.
type Att = (Vec<u8>, Option<Vec<u8>>);

fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_char(c: u32) -> bool {
    matches!(c, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// An XML Name character beyond ASCII (IS_LETTER, IS_DIGIT, IS_COMBINING and
/// IS_EXTENDER, near enough).
fn is_name_char_wide(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '\u{B7}' | '\u{300}'..='\u{36F}')
}

fn is_pubid_char(c: u8) -> bool {
    matches!(c, 0x20 | 0xD | 0xA | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'\'' | b'(' | b')' | b'+' | b',' | b'.' | b'/' | b':' | b'=' | b'?' | b';' | b'!' | b'*' | b'#' | b'@' | b'$' | b'_' | b'%')
}

/// COPY_BUF: one character into the text buffer; a one-byte length copies
/// the raw value.
fn copy_buf(out: &mut Vec<u8>, len: usize, c: u32) {
    if len == 1 {
        out.push(c as u8);
    } else {
        push_utf8(out, c);
    }
}

fn push_utf8(out: &mut Vec<u8>, c: u32) {
    match char::from_u32(c) {
        Some(ch) => {
            let mut b = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
        }
        None => out.push(c as u8),
    }
}

/// The libxml2 UTF-8 bytes of a code point (no surrogate check).
fn encode_cp(out: &mut Vec<u8>, c: u32) {
    if c < 0x80 {
        out.push(c as u8);
    } else if c < 0x800 {
        out.extend_from_slice(&[((c >> 6) & 0x1F) as u8 | 0xC0, (c & 0x3F) as u8 | 0x80]);
    } else if c < 0x10000 {
        out.extend_from_slice(&[((c >> 12) & 0x0F) as u8 | 0xE0, ((c >> 6) & 0x3F) as u8 | 0x80, (c & 0x3F) as u8 | 0x80]);
    } else {
        out.extend_from_slice(&[((c >> 18) & 0x07) as u8 | 0xF0, ((c >> 12) & 0x3F) as u8 | 0x80, ((c >> 6) & 0x3F) as u8 | 0x80, (c & 0x3F) as u8 | 0x80]);
    }
}

fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle))
}

/// What iconv makes of an encoding name with trailing junk (`utf-8;`,
/// `latin1/`, `x//IGNORE`): the name without it.
fn trim_junk(name: &[u8]) -> &[u8] {
    let name = match name.windows(2).position(|w| w == b"//") {
        Some(at) => &name[..at],
        None => name,
    };
    let end = name.iter().rposition(|c| c.is_ascii_alphanumeric()).map_or(0, |i| i + 1);
    &name[..end]
}

/// xmlSplitQName for an element name: the part after the prefix (the HTML
/// handler drops the prefix).
fn local_name(full: &[u8]) -> Vec<u8> {
    if full.first() == Some(&b':') {
        return full.to_vec();
    }
    match full.iter().position(|&c| c == b':') {
        Some(i) if i + 1 < full.len() => {
            let rest = &full[i + 1..];
            if rest[0].is_ascii_alphabetic() || rest[0] == b'_' || rest[0] == b':' { rest.to_vec() } else { full.to_vec() }
        }
        _ => full.to_vec(),
    }
}

pub struct Parser<'d> {
    doc: &'d mut Doc,
    buf: Vec<u8>,
    pos: usize,
    filename: Option<String>,
    options: i64,
    keep_blanks: bool,
    recovery: bool,
    /// ctxt->charset == UTF-8: the input is read as UTF-8 from here on.
    charset_utf8: bool,
    /// An encoder has converted the input (its tail) to UTF-8.
    converted: bool,
    /// ctxt->input->encoding: the encoding a meta element named.
    input_encoding: Option<Vec<u8>>,
    names: Vec<Vec<u8>>,
    /// ctxt->nodeTab: the elements being built.
    nodes: Vec<Id>,
    /// ctxt->html: 3 once a head was seen, 10 once a body was.
    html: u8,
    depth: i32,
    eof: bool,
    errors: Vec<XmlError>,
    /// The line and column libxml2 reports (input->line, input->col).
    line: u32,
    col: u32,
    /// ctxt->disableSAX: an encoding error stops text, comments and the like.
    disable_sax: bool,
    /// How often the unread input was replaced by a conversion of it.
    generation: u32,
    /// The ID attribute values seen (xmlAddID).
    ids: std::collections::HashSet<Vec<u8>>,
}

/// Parses `data` into `doc` (empty) as libxml2's HTML parser does with
/// `options` (the HTML_PARSE_* flags): the errors it raised.
pub fn parse(doc: &mut Doc, data: Vec<u8>, filename: Option<String>, options: i64) -> Vec<XmlError> {
    let mut p = Parser {
        doc,
        buf: data,
        pos: 0,
        filename,
        options,
        keep_blanks: options & PARSE_NOBLANKS == 0,
        recovery: options & PARSE_RECOVER != 0,
        charset_utf8: false,
        converted: false,
        input_encoding: None,
        names: Vec::new(),
        nodes: Vec::new(),
        html: 0,
        depth: 0,
        eof: false,
        errors: Vec::new(),
        line: 1,
        col: 1,
        disable_sax: false,
        generation: 0,
        ids: std::collections::HashSet::new(),
    };
    p.parse_document();
    p.errors
}

impl Parser<'_> {
    // ---- input ----

    fn cur(&self) -> u8 {
        self.buf.get(self.pos).copied().unwrap_or(0)
    }

    fn nxt(&self, n: usize) -> u8 {
        self.buf.get(self.pos + n).copied().unwrap_or(0)
    }

    fn upp(&self, n: usize) -> u8 {
        self.nxt(n).to_ascii_uppercase()
    }

    fn skip_blanks(&mut self) {
        while self.pos < self.buf.len() && is_blank(self.buf[self.pos]) {
            self.count_col();
            self.pos += 1;
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// The line and column count for the character at the cursor.
    fn count_col(&mut self) {
        if self.cur() == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
    }

    /// NEXTL: past a character `l` bytes long.
    fn nextl(&mut self, l: usize) {
        self.count_col();
        self.pos += l;
    }

    /// SKIP: past `n` ASCII characters.
    fn skip(&mut self, n: usize) {
        self.col += n as u32;
        self.pos += n;
    }

    /// (line, column) of the current position, as libxml2 counts them.
    fn line_col(&self) -> (u32, u32) {
        (self.line, self.col)
    }

    fn err(&mut self, code: i32, message: String) {
        self.err_in(XML_FROM_HTML, code, message);
    }

    fn err_in(&mut self, domain: i32, code: i32, message: String) {
        self.err_level(domain, ERR_ERROR, code, message);
    }

    fn err_level(&mut self, domain: i32, level: i32, code: i32, message: String) {
        if self.eof {
            return;
        }
        let (line, col) = self.line_col();
        self.errors.push(XmlError {
            domain,
            code,
            level,
            message,
            line: line as i32,
            column: col as i32,
            file: self.filename.clone(),
            context: Vec::new(),
            context_col: 0,
        });
    }

    /// Replaces the unread input by `decode` of it.
    fn convert_tail(&mut self, name: &[u8]) -> bool {
        let Some(d) = decoder(name).or_else(|| decoder(trim_junk(name))) else { return false };
        if !self.converted {
            let tail = d.decode(&self.buf[self.pos..]);
            self.buf.truncate(self.pos);
            self.buf.extend_from_slice(&tail);
            self.converted = true;
            self.generation += 1;
        }
        true
    }

    /// htmlFindEncoding: a charset named by a later http-equiv meta.
    fn find_encoding(&self) -> Option<Vec<u8>> {
        if self.input_encoding.is_some() || self.converted {
            return None;
        }
        let rest = &self.buf[self.pos..];
        let rest = &rest[..rest.iter().position(|&b| b == 0).unwrap_or(rest.len())];
        let a = find_ci(rest, b"HTTP-EQUIV")?;
        let b = find_ci(&rest[a..], b"CONTENT")? + a;
        let c = find_ci(&rest[b..], b"CHARSET=")? + b + 8;
        let end = rest[c..].iter().position(|&x| !(x.is_ascii_alphanumeric() || matches!(x, b'-' | b'_' | b':' | b'/'))).map_or(rest.len(), |i| c + i);
        if end == c { None } else { Some(rest[c..end].to_vec()) }
    }

    /// htmlCurrentChar: the character at the cursor and its length. Input
    /// that is not UTF-8 is taken as ISO-8859-1 (or what a meta element
    /// names) from the first non-ASCII byte on.
    fn cur_char(&mut self) -> (u32, usize) {
        if self.eof {
            return (0, 0);
        }
        if !self.charset_utf8 {
            let b = self.cur();
            if b < 0x80 {
                if b == 0 && !self.at_end() {
                    self.err(ERR_INVALID_CHAR, "Char 0x0 out of allowed range\n".into());
                    return (b' ' as u32, 1);
                }
                return (b as u32, 1);
            }
            match self.find_encoding() {
                None => {
                    self.convert_tail(b"ISO-8859-1");
                }
                Some(guess) => {
                    self.input_encoding = Some(guess.clone());
                    if !guess.eq_ignore_ascii_case(b"UTF-8") && !guess.eq_ignore_ascii_case(b"UTF8") && !self.convert_tail(&guess) {
                        self.err(ERR_INVALID_ENCODING, format!("Unsupported encoding {}", String::from_utf8_lossy(&guess)));
                    }
                }
            }
            self.charset_utf8 = true;
        }
        let c = self.cur();
        if c & 0x80 != 0 {
            let at = |s: &Self, i: usize| s.nxt(i);
            let bad = 'dec: {
                if c & 0x40 == 0 || at(self, 1) & 0xC0 != 0x80 {
                    break 'dec true;
                }
                if c & 0xE0 == 0xE0 {
                    if at(self, 2) & 0xC0 != 0x80 {
                        break 'dec true;
                    }
                    if c & 0xF0 == 0xF0 && (c & 0xF8 != 0xF0 || at(self, 3) & 0xC0 != 0x80) {
                        break 'dec true;
                    }
                }
                false
            };
            if !bad {
                let (len, val) = if c & 0xE0 != 0xE0 {
                    (2, ((c as u32 & 0x1F) << 6) | (self.nxt(1) as u32 & 0x3F))
                } else if c & 0xF0 != 0xF0 {
                    (3, ((c as u32 & 0xF) << 12) | ((self.nxt(1) as u32 & 0x3F) << 6) | (self.nxt(2) as u32 & 0x3F))
                } else {
                    (
                        4,
                        ((c as u32 & 0x7) << 18) | ((self.nxt(1) as u32 & 0x3F) << 12) | ((self.nxt(2) as u32 & 0x3F) << 6) | (self.nxt(3) as u32 & 0x3F),
                    )
                };
                let min = [0, 0, 0x80, 0x800, 0x10000][len];
                if val >= min {
                    if !is_char(val) {
                        self.err(ERR_INVALID_CHAR, format!("Char 0x{val:X} out of allowed range\n"));
                    }
                    return (val, len);
                }
            }
            // Not UTF-8: say so, and read the rest as ISO-8859-1.
            self.err(ERR_INVALID_ENCODING, "Input is not proper UTF-8, indicate encoding !\n".into());
            if !self.converted {
                self.convert_tail(b"ISO-8859-1");
            }
            return (self.cur() as u32, 1);
        }
        if c == 0 && !self.at_end() {
            self.err(ERR_INVALID_CHAR, "Char 0x0 out of allowed range\n".into());
            return (b' ' as u32, 1);
        }
        (c as u32, 1)
    }

    /// xmlNextChar: past the current character.
    fn next(&mut self) {
        if self.eof || self.cur() == 0 {
            return;
        }
        let c = self.cur();
        self.count_col();
        if !self.charset_utf8 {
            self.pos += 1;
            return;
        }
        if c & 0x80 == 0 {
            self.pos += 1;
            return;
        }
        let bad = c == 0xC0
            || self.nxt(1) & 0xC0 != 0x80
            || (c & 0xE0 == 0xE0 && (self.nxt(2) & 0xC0 != 0x80 || (c & 0xF0 == 0xF0 && (c & 0xF8 != 0xF0 || self.nxt(3) & 0xC0 != 0x80))));
        if bad {
            // The input is not UTF-8 after all; it is read as ISO-8859-1 from here.
            let mut msg = "Input is not proper UTF-8, indicate encoding !\n".to_string();
            if self.buf.len() - self.pos >= 4 {
                msg.push_str(&format!("Bytes: 0x{:02X} 0x{:02X} 0x{:02X} 0x{:02X}\n", self.nxt(0), self.nxt(1), self.nxt(2), self.nxt(3)));
            }
            self.err_level(1, 3, ERR_INVALID_CHAR, msg);
            if !self.recovery {
                self.disable_sax = true;
            }
            self.charset_utf8 = false;
            self.pos += 1;
            return;
        }
        self.pos += if c & 0xE0 != 0xE0 {
            2
        } else if c & 0xF0 != 0xF0 {
            3
        } else {
            4
        };
    }

    // ---- names ----

    /// htmlParseHTMLName: an element or attribute name, lower-cased.
    fn parse_html_name(&mut self) -> Option<Vec<u8>> {
        let c = self.cur();
        if !c.is_ascii_alphabetic() && c != b'_' && c != b':' && c != b'.' {
            return None;
        }
        let mut name = Vec::new();
        while name.len() < 100 {
            let c = self.cur();
            if c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_' | b'.') {
                name.push(c.to_ascii_lowercase());
                self.next();
            } else {
                break;
            }
        }
        Some(name)
    }

    /// htmlParseHTMLName_nonInvasive: the name of the tag after the `<`,
    /// without moving.
    fn peek_html_name(&self) -> Option<Vec<u8>> {
        let c = self.nxt(1);
        if !c.is_ascii_alphabetic() && c != b'_' && c != b':' {
            return None;
        }
        let mut name = Vec::new();
        let mut i = 0;
        while i < 100 {
            let c = self.nxt(1 + i);
            if c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_') {
                name.push(c.to_ascii_lowercase());
                i += 1;
            } else {
                break;
            }
        }
        Some(name)
    }

    /// htmlParseName: an XML name (entity references, the DOCTYPE and PI
    /// names).
    fn parse_name(&mut self) -> Option<Vec<u8>> {
        let first = self.cur();
        if first.is_ascii_alphabetic() || first == b'_' || first == b':' {
            let mut i = self.pos + 1;
            while i < self.buf.len() && (self.buf[i].is_ascii_alphanumeric() || matches!(self.buf[i], b'_' | b'-' | b':' | b'.')) {
                i += 1;
            }
            if i >= self.buf.len() {
                return None;
            }
            if self.buf[i] > 0 && self.buf[i] < 0x80 {
                let name = self.buf[self.pos..i].to_vec();
                self.col += (i - self.pos) as u32;
                self.pos = i;
                return Some(name);
            }
        }
        self.parse_name_complex()
    }

    fn parse_name_complex(&mut self) -> Option<Vec<u8>> {
        let start = self.pos;
        let generation = self.generation;
        let (c, mut l) = self.cur_char();
        let first = char::from_u32(c)?;
        if c == b' ' as u32 || c == b'>' as u32 || c == b'/' as u32 || !(first.is_alphabetic() || first == '_' || first == ':') {
            return None;
        }
        let mut cc = c;
        loop {
            let ch = char::from_u32(cc);
            let ok = cc != b' ' as u32
                && cc != b'>' as u32
                && cc != b'/' as u32
                && ch.is_some_and(|ch| is_name_char_wide(ch) || matches!(ch, '.' | '-' | '_' | ':'));
            if !ok {
                break;
            }
            self.nextl(l);
            let (c2, l2) = self.cur_char();
            cc = c2;
            l = l2;
            if self.generation != generation {
                // The input was converted under us: start again from here.
                return self.parse_name_complex();
            }
            if l == 0 {
                break;
            }
        }
        Some(self.buf[start..self.pos].to_vec())
    }

    // ---- the element stack ----

    fn name(&self) -> Option<&[u8]> {
        self.names.last().map(Vec::as_slice)
    }

    fn name_push(&mut self, name: &[u8]) {
        if self.html < 3 && name == b"head" {
            self.html = 3;
        }
        if self.html < 10 && name == b"body" {
            self.html = 10;
        }
        self.names.push(name.to_vec());
    }

    /// The SAX endElement and the stack pop that always go together.
    fn end_element(&mut self) {
        self.nodes.pop();
        self.names.pop();
    }

    // ---- tree building (SAX2) ----

    fn line_u16(&mut self) -> u32 {
        self.line.min(65535)
    }

    /// xmlSAX2StartElement for an HTML document.
    fn sax_start_element(&mut self, full: &[u8], atts: &[Att]) {
        let parent = self.nodes.last().copied();
        let mut node = Node::new(Kind::Element, local_name(full));
        node.line = self.line_u16();
        let id = self.doc.add(node);
        if self.doc.node(DOC).children.is_empty() {
            self.doc.insert(DOC, id, None);
        } else {
            let parent = parent.unwrap_or(self.doc.node(DOC).children[0]);
            if self.doc.node(parent).kind == Kind::Element {
                self.doc.insert(parent, id, None);
            } else {
                // xmlAddSibling: the end of the parent's sibling list.
                let up = self.doc.parent(parent).unwrap_or(DOC);
                self.doc.insert(up, id, None);
            }
        }
        self.nodes.push(id);
        for (name, value) in atts {
            let mut attr = Node::new(Kind::Attribute, name.clone());
            match value {
                Some(v) => attr.value = v.clone(),
                None if is_boolean_attr(name) => attr.value = name.clone(),
                None => attr.no_value = true,
            }
            let aid = self.doc.add(attr);
            self.doc.set_attr(id, aid);
            // xmlIsID, then xmlAddID: a second ID with a value is an error.
            let local = self.doc.node(id).name.clone();
            if (name == b"id" || (name == b"name" && local == b"a"))
                && let Some(v) = value
                && !self.ids.insert(v.clone())
            {
                self.err_in(23, 513, format!("ID {} already defined\n", String::from_utf8_lossy(v)));
            }
        }
    }

    /// xmlSAX2Characters / xmlSAX2CDataBlock.
    fn sax_text(&mut self, text: &[u8], kind: Kind) {
        let Some(&node) = self.nodes.last() else { return };
        if let Some(&last) = self.doc.node(node).children.last()
            && self.doc.node(last).kind == kind
        {
            self.doc.node_mut(last).value.extend_from_slice(text);
            return;
        }
        let mut n = Node::new(kind, Vec::new());
        n.value = text.to_vec();
        let id = self.doc.add(n);
        self.doc.insert(node, id, None);
    }

    fn characters(&mut self, text: &[u8]) {
        self.sax_text(text, Kind::Text);
    }

    /// Where a comment or PI goes: the current element, else the document.
    fn add_misc(&mut self, mut node: Node) {
        node.line = self.line_u16();
        let id = self.doc.add(node);
        match self.nodes.last().copied() {
            None => self.doc.insert(DOC, id, None),
            Some(p) => self.doc.insert(p, id, None),
        }
    }

    fn int_subset(&mut self, name: Option<Vec<u8>>, external_id: Option<Vec<u8>>, system_id: Option<Vec<u8>>) {
        if self.doc.node(DOC).children.iter().any(|&c| self.doc.node(c).kind == Kind::DocType) {
            return;
        }
        let mut node = Node::new(Kind::DocType, name.unwrap_or_default());
        node.public_id = external_id;
        node.system_id = system_id;
        let id = self.doc.add(node);
        // xmlCreateIntSubset puts the DTD first in an HTML document.
        let first = self.doc.node(DOC).children.first().copied();
        self.doc.insert(DOC, id, first);
    }

    // ---- implied elements and auto-closing ----

    /// htmlAutoCloseOnClose
    fn auto_close_on_close(&mut self, newtag: &[u8]) {
        let priority = end_priority(newtag);
        let mut i = self.names.len();
        let mut found = false;
        while i > 0 {
            i -= 1;
            if self.names[i] == newtag {
                found = true;
                break;
            }
            if end_priority(&self.names[i]) > priority {
                return;
            }
        }
        if !found {
            return;
        }
        while self.name().is_some_and(|n| n != newtag) {
            let top = self.names.last().cloned().unwrap_or_default();
            if tag_lookup(&top).is_some_and(|i| i.end_tag == 3) {
                self.err(
                    ERR_TAG_NAME_MISMATCH,
                    format!("Opening and ending tag mismatch: {} and {}\n", String::from_utf8_lossy(newtag), String::from_utf8_lossy(&top)),
                );
            }
            self.end_element();
        }
    }

    fn auto_close_on_end(&mut self) {
        while !self.names.is_empty() {
            self.end_element();
        }
    }

    /// htmlAutoClose
    fn auto_close(&mut self, newtag: Option<&[u8]>) {
        while let (Some(new), Some(top)) = (newtag, self.name()) {
            if !check_auto_close(new, top) {
                break;
            }
            self.end_element();
        }
        if newtag.is_none() {
            self.auto_close_on_end();
        }
    }

    /// htmlCheckImplied
    fn check_implied(&mut self, newtag: &[u8]) {
        if self.options & PARSE_NOIMPLIED != 0 || newtag == b"html" {
            return;
        }
        if self.names.is_empty() {
            self.name_push(b"html");
            self.sax_start_element(b"html", &[]);
        }
        if newtag == b"body" || newtag == b"head" {
            return;
        }
        if self.names.len() <= 1 && matches!(newtag, b"script" | b"style" | b"meta" | b"link" | b"title" | b"base") {
            if self.html >= 3 {
                return;
            }
            self.name_push(b"head");
            self.sax_start_element(b"head", &[]);
        } else if !matches!(newtag, b"noframes" | b"frame" | b"frameset") {
            if self.html >= 10 {
                return;
            }
            if self.names.iter().any(|n| n == b"body" || n == b"head") {
                return;
            }
            self.name_push(b"body");
            self.sax_start_element(b"body", &[]);
        }
    }

    /// htmlCheckParagraph: character data where none is allowed gets a p.
    fn check_paragraph(&mut self) {
        let needs = match self.name() {
            None => true,
            Some(t) => t == b"html" || t == b"head",
        };
        if needs {
            self.auto_close(Some(b"p"));
            self.check_implied(b"p");
            self.name_push(b"p");
            self.sax_start_element(b"p", &[]);
        }
    }

    // ---- character data ----

    /// areBlanks: whether a run of blank characters can be ignored.
    fn are_blanks(&self, s: &[u8]) -> bool {
        if !s.iter().all(|&c| is_blank(c)) {
            return false;
        }
        if self.cur() == 0 {
            return true;
        }
        if self.cur() != b'<' {
            return false;
        }
        let Some(name) = self.name() else { return true };
        if name == b"html" || name == b"head" {
            return true;
        }
        if name == b"body"
            && let Some(&dtd) = self.doc.node(DOC).children.iter().find(|&&c| self.doc.node(c).kind == Kind::DocType)
            && let Some(ext) = &self.doc.node(dtd).public_id
            && (ext.eq_ignore_ascii_case(b"-//W3C//DTD HTML 4.01//EN") || ext.eq_ignore_ascii_case(b"-//W3C//DTD HTML 4//EN"))
        {
            return true;
        }
        let Some(&node) = self.nodes.last() else { return false };
        let allows = |n: &[u8]| ALLOW_PCDATA.iter().any(|a| a.as_bytes() == n);
        let mut last = self.doc.node(node).children.last().copied();
        while let Some(l) = last
            && self.doc.node(l).kind == Kind::Comment
        {
            let p = self.doc.parent(l);
            last = p.and_then(|p| {
                let kids = &self.doc.node(p).children;
                let at = kids.iter().position(|&k| k == l)?;
                at.checked_sub(1).map(|j| kids[j])
            });
        }
        match last {
            None => !allows(name),
            Some(l) if self.doc.node(l).kind == Kind::Text => false,
            Some(l) => !allows(&self.doc.node(l).name),
        }
    }

    fn flush_chars(&mut self, buf: &[u8]) {
        if self.disable_sax {
            return;
        }
        if self.are_blanks(buf) {
            if self.keep_blanks {
                self.characters(buf);
            }
        } else {
            self.check_paragraph();
            self.characters(buf);
        }
    }

    /// htmlParseCharData
    fn parse_char_data(&mut self) {
        let mut buf = Vec::new();
        let (mut c, mut l) = self.cur_char();
        while c != b'<' as u32 && c != b'&' as u32 && c != 0 {
            if !is_char(c) {
                self.err(ERR_INVALID_CHAR, format!("Invalid char in CDATA 0x{c:X}\n"));
            } else {
                copy_buf(&mut buf, l, c);
            }
            self.nextl(l);
            (c, l) = self.cur_char();
        }
        if !buf.is_empty() {
            self.flush_chars(&buf);
        } else if c == 0 {
            self.eof = true;
        }
    }

    /// htmlParseScript: the raw text of a script or style element.
    fn parse_script(&mut self) {
        let mut buf = Vec::new();
        let (mut c, mut l) = self.cur_char();
        while c != 0 {
            if c == b'<' as u32 && self.nxt(1) == b'/' {
                if self.recovery {
                    let name = self.name().unwrap_or_default().to_vec();
                    let rest = &self.buf[(self.pos + 2).min(self.buf.len())..];
                    if rest.len() >= name.len() && rest[..name.len()].eq_ignore_ascii_case(&name) {
                        break;
                    }
                    self.err(ERR_TAG_NAME_MISMATCH, format!("Element {} embeds close tag\n", String::from_utf8_lossy(&name)));
                } else if self.nxt(2).is_ascii_alphabetic() {
                    break;
                }
            }
            if is_char(c) {
                copy_buf(&mut buf, l, c);
            } else {
                self.err(ERR_INVALID_CHAR, format!("Invalid char in CDATA 0x{c:X}\n"));
            }
            self.nextl(l);
            (c, l) = self.cur_char();
        }
        if !buf.is_empty() && !self.disable_sax {
            self.sax_text(&buf, Kind::CData);
        }
    }

    // ---- references ----

    /// htmlParseCharRef: the code point of `&#N;` (0 when it is invalid).
    fn parse_char_ref(&mut self) -> u32 {
        let mut val: u32 = 0;
        if self.cur() == b'&' && self.nxt(1) == b'#' && matches!(self.nxt(2), b'x' | b'X') {
            self.skip(3);
            while self.cur() != b';' {
                let c = self.cur();
                let d = match c {
                    b'0'..=b'9' => c - b'0',
                    b'a'..=b'f' => c - b'a' + 10,
                    b'A'..=b'F' => c - b'A' + 10,
                    _ => {
                        self.err(ERR_INVALID_HEX_CHARREF, "htmlParseCharRef: missing semicolon\n".into());
                        break;
                    }
                };
                if val < 0x110000 {
                    val = val * 16 + d as u32;
                }
                self.next();
            }
            if self.cur() == b';' {
                self.next();
            }
        } else if self.cur() == b'&' && self.nxt(1) == b'#' {
            self.skip(2);
            while self.cur() != b';' {
                let c = self.cur();
                if c.is_ascii_digit() {
                    if val < 0x110000 {
                        val = val * 10 + (c - b'0') as u32;
                    }
                } else {
                    self.err(ERR_INVALID_DEC_CHARREF, "htmlParseCharRef: missing semicolon\n".into());
                    break;
                }
                self.next();
            }
            if self.cur() == b';' {
                self.next();
            }
        } else {
            self.err(ERR_INVALID_CHARREF, "htmlParseCharRef: invalid value\n".into());
        }
        if is_char(val) {
            val
        } else {
            if val >= 0x110000 {
                self.err(ERR_INVALID_CHAR, "htmlParseCharRef: value too large\n".into());
            } else {
                self.err(ERR_INVALID_CHAR, format!("htmlParseCharRef: invalid xmlChar value {val}\n"));
            }
            0
        }
    }

    /// htmlParseEntityRef: the entity's code point (when `name;` is a known
    /// entity), and the name read.
    fn parse_entity_ref(&mut self) -> (Option<u32>, Option<Vec<u8>>) {
        if self.cur() != b'&' {
            return (None, None);
        }
        self.next();
        let Some(name) = self.parse_name() else {
            self.err(ERR_NAME_REQUIRED, "htmlParseEntityRef: no name\n".into());
            return (None, None);
        };
        if self.cur() == b';' {
            let ent = entity_lookup(&name);
            if ent.is_some() {
                self.next();
            }
            (ent, Some(name))
        } else {
            self.err(ERR_ENTITYREF_SEMICOL_MISSING, "htmlParseEntityRef: expecting ';'\n".into());
            (None, Some(name))
        }
    }

    /// htmlParseReference: `&...;` in content.
    fn parse_reference(&mut self) {
        if self.cur() != b'&' {
            return;
        }
        if self.nxt(1) == b'#' {
            let c = self.parse_char_ref();
            if c == 0 {
                return;
            }
            let mut out = Vec::new();
            encode_cp(&mut out, c);
            self.check_paragraph();
            self.characters(&out);
        } else {
            let (ent, name) = self.parse_entity_ref();
            let Some(name) = name else {
                self.check_paragraph();
                self.characters(b"&");
                return;
            };
            match ent {
                Some(v) if v > 0 => {
                    let mut out = Vec::new();
                    encode_cp(&mut out, v);
                    self.check_paragraph();
                    self.characters(&out);
                }
                _ => {
                    self.check_paragraph();
                    self.characters(b"&");
                    self.characters(&name);
                }
            }
        }
    }

    // ---- comments, PIs, DOCTYPE ----

    /// htmlParseComment
    fn parse_comment(&mut self) {
        if !(self.cur() == b'<' && self.nxt(1) == b'!' && self.nxt(2) == b'-' && self.nxt(3) == b'-') {
            return;
        }
        self.skip(4);
        let mut buf = Vec::new();
        'body: {
            let (q0, ql0) = self.cur_char();
            if q0 == 0 {
                break 'body;
            }
            self.nextl(ql0);
            let (r0, rl0) = self.cur_char();
            if r0 == 0 {
                break 'body;
            }
            self.nextl(rl0);
            let (mut q, mut ql, mut r, mut rl) = (q0, ql0, r0, rl0);
            let (mut cur, mut l) = self.cur_char();
            while cur != 0 && (cur != b'>' as u32 || r != b'-' as u32 || q != b'-' as u32) {
                self.nextl(l);
                let (next, nl) = self.cur_char();
                if q == b'-' as u32 && r == b'-' as u32 && cur == b'!' as u32 && next == b'>' as u32 {
                    self.err(ERR_COMMENT_NOT_FINISHED, "Comment incorrectly closed by '--!>'".into());
                    cur = b'>' as u32;
                    break;
                }
                if is_char(q) {
                    copy_buf(&mut buf, ql, q);
                } else {
                    self.err(ERR_INVALID_CHAR, format!("Invalid char in comment 0x{q:X}\n"));
                }
                q = r;
                ql = rl;
                r = cur;
                rl = l;
                cur = next;
                l = nl;
            }
            if cur == b'>' as u32 {
                self.next();
                if !self.disable_sax {
                    let mut node = Node::new(Kind::Comment, Vec::new());
                    node.value = buf;
                    self.add_misc(node);
                }
                return;
            }
        }
        let shown = String::from_utf8_lossy(&buf[..buf.len().min(50)]).into_owned();
        self.err(ERR_COMMENT_NOT_FINISHED, format!("Comment not terminated \n<!--{shown}\n"));
    }

    /// htmlParsePI
    fn parse_pi(&mut self) {
        if !(self.cur() == b'<' && self.nxt(1) == b'?') {
            return;
        }
        self.skip(2);
        let Some(target) = self.parse_name() else {
            self.err(ERR_PI_NOT_STARTED, "PI is not started correctly".into());
            return;
        };
        let target_s = String::from_utf8_lossy(&target).into_owned();
        if self.cur() == b'>' {
            self.skip(1);
            if !self.disable_sax {
                let mut node = Node::new(Kind::Pi, target);
                node.no_value = true;
                self.add_misc(node);
            }
            return;
        }
        if !is_blank(self.cur()) {
            self.err(ERR_SPACE_REQUIRED, format!("ParsePI: PI {target_s} space expected\n"));
        }
        self.skip_blanks();
        let mut buf = Vec::new();
        let (mut cur, mut l) = self.cur_char();
        while cur != 0 && cur != b'>' as u32 {
            if is_char(cur) {
                copy_buf(&mut buf, l, cur);
            } else {
                self.err(ERR_INVALID_CHAR, format!("Invalid char in processing instruction 0x{cur:X}\n"));
            }
            self.nextl(l);
            (cur, l) = self.cur_char();
        }
        if cur != b'>' as u32 {
            self.err(ERR_PI_NOT_FINISHED, format!("ParsePI: PI {target_s} never end ...\n"));
        } else {
            self.skip(1);
            if !self.disable_sax {
                let mut node = Node::new(Kind::Pi, target);
                node.value = buf;
                self.add_misc(node);
            }
        }
    }

    fn parse_system_literal(&mut self) -> Option<Vec<u8>> {
        let quote = self.cur();
        if quote != b'"' && quote != b'\'' {
            self.err(ERR_LITERAL_NOT_STARTED, "SystemLiteral \" or ' expected\n".into());
            return None;
        }
        self.next();
        let start = self.pos;
        let mut err = false;
        while self.cur() != 0 && self.cur() != quote {
            let c = self.cur();
            if !is_char(c as u32) {
                self.err(ERR_INVALID_CHAR, format!("Invalid char in SystemLiteral 0x{c:X}\n"));
                err = true;
            }
            self.next();
        }
        if self.cur() != quote {
            self.err(ERR_LITERAL_NOT_FINISHED, "Unfinished SystemLiteral\n".into());
            None
        } else {
            let v = self.buf[start..self.pos].to_vec();
            self.next();
            if err { None } else { Some(v) }
        }
    }

    fn parse_pubid_literal(&mut self) -> Option<Vec<u8>> {
        let quote = self.cur();
        if quote != b'"' && quote != b'\'' {
            self.err(ERR_LITERAL_NOT_STARTED, "PubidLiteral \" or ' expected\n".into());
            return None;
        }
        self.next();
        let start = self.pos;
        let mut err = false;
        while self.cur() != 0 && self.cur() != quote {
            let c = self.cur();
            if !is_pubid_char(c) {
                self.err(ERR_INVALID_CHAR, format!("Invalid char in PubidLiteral 0x{c:X}\n"));
                err = true;
            }
            self.next();
        }
        if self.cur() != quote {
            self.err(ERR_LITERAL_NOT_FINISHED, "Unfinished PubidLiteral\n".into());
            None
        } else {
            let v = self.buf[start..self.pos].to_vec();
            self.next();
            if err { None } else { Some(v) }
        }
    }

    /// htmlParseExternalID: (system id, public id).
    fn parse_external_id(&mut self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let word = |s: &Self, w: &[u8]| (0..6).all(|i| s.upp(i) == w[i]);
        let mut uri = None;
        let mut public = None;
        if word(self, b"SYSTEM") {
            self.skip(6);
            if !is_blank(self.cur()) {
                self.err(ERR_SPACE_REQUIRED, "Space required after 'SYSTEM'\n".into());
            }
            self.skip_blanks();
            uri = self.parse_system_literal();
            if uri.is_none() {
                self.err(ERR_URI_REQUIRED, "htmlParseExternalID: SYSTEM, no URI\n".into());
            }
        } else if word(self, b"PUBLIC") {
            self.skip(6);
            if !is_blank(self.cur()) {
                self.err(ERR_SPACE_REQUIRED, "Space required after 'PUBLIC'\n".into());
            }
            self.skip_blanks();
            public = self.parse_pubid_literal();
            if public.is_none() {
                self.err(ERR_PUBID_REQUIRED, "htmlParseExternalID: PUBLIC, no Public Identifier\n".into());
            }
            self.skip_blanks();
            if self.cur() == b'"' || self.cur() == b'\'' {
                uri = self.parse_system_literal();
            }
        }
        (uri, public)
    }

    /// htmlParseDocTypeDecl
    fn parse_doctype(&mut self) {
        self.skip(9);
        self.skip_blanks();
        let name = self.parse_name();
        if name.is_none() {
            self.err(ERR_NAME_REQUIRED, "htmlParseDocTypeDecl : no DOCTYPE name !\n".into());
        }
        self.skip_blanks();
        let (uri, public) = self.parse_external_id();
        self.skip_blanks();
        if self.cur() != b'>' {
            self.err(ERR_DOCTYPE_NOT_FINISHED, "DOCTYPE improperly terminated\n".into());
            while self.cur() != 0 && self.cur() != b'>' {
                self.next();
            }
        }
        if self.cur() == b'>' {
            self.next();
        }
        if !self.disable_sax {
            self.int_subset(name, public, uri);
        }
    }

    fn at_doctype(&self) -> bool {
        self.cur() == b'<' && self.nxt(1) == b'!' && b"DOCTYPE".iter().enumerate().all(|(i, &c)| self.upp(2 + i) == c)
    }

    fn at_comment(&self) -> bool {
        self.cur() == b'<' && self.nxt(1) == b'!' && self.nxt(2) == b'-' && self.nxt(3) == b'-'
    }

    // ---- attributes and tags ----

    /// htmlParseHTMLAttribute: an attribute value up to `stop` (0: a blank
    /// or `>`), references resolved.
    fn parse_html_attribute(&mut self, stop: u8) -> Vec<u8> {
        let mut out = Vec::new();
        while self.cur() != 0 && self.cur() != stop {
            if stop == 0 && (self.cur() == b'>' || is_blank(self.cur())) {
                break;
            }
            if self.cur() == b'&' {
                if self.nxt(1) == b'#' {
                    let c = self.parse_char_ref();
                    encode_cp(&mut out, c);
                } else {
                    let (ent, name) = self.parse_entity_ref();
                    match (name, ent) {
                        (None, _) => out.push(b'&'),
                        (Some(n), None) => {
                            out.push(b'&');
                            out.extend_from_slice(&n);
                        }
                        (Some(_), Some(v)) => encode_cp(&mut out, v),
                    }
                }
            } else {
                let (c, _) = self.cur_char();
                encode_cp(&mut out, c);
                self.next();
            }
        }
        // C strings end at the first NUL.
        if let Some(z) = out.iter().position(|&b| b == 0) {
            out.truncate(z);
        }
        out
    }

    /// htmlParseAttValue
    fn parse_att_value(&mut self) -> Vec<u8> {
        let q = self.cur();
        if q == b'"' || q == b'\'' {
            self.next();
            let v = self.parse_html_attribute(q);
            if self.cur() != q {
                self.err(ERR_ATTRIBUTE_NOT_FINISHED, format!("AttValue: {} expected\n", q as char));
            } else {
                self.next();
            }
            v
        } else {
            self.parse_html_attribute(0)
        }
    }

    /// htmlParseAttribute
    fn parse_attribute(&mut self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let Some(name) = self.parse_html_name() else {
            self.err(ERR_NAME_REQUIRED, "error parsing attribute name\n".into());
            return (None, None);
        };
        self.skip_blanks();
        let mut val = None;
        if self.cur() == b'=' {
            self.next();
            self.skip_blanks();
            val = Some(self.parse_att_value());
        }
        (Some(name), val)
    }

    /// htmlCheckEncodingDirect: switch to the encoding a meta element names.
    fn check_encoding_direct(&mut self, encoding: &[u8]) {
        if self.options & PARSE_IGNORE_ENC != 0 || self.input_encoding.is_some() {
            return;
        }
        let start = encoding.iter().position(|&c| c != b' ' && c != b'\t').unwrap_or(encoding.len());
        let encoding = &encoding[start..];
        self.input_encoding = Some(encoding.to_vec());
        let upper = encoding.to_ascii_uppercase();
        match upper.as_slice() {
            b"UTF-8" | b"UTF8" | b"ASCII" | b"US-ASCII" => self.charset_utf8 = true,
            b"UTF-16" | b"UTF16" | b"UTF-16LE" | b"UTF-16BE" | b"UTF16LE" | b"UTF16BE" | b"ISO-10646-UCS-2" | b"UCS-2" | b"UCS2" | b"ISO-10646-UCS-4" | b"UCS-4" | b"UCS4" => {
                if !self.converted {
                    self.err(ERR_INVALID_ENCODING, "htmlCheckEncoding: wrong encoding meta\n".into());
                }
                self.charset_utf8 = true;
            }
            _ => {
                let alt = trim_junk(&upper);
                if alt == b"UTF-8" || alt == b"UTF8" || self.convert_tail(encoding) {
                    self.charset_utf8 = true;
                } else {
                    self.err(ERR_UNSUPPORTED_ENCODING, format!("htmlCheckEncoding: unknown encoding {}\n", String::from_utf8_lossy(encoding)));
                }
            }
        }
    }

    /// htmlCheckEncoding: the charset in a Content-Type value.
    fn check_encoding(&mut self, value: &[u8]) {
        let Some(at) = find_ci(value, b"charset") else { return };
        let mut enc = &value[at + 7..];
        if enc.first().is_some_and(|&c| is_blank(c)) {
            match value.iter().position(|&c| c == b'=') {
                Some(eq) => enc = &value[eq..],
                None => return,
            }
        }
        if enc.first() == Some(&b'=') {
            self.check_encoding_direct(&enc[1..]);
        }
    }

    /// htmlCheckMeta
    fn check_meta(&mut self, atts: &[Att]) {
        let mut http = false;
        let mut content: Option<&[u8]> = None;
        let mut direct: Vec<Vec<u8>> = Vec::new();
        for (att, value) in atts {
            if let Some(value) = value {
                if att.eq_ignore_ascii_case(b"http-equiv") && value.eq_ignore_ascii_case(b"Content-Type") {
                    http = true;
                } else if att.eq_ignore_ascii_case(b"charset") {
                    direct.push(value.clone());
                } else if att.eq_ignore_ascii_case(b"content") {
                    content = Some(value);
                }
            }
        }
        for d in direct {
            self.check_encoding_direct(&d);
        }
        if http && let Some(c) = content {
            let c = c.to_vec();
            self.check_encoding(&c);
        }
    }

    /// htmlParseStartTag: -1 for no tag, 1 for one discarded, else 0.
    fn parse_start_tag(&mut self) -> i32 {
        if self.eof || self.cur() != b'<' {
            return -1;
        }
        self.next();
        let Some(name) = self.parse_html_name() else {
            self.err(ERR_NAME_REQUIRED, "htmlParseStartTag: invalid element name\n".into());
            while self.cur() != 0 && self.cur() != b'>' && !self.eof {
                self.next();
            }
            return -1;
        };
        let meta = name == b"meta";
        self.auto_close(Some(&name));
        self.check_implied(&name);
        let mut discard = false;
        if !self.names.is_empty() && name == b"html" {
            self.err(ERR_HTML_STRUCTURE, "htmlParseStartTag: misplaced <html> tag\n".into());
            discard = true;
            self.depth += 1;
        }
        if self.names.len() != 1 && name == b"head" {
            self.err(ERR_HTML_STRUCTURE, "htmlParseStartTag: misplaced <head> tag\n".into());
            discard = true;
            self.depth += 1;
        }
        if name == b"body" {
            for i in 0..self.names.len() {
                if self.names[i] == b"body" {
                    self.err(ERR_HTML_STRUCTURE, "htmlParseStartTag: misplaced <body> tag\n".into());
                    discard = true;
                    self.depth += 1;
                }
            }
        }
        self.skip_blanks();
        let mut atts: Vec<Att> = Vec::new();
        while self.cur() != 0 && self.cur() != b'>' && (self.cur() != b'/' || self.nxt(1) != b'>') {
            let (attname, attvalue) = self.parse_attribute();
            if let Some(attname) = attname {
                if atts.iter().any(|(n, _)| *n == attname) {
                    self.err(ERR_ATTRIBUTE_REDEFINED, format!("Attribute {} redefined\n", String::from_utf8_lossy(&attname)));
                } else {
                    atts.push((attname, attvalue));
                }
            } else {
                while self.cur() != 0 && !is_blank(self.cur()) && self.cur() != b'>' && (self.cur() != b'/' || self.nxt(1) != b'>') {
                    self.next();
                }
            }
            self.skip_blanks();
        }
        if meta && !atts.is_empty() {
            self.check_meta(&atts);
        }
        if !discard {
            self.name_push(&name);
            self.sax_start_element(&name, &atts);
        }
        i32::from(discard)
    }

    /// htmlParseEndTag: whether it closed an element.
    fn parse_end_tag(&mut self) -> bool {
        if self.cur() != b'<' || self.nxt(1) != b'/' {
            self.err(ERR_LTSLASH_REQUIRED, "htmlParseEndTag: '</' not found\n".into());
            return false;
        }
        self.skip(2);
        let Some(name) = self.parse_html_name() else { return false };
        self.skip_blanks();
        if self.cur() != b'>' {
            self.err(ERR_GT_REQUIRED, "End tag : expected '>'\n".into());
            while self.cur() != 0 && self.cur() != b'>' {
                self.next();
            }
        }
        if self.cur() == b'>' {
            self.next();
        }
        if self.depth > 0 && (name == b"html" || name == b"body" || name == b"head") {
            self.depth -= 1;
            return false;
        }
        if !self.names.contains(&name) {
            self.err(ERR_TAG_NAME_MISMATCH, format!("Unexpected end tag : {}\n", String::from_utf8_lossy(&name)));
            return false;
        }
        self.auto_close_on_close(&name);
        if let Some(top) = self.name()
            && top != name.as_slice()
        {
            let top = String::from_utf8_lossy(top).into_owned();
            self.err(ERR_TAG_NAME_MISMATCH, format!("Opening and ending tag mismatch: {} and {}\n", String::from_utf8_lossy(&name), top));
        }
        if self.name() == Some(name.as_slice()) {
            self.end_element();
            true
        } else {
            false
        }
    }

    // ---- elements and content ----

    fn finish_element_parsing(&mut self) {
        if self.cur() == 0 {
            self.auto_close_on_end();
        }
    }

    /// htmlParseElementInternal
    fn parse_element(&mut self) {
        if self.eof {
            return;
        }
        let failed = self.parse_start_tag();
        let Some(name) = self.name().map(<[u8]>::to_vec) else {
            if self.cur() == b'>' {
                self.next();
            }
            return;
        };
        if failed == -1 {
            if self.cur() == b'>' {
                self.next();
            }
            return;
        }
        let info = tag_lookup(&name);
        if info.is_none() {
            self.err(ERR_HTML_UNKNOWN_TAG, format!("Tag {} invalid\n", String::from_utf8_lossy(&name)));
        }
        if self.cur() == b'/' && self.nxt(1) == b'>' {
            self.skip(2);
            self.end_element();
            return;
        }
        if self.cur() == b'>' {
            self.next();
        } else {
            self.err(ERR_GT_REQUIRED, format!("Couldn't find end of Start Tag {}\n", String::from_utf8_lossy(&name)));
            if self.name() == Some(name.as_slice()) {
                self.end_element();
            }
            self.finish_element_parsing();
            return;
        }
        if info.is_some_and(|i| i.empty) {
            self.end_element();
        }
    }

    /// htmlParseContentInternal
    fn parse_content(&mut self) {
        let mut current: Option<Vec<u8>> = self.name().map(<[u8]>::to_vec);
        let mut depth = self.names.len();
        loop {
            if self.eof {
                break;
            }
            if self.cur() == b'<' && self.nxt(1) == b'/' {
                if self.parse_end_tag() && (current.is_some() || self.names.is_empty()) {
                    current = self.name().map(<[u8]>::to_vec);
                    depth = self.names.len();
                }
                continue;
            } else if self.cur() == b'<' && (self.nxt(1).is_ascii_alphabetic() || self.nxt(1) == b'_' || self.nxt(1) == b':') {
                let Some(name) = self.peek_html_name() else {
                    self.err(ERR_NAME_REQUIRED, "htmlParseStartTag: invalid element name\n".into());
                    self.finish_element_parsing();
                    current = self.name().map(<[u8]>::to_vec);
                    depth = self.names.len();
                    continue;
                };
                if let Some(top) = self.name()
                    && check_auto_close(&name, top)
                {
                    self.auto_close(Some(&name));
                    continue;
                }
            }
            if !self.names.is_empty() && depth >= self.names.len() && current.as_deref() != self.name() {
                self.finish_element_parsing();
                current = self.name().map(<[u8]>::to_vec);
                depth = self.names.len();
                continue;
            }
            if self.cur() != 0 && matches!(current.as_deref(), Some(b"script" | b"style")) {
                self.parse_script();
            } else {
                if self.at_doctype() {
                    self.err(ERR_HTML_STRUCTURE, "Misplaced DOCTYPE declaration\n".into());
                    self.parse_doctype();
                }
                if self.at_comment() {
                    self.parse_comment();
                } else if self.cur() == b'<' && self.nxt(1) == b'?' {
                    self.parse_pi();
                } else if self.cur() == b'<' && self.nxt(1).is_ascii_alphabetic() {
                    self.parse_element();
                    current = self.name().map(<[u8]>::to_vec);
                    depth = self.names.len();
                } else if self.cur() == b'<' {
                    if !self.disable_sax {
                        self.characters(b"<");
                    }
                    self.next();
                } else if self.cur() == b'&' {
                    self.parse_reference();
                } else if self.cur() == 0 {
                    self.auto_close_on_end();
                    break;
                } else {
                    self.parse_char_data();
                }
            }
        }
    }

    /// htmlParseDocument
    fn parse_document(&mut self) {
        self.html = 1;
        if self.buf.len() >= 4 {
            let b = &self.buf;
            // xmlDetectCharEncoding on the first four bytes.
            let utf16le = (b[0] == 0x3C && b[1] == 0 && b[2] == 0x3F && b[3] == 0) || (b[0] == 0xFF && b[1] == 0xFE);
            let utf16be = (b[0] == 0 && b[1] == 0x3C && b[2] == 0 && b[3] == 0x3F) || (b[0] == 0xFE && b[1] == 0xFF);
            if b[0] == 0xEF && b[1] == 0xBB && b[2] == 0xBF {
                // A UTF-8 byte order mark is skipped.
                self.charset_utf8 = true;
                self.pos = 3;
            } else if b[0] == 0x3C && b[1] == 0x3F && b[2] == 0x78 && b[3] == 0x6D {
                self.charset_utf8 = true;
            } else if utf16le || utf16be {
                let bom = b[0] == 0xFF || b[0] == 0xFE;
                self.pos = if bom { 2 } else { 0 };
                self.convert_tail(if utf16le { b"UTF-16LE" } else { b"UTF-16BE" });
                self.charset_utf8 = true;
            }
        }
        self.skip_blanks();
        if self.cur() == 0 {
            self.err(ERR_DOCUMENT_EMPTY, "Document is empty\n".into());
        }
        self.parse_misc();
        if self.at_doctype() {
            self.parse_doctype();
        }
        self.skip_blanks();
        self.parse_misc();
        self.parse_content();
        if self.cur() == 0 {
            self.auto_close_on_end();
        }
        // endDocument
        if let Some(e) = &self.input_encoding {
            self.doc.settings.encoding = Some(e.clone());
        }
        if self.options & PARSE_NODEFDTD == 0 && !self.doc.node(DOC).children.iter().any(|&c| self.doc.node(c).kind == Kind::DocType) {
            self.int_subset(
                Some(b"html".to_vec()),
                Some(b"-//W3C//DTD HTML 4.0 Transitional//EN".to_vec()),
                Some(b"http://www.w3.org/TR/REC-html40/loose.dtd".to_vec()),
            );
        }
        self.doc.html = true;
        self.doc.settings.version = None;
        self.doc.settings.standalone = 1;
    }

    /// Comments and processing instructions before the content.
    fn parse_misc(&mut self) {
        while self.at_comment() || (self.cur() == b'<' && self.nxt(1) == b'?') {
            self.parse_comment();
            self.parse_pi();
            self.skip_blanks();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(html: &[u8], options: i64) -> (Doc, Vec<XmlError>) {
        let mut doc = Doc::new(b"DOMDocument".to_vec());
        let errors = parse(&mut doc, html.to_vec(), None, options);
        (doc, errors)
    }

    fn shape(doc: &Doc, id: Id) -> String {
        let n = doc.node(id);
        match n.kind {
            Kind::Element => {
                let kids: Vec<String> = n.children.iter().map(|&c| shape(doc, c)).collect();
                format!("<{}>{}", String::from_utf8_lossy(&n.name), kids.join(""))
            }
            Kind::Text => String::from_utf8_lossy(&n.value).into_owned(),
            _ => String::new(),
        }
    }

    #[test]
    fn a_fragment_gets_html_and_body() {
        let (doc, errors) = parsed(b"<p>x</p>", 0);
        assert!(errors.is_empty());
        assert_eq!(shape(&doc, doc.root().unwrap()), "<html><body><p>x");
        assert!(doc.html);
        let (doc, _) = parsed(b"<p>x</p>", PARSE_NOIMPLIED | PARSE_NODEFDTD);
        assert_eq!(shape(&doc, doc.root().unwrap()), "<p>x");
        assert_eq!(doc.node(DOC).children.len(), 1);
    }

    #[test]
    fn unknown_tags_are_kept_and_reported() {
        let (doc, errors) = parsed(b"<section>x</section>", 0);
        assert_eq!(shape(&doc, doc.root().unwrap()), "<html><body><section>x");
        assert_eq!((errors[0].code, errors[0].message.as_str(), errors[0].line, errors[0].column), (801, "Tag section invalid\n", 1, 9));
    }

    #[test]
    fn start_tags_close_what_they_cannot_nest_in() {
        let (doc, _) = parsed(b"<ul><li>a<li>b</ul>", PARSE_NOIMPLIED);
        assert_eq!(shape(&doc, doc.root().unwrap()), "<ul><li>a<li>b");
        assert_eq!(doc.node(doc.root().unwrap()).children.len(), 2);
    }

    #[test]
    fn input_that_is_not_utf8_is_read_as_latin1() {
        let (doc, _) = parsed(b"<p>caf\xe9</p>", PARSE_NOIMPLIED);
        assert_eq!(shape(&doc, doc.root().unwrap()), "<p>caf\u{e9}");
        let (doc, _) = parsed(b"<meta charset=utf-8><p>caf\xc3\xa9</p>", PARSE_NOIMPLIED);
        assert_eq!(doc.settings.encoding.as_deref(), Some(&b"utf-8"[..]));
        assert_eq!(doc.text_content(DOC), "caf\u{e9}".as_bytes());
    }

    #[test]
    fn a_stray_end_tag_is_an_error() {
        let (_, errors) = parsed(b"<p>a</b>", PARSE_NOIMPLIED);
        assert_eq!((errors[0].code, errors[0].message.as_str(), errors[0].column), (76, "Unexpected end tag : b\n", 9));
    }
}

//! libxml2's XML parser, ported: the pull parser (xmlParseDocument and
//! the productions it calls) with libxml2's error codes, messages and
//! recovery, so that PHP's xml, dom, simplexml and xmlreader see the
//! events, and print the warnings, that libxml2 2.9.14 gives them.
//! libxml2 is MIT licensed, Copyright (C) Daniel Veillard.
//!
//! The input is UTF-8 (other encodings are converted first, as libxml2's
//! input buffer does); reading past its end gives 0, as libxml2's
//! NUL-terminated buffers do.
//!
//! Reference: libxml2 parser.c, parserInternals.c

use crate::errors::*;
use std::collections::HashMap;

/// Where the parser is (libxml2's xmlParserInputState).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum State {
    Eof,
    Start,
    Misc,
    Pi,
    Dtd,
    Prolog,
    Comment,
    Content,
    CdataSection,
    EntityDecl,
    EntityValue,
    AttributeValue,
    SystemLiteral,
    Epilog,
    PublicLiteral,
}

/// Error levels, as libxml_get_errors() reports them.
pub(crate) const LEVEL_WARNING: i32 = 1;
pub(crate) const LEVEL_ERROR: i32 = 2;
pub(crate) const LEVEL_FATAL: i32 = 3;

/// Error domains (xmlErrorDomain).
pub(crate) const FROM_PARSER: i32 = 1;
pub(crate) const FROM_NAMESPACE: i32 = 3;
pub(crate) const FROM_DTD: i32 = 4;

/// An error libxml2 raised (xmlError).
#[derive(Clone, Debug)]
#[allow(dead_code)] // what libxml_get_errors() and DOM's warnings show; ext/xml reads only the level
pub(crate) struct XmlError {
    pub domain: i32,
    pub code: i32,
    pub level: i32,
    /// With libxml2's trailing newline.
    pub message: String,
    pub line: i32,
    pub column: i32,
    pub file: Option<String>,
    /// The source line around the error and the column of the problem in
    /// it, as xmlParserPrintFileContextInternal prints them (generic
    /// error reports: SimpleXML's warnings).
    pub context: Vec<u8>,
    pub context_col: usize,
}

impl XmlError {
    /// libxml2's generic report of this error (xmlReportError), one entry
    /// per line it prints: `Entity: line 1: parser error : ...`, the
    /// source line, a caret under the problem.
    pub(crate) fn report_lines(&self) -> Vec<String> {
        let file = self.file.as_deref().unwrap_or("Entity");
        let domain = match self.domain {
            FROM_NAMESPACE => "namespace ",
            FROM_DTD => "validity ",
            _ => "parser ",
        };
        let level = if self.level == LEVEL_WARNING { "warning" } else { "error" };
        let place = if self.file.is_some() { format!("{file}:{}:", self.line) } else { format!("{file}: line {}:", self.line) };
        let mut lines = vec![format!("{place} {domain}{level} : {}", self.message.trim_end_matches('\n'))];
        let content = String::from_utf8_lossy(&self.context).into_owned();
        let mut caret: String = content.chars().take(self.context_col).map(|c| if c == '\t' { '\t' } else { ' ' }).collect();
        caret.push('^');
        lines.push(content);
        lines.push(caret);
        lines
    }
}

/// The line around `cur` in `buf` and the column of `cur` in it (at most 80
/// bytes), as xmlParserPrintFileContextInternal takes them.
fn file_context(buf: &[u8], cur: usize) -> (Vec<u8>, usize) {
    let at = |i: usize| buf.get(i).copied().unwrap_or(0);
    let mut c = cur.min(buf.len());
    while c > 0 && matches!(at(c), b'\n' | b'\r') {
        c -= 1;
    }
    let mut n = 0;
    while n < 80 && c > 0 && !matches!(at(c), b'\n' | b'\r') {
        c -= 1;
        n += 1;
    }
    if matches!(at(c), b'\n' | b'\r') {
        c += 1;
    }
    let col = cur.saturating_sub(c);
    let mut content = Vec::new();
    while at(c) != 0 && !matches!(at(c), b'\n' | b'\r') && content.len() < 80 {
        content.push(at(c));
        c += 1;
    }
    let col = col.min(content.len());
    (content, col)
}

/// A namespace binding: (prefix, URI).
pub(crate) type NsBinding = (Option<Vec<u8>>, Vec<u8>);

/// (name, value): an attribute's.
pub(crate) type NameValue = (Vec<u8>, Vec<u8>);

/// A start tag's element: (local name, prefix, namespace URI).
type TagName = (Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>);

/// An attribute of a start tag, as SAX2's startElementNs gets it.
#[derive(Clone, Debug)]
pub(crate) struct Attr {
    pub local: Vec<u8>,
    pub prefix: Option<Vec<u8>>,
    pub uri: Option<Vec<u8>>,
    pub value: Vec<u8>,
    /// Came from an ATTLIST default, not the tag (DOM leaves it out
    /// unless LIBXML_DTDATTR).
    #[allow(dead_code)] // read by the DOM tree builder
    pub defaulted: bool,
}

/// What an entity is (xmlEntityType).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EntityKind {
    InternalGeneral = 1,
    ExternalGeneralParsed = 2,
    ExternalGeneralUnparsed = 3,
    InternalParameter = 4,
    ExternalParameter = 5,
    Predefined = 6,
}

#[derive(Clone, Debug)]
pub(crate) struct Entity {
    pub name: Vec<u8>,
    pub kind: EntityKind,
    pub external_id: Option<Vec<u8>>,
    pub system_id: Option<Vec<u8>>,
    /// The replacement text (internal entities).
    pub content: Option<Vec<u8>>,
    /// The notation (unparsed entities).
    #[allow(dead_code)] // DOMEntity::$notationName reads it
    pub notation: Option<Vec<u8>>,
    /// libxml2's `checked`: 0 unchecked; else 2 * entities referenced, | 1
    /// if the replacement holds markup.
    pub checked: u32,
}

/// The events (libxml2's SAX2 handler block). Every method has a
/// default, so a consumer implements what it wants.
#[allow(unused_variables)]
pub(crate) trait Sax {
    fn start_document(&mut self, p: &mut Parser) {}
    fn end_document(&mut self, p: &mut Parser) {}
    fn internal_subset(&mut self, p: &mut Parser, name: &[u8], external_id: Option<&[u8]>, system_id: Option<&[u8]>) {}
    #[allow(dead_code)] // the pull parser's event (DOM)
    fn external_subset(&mut self, p: &mut Parser, name: &[u8], external_id: Option<&[u8]>, system_id: Option<&[u8]>) {}
    fn entity_decl(&mut self, p: &mut Parser, entity: &Entity) {}
    fn notation_decl(&mut self, p: &mut Parser, name: &[u8], public_id: Option<&[u8]>, system_id: Option<&[u8]>) {}
    fn unparsed_entity_decl(&mut self, p: &mut Parser, name: &[u8], public_id: Option<&[u8]>, system_id: Option<&[u8]>, notation: &[u8]) {}
    fn attribute_decl(&mut self, p: &mut Parser, element: &[u8], attribute: &[u8], kind: i32, default: i32, value: Option<&[u8]>) {}
    fn element_decl(&mut self, p: &mut Parser, name: &[u8], kind: i32) {}
    /// An entity referenced in content: Some to use it (None: not found).
    fn get_entity(&mut self, p: &mut Parser, name: &[u8]) -> Option<Entity> {
        p.entities.get(name).cloned()
    }
    fn start_element_ns(&mut self, p: &mut Parser, local: &[u8], prefix: Option<&[u8]>, uri: Option<&[u8]>, namespaces: &[NsBinding], attrs: &[Attr]) {}
    fn end_element_ns(&mut self, p: &mut Parser, local: &[u8], prefix: Option<&[u8]>, uri: Option<&[u8]>) {}
    fn characters(&mut self, p: &mut Parser, text: &[u8]) {}
    fn ignorable_whitespace(&mut self, p: &mut Parser, text: &[u8]) {
        self.characters(p, text);
    }
    /// A CDATA section (None from a consumer that takes it as characters).
    fn cdata_block(&mut self, p: &mut Parser, text: &[u8]) {
        self.characters(p, text);
    }
    fn processing_instruction(&mut self, p: &mut Parser, target: &[u8], data: Option<&[u8]>) {}
    fn comment(&mut self, p: &mut Parser, text: &[u8]) {}
    /// An entity reference left as it is (replaceEntities off).
    fn reference(&mut self, p: &mut Parser, name: &[u8]) {}
}

/// One input on the stack: the document, or an entity being read.
pub(crate) struct Input {
    pub buf: Vec<u8>,
    pub cur: usize,
    pub line: i32,
    pub col: i32,
    pub id: u32,
    pub filename: Option<String>,
    /// The entity this input reads, if one.
    pub entity: Option<Vec<u8>>,
    /// Converted to UTF-8 from another encoding.
    pub converted: bool,
    /// Where the converted text starts (libxml2 counts bytes from there).
    pub byte_base: usize,
}

impl Input {
    pub(crate) fn new(buf: Vec<u8>, filename: Option<String>) -> Input {
        Input { buf, cur: 0, line: 1, col: 1, id: 1, filename, entity: None, converted: false, byte_base: 0 }
    }
}

/// libxml2's parser options (XML_PARSE_*) the port honours.
#[derive(Clone, Copy, Default)]
pub(crate) struct Options {
    /// XML_PARSE_NOENT: substitute entities.
    pub replace_entities: bool,
    /// XML_PARSE_RECOVER: go on after a well-formedness error, SAX on.
    pub recover: bool,
    /// XML_PARSE_NOCDATA: CDATA as text.
    pub no_cdata: bool,
    /// XML_PARSE_NSCLEAN.
    pub ns_clean: bool,
    /// XML_PARSE_HUGE.
    pub huge: bool,
    /// SAX1: no namespace processing (names whole, xmlns as attributes).
    pub sax1: bool,
    /// XML_PARSE_OLDSAX: predefined entities go through getEntity too.
    pub oldsax: bool,
    /// The push parser's rules (xmlParseChunk): it stops at the first
    /// fatal error, and an unfinished document is a document-end error.
    pub push: bool,
}

pub(crate) struct Parser {
    pub inputs: Vec<Input>,
    pub instate: State,
    pub well_formed: bool,
    pub ns_well_formed: bool,
    pub valid: bool,
    pub disable_sax: bool,
    pub err_no: i32,
    pub options: Options,
    /// After an encoding error the input is read as Latin-1.
    latin1: bool,
    pub version: Option<Vec<u8>>,
    pub encoding: Option<Vec<u8>>,
    /// -2 none given, 0 no, 1 yes.
    pub standalone: i32,
    pub int_subset_name: Option<Vec<u8>>,
    pub ext_sub_system: Option<Vec<u8>>,
    pub ext_sub_uri: Option<Vec<u8>>,
    /// 0 outside the DTD, 1 in the internal subset, 2 in the external one.
    pub in_subset: i32,
    pub has_external_subset: bool,
    pub has_pe_refs: bool,
    /// General and parameter entities declared.
    pub entities: HashMap<Vec<u8>, Entity>,
    pub pentities: HashMap<Vec<u8>, Entity>,
    /// ATTLIST defaults: element name → (attribute qname, value).
    pub atts_default: HashMap<Vec<u8>, Vec<NameValue>>,
    /// ATTLIST types other than CDATA: (element, attribute) → type.
    pub atts_special: HashMap<(Vec<u8>, Vec<u8>), i32>,
    /// In-scope namespace bindings: (prefix, uri).
    pub ns_stack: Vec<NsBinding>,
    /// xml:space values of the open elements (-1 none, 0 default, 1 preserve).
    pub space_stack: Vec<i32>,
    pub depth: i32,
    pub nb_entities: u64,
    next_input_id: u32,
    /// Every error raised (libxml_get_errors), and the last.
    pub errors: Vec<XmlError>,
    pub last_error: Option<XmlError>,
    /// Blank text is kept as text (keepBlanks; LIBXML_NOBLANKS clears it).
    pub keep_blanks: bool,
    /// An undeclared entity xmlParseEntityRef reported as a warning (SAX
    /// still gets a reference to it).
    undeclared_reference: Option<Vec<u8>>,
    /// The input was converted from a BOM or UTF-16 start.
    pub encoding_detected: bool,
    /// The document ended inside an element (push mode).
    pub unfinished: bool,
    /// More input may follow (a non-final xml_parse): a construct not yet
    /// complete is left for later, and parsing stops there.
    pub partial: bool,
    /// Parsing stopped at an incomplete construct (`partial`).
    pub need_more: bool,
    /// ELEMENT declarations: name → (type, content model).
    pub element_decls: HashMap<Vec<u8>, (i32, Option<Content>)>,
}

/// XML's Char production.
pub(crate) fn is_char(c: u32) -> bool {
    matches!(c, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

pub(crate) fn is_blank(c: u32) -> bool {
    matches!(c, 0x20 | 0x9 | 0xA | 0xD)
}

/// XML 1.0 fifth edition NameStartChar.
pub(crate) fn is_name_start_char(c: u32) -> bool {
    matches!(c,
        0x61..=0x7A | 0x41..=0x5A | 0x5F | 0x3A | 0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0x2FF | 0x370..=0x37D
        | 0x37F..=0x1FFF | 0x200C..=0x200D | 0x2070..=0x218F | 0x2C00..=0x2FEF | 0x3001..=0xD7FF
        | 0xF900..=0xFDCF | 0xFDF0..=0xFFFD | 0x10000..=0xEFFFF)
}

/// XML 1.0 fifth edition NameChar.
pub(crate) fn is_name_char(c: u32) -> bool {
    is_name_start_char(c) || matches!(c, 0x30..=0x39 | 0x2D | 0x2E | 0xB7 | 0x300..=0x36F | 0x203F..=0x2040)
}

/// A character as UTF-8.
pub(crate) fn push_char(buf: &mut Vec<u8>, c: u32) {
    let ch = char::from_u32(c).unwrap_or('\u{FFFD}');
    let mut tmp = [0u8; 4];
    buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

impl Parser {
    pub(crate) fn new(data: Vec<u8>, filename: Option<String>, options: Options) -> Parser {
        Parser {
            inputs: vec![Input::new(data, filename)],
            instate: State::Start,
            well_formed: true,
            ns_well_formed: true,
            valid: true,
            disable_sax: false,
            err_no: 0,
            options,
            latin1: false,
            version: None,
            encoding: None,
            standalone: -1,
            int_subset_name: None,
            ext_sub_system: None,
            ext_sub_uri: None,
            in_subset: 0,
            has_external_subset: false,
            has_pe_refs: false,
            entities: HashMap::new(),
            pentities: HashMap::new(),
            atts_default: HashMap::new(),
            atts_special: HashMap::new(),
            ns_stack: Vec::new(),
            space_stack: vec![-1],
            depth: 0,
            nb_entities: 0,
            next_input_id: 2,
            errors: Vec::new(),
            last_error: None,
            keep_blanks: true,
            undeclared_reference: None,
            encoding_detected: false,
            element_decls: HashMap::new(),
            unfinished: false,
            partial: false,
            need_more: false,
        }
    }

    // --- input -------------------------------------------------------------

    fn inp(&self) -> &Input {
        self.inputs.last().expect("an input")
    }

    fn inp_mut(&mut self) -> &mut Input {
        self.inputs.last_mut().expect("an input")
    }

    /// The byte `n` ahead (0 past the end): libxml2's NXT(n).
    pub(crate) fn nxt(&self, n: usize) -> u8 {
        let i = self.inp();
        i.buf.get(i.cur + n).copied().unwrap_or(0)
    }

    /// RAW / CUR.
    pub(crate) fn raw(&self) -> u8 {
        self.nxt(0)
    }

    /// Bytes left in the current input.
    fn avail(&self) -> usize {
        let i = self.inp();
        i.buf.len().saturating_sub(i.cur)
    }

    pub(crate) fn starts(&self, s: &[u8]) -> bool {
        let i = self.inp();
        i.buf.get(i.cur..).is_some_and(|rest| rest.starts_with(s))
    }

    /// SKIP(n): forward over ASCII known not to hold a newline.
    pub(crate) fn skip(&mut self, n: usize) {
        let i = self.inp_mut();
        i.cur += n;
        i.col += n as i32;
    }

    /// NEXTL(l): forward over a character of `l` bytes.
    fn nextl(&mut self, l: usize) {
        let i = self.inp_mut();
        if i.buf.get(i.cur) == Some(&b'\n') {
            i.line += 1;
            i.col = 1;
        } else {
            i.col += 1;
        }
        i.cur += l;
    }

    /// CUR_CHAR: the character at the cursor and its length (xmlCurrentChar).
    pub(crate) fn cur_char(&mut self) -> (u32, usize) {
        if self.instate == State::Eof {
            return (0, 0);
        }
        let c = self.raw();
        if (0x20..=0x7F).contains(&c) {
            return (u32::from(c), 1);
        }
        if !self.latin1 {
            if c & 0x80 != 0 {
                let c1 = self.nxt(1);
                let bad = (c & 0x40) == 0 || c == 0xC0 || (c1 & 0xC0) != 0x80;
                let decoded = if bad {
                    None
                } else if (c & 0xE0) == 0xE0 {
                    let c2 = self.nxt(2);
                    if (c2 & 0xC0) != 0x80 {
                        None
                    } else if (c & 0xF0) == 0xF0 {
                        let c3 = self.nxt(3);
                        if (c & 0xF8) != 0xF0 || (c3 & 0xC0) != 0x80 {
                            None
                        } else {
                            let v = (u32::from(c) & 7) << 18 | (u32::from(c1) & 0x3F) << 12 | (u32::from(c2) & 0x3F) << 6 | (u32::from(c3) & 0x3F);
                            (v >= 0x10000).then_some((v, 4))
                        }
                    } else {
                        let v = (u32::from(c) & 0xF) << 12 | (u32::from(c1) & 0x3F) << 6 | (u32::from(c2) & 0x3F);
                        (v >= 0x800).then_some((v, 3))
                    }
                } else {
                    let v = (u32::from(c) & 0x1F) << 6 | (u32::from(c1) & 0x3F);
                    (v >= 0x80).then_some((v, 2))
                };
                return match decoded {
                    Some((v, l)) => {
                        if !is_char(v) {
                            self.err_encoding_int(ERR_INVALID_CHAR, &format!("Char 0x{v:X} out of allowed range\n"));
                        }
                        (v, l)
                    }
                    None => self.encoding_error(),
                };
            }
            if c == 0 && self.avail() > 0 {
                self.err_encoding_int(ERR_INVALID_CHAR, "Char 0x0 out of allowed range\n");
            }
        }
        if c == 0xD {
            if self.nxt(1) == 0xA {
                self.inp_mut().cur += 1;
            }
            return (0xA, 1);
        }
        (u32::from(c), 1)
    }

    /// Not UTF-8: libxml2 says so and reads on as Latin-1.
    fn encoding_error(&mut self) -> (u32, usize) {
        if self.avail() < 4 {
            return (0, 0);
        }
        let msg = format!(
            "Input is not proper UTF-8, indicate encoding !\nBytes: 0x{:02X} 0x{:02X} 0x{:02X} 0x{:02X}\n",
            self.nxt(0),
            self.nxt(1),
            self.nxt(2),
            self.nxt(3)
        );
        self.raise(FROM_PARSER, ERR_INVALID_CHAR, LEVEL_FATAL, msg);
        self.err_no = ERR_INVALID_CHAR;
        self.well_formed = false;
        if !self.options.recover {
            self.disable_sax = true;
        }
        self.latin1 = true;
        (u32::from(self.raw()), 1)
    }

    /// NEXT (xmlNextChar): one character forward.
    pub(crate) fn next(&mut self) {
        if self.instate == State::Eof || self.avail() == 0 {
            return;
        }
        if self.latin1 {
            self.nextl(1);
            return;
        }
        let c = self.raw();
        let l = if c & 0x80 == 0 {
            1
        } else if c & 0xE0 == 0xC0 {
            2
        } else if c & 0xF0 == 0xE0 {
            3
        } else {
            4
        };
        if c & 0x80 != 0 {
            let (v, len) = self.cur_char();
            if len == 0 {
                return;
            }
            let _ = v;
            self.nextl(len.min(l.max(1)));
        } else {
            self.nextl(1);
        }
    }

    /// SKIP_BLANKS: the blanks skipped.
    pub(crate) fn skip_blanks(&mut self) -> usize {
        let mut n = 0;
        while is_blank(u32::from(self.raw())) && self.avail() > 0 {
            self.nextl(1);
            n += 1;
        }
        // A parameter entity ends inside the DTD: back to the one around it.
        if n == 0 && self.raw() == 0 && self.inputs.len() > 1 && self.instate == State::Dtd {
            self.pop_input();
            return self.skip_blanks() + 1;
        }
        n
    }

    pub(crate) fn push_input(&mut self, buf: Vec<u8>, entity: Option<Vec<u8>>) {
        let id = self.next_input_id;
        self.next_input_id += 1;
        let mut input = Input::new(buf, None);
        input.id = id;
        input.entity = entity;
        self.inputs.push(input);
    }

    pub(crate) fn pop_input(&mut self) {
        if self.inputs.len() > 1 {
            self.inputs.pop();
        }
    }

    pub(crate) fn input_id(&self) -> u32 {
        self.inp().id
    }

    pub(crate) fn line(&self) -> i32 {
        self.inp().line
    }

    pub(crate) fn column(&self) -> i32 {
        self.inp().col
    }

    /// Bytes consumed of the document (xmlByteConsumed).
    pub(crate) fn byte_index(&self) -> usize {
        self.inputs[0].cur - self.inputs[0].byte_base
    }

    /// Stop parsing (xmlHaltParser).
    pub(crate) fn halt(&mut self) {
        self.instate = State::Eof;
        self.disable_sax = true;
        while self.inputs.len() > 1 {
            self.inputs.pop();
        }
        let i = self.inp_mut();
        i.cur = i.buf.len();
    }

    // --- errors ------------------------------------------------------------

    fn silenced(&self) -> bool {
        self.disable_sax && self.instate == State::Eof
    }

    /// __xmlRaiseError: record it at the current position (an entity's
    /// with no file of its own reports the input around it).
    pub(crate) fn raise(&mut self, domain: i32, code: i32, level: i32, message: String) {
        let n = self.inputs.len();
        let input = if self.inputs[n - 1].filename.is_none() && n > 1 { &self.inputs[n - 2] } else { &self.inputs[n - 1] };
        let (context, context_col) = file_context(&input.buf, input.cur);
        let e = XmlError { domain, code, level, message, line: input.line, column: input.col, file: input.filename.clone(), context, context_col };
        self.last_error = Some(e.clone());
        self.errors.push(e);
    }

    fn mark_fatal(&mut self, code: i32) {
        self.err_no = code;
        self.well_formed = false;
        if !self.options.recover {
            self.disable_sax = true;
        }
    }

    /// xmlFatalErr: its standard message (with `info`).
    pub(crate) fn fatal(&mut self, code: i32, info: Option<&str>) {
        if self.silenced() {
            return;
        }
        let msg = fatal_message(code);
        let message = match info {
            Some(info) => format!("{msg}: {info}\n"),
            None => format!("{msg}\n"),
        };
        self.err_no = code;
        self.raise(FROM_PARSER, code, LEVEL_FATAL, message);
        self.mark_fatal(code);
    }

    /// xmlFatalErrMsg / xmlFatalErrMsgStr / xmlFatalErrMsgInt: a message
    /// already formatted.
    pub(crate) fn fatal_msg(&mut self, code: i32, message: String) {
        if self.silenced() {
            return;
        }
        self.err_no = code;
        self.raise(FROM_PARSER, code, LEVEL_FATAL, message);
        self.mark_fatal(code);
    }

    pub(crate) fn warning(&mut self, code: i32, message: String) {
        if self.silenced() {
            return;
        }
        self.raise(FROM_PARSER, code, LEVEL_WARNING, message);
    }

    /// xmlErrMsgStr: an error that is not fatal.
    pub(crate) fn error_msg(&mut self, code: i32, message: String) {
        if self.silenced() {
            return;
        }
        self.err_no = code;
        self.raise(FROM_PARSER, code, LEVEL_ERROR, message);
    }

    pub(crate) fn ns_error(&mut self, code: i32, message: String) {
        if self.silenced() {
            return;
        }
        self.err_no = code;
        self.raise(FROM_NAMESPACE, code, LEVEL_ERROR, message);
        self.ns_well_formed = false;
    }

    pub(crate) fn ns_warning(&mut self, code: i32, message: String) {
        if self.silenced() {
            return;
        }
        self.raise(FROM_NAMESPACE, code, LEVEL_WARNING, message);
    }

    pub(crate) fn validity_error(&mut self, code: i32, message: String) {
        if self.silenced() {
            return;
        }
        self.err_no = code;
        self.raise(FROM_DTD, code, LEVEL_ERROR, message);
        self.valid = false;
    }


    /// __xmlErrEncoding with an int (a fatal error).
    fn err_encoding_int(&mut self, code: i32, message: &str) {
        self.fatal_msg(code, message.to_string());
    }

    // --- names -------------------------------------------------------------

    /// xmlParseName.
    pub(crate) fn parse_name(&mut self) -> Option<Vec<u8>> {
        let start = self.inp().cur;
        let c = self.raw();
        if c.is_ascii_alphabetic() || c == b'_' || c == b':' {
            let mut n = 1;
            loop {
                let d = self.nxt(n);
                if d.is_ascii_alphanumeric() || matches!(d, b'_' | b'-' | b':' | b'.') {
                    n += 1;
                } else {
                    break;
                }
            }
            let d = self.nxt(n);
            if d > 0 && d < 0x80 {
                if n > self.max_name_length() {
                    self.fatal(ERR_NAME_TOO_LONG, Some("Name"));
                    return None;
                }
                let name = self.inp().buf[start..start + n].to_vec();
                self.skip(n);
                return Some(name);
            }
        }
        let name = self.parse_name_complex()?;
        if name.len() > self.max_name_length() {
            self.fatal(ERR_NAME_TOO_LONG, Some("Name"));
            return None;
        }
        Some(name)
    }

    /// XML_MAX_NAME_LENGTH, or XML_MAX_TEXT_LENGTH with XML_PARSE_HUGE.
    fn max_name_length(&self) -> usize {
        if self.options.huge { 10_000_000 } else { 50_000 }
    }

    fn parse_name_complex(&mut self) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let (c, l) = self.cur_char();
        if c == u32::from(b' ') || c == u32::from(b'>') || c == u32::from(b'/') || !is_name_start_char(c) {
            return None;
        }
        push_char(&mut out, c);
        self.nextl(l);
        loop {
            let (c, l) = self.cur_char();
            if c == u32::from(b' ') || c == u32::from(b'>') || c == u32::from(b'/') || !is_name_char(c) || l == 0 {
                break;
            }
            push_char(&mut out, c);
            self.nextl(l);
        }
        Some(out)
    }

    /// xmlParseNCName: a name without a colon.
    pub(crate) fn parse_ncname(&mut self) -> Option<Vec<u8>> {
        let start = self.inp().cur;
        let c = self.raw();
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut n = 1;
            loop {
                let d = self.nxt(n);
                if d.is_ascii_alphanumeric() || matches!(d, b'_' | b'-' | b'.') {
                    n += 1;
                } else {
                    break;
                }
            }
            let d = self.nxt(n);
            if d > 0 && d < 0x80 {
                if n > self.max_name_length() {
                    self.fatal(ERR_NAME_TOO_LONG, Some("NCName"));
                    return None;
                }
                let name = self.inp().buf[start..start + n].to_vec();
                self.skip(n);
                return Some(name);
            }
        }
        let mut out = Vec::new();
        let (c, l) = self.cur_char();
        if c == u32::from(b' ') || c == u32::from(b'>') || c == u32::from(b'/') || !is_name_start_char(c) || c == u32::from(b':') {
            return None;
        }
        let max = self.max_name_length();
        push_char(&mut out, c);
        self.nextl(l);
        loop {
            let (c, l) = self.cur_char();
            if c == u32::from(b' ') || c == u32::from(b'>') || c == u32::from(b'/') || !is_name_char(c) || c == u32::from(b':') || l == 0 {
                break;
            }
            push_char(&mut out, c);
            self.nextl(l);
        }
        if out.len() > max {
            self.fatal(ERR_NAME_TOO_LONG, Some("NCName"));
            return None;
        }
        Some(out)
    }

    /// xmlParseQName: (prefix, local name).
    pub(crate) fn parse_qname(&mut self) -> Option<(Option<Vec<u8>>, Vec<u8>)> {
        let Some(l) = self.parse_ncname() else {
            if self.raw() == b':'
                && let Some(l) = self.parse_name()
            {
                self.ns_error(NS_ERR_QNAME, format!("Failed to parse QName '{}'\n", text(&l)));
                return Some((None, l));
            }
            return None;
        };
        if self.raw() != b':' {
            return Some((None, l));
        }
        self.next();
        let p = l;
        let Some(l) = self.parse_ncname() else {
            if self.instate == State::Eof {
                return None;
            }
            self.ns_error(NS_ERR_QNAME, format!("Failed to parse QName '{}:'\n", text(&p)));
            let rest = self.parse_nmtoken().unwrap_or_default();
            let name = [p.as_slice(), b":", rest.as_slice()].concat();
            return Some((None, name));
        };
        if self.raw() == b':' {
            self.ns_error(NS_ERR_QNAME, format!("Failed to parse QName '{}:{}:'\n", text(&p), text(&l)));
            self.next();
            let rest = self.parse_name().unwrap_or_default();
            let name = [l.as_slice(), b":", rest.as_slice()].concat();
            return Some((Some(p), name));
        }
        Some((Some(p), l))
    }

    /// xmlParseNmtoken.
    pub(crate) fn parse_nmtoken(&mut self) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let (c, l) = self.cur_char();
            if !is_name_char(c) || l == 0 {
                break;
            }
            push_char(&mut out, c);
            self.nextl(l);
        }
        (!out.is_empty()).then_some(out)
    }

    // --- the prolog --------------------------------------------------------

    /// xmlParseXMLDecl.
    pub(crate) fn parse_xml_decl(&mut self) {
        self.standalone = -2;
        self.skip(5);
        if !is_blank(u32::from(self.raw())) {
            self.fatal_msg(ERR_SPACE_REQUIRED, "Blank needed after '<?xml'\n".into());
        }
        self.skip_blanks();
        match self.parse_version_info() {
            None => self.fatal(ERR_VERSION_MISSING, None),
            Some(version) => {
                if version != b"1.0" {
                    if version.len() >= 2 && version[0] == b'1' && version[1] == b'.' {
                        self.warning(WAR_UNKNOWN_VERSION, format!("Unsupported version '{}'\n", text(&version)));
                    } else {
                        self.fatal_msg(ERR_UNKNOWN_VERSION, format!("Unsupported version '{}'\n", text(&version)));
                    }
                }
                self.version = Some(version);
            }
        }
        if !is_blank(u32::from(self.raw())) {
            if self.raw() == b'?' && self.nxt(1) == b'>' {
                self.skip(2);
                return;
            }
            self.fatal_msg(ERR_SPACE_REQUIRED, "Blank needed here\n".into());
        }
        let encoding = self.parse_encoding_decl();
        if self.err_no == ERR_UNSUPPORTED_ENCODING || self.instate == State::Eof {
            return;
        }
        if encoding.is_some() && !is_blank(u32::from(self.raw())) {
            if self.raw() == b'?' && self.nxt(1) == b'>' {
                self.skip(2);
                return;
            }
            self.fatal_msg(ERR_SPACE_REQUIRED, "Blank needed here\n".into());
        }
        self.skip_blanks();
        self.standalone = self.parse_sd_decl();
        self.skip_blanks();
        if self.raw() == b'?' && self.nxt(1) == b'>' {
            self.skip(2);
        } else if self.raw() == b'>' {
            self.fatal(ERR_XMLDECL_NOT_FINISHED, None);
            self.next();
        } else {
            self.fatal(ERR_XMLDECL_NOT_FINISHED, None);
            // MOVETO_ENDTAG: on to the next '>'.
            while self.raw() != 0 && self.raw() != b'>' {
                self.nextl(1);
            }
            self.next();
        }
    }

    /// A quoted value read by `body`, with libxml2's errors for the quotes.
    fn quoted<T>(&mut self, body: impl FnOnce(&mut Parser) -> Option<T>, unclosed_fails: bool) -> Option<T> {
        let q = self.raw();
        if q != b'"' && q != b'\'' {
            self.fatal(ERR_STRING_NOT_STARTED, None);
            return None;
        }
        self.next();
        let v = body(self);
        if self.raw() != q {
            self.fatal(ERR_STRING_NOT_CLOSED, None);
            if unclosed_fails {
                return None;
            }
        } else {
            self.next();
        }
        v
    }

    fn parse_version_info(&mut self) -> Option<Vec<u8>> {
        if !self.starts(b"version") {
            return None;
        }
        self.skip(7);
        self.skip_blanks();
        if self.raw() != b'=' {
            self.fatal(ERR_EQUAL_REQUIRED, None);
            return None;
        }
        self.next();
        self.skip_blanks();
        self.quoted(Parser::parse_version_num, false)
    }

    fn parse_version_num(&mut self) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let c = self.raw();
        if !c.is_ascii_digit() {
            return None;
        }
        out.push(c);
        self.next();
        if self.raw() != b'.' {
            return None;
        }
        out.push(b'.');
        self.next();
        while self.raw().is_ascii_digit() {
            out.push(self.raw());
            self.next();
        }
        Some(out)
    }

    /// xmlParseEncodingDecl: the encoding named (the input switches to it).
    fn parse_encoding_decl(&mut self) -> Option<Vec<u8>> {
        self.skip_blanks();
        if !self.starts(b"encoding") {
            return None;
        }
        self.skip(8);
        self.skip_blanks();
        if self.raw() != b'=' {
            self.fatal(ERR_EQUAL_REQUIRED, None);
            return None;
        }
        self.next();
        self.skip_blanks();
        let encoding = self.quoted(Parser::parse_enc_name, true)?;
        let upper = encoding.to_ascii_uppercase();
        if upper == b"UTF-16" || upper == b"UTF16" {
            if self.encoding.is_none() && !self.inputs[0].converted {
                self.fatal_msg(ERR_INVALID_ENCODING, "Document labelled UTF-16 but has UTF-8 content\n".into());
            }
        } else if upper != b"UTF-8" && upper != b"UTF8" {
            match crate::encoding::decoder(&encoding) {
                Some(dec) => {
                    if self.encoding.is_none() {
                        self.switch_input_encoding(dec);
                    }
                }
                None => {
                    self.fatal_msg(ERR_UNSUPPORTED_ENCODING, format!("Unsupported encoding {}\n", text(&encoding)));
                    return None;
                }
            }
        }
        if self.encoding.is_none() {
            self.encoding = Some(encoding.clone());
        }
        Some(encoding)
    }

    /// The rest of the document input, from the cursor, decoded.
    fn switch_input_encoding(&mut self, dec: crate::encoding::Decoder) {
        let input = &mut self.inputs[0];
        let rest = input.buf.split_off(input.cur);
        input.buf.extend_from_slice(&dec.decode(&rest));
        input.converted = true;
        input.byte_base = input.cur;
    }

    fn parse_enc_name(&mut self) -> Option<Vec<u8>> {
        let c = self.raw();
        if !c.is_ascii_alphabetic() {
            self.fatal(ERR_ENCODING_NAME, None);
            return None;
        }
        let mut out = vec![c];
        self.next();
        while self.raw().is_ascii_alphanumeric() || matches!(self.raw(), b'.' | b'_' | b'-') {
            out.push(self.raw());
            self.next();
        }
        Some(out)
    }

    fn parse_sd_decl(&mut self) -> i32 {
        let mut standalone = -2;
        self.skip_blanks();
        if !self.starts(b"standalone") {
            return standalone;
        }
        self.skip(10);
        self.skip_blanks();
        if self.raw() != b'=' {
            self.fatal(ERR_EQUAL_REQUIRED, None);
            return standalone;
        }
        self.next();
        self.skip_blanks();
        let q = self.raw();
        if q != b'\'' && q != b'"' {
            self.fatal(ERR_STRING_NOT_STARTED, None);
            return standalone;
        }
        self.next();
        if self.starts(b"no") {
            standalone = 0;
            self.skip(2);
        } else if self.starts(b"yes") {
            standalone = 1;
            self.skip(3);
        } else {
            self.fatal(ERR_STANDALONE_VALUE, None);
        }
        if self.raw() != q {
            self.fatal(ERR_STRING_NOT_CLOSED, None);
        } else {
            self.next();
        }
        standalone
    }

    /// xmlParseMisc: comments, PIs and blanks.
    pub(crate) fn parse_misc(&mut self, sax: &mut dyn Sax) {
        while self.instate != State::Eof {
            self.skip_blanks();
            if (self.raw() == b'<' && (self.nxt(1) == b'?' || self.nxt(1) == b'!')) && self.wait_for_more() {
                return;
            }
            if self.raw() == b'<' && self.nxt(1) == b'?' {
                self.parse_pi(sax);
            } else if self.starts(b"<!--") {
                self.parse_comment(sax);
            } else {
                break;
            }
        }
    }

    // --- PIs, comments, CDATA ----------------------------------------------

    fn parse_pi_target(&mut self) -> Option<Vec<u8>> {
        let name = self.parse_name()?;
        if name.len() >= 3 && name[..3].eq_ignore_ascii_case(b"xml") {
            if name == b"xml" {
                self.fatal_msg(ERR_RESERVED_XML_NAME, "XML declaration allowed only at the start of the document\n".into());
                return Some(name);
            } else if name.len() == 3 {
                self.fatal(ERR_RESERVED_XML_NAME, None);
                return Some(name);
            }
            if name != b"xml-stylesheet" && name != b"xml-model" {
                self.warning(ERR_RESERVED_XML_NAME, "xmlParsePITarget: invalid name prefix 'xml'\n".into());
            }
        }
        if name.contains(&b':') {
            self.ns_error(NS_ERR_COLON, format!("colons are forbidden from PI names '{}'\n", text(&name)));
        }
        Some(name)
    }

    /// xmlParsePI.
    pub(crate) fn parse_pi(&mut self, sax: &mut dyn Sax) {
        if !(self.raw() == b'<' && self.nxt(1) == b'?') {
            return;
        }
        let input_id = self.input_id();
        let state = self.instate;
        self.instate = State::Pi;
        self.skip(2);
        let Some(target) = self.parse_pi_target() else {
            self.fatal(ERR_PI_NOT_STARTED, None);
            if self.instate != State::Eof {
                self.instate = state;
            }
            return;
        };
        if self.raw() == b'?' && self.nxt(1) == b'>' {
            if input_id != self.input_id() {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "PI declaration doesn't start and stop in the same entity\n".into());
            }
            self.skip(2);
            if !self.disable_sax {
                sax.processing_instruction(self, &target, None);
            }
            if self.instate != State::Eof {
                self.instate = state;
            }
            return;
        }
        if self.skip_blanks() == 0 {
            self.fatal_msg(ERR_SPACE_REQUIRED, format!("ParsePI: PI {} space expected\n", text(&target)));
        }
        let mut buf = Vec::new();
        let (mut cur, mut l) = self.cur_char();
        while is_char(cur) && (cur != u32::from(b'?') || self.nxt(1) != b'>') {
            push_char(&mut buf, cur);
            self.nextl(l);
            (cur, l) = self.cur_char();
        }
        if cur != u32::from(b'?') {
            self.fatal_msg(ERR_PI_NOT_FINISHED, format!("ParsePI: PI {} never end ...\n", text(&target)));
        } else {
            if input_id != self.input_id() {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "PI declaration doesn't start and stop in the same entity\n".into());
            }
            self.skip(2);
            if !self.disable_sax {
                sax.processing_instruction(self, &target, Some(&buf));
            }
        }
        if self.instate != State::Eof {
            self.instate = state;
        }
    }

    /// xmlParseComment (the fast path and xmlParseCommentComplex in one:
    /// the same characters, the same errors).
    pub(crate) fn parse_comment(&mut self, sax: &mut dyn Sax) {
        if !self.starts(b"<!--") {
            return;
        }
        let state = self.instate;
        self.instate = State::Comment;
        let input_id = self.input_id();
        self.skip(4);
        let mut buf: Vec<u8> = Vec::new();
        // The fast path: ASCII, checking each "--" as it comes.
        loop {
            let c = self.raw();
            if c == b'-' && self.nxt(1) == b'-' {
                if self.nxt(2) == b'>' {
                    if self.input_id() != input_id {
                        self.fatal_msg(ERR_ENTITY_BOUNDARY, "comment doesn't start and stop in the same entity\n".into());
                    }
                    self.skip(3);
                    if !self.disable_sax {
                        sax.comment(self, &buf);
                    }
                    if self.instate != State::Eof {
                        self.instate = state;
                    }
                    return;
                }
                if buf.is_empty() {
                    self.fatal_msg(ERR_HYPHEN_IN_COMMENT, "Double hyphen within comment\n".into());
                } else {
                    let shown: String = text(&buf).chars().take(50).collect();
                    self.fatal_msg(ERR_HYPHEN_IN_COMMENT, format!("Double hyphen within comment: <!--{shown}\n"));
                }
                if self.instate == State::Eof {
                    return;
                }
                buf.extend_from_slice(b"--");
                self.skip(2);
                continue;
            }
            if (0x20..=0x7F).contains(&c) || c == 0x09 || c == 0x0A {
                buf.push(c);
                self.nextl(1);
                continue;
            }
            if c == 0x0D && self.nxt(1) == 0x0A {
                self.inp_mut().cur += 1;
                continue;
            }
            break;
        }
        self.parse_comment_complex(sax, buf, input_id);
        self.instate = state;
    }

    fn parse_comment_complex(&mut self, sax: &mut dyn Sax, mut buf: Vec<u8>, input_id: u32) {
        let (mut q, mut ql) = self.cur_char();
        if q == 0 {
            self.fatal_msg(ERR_COMMENT_NOT_FINISHED, "Comment not terminated\n".into());
            return;
        }
        if !is_char(q) {
            self.fatal_msg(ERR_INVALID_CHAR, format!("xmlParseComment: invalid xmlChar value {q}\n"));
            return;
        }
        self.nextl(ql);
        let (mut r, mut rl) = self.cur_char();
        if r == 0 {
            self.fatal_msg(ERR_COMMENT_NOT_FINISHED, "Comment not terminated\n".into());
            return;
        }
        if !is_char(r) {
            self.fatal_msg(ERR_INVALID_CHAR, format!("xmlParseComment: invalid xmlChar value {q}\n"));
            return;
        }
        self.nextl(rl);
        let (mut cur, mut l) = self.cur_char();
        if cur == 0 {
            self.fatal_msg(ERR_COMMENT_NOT_FINISHED, "Comment not terminated\n".into());
            return;
        }
        while is_char(cur) && (cur != u32::from(b'>') || r != u32::from(b'-') || q != u32::from(b'-')) {
            if r == u32::from(b'-') && q == u32::from(b'-') {
                self.fatal(ERR_HYPHEN_IN_COMMENT, None);
            }
            push_char(&mut buf, q);
            (q, ql) = (r, rl);
            (r, rl) = (cur, l);
            self.nextl(l);
            (cur, l) = self.cur_char();
        }
        let _ = (ql, rl);
        if cur == 0 {
            let shown: String = text(&buf).chars().take(50).collect();
            self.fatal_msg(ERR_COMMENT_NOT_FINISHED, format!("Comment not terminated \n<!--{shown}\n"));
        } else if !is_char(cur) {
            self.fatal_msg(ERR_INVALID_CHAR, format!("xmlParseComment: invalid xmlChar value {cur}\n"));
        } else {
            if input_id != self.input_id() {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "Comment doesn't start and stop in the same entity\n".into());
            }
            self.next();
            if !self.disable_sax {
                sax.comment(self, &buf);
            }
        }
    }

    /// xmlParseCDSect.
    pub(crate) fn parse_cdsect(&mut self, sax: &mut dyn Sax) {
        if !self.starts(b"<![CDATA[") {
            return;
        }
        self.skip(9);
        self.instate = State::CdataSection;
        let (mut r, mut rl) = self.cur_char();
        if !is_char(r) {
            self.fatal(ERR_CDATA_NOT_FINISHED, None);
            self.instate = State::Content;
            return;
        }
        self.nextl(rl);
        let (mut s, mut sl) = self.cur_char();
        if !is_char(s) {
            self.fatal(ERR_CDATA_NOT_FINISHED, None);
            self.instate = State::Content;
            return;
        }
        self.nextl(sl);
        let (mut cur, mut l) = self.cur_char();
        let mut buf = Vec::new();
        while is_char(cur) && (r != u32::from(b']') || s != u32::from(b']') || cur != u32::from(b'>')) {
            push_char(&mut buf, r);
            (r, rl) = (s, sl);
            (s, sl) = (cur, l);
            self.nextl(l);
            (cur, l) = self.cur_char();
        }
        let _ = (rl, sl);
        self.instate = State::Content;
        if cur != u32::from(b'>') {
            let shown: String = text(&buf).chars().take(50).collect();
            self.fatal_msg(ERR_CDATA_NOT_FINISHED, format!("CData section not finished\n{shown}\n"));
            return;
        }
        self.nextl(l);
        if !self.disable_sax {
            if self.options.no_cdata {
                sax.characters(self, &buf);
            } else {
                sax.cdata_block(self, &buf);
            }
        }
    }
}

/// The XML namespace (the `xml` prefix is always bound to it).
pub(crate) const XML_NS: &[u8] = b"http://www.w3.org/XML/1998/namespace";

/// An open element: libxml2's nameTab and pushTab entries.
#[derive(Clone)]
pub(crate) struct OpenTag {
    pub local: Vec<u8>,
    pub prefix: Option<Vec<u8>>,
    pub uri: Option<Vec<u8>>,
    pub line: i32,
    /// Namespace bindings it pushed.
    pub ns_nr: usize,
}

fn predefined(name: &[u8]) -> Option<&'static [u8]> {
    Some(match name {
        b"lt" => b"<",
        b"gt" => b">",
        b"amp" => b"&",
        b"apos" => b"'",
        b"quot" => b"\"",
        _ => return None,
    })
}

/// A reference's target: a predefined entity's text, or a declared one.
enum Ref {
    Predefined(&'static [u8]),
    Entity(Entity),
}

impl Parser {
    // --- namespaces --------------------------------------------------------

    /// xmlGetNamespace: the URI `prefix` is bound to (an empty default
    /// namespace is none).
    pub(crate) fn get_namespace(&self, prefix: Option<&[u8]>) -> Option<Vec<u8>> {
        if prefix == Some(b"xml") {
            return Some(XML_NS.to_vec());
        }
        for (p, uri) in self.ns_stack.iter().rev() {
            if p.as_deref() == prefix {
                if prefix.is_none() && uri.is_empty() {
                    return None;
                }
                return Some(uri.clone());
            }
        }
        None
    }

    /// nsPush: false when NSCLEAN finds the same binding already in scope.
    fn ns_push(&mut self, prefix: Option<Vec<u8>>, uri: Vec<u8>) -> bool {
        if self.options.ns_clean {
            for (p, u) in self.ns_stack.iter().rev() {
                if *p == prefix {
                    if *u == uri {
                        return false;
                    }
                    break;
                }
            }
        }
        self.ns_stack.push((prefix, uri));
        true
    }

    /// xmlErrAttributeDup.
    fn err_attribute_dup(&mut self, prefix: Option<&[u8]>, local: &[u8]) {
        if self.silenced() {
            return;
        }
        let message = match prefix {
            None => format!("Attribute {} redefined\n", text(local)),
            Some(p) => format!("Attribute {}:{} redefined\n", text(p), text(local)),
        };
        self.err_no = ERR_ATTRIBUTE_REDEFINED;
        self.raise(FROM_PARSER, ERR_ATTRIBUTE_REDEFINED, LEVEL_FATAL, message);
        self.mark_fatal(ERR_ATTRIBUTE_REDEFINED);
    }

    // --- start and end tags ------------------------------------------------

    /// xmlParseAttribute2: (prefix, name, value).
    fn parse_attribute2(&mut self, elem_prefix: Option<&[u8]>, elem: &[u8]) -> Option<(Option<Vec<u8>>, NameValue)> {
        let Some((prefix, name)) = self.parse_qname() else {
            self.fatal_msg(ERR_NAME_REQUIRED, "error parsing attribute name\n".into());
            return None;
        };
        let qelem = qualified(elem_prefix, elem);
        let qattr = qualified(prefix.as_deref(), &name);
        let normalize = self.atts_special.contains_key(&(qelem, qattr));
        self.skip_blanks();
        if self.raw() != b'=' {
            self.fatal_msg(ERR_ATTRIBUTE_WITHOUT_VALUE, format!("Specification mandates value for attribute {}\n", text(&name)));
            return None;
        }
        self.next();
        self.skip_blanks();
        let value = self.parse_att_value(normalize);
        self.instate = State::Content;
        let value = value?;
        if prefix.as_deref() == Some(b"xml") && name == b"space" {
            match value.as_slice() {
                b"default" => *self.space_stack.last_mut().expect("a space entry") = 0,
                b"preserve" => *self.space_stack.last_mut().expect("a space entry") = 1,
                _ => self.warning(
                    WAR_SPACE_VALUE,
                    format!("Invalid value \"{}\" for xml:space : \"default\" or \"preserve\" expected\n", text(&value)),
                ),
            }
        }
        Some((prefix, (name, value)))
    }

    /// xmlParseAttValue(Complex): the value, references replaced or kept
    /// (`&name;`) as replaceEntities says, whitespace normalized.
    pub(crate) fn parse_att_value(&mut self, normalize: bool) -> Option<Vec<u8>> {
        let limit = self.raw();
        if limit != b'"' && limit != b'\'' {
            self.fatal(ERR_ATTRIBUTE_NOT_STARTED, None);
            return None;
        }
        self.instate = State::AttributeValue;
        self.next();
        let mut buf = Vec::new();
        let mut in_space = false;
        let (mut c, mut l) = self.cur_char();
        while self.raw() != limit && is_char(c) && c != u32::from(b'<') && self.instate != State::Eof {
            if c == u32::from(b'&') {
                in_space = false;
                if self.nxt(1) == b'#' {
                    let val = self.parse_char_ref();
                    if val == u32::from(b'&') {
                        if self.options.replace_entities {
                            buf.push(b'&');
                        } else {
                            buf.extend_from_slice(b"&#38;");
                        }
                    } else if val != 0 {
                        push_char(&mut buf, val);
                    }
                } else {
                    let r = self.parse_entity_ref_in(None);
                    self.nb_entities += 1;
                    match r {
                        Some(Ref::Predefined(t)) => {
                            if !self.options.replace_entities && t == b"&" {
                                buf.extend_from_slice(b"&#38;");
                            } else {
                                buf.push(t[0]);
                            }
                        }
                        Some(Ref::Entity(ent)) if self.options.replace_entities => {
                            self.depth += 1;
                            let rep = self.decode_entities(ent.content.as_deref().unwrap_or(b""), false);
                            self.depth -= 1;
                            for &b in rep.as_deref().unwrap_or(b"") {
                                buf.push(if matches!(b, 0xD | 0xA | 0x9) { 0x20 } else { b });
                            }
                        }
                        Some(Ref::Entity(ent)) => {
                            if ent.checked == 0
                                && let Some(content) = ent.content.clone()
                            {
                                self.depth += 1;
                                let rep = self.decode_entities(&content, false);
                                self.depth -= 1;
                                let markup = rep.as_ref().is_some_and(|r| r.contains(&b'<'));
                                if let Some(e) = self.entities.get_mut(&ent.name) {
                                    e.checked = 2 | u32::from(markup);
                                    if rep.is_none() {
                                        e.content = Some(Vec::new());
                                    }
                                }
                            }
                            buf.push(b'&');
                            buf.extend_from_slice(&ent.name);
                            buf.push(b';');
                        }
                        None => {}
                    }
                }
            } else {
                if matches!(c, 0x20 | 0xD | 0xA | 0x9) {
                    if !buf.is_empty() || !normalize {
                        if !normalize || !in_space {
                            buf.push(0x20);
                        }
                        in_space = true;
                    }
                } else {
                    in_space = false;
                    push_char(&mut buf, c);
                }
                self.nextl(l);
            }
            (c, l) = self.cur_char();
        }
        if in_space && normalize {
            while buf.last() == Some(&0x20) {
                buf.pop();
            }
        }
        if self.raw() == b'<' {
            self.fatal(ERR_LT_IN_ATTRIBUTE, None);
        } else if self.raw() != limit {
            if c != 0 && !is_char(c) {
                self.fatal_msg(ERR_INVALID_CHAR, "invalid character in attribute value\n".into());
            } else {
                self.fatal_msg(ERR_ATTRIBUTE_NOT_FINISHED, "AttValue: ' expected\n".into());
            }
        } else {
            self.next();
        }
        Some(buf)
    }

    /// xmlParseStartTag2: the element's local name, prefix and URI; SAX's
    /// startElementNs with its namespaces and attributes.
    fn parse_start_tag2(&mut self, sax: &mut dyn Sax) -> Option<TagName> {
        if self.raw() != b'<' {
            return None;
        }
        if self.options.sax1 {
            return self.parse_start_tag1(sax);
        }
        self.skip(1);
        let input_id = self.input_id();
        let ns_base = self.ns_stack.len();
        let Some((prefix, local)) = self.parse_qname() else {
            self.fatal_msg(ERR_NAME_REQUIRED, "StartTag: invalid element name\n".into());
            return None;
        };
        let mut atts: Vec<Attr> = Vec::new();
        let mut nb_ns = 0usize;
        self.skip_blanks();
        while self.raw() != b'>' && !(self.raw() == b'/' && self.nxt(1) == b'>') && self.raw() != 0 && self.instate != State::Eof {
            let before = (self.input_id(), self.inp().cur);
            let attr = self.parse_attribute2(prefix.as_deref(), &local);
            if let Some((aprefix, (aname, value))) = attr.clone() {
                if aprefix.is_none() && aname == b"xmlns" {
                    self.default_ns_decl(&value, ns_base, &mut nb_ns);
                } else if aprefix.as_deref() == Some(b"xmlns") {
                    self.prefixed_ns_decl(&aname, &value, ns_base, &mut nb_ns);
                } else {
                    atts.push(Attr { local: aname, prefix: aprefix, uri: None, value, defaulted: false });
                }
            }
            if self.instate == State::Eof {
                break;
            }
            if self.raw() == b'>' || (self.raw() == b'/' && self.nxt(1) == b'>') {
                break;
            }
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "attributes construct error\n".into());
                break;
            }
            if before == (self.input_id(), self.inp().cur) && attr.is_none() {
                self.fatal(ERR_INTERNAL_ERROR, Some("xmlParseStartTag: problem parsing attributes"));
                break;
            }
        }
        if self.input_id() != input_id {
            self.fatal(ERR_INTERNAL_ERROR, Some("Unexpected change of input"));
            return None;
        }
        // Defaulted attributes (ATTLIST).
        let mut nbdef = 0;
        let qelem = qualified(prefix.as_deref(), &local);
        if let Some(defaults) = self.atts_default.get(&qelem).cloned() {
            for (qattr, value) in defaults {
                let (aprefix, aname) = split_qname(&qattr);
                if aprefix.is_none() && aname == b"xmlns" {
                    if self.ns_stack[ns_base..].iter().any(|(p, _)| p.is_none()) {
                        continue;
                    }
                    if self.get_namespace(None).as_deref() != Some(value.as_slice()) && self.ns_push(None, value.clone()) {
                        nb_ns += 1;
                    }
                } else if aprefix.as_deref() == Some(b"xmlns") {
                    if self.ns_stack[ns_base..].iter().any(|(p, _)| p.as_deref() == Some(aname.as_slice())) {
                        continue;
                    }
                    if self.get_namespace(Some(&aname)).as_deref() != Some(value.as_slice()) && self.ns_push(Some(aname.clone()), value.clone()) {
                        nb_ns += 1;
                    }
                } else {
                    if atts.iter().any(|a| a.local == aname && a.prefix == aprefix) {
                        continue;
                    }
                    atts.push(Attr { local: aname, prefix: aprefix, uri: None, value, defaulted: true });
                    nbdef += 1;
                }
            }
        }
        let _ = nbdef;
        // Attribute namespaces and duplicates.
        for i in 0..atts.len() {
            let uri = match atts[i].prefix.clone() {
                Some(p) => {
                    let uri = self.get_namespace(Some(&p));
                    if uri.is_none() {
                        self.ns_error(
                            NS_ERR_UNDEFINED_NAMESPACE,
                            format!("Namespace prefix {} for {} on {} is not defined\n", text(&p), text(&atts[i].local), text(&local)),
                        );
                    }
                    uri
                }
                None => None,
            };
            atts[i].uri = uri.clone();
            for j in 0..i {
                if atts[i].local == atts[j].local {
                    if atts[i].prefix == atts[j].prefix {
                        let (p, l) = (atts[i].prefix.clone(), atts[i].local.clone());
                        self.err_attribute_dup(p.as_deref(), &l);
                        break;
                    }
                    if let Some(u) = &uri
                        && atts[j].uri.as_ref() == Some(u)
                    {
                        let (l, u) = (atts[i].local.clone(), u.clone());
                        self.ns_error(NS_ERR_ATTRIBUTE_REDEFINED, format!("Namespaced Attribute {} in '{}' redefined\n", text(&l), text(&u)));
                        break;
                    }
                }
            }
        }
        let uri = self.get_namespace(prefix.as_deref());
        if let Some(p) = &prefix
            && uri.is_none()
        {
            self.ns_error(NS_ERR_UNDEFINED_NAMESPACE, format!("Namespace prefix {} on {} is not defined\n", text(p), text(&local)));
        }
        if !self.disable_sax {
            let namespaces: Vec<NsBinding> = self.ns_stack[self.ns_stack.len() - nb_ns..].to_vec();
            sax.start_element_ns(self, &local, prefix.as_deref(), uri.as_deref(), &namespaces, &atts);
        }
        Some((local, prefix, uri))
    }

    /// xmlParseStartTag (SAX1): names whole, no namespaces, xmlns
    /// attributes as any other.
    fn parse_start_tag1(&mut self, sax: &mut dyn Sax) -> Option<TagName> {
        self.skip(1);
        let Some(name) = self.parse_name() else {
            self.fatal_msg(ERR_NAME_REQUIRED, "xmlParseStartTag: invalid element name\n".into());
            return None;
        };
        let mut atts: Vec<Attr> = Vec::new();
        self.skip_blanks();
        while self.raw() != b'>' && !(self.raw() == b'/' && self.nxt(1) == b'>') && self.raw() != 0 && self.instate != State::Eof {
            let before = (self.input_id(), self.inp().cur);
            // xmlParseAttribute.
            let attr = match self.parse_name() {
                None => {
                    self.fatal_msg(ERR_NAME_REQUIRED, "error parsing attribute name\n".into());
                    None
                }
                Some(aname) => {
                    let normalize = self.atts_special.contains_key(&(name.clone(), aname.clone()));
                    self.skip_blanks();
                    if self.raw() == b'=' {
                        self.next();
                        self.skip_blanks();
                        let v = self.parse_att_value(normalize);
                        self.instate = State::Content;
                        v.map(|v| (aname, v))
                    } else {
                        self.fatal_msg(ERR_ATTRIBUTE_WITHOUT_VALUE, format!("Specification mandates value for attribute {}\n", text(&aname)));
                        None
                    }
                }
            };
            if let Some((aname, value)) = attr.clone() {
                if atts.iter().any(|a| a.local == aname) {
                    self.err_attribute_dup(None, &aname);
                } else {
                    atts.push(Attr { local: aname, prefix: None, uri: None, value, defaulted: false });
                }
            }
            if self.raw() == b'>' || (self.raw() == b'/' && self.nxt(1) == b'>') {
                break;
            }
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "attributes construct error\n".into());
                break;
            }
            if before == (self.input_id(), self.inp().cur) && attr.is_none() {
                self.fatal(ERR_INTERNAL_ERROR, Some("xmlParseStartTag: problem parsing attributes"));
                break;
            }
        }
        if !self.disable_sax {
            sax.start_element_ns(self, &name, None, None, &[], &atts);
        }
        Some((name, None, None))
    }

    /// `xmlns="…"` on a start tag.
    fn default_ns_decl(&mut self, url: &[u8], ns_base: usize, nb_ns: &mut usize) {
        if !url.is_empty() {
            if !looks_like_uri(url) {
                self.ns_error(WAR_NS_URI, format!("xmlns: '{}' is not a valid URI\n", text(url)));
            } else if !has_scheme(url) {
                self.ns_warning(WAR_NS_URI_RELATIVE, format!("xmlns: URI {} is not absolute\n", text(url)));
            }
            if url == XML_NS {
                self.ns_error(NS_ERR_XML_NAMESPACE, "xml namespace URI cannot be the default namespace\n".into());
                return;
            }
            if url == b"http://www.w3.org/2000/xmlns/" {
                self.ns_error(NS_ERR_XML_NAMESPACE, "reuse of the xmlns namespace name is forbidden\n".into());
                return;
            }
        }
        if self.ns_stack[ns_base..].iter().any(|(p, _)| p.is_none()) {
            self.err_attribute_dup(None, b"xmlns");
        } else if self.ns_push(None, url.to_vec()) {
            *nb_ns += 1;
        }
    }

    /// `xmlns:prefix="…"` on a start tag.
    fn prefixed_ns_decl(&mut self, name: &[u8], url: &[u8], ns_base: usize, nb_ns: &mut usize) {
        if name == b"xml" {
            if url != XML_NS {
                self.ns_error(NS_ERR_XML_NAMESPACE, "xml namespace prefix mapped to wrong URI\n".into());
            }
            return;
        }
        if url == XML_NS {
            self.ns_error(NS_ERR_XML_NAMESPACE, "xml namespace URI mapped to wrong prefix\n".into());
            return;
        }
        if name == b"xmlns" {
            self.ns_error(NS_ERR_XML_NAMESPACE, "redefinition of the xmlns prefix is forbidden\n".into());
            return;
        }
        if url == b"http://www.w3.org/2000/xmlns/" {
            self.ns_error(NS_ERR_XML_NAMESPACE, "reuse of the xmlns namespace name is forbidden\n".into());
            return;
        }
        if url.is_empty() {
            self.ns_error(NS_ERR_XML_NAMESPACE, format!("xmlns:{}: Empty XML namespace is not allowed\n", text(name)));
            return;
        }
        if !looks_like_uri(url) {
            self.ns_error(WAR_NS_URI, format!("xmlns:{}: '{}' is not a valid URI\n", text(name), text(url)));
        }
        if self.ns_stack[ns_base..].iter().any(|(p, _)| p.as_deref() == Some(name)) {
            self.err_attribute_dup(Some(b"xmlns"), name);
        } else if self.ns_push(Some(name.to_vec()), url.to_vec()) {
            *nb_ns += 1;
        }
    }

    /// xmlParseNameAndCompare / xmlParseQNameAndCompare: Ok when the end
    /// tag names `tag`, else the name found.
    fn parse_end_name(&mut self, tag: &OpenTag) -> Result<(), Option<Vec<u8>>> {
        let expect = qualified(tag.prefix.as_deref(), &tag.local);
        if self.starts(&expect) {
            let after = self.nxt(expect.len());
            if after == b'>' || is_blank(u32::from(after)) {
                self.skip(expect.len());
                return Ok(());
            }
        }
        match &tag.prefix {
            None => match self.parse_name() {
                Some(n) if n == tag.local => Ok(()),
                other => Err(other),
            },
            Some(p) => match self.parse_qname() {
                Some((Some(p2), l)) if &p2 == p && l == tag.local => Ok(()),
                Some((_, l)) => Err(Some(l)),
                None => Err(None),
            },
        }
    }

    /// xmlParseEndTag2.
    fn parse_end_tag2(&mut self, sax: &mut dyn Sax, tag: &OpenTag) {
        if self.raw() != b'<' || self.nxt(1) != b'/' {
            self.fatal(ERR_LTSLASH_REQUIRED, None);
            return;
        }
        self.skip(2);
        let found = self.parse_end_name(tag);
        if self.instate == State::Eof {
            return;
        }
        self.skip_blanks();
        if self.raw() != b'>' {
            self.fatal(ERR_GT_REQUIRED, None);
        } else {
            self.skip(1);
        }
        if let Err(name) = found {
            let name = name.unwrap_or_else(|| b"unparsable".to_vec());
            self.fatal_msg(
                ERR_TAG_NAME_MISMATCH,
                format!("Opening and ending tag mismatch: {} line {} and {}\n", text(&tag.local), tag.line, text(&name)),
            );
        }
        if !self.disable_sax {
            sax.end_element_ns(self, &tag.local, tag.prefix.as_deref(), tag.uri.as_deref());
        }
        let keep = self.ns_stack.len().saturating_sub(tag.ns_nr);
        self.ns_stack.truncate(keep);
    }

    // --- elements and content ----------------------------------------------

    /// xmlParseElementStart: 0 when content follows, 1 for an empty
    /// element, -1 on failure.
    fn parse_element_start(&mut self, sax: &mut dyn Sax, open: &mut Vec<OpenTag>) -> i32 {
        if open.len() > 256 && !self.options.huge {
            self.fatal_msg(ERR_INTERNAL_ERROR, "Excessive depth in document: 256 use XML_PARSE_HUGE option\n".into());
            self.halt();
            return -1;
        }
        let parent_space = *self.space_stack.last().expect("a space entry");
        self.space_stack.push(if parent_space == -2 { -1 } else { parent_space });
        let line = self.line();
        let ns_before = self.ns_stack.len();
        let tag = self.parse_start_tag2(sax);
        if self.instate == State::Eof {
            return -1;
        }
        let Some((local, prefix, uri)) = tag else {
            self.space_stack.pop();
            return -1;
        };
        let ns_nr = self.ns_stack.len() - ns_before;
        if self.raw() == b'/' && self.nxt(1) == b'>' {
            self.skip(2);
            if !self.disable_sax {
                sax.end_element_ns(self, &local, prefix.as_deref(), uri.as_deref());
            }
            self.space_stack.pop();
            self.ns_stack.truncate(ns_before);
            return 1;
        }
        if self.raw() == b'>' {
            self.skip(1);
        } else {
            self.fatal_msg(ERR_GT_REQUIRED, format!("Couldn't find end of Start Tag {} line {}\n", text(&local), line));
            self.space_stack.pop();
            self.ns_stack.truncate(ns_before);
            return -1;
        }
        open.push(OpenTag { local, prefix, uri, line, ns_nr });
        0
    }

    /// xmlParseElementEnd.
    fn parse_element_end(&mut self, sax: &mut dyn Sax, open: &mut Vec<OpenTag>) {
        let Some(tag) = open.last().cloned() else { return };
        self.parse_end_tag2(sax, &tag);
        open.pop();
        self.space_stack.pop();
    }

    /// xmlParseContentInternal: content until the end tag of the element
    /// open when it started (`base` elements open), or the input's end.
    fn parse_content_internal(&mut self, sax: &mut dyn Sax, open: &mut Vec<OpenTag>, base: usize) {
        while self.raw() != 0 && self.instate != State::Eof {
            if self.wait_for_more() {
                return;
            }
            let before = (self.input_id(), self.inp().cur, self.inputs.len());
            if self.raw() == b'<' && self.nxt(1) == b'?' {
                self.parse_pi(sax);
            } else if self.starts(b"<![CDATA[") {
                self.parse_cdsect(sax);
            } else if self.starts(b"<!--") {
                self.parse_comment(sax);
                self.instate = State::Content;
            } else if self.raw() == b'<' {
                if self.nxt(1) == b'/' {
                    if open.len() <= base {
                        break;
                    }
                    self.parse_element_end(sax, open);
                } else {
                    self.parse_element_start(sax, open);
                }
            } else if self.raw() == b'&' {
                self.parse_reference(sax);
            } else {
                self.parse_char_data(sax);
            }
            if before == (self.input_id(), self.inp().cur, self.inputs.len()) {
                self.fatal(ERR_INTERNAL_ERROR, Some("detected an error in element content"));
                self.halt();
                break;
            }
            if self.push_stopped() {
                break;
            }
        }
    }

    /// The push parser stops at a fatal error (after the construct), or
    /// at a construct it does not have all of yet.
    pub(crate) fn push_stopped(&self) -> bool {
        self.need_more || (self.options.push && self.err_no != ERR_OK && self.disable_sax)
    }

    /// With more input to come, whether the construct at the cursor is
    /// incomplete (libxml2's push parser waits for its terminator);
    /// stops parsing if so.
    pub(crate) fn wait_for_more(&mut self) -> bool {
        if !self.partial || self.inputs.len() > 1 {
            return false;
        }
        let input = self.inp();
        let rest = &input.buf[input.cur..];
        let find = |needle: &[u8]| rest.windows(needle.len()).any(|w| w == needle);
        let incomplete = if rest.is_empty() {
            true
        } else if rest.starts_with(b"<?") {
            !find(b"?>")
        } else if rest.starts_with(b"<!--") {
            !find(b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            !find(b"]]>")
        } else if rest.starts_with(b"<!DOCTYPE") {
            match rest.iter().position(|&c| c == b'[' || c == b'>') {
                Some(i) if rest[i] == b'[' => !rest[i..].windows(2).any(|w| w[0] == b']') || !rest[i..].contains(&b'>'),
                Some(_) => false,
                None => true,
            }
        } else if rest.starts_with(b"<") {
            // A tag: its '>' outside quoted attribute values.
            let mut quote = 0u8;
            !rest.iter().skip(1).any(|&c| {
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                    false
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                    false
                } else {
                    c == b'>'
                }
            })
        } else if rest.starts_with(b"&") {
            !rest.contains(&b';')
        } else {
            // Text: until the next '<'.
            !rest.contains(&b'<')
        };
        if incomplete {
            self.need_more = true;
        }
        incomplete
    }

    /// The start tag the cursor is in, from its '<' to the cursor (the
    /// '>' or '/' it stopped at): what an expat default handler shows.
    pub(crate) fn start_tag_text(&self) -> Vec<u8> {
        let input = self.inp();
        let end = input.cur.min(input.buf.len().saturating_sub(1));
        let start = input.buf[..=end].iter().rposition(|&c| c == b'<').unwrap_or(0);
        input.buf[start..=end].to_vec()
    }

    /// xmlParseElement.
    pub(crate) fn parse_element(&mut self, sax: &mut dyn Sax) {
        let mut open = Vec::new();
        if self.wait_for_more() {
            return;
        }
        if self.parse_element_start(sax, &mut open) != 0 {
            return;
        }
        let base = open.len();
        self.parse_content_internal(sax, &mut open, base);
        if self.instate == State::Eof || self.push_stopped() {
            return;
        }
        if self.need_more {
            return;
        }
        if self.raw() == 0 && self.options.push {
            // The push parser waits for more; the end of input is
            // reported by the caller.
            self.instate = State::Content;
            self.unfinished = true;
            return;
        }
        if self.raw() == 0 {
            if let Some(tag) = open.last() {
                let message = format!("Premature end of data in tag {} line {}\n", text(&tag.local), tag.line);
                self.fatal_msg(ERR_TAG_NOT_FINISHED, message);
            }
            return;
        }
        self.parse_element_end(sax, &mut open);
    }

    /// characters / ignorableWhitespace for a run of text.
    fn emit_text(&mut self, sax: &mut dyn Sax, s: &[u8], blank_chars: bool) {
        if self.are_blanks(s, blank_chars) {
            sax.ignorable_whitespace(self, s);
        } else {
            sax.characters(self, s);
            if let Some(top) = self.space_stack.last_mut()
                && *top == -1
            {
                *top = -2;
            }
        }
    }

    /// areBlanks: ignorable whitespace (only with keepBlanks off).
    fn are_blanks(&self, s: &[u8], blank_chars: bool) -> bool {
        if self.keep_blanks {
            return false;
        }
        let space = *self.space_stack.last().unwrap_or(&-1);
        if space == 1 || space == -2 {
            return false;
        }
        if !blank_chars && !s.iter().all(|&c| is_blank(u32::from(c))) {
            return false;
        }
        self.raw() == b'<' || self.raw() == 0xD
    }

    /// xmlParseCharData: text up to the next '<' or '&', handed over in
    /// libxml2's pieces (a run of blanks, then each line's text).
    fn parse_char_data(&mut self, sax: &mut dyn Sax) {
        let line = self.line();
        let col = self.column();
        let (mut line, mut col) = (line, col);
        let mut in_pos = self.inp().cur;
        let at = |p: &Parser, i: usize| p.inp().buf.get(i).copied().unwrap_or(0);
        loop {
            // get_more_space
            loop {
                while at(self, in_pos) == 0x20 {
                    in_pos += 1;
                    self.inp_mut().col += 1;
                }
                if at(self, in_pos) == 0xA {
                    while at(self, in_pos) == 0xA {
                        self.inp_mut().line += 1;
                        self.inp_mut().col = 1;
                        in_pos += 1;
                    }
                    continue;
                }
                break;
            }
            if at(self, in_pos) == b'<' {
                let start = self.inp().cur;
                if in_pos > start {
                    let run = self.inp().buf[start..in_pos].to_vec();
                    self.inp_mut().cur = in_pos;
                    self.emit_text(sax, &run, true);
                }
                return;
            }
            // get_more
            loop {
                while is_char_data_byte(at(self, in_pos)) {
                    in_pos += 1;
                    self.inp_mut().col += 1;
                }
                if at(self, in_pos) == 0xA {
                    while at(self, in_pos) == 0xA {
                        self.inp_mut().line += 1;
                        self.inp_mut().col = 1;
                        in_pos += 1;
                    }
                    continue;
                }
                if at(self, in_pos) == b']' {
                    if at(self, in_pos + 1) == b']' && at(self, in_pos + 2) == b'>' {
                        self.fatal(ERR_MISPLACED_CDATA_END, None);
                        self.inp_mut().cur = in_pos + 1;
                        return;
                    }
                    in_pos += 1;
                    self.inp_mut().col += 1;
                    continue;
                }
                break;
            }
            let start = self.inp().cur;
            if in_pos > start {
                let run = self.inp().buf[start..in_pos].to_vec();
                if is_blank(u32::from(run[0])) {
                    self.inp_mut().cur = in_pos;
                    self.emit_text(sax, &run, false);
                } else {
                    sax.characters(self, &run);
                }
                line = self.line();
                col = self.column();
                if self.instate != State::Content {
                    return;
                }
            }
            self.inp_mut().cur = in_pos;
            if at(self, in_pos) == 0xD && at(self, in_pos + 1) == 0xA {
                self.inp_mut().cur = in_pos + 1;
                in_pos += 2;
                self.inp_mut().line += 1;
                self.inp_mut().col = 1;
                continue;
            }
            let c = at(self, in_pos);
            if c == b'<' || c == b'&' {
                return;
            }
            if self.instate == State::Eof {
                return;
            }
            in_pos = self.inp().cur;
            let c = at(self, in_pos);
            if !((0x20..=0x7F).contains(&c) || c == 0x09 || c == 0x0A) {
                break;
            }
        }
        self.inp_mut().line = line;
        self.inp_mut().col = col;
        self.parse_char_data_complex(sax);
    }

    /// xmlParseCharDataComplex: text with non-ASCII characters, handed over
    /// 300 bytes at a time.
    fn parse_char_data_complex(&mut self, sax: &mut dyn Sax) {
        let mut buf = Vec::new();
        let (mut cur, mut l) = self.cur_char();
        while cur != u32::from(b'<') && cur != u32::from(b'&') && is_char(cur) {
            if cur == u32::from(b']') && self.nxt(1) == b']' && self.nxt(2) == b'>' {
                self.fatal(ERR_MISPLACED_CDATA_END, None);
            }
            push_char(&mut buf, cur);
            self.nextl(l);
            (cur, l) = self.cur_char();
            if buf.len() >= 300 {
                if !self.disable_sax {
                    let chunk = std::mem::take(&mut buf);
                    self.emit_text(sax, &chunk, false);
                }
                buf.clear();
                if self.instate != State::Content {
                    return;
                }
            }
        }
        if !buf.is_empty() && !self.disable_sax {
            self.emit_text(sax, &buf, false);
        }
        if cur != 0 && !is_char(cur) {
            self.fatal_msg(ERR_INVALID_CHAR, format!("PCDATA invalid Char value {cur}\n"));
            self.nextl(l);
        }
    }

    // --- references --------------------------------------------------------

    /// xmlParseCharRef: the character, or 0 after an error.
    pub(crate) fn parse_char_ref(&mut self) -> u32 {
        let mut val: u32 = 0;
        if self.raw() == b'&' && self.nxt(1) == b'#' && self.nxt(2) == b'x' {
            self.skip(3);
            let mut count = 0;
            while self.raw() != b';' {
                let c = self.raw();
                if c.is_ascii_digit() {
                    val = val * 16 + u32::from(c - b'0');
                } else if (b'a'..=b'f').contains(&c) && count < 20 {
                    val = val * 16 + u32::from(c - b'a') + 10;
                } else if (b'A'..=b'F').contains(&c) && count < 20 {
                    val = val * 16 + u32::from(c - b'A') + 10;
                } else {
                    self.fatal(ERR_INVALID_HEX_CHARREF, None);
                    val = 0;
                    break;
                }
                val = val.min(0x110000);
                self.next();
                count += 1;
            }
            if self.raw() == b';' {
                self.skip(1);
            }
        } else if self.raw() == b'&' && self.nxt(1) == b'#' {
            self.skip(2);
            while self.raw() != b';' {
                let c = self.raw();
                if c.is_ascii_digit() {
                    val = val * 10 + u32::from(c - b'0');
                } else {
                    self.fatal(ERR_INVALID_DEC_CHARREF, None);
                    val = 0;
                    break;
                }
                val = val.min(0x110000);
                self.next();
            }
            if self.raw() == b';' {
                self.skip(1);
            }
        } else {
            self.fatal(ERR_INVALID_CHARREF, None);
        }
        if val >= 0x110000 {
            self.fatal_msg(ERR_INVALID_CHAR, "xmlParseCharRef: character reference out of bounds\n".into());
        } else if is_char(val) {
            return val;
        } else {
            self.fatal_msg(ERR_INVALID_CHAR, format!("xmlParseCharRef: invalid xmlChar value {val}\n"));
        }
        0
    }

    /// xmlParseEntityRef: the entity `&name;` names (errors for an
    /// undeclared one), using `sax`'s getEntity when given.
    fn parse_entity_ref_in(&mut self, sax: Option<&mut dyn Sax>) -> Option<Ref> {
        if self.instate == State::Eof || self.raw() != b'&' {
            return None;
        }
        self.next();
        let Some(name) = self.parse_name() else {
            self.fatal_msg(ERR_NAME_REQUIRED, "xmlParseEntityRef: no name\n".into());
            return None;
        };
        if self.raw() != b';' {
            self.fatal(ERR_ENTITYREF_SEMICOL_MISSING, None);
            return None;
        }
        self.next();
        if !(self.options.oldsax && sax.is_some())
            && let Some(t) = predefined(&name)
        {
            return Some(Ref::Predefined(t));
        }
        self.nb_entities += 1;
        let ent = match sax {
            Some(sax) => sax.get_entity(self, &name),
            None => self.entities.get(&name).cloned(),
        };
        if let Some(e) = &ent
            && e.kind == EntityKind::Predefined
            && let Some(t) = predefined(&e.name)
        {
            return Some(Ref::Predefined(t));
        }
        if self.instate == State::Eof {
            return None;
        }
        let Some(ent) = ent else {
            if self.standalone == 1 || (!self.has_external_subset && !self.has_pe_refs) {
                self.fatal_msg(ERR_UNDECLARED_ENTITY, format!("Entity '{}' not defined\n", text(&name)));
            } else {
                self.error_msg(WAR_UNDECLARED_ENTITY, format!("Entity '{}' not defined\n", text(&name)));
                self.undeclared_reference = Some(name);
            }
            self.valid = false;
            return None;
        };
        match ent.kind {
            EntityKind::ExternalGeneralUnparsed => {
                self.fatal_msg(ERR_UNPARSED_ENTITY, format!("Entity reference to unparsed entity {}\n", text(&name)));
            }
            EntityKind::ExternalGeneralParsed if self.instate == State::AttributeValue => {
                self.fatal_msg(ERR_ENTITY_IS_EXTERNAL, format!("Attribute references external entity '{}'\n", text(&name)));
            }
            _ if self.instate == State::AttributeValue => {
                if (ent.checked & 1 != 0 || ent.checked == 0) && ent.content.as_ref().is_some_and(|c| c.contains(&b'<')) {
                    self.fatal_msg(ERR_LT_IN_ATTRIBUTE, format!("'<' in entity '{}' is not allowed in attributes values\n", text(&name)));
                }
            }
            EntityKind::InternalParameter | EntityKind::ExternalParameter => {
                self.fatal_msg(ERR_ENTITY_IS_PARAMETER, format!("Attempt to reference the parameter entity '{}'\n", text(&name)));
            }
            _ => {}
        }
        Some(Ref::Entity(ent))
    }

    /// xmlStringDecodeEntities (XML_SUBSTITUTE_REF): character and entity
    /// references in `s` replaced (None after an entity loop).
    pub(crate) fn decode_entities(&mut self, s: &[u8], peref: bool) -> Option<Vec<u8>> {
        if (self.depth > 40 && !self.options.huge) || self.depth > 1024 {
            self.fatal(ERR_ENTITY_LOOP, None);
            return None;
        }
        self.push_input(s.to_vec(), None);
        let mut out = Vec::new();
        let mut ok = true;
        loop {
            let (c, l) = self.cur_char();
            if c == 0 || self.instate == State::Eof || l == 0 {
                break;
            }
            if c == u32::from(b'&') && self.nxt(1) == b'#' {
                let v = self.parse_char_ref();
                if v == 0 {
                    ok = false;
                    break;
                }
                push_char(&mut out, v);
            } else if c == u32::from(b'&') {
                match self.parse_entity_ref_in(None) {
                    Some(Ref::Predefined(t)) => out.push(t[0]),
                    Some(Ref::Entity(ent)) => {
                        if let Some(content) = ent.content.clone() {
                            self.depth += 1;
                            let rep = self.decode_entities(&content, peref);
                            self.depth -= 1;
                            match rep {
                                Some(r) => out.extend_from_slice(&r),
                                None => {
                                    if let Some(e) = self.entities.get_mut(&ent.name) {
                                        e.content = Some(Vec::new());
                                    }
                                }
                            }
                        } else {
                            out.push(b'&');
                            out.extend_from_slice(&ent.name);
                            out.push(b';');
                        }
                    }
                    None => {}
                }
            } else if c == u32::from(b'%') && peref {
                self.next();
                let name = self.parse_name();
                if self.raw() == b';' {
                    self.next();
                }
                if let Some(ent) = name.and_then(|n| self.pentities.get(&n).cloned()) {
                    self.depth += 1;
                    let rep = self.decode_entities(ent.content.as_deref().unwrap_or(b""), peref);
                    self.depth -= 1;
                    out.extend_from_slice(rep.as_deref().unwrap_or(b""));
                }
            } else {
                push_char(&mut out, c);
                self.nextl(l);
            }
        }
        self.pop_input();
        let _ = ok;
        if self.err_no == ERR_ENTITY_LOOP {
            return None;
        }
        Some(out)
    }

    /// xmlParseReference: a reference in content.
    fn parse_reference(&mut self, sax: &mut dyn Sax) {
        if self.raw() != b'&' {
            return;
        }
        if self.nxt(1) == b'#' {
            let value = self.parse_char_ref();
            if value == 0 {
                return;
            }
            let mut out = Vec::new();
            push_char(&mut out, value);
            if !self.disable_sax {
                sax.characters(self, &out);
            }
            return;
        }
        self.undeclared_reference = None;
        let ent = self.parse_entity_ref_in(Some(&mut *sax));
        if let Some(name) = self.undeclared_reference.take()
            && self.in_subset == 0
        {
            sax.reference(self, &name);
        }
        let Some(ent) = ent else { return };
        if !self.well_formed {
            return;
        }
        let ent = match ent {
            Ref::Predefined(t) => {
                if !self.disable_sax {
                    sax.characters(self, t);
                }
                return;
            }
            Ref::Entity(e) => e,
        };
        let name = ent.name.clone();
        if ent.kind != EntityKind::InternalGeneral {
            // External parsed entities are not loaded (no network, no
            // file access from a document): left as a reference.
            if !self.disable_sax && !self.options.replace_entities {
                sax.reference(self, &name);
            }
            return;
        }
        let content = ent.content.clone().unwrap_or_default();
        if self.options.replace_entities {
            // The content's events, in place of the reference.
            self.depth += 1;
            let ret = self.parse_balanced_chunk(sax, &content);
            self.depth -= 1;
            self.after_entity_parse(&name, &content, ret);
            return;
        }
        if ent.checked == 0 {
            // Checked once (its errors reported), not delivered.
            self.depth += 1;
            let ret = self.parse_balanced_chunk(&mut NullSax, &content);
            self.depth -= 1;
            if self.after_entity_parse(&name, &content, ret) {
                return;
            }
        }
        if !self.disable_sax {
            sax.reference(self, &name);
        }
    }

    /// After an entity's content was parsed: its `checked`, the errors
    /// for a loop or a failure. True when parsing must stop here.
    fn after_entity_parse(&mut self, name: &[u8], content: &[u8], ret: i32) -> bool {
        let markup = content.contains(&b'<');
        if let Some(e) = self.entities.get_mut(name) {
            e.checked = 2 | u32::from(markup);
        }
        if ret == ERR_ENTITY_LOOP {
            self.fatal(ERR_ENTITY_LOOP, None);
            self.halt();
            return true;
        }
        if ret != ERR_OK && ret != WAR_UNDECLARED_ENTITY {
            self.fatal_msg(ERR_UNDECLARED_ENTITY, format!("Entity '{}' failed to parse\n", text(name)));
            if let Some(e) = self.entities.get_mut(name) {
                e.content = Some(Vec::new());
            }
        }
        false
    }

    /// xmlParseBalancedChunkMemoryInternal: `content` parsed as element
    /// content, its events to `sax`; libxml2's error code for it.
    pub(crate) fn parse_balanced_chunk(&mut self, sax: &mut dyn Sax, content: &[u8]) -> i32 {
        if (self.depth > 40 && !self.options.huge) || self.depth > 1024 {
            return ERR_ENTITY_LOOP;
        }
        let saved_state = self.instate;
        let saved_wf = self.well_formed;
        let saved_err = self.err_no;
        let saved_disable = self.disable_sax;
        self.well_formed = true;
        self.err_no = 0;
        self.instate = State::Content;
        self.push_input(content.to_vec(), None);
        let depth = self.inputs.len();
        let mut open = Vec::new();
        self.parse_content_internal(sax, &mut open, 0);
        let mut ret = ERR_OK;
        if self.inputs.len() == depth {
            if self.raw() == b'<' && self.nxt(1) == b'/' {
                self.fatal(ERR_NOT_WELL_BALANCED, None);
            } else if self.raw() != 0 {
                self.fatal(ERR_EXTRA_CONTENT, None);
            }
            self.pop_input();
        }
        if !open.is_empty() {
            self.fatal(ERR_NOT_WELL_BALANCED, None);
        }
        if !self.well_formed {
            ret = if self.err_no == 0 { ERR_INTERNAL_ERROR } else { self.err_no };
        }
        if self.instate != State::Eof {
            self.instate = saved_state;
        }
        self.well_formed = saved_wf;
        if ret == ERR_OK {
            self.err_no = saved_err;
        }
        self.disable_sax = saved_disable || self.disable_sax && !saved_wf;
        ret
    }

    // --- the document ------------------------------------------------------

    /// xmlParseDocument: 0 when well formed, -1 otherwise.
    #[allow(dead_code)] // the pull parser DOM and SimpleXML load with
    pub(crate) fn parse_document(&mut self, sax: &mut dyn Sax) -> i32 {
        self.detect_encoding();
        if self.raw() == 0 && self.avail() == 0 {
            self.fatal(ERR_DOCUMENT_EMPTY, None);
            return -1;
        }
        if self.starts(b"<?xml") && is_blank(u32::from(self.nxt(5))) {
            self.parse_xml_decl();
            if self.err_no == ERR_UNSUPPORTED_ENCODING || self.instate == State::Eof {
                return -1;
            }
            self.skip_blanks();
        } else {
            self.version = Some(b"1.0".to_vec());
        }
        if !self.disable_sax {
            sax.start_document(self);
        }
        if self.instate == State::Eof {
            return -1;
        }
        self.instate = State::Misc;
        self.parse_misc(sax);
        if self.starts(b"<!DOCTYPE") {
            self.in_subset = 1;
            self.parse_doctype_decl(sax);
            if self.raw() == b'[' {
                self.instate = State::Dtd;
                self.parse_internal_subset(sax);
                if self.instate == State::Eof {
                    return -1;
                }
            }
            self.in_subset = 2;
            if !self.disable_sax {
                let name = self.int_subset_name.clone().unwrap_or_default();
                let (ext, sys) = (self.ext_sub_system.clone(), self.ext_sub_uri.clone());
                sax.external_subset(self, &name, ext.as_deref(), sys.as_deref());
            }
            if self.instate == State::Eof {
                return -1;
            }
            self.in_subset = 0;
            self.instate = State::Prolog;
            self.parse_misc(sax);
        }
        if self.raw() != b'<' {
            self.fatal_msg(ERR_DOCUMENT_EMPTY, "Start tag expected, '<' not found\n".into());
        } else {
            self.instate = State::Content;
            self.parse_element(sax);
            self.instate = State::Epilog;
            self.parse_misc(sax);
            if self.raw() != 0 || self.avail() > 0 {
                self.fatal(ERR_DOCUMENT_END, None);
            }
            self.instate = State::Eof;
        }
        sax.end_document(self);
        if !self.well_formed {
            self.valid = false;
            return -1;
        }
        0
    }

    /// The push parser over the whole input (xmlParseChunk with
    /// terminate): libxml2's error code for it (0 when none).
    pub(crate) fn parse_push_document(&mut self, sax: &mut dyn Sax) -> i32 {
        self.detect_encoding();
        if self.partial && (self.avail() < 4 || (self.starts(b"<?xml") && self.wait_for_more())) {
            self.need_more = true;
            return self.err_no;
        }
        if self.avail() > 0 {
            if self.starts(b"<?xml") && is_blank(u32::from(self.nxt(5))) {
                self.parse_xml_decl();
                if self.err_no == ERR_UNSUPPORTED_ENCODING || self.instate == State::Eof {
                    return self.err_no;
                }
                self.skip_blanks();
            }
            if !self.disable_sax {
                sax.start_document(self);
            }
            self.instate = State::Misc;
            self.parse_misc(sax);
            if !self.push_stopped() && self.starts(b"<!DOCTYPE") && !self.wait_for_more() {
                self.in_subset = 1;
                self.parse_doctype_decl(sax);
                if self.raw() == b'[' && !self.push_stopped() {
                    self.instate = State::Dtd;
                    self.parse_internal_subset(sax);
                }
                self.in_subset = 0;
                self.instate = State::Prolog;
                if !self.push_stopped() {
                    self.parse_misc(sax);
                }
            }
            if !self.push_stopped() && self.avail() > 0 {
                if self.raw() != b'<' {
                    self.fatal_msg(ERR_DOCUMENT_EMPTY, "Start tag expected, '<' not found\n".into());
                } else {
                    self.instate = State::Content;
                    self.parse_element(sax);
                    if !self.unfinished && !self.push_stopped() && self.instate != State::Eof {
                        self.instate = State::Epilog;
                        self.parse_misc(sax);
                        if self.avail() > 0 && !self.push_stopped() {
                            self.fatal(ERR_DOCUMENT_END, None);
                            self.halt();
                        }
                    }
                }
            }
        }
        if self.push_stopped() || self.partial {
            return self.err_no;
        }
        // xmlParseChunk's terminate.
        if self.instate != State::Eof && self.instate != State::Epilog {
            self.fatal(ERR_DOCUMENT_END, None);
        }
        if self.instate == State::Epilog && self.avail() > 0 {
            self.fatal(ERR_DOCUMENT_END, None);
        }
        if self.instate != State::Eof {
            sax.end_document(self);
        }
        self.instate = State::Eof;
        self.err_no
    }

    /// xmlDetectCharEncoding + xmlSwitchEncoding: a BOM or UTF-16/UCS-4
    /// start converts the input to UTF-8.
    fn detect_encoding(&mut self) {
        let input = &mut self.inputs[0];
        let head: Vec<u8> = input.buf.iter().take(4).copied().collect();
        let dec = match head.as_slice() {
            [0xEF, 0xBB, 0xBF, ..] => {
                input.cur = 3;
                return;
            }
            [0xFE, 0xFF, ..] | [0x00, b'<', 0x00, b'?', ..] => Some(crate::encoding::Decoder::Utf16Be),
            [0xFF, 0xFE, ..] | [b'<', 0x00, b'?', 0x00, ..] => Some(crate::encoding::Decoder::Utf16Le),
            _ => None,
        };
        if let Some(dec) = dec {
            let bom = matches!(head.as_slice(), [0xFE, 0xFF, ..] | [0xFF, 0xFE, ..]);
            let rest = if bom { input.buf[2..].to_vec() } else { input.buf.clone() };
            input.buf = dec.decode(&rest);
            input.cur = 0;
            input.converted = true;
            self.encoding_detected = true;
        }
    }
}

/// libxml2's test_char_data: the ASCII bytes text runs over quickly
/// (not '<', '&', ']', or controls other than tab).
fn is_char_data_byte(c: u8) -> bool {
    c == 0x09 || ((0x20..=0x7F).contains(&c) && c != b'<' && c != b'&' && c != b']')
}

pub(crate) fn qualified(prefix: Option<&[u8]>, name: &[u8]) -> Vec<u8> {
    match prefix {
        Some(p) => [p, b":", name].concat(),
        None => name.to_vec(),
    }
}

/// "p:n" → (Some(p), n); "n" → (None, n).
pub(crate) fn split_qname(q: &[u8]) -> (Option<Vec<u8>>, Vec<u8>) {
    match q.iter().position(|&c| c == b':') {
        Some(i) if i > 0 && i + 1 < q.len() => (Some(q[..i].to_vec()), q[i + 1..].to_vec()),
        _ => (None, q.to_vec()),
    }
}

/// xmlParseURI's verdict, roughly: no spaces or characters a URI cannot hold.
fn looks_like_uri(u: &[u8]) -> bool {
    !u.iter().any(|&c| matches!(c, b' ' | b'"' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}') || c < 0x20)
}

/// A URI scheme ("http:").
fn has_scheme(u: &[u8]) -> bool {
    match u.iter().position(|&c| c == b':') {
        Some(i) if i > 0 => u[0].is_ascii_alphabetic() && u[..i].iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')),
        _ => false,
    }
}

/// A SAX that takes nothing (an entity checked, not delivered).
pub(crate) struct NullSax;

impl Sax for NullSax {}

/// Attribute types (xmlAttributeType).
pub(crate) const ATTRIBUTE_CDATA: i32 = 1;
pub(crate) const ATTRIBUTE_ID: i32 = 2;
pub(crate) const ATTRIBUTE_IDREF: i32 = 3;
pub(crate) const ATTRIBUTE_IDREFS: i32 = 4;
pub(crate) const ATTRIBUTE_ENTITY: i32 = 5;
pub(crate) const ATTRIBUTE_ENTITIES: i32 = 6;
pub(crate) const ATTRIBUTE_NMTOKEN: i32 = 7;
pub(crate) const ATTRIBUTE_NMTOKENS: i32 = 8;
pub(crate) const ATTRIBUTE_ENUMERATION: i32 = 9;
pub(crate) const ATTRIBUTE_NOTATION: i32 = 10;

/// Attribute defaults (xmlAttributeDefault).
pub(crate) const ATTRIBUTE_NONE: i32 = 1;
pub(crate) const ATTRIBUTE_REQUIRED: i32 = 2;
pub(crate) const ATTRIBUTE_IMPLIED: i32 = 3;
pub(crate) const ATTRIBUTE_FIXED: i32 = 4;

/// Element types (xmlElementTypeVal).
pub(crate) const ELEMENT_TYPE_EMPTY: i32 = 1;
pub(crate) const ELEMENT_TYPE_ANY: i32 = 2;
pub(crate) const ELEMENT_TYPE_MIXED: i32 = 3;
pub(crate) const ELEMENT_TYPE_ELEMENT: i32 = 4;

/// An element content model ((a, b)* | #PCDATA ...).
#[derive(Clone, Debug)]
#[allow(dead_code)] // DOMDocumentType::$internalSubset prints it back
pub(crate) enum Content {
    Pcdata,
    Element(Vec<u8>, u8),
    Seq(Vec<Content>, u8),
    Or(Vec<Content>, u8),
}

impl Parser {
    /// xmlParseDocTypeDecl.
    fn parse_doctype_decl(&mut self, sax: &mut dyn Sax) {
        self.skip(9);
        self.skip_blanks();
        let name = self.parse_name();
        if name.is_none() {
            self.fatal_msg(ERR_NAME_REQUIRED, "xmlParseDocTypeDecl : no DOCTYPE name !\n".into());
        }
        self.int_subset_name = name.clone();
        self.skip_blanks();
        let (uri, external_id) = self.parse_external_id(true);
        if uri.is_some() || external_id.is_some() {
            self.has_external_subset = true;
        }
        self.ext_sub_uri = uri.clone();
        self.ext_sub_system = external_id.clone();
        self.skip_blanks();
        if !self.disable_sax {
            sax.internal_subset(self, name.as_deref().unwrap_or(b""), external_id.as_deref(), uri.as_deref());
        }
        if self.instate == State::Eof || self.raw() == b'[' {
            return;
        }
        if self.raw() != b'>' {
            self.fatal(ERR_DOCTYPE_NOT_FINISHED, None);
        }
        self.next();
    }

    /// xmlParseInternalSubset.
    fn parse_internal_subset(&mut self, sax: &mut dyn Sax) {
        if self.raw() == b'[' {
            let base = self.inputs.len();
            self.instate = State::Dtd;
            self.next();
            while (self.raw() != b']' || self.inputs.len() > base) && self.instate != State::Eof {
                let before = (self.input_id(), self.inp().cur);
                self.skip_blanks();
                self.parse_markup_decl(sax);
                self.parse_pe_reference();
                if before == (self.input_id(), self.inp().cur) {
                    self.fatal(ERR_INTERNAL_ERROR, Some("xmlParseInternalSubset: error detected in Markup declaration"));
                    if self.inputs.len() > base {
                        self.pop_input();
                    } else {
                        break;
                    }
                }
            }
            if self.raw() == b']' {
                self.next();
                self.skip_blanks();
            }
        }
        if self.raw() != b'>' {
            self.fatal(ERR_DOCTYPE_NOT_FINISHED, None);
            return;
        }
        self.next();
    }

    /// xmlParseMarkupDecl.
    fn parse_markup_decl(&mut self, sax: &mut dyn Sax) {
        if self.raw() == b'<' {
            if self.nxt(1) == b'!' {
                match self.nxt(2) {
                    b'E' if self.nxt(3) == b'L' => {
                        self.parse_element_decl(sax);
                    }
                    b'E' if self.nxt(3) == b'N' => self.parse_entity_decl(sax),
                    b'A' => self.parse_attribute_list_decl(sax),
                    b'N' => self.parse_notation_decl(sax),
                    b'-' => self.parse_comment(sax),
                    _ => {}
                }
            } else if self.nxt(1) == b'?' {
                self.parse_pi(sax);
            }
        }
        if self.instate == State::Eof {
            return;
        }
        self.instate = State::Dtd;
    }

    /// xmlParseExternalID: (system literal, public id).
    fn parse_external_id(&mut self, strict: bool) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let mut public = None;
        let mut uri = None;
        if self.starts(b"SYSTEM") {
            self.skip(6);
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after 'SYSTEM'\n".into());
            }
            uri = self.parse_system_literal();
            if uri.is_none() {
                self.fatal(ERR_URI_REQUIRED, None);
            }
        } else if self.starts(b"PUBLIC") {
            self.skip(6);
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after 'PUBLIC'\n".into());
            }
            public = self.parse_pubid_literal();
            if public.is_none() {
                self.fatal(ERR_PUBID_REQUIRED, None);
            }
            if strict {
                if self.skip_blanks() == 0 {
                    self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after the Public Identifier\n".into());
                }
            } else {
                if self.skip_blanks() == 0 {
                    return (None, public);
                }
                if self.raw() != b'\'' && self.raw() != b'"' {
                    return (None, public);
                }
            }
            uri = self.parse_system_literal();
            if uri.is_none() {
                self.fatal(ERR_URI_REQUIRED, None);
            }
        }
        (uri, public)
    }

    fn parse_system_literal(&mut self) -> Option<Vec<u8>> {
        let stop = self.raw();
        if stop != b'"' && stop != b'\'' {
            self.fatal(ERR_LITERAL_NOT_STARTED, None);
            return None;
        }
        self.next();
        let state = self.instate;
        self.instate = State::SystemLiteral;
        let mut buf = Vec::new();
        let (mut cur, mut l) = self.cur_char();
        while is_char(cur) && cur != u32::from(stop) {
            push_char(&mut buf, cur);
            self.nextl(l);
            (cur, l) = self.cur_char();
        }
        self.instate = state;
        if !is_char(cur) {
            self.fatal(ERR_LITERAL_NOT_FINISHED, None);
        } else {
            self.next();
        }
        Some(buf)
    }

    fn parse_pubid_literal(&mut self) -> Option<Vec<u8>> {
        let stop = self.raw();
        if stop != b'"' && stop != b'\'' {
            self.fatal(ERR_LITERAL_NOT_STARTED, None);
            return None;
        }
        self.next();
        let old = self.instate;
        self.instate = State::PublicLiteral;
        let mut buf = Vec::new();
        let is_pubid = |c: u8| c.is_ascii_alphanumeric() || b" \r\n-'()+,./:=?;!*#@$_%".contains(&c);
        let mut cur = self.raw();
        while is_pubid(cur) && cur != stop {
            buf.push(cur);
            self.next();
            cur = self.raw();
        }
        if cur != stop {
            self.fatal(ERR_LITERAL_NOT_FINISHED, None);
        } else {
            self.next();
        }
        self.instate = old;
        Some(buf)
    }

    /// xmlParsePEReference: %name; in the DTD, its replacement read next.
    pub(crate) fn parse_pe_reference(&mut self) {
        if self.raw() != b'%' {
            return;
        }
        self.next();
        let Some(name) = self.parse_name() else {
            self.fatal_msg(ERR_PEREF_NO_NAME, "PEReference: no name\n".into());
            return;
        };
        if self.raw() != b';' {
            self.fatal(ERR_PEREF_SEMICOL_MISSING, None);
            return;
        }
        self.next();
        self.nb_entities += 1;
        match self.pentities.get(&name).cloned() {
            None => {
                if self.standalone == 1 || (!self.has_external_subset && !self.has_pe_refs) {
                    self.fatal_msg(ERR_UNDECLARED_ENTITY, format!("PEReference: %{}; not found\n", text(&name)));
                } else {
                    self.warning(WAR_UNDECLARED_ENTITY, format!("PEReference: %{}; not found\n", text(&name)));
                    self.valid = false;
                }
            }
            Some(ent) => {
                if ent.kind == EntityKind::ExternalParameter {
                    // External parameter entities are not loaded.
                    self.has_pe_refs = true;
                    return;
                }
                // The replacement text, padded with a space each side.
                let mut buf = vec![b' '];
                buf.extend_from_slice(ent.content.as_deref().unwrap_or(b""));
                buf.push(b' ');
                self.push_input(buf, Some(name));
            }
        }
        self.has_pe_refs = true;
    }

    /// Store an entity as xmlSAX2EntityDecl does (the first declaration
    /// wins), then tell `sax`.
    fn declare_entity(&mut self, sax: &mut dyn Sax, entity: Entity) {
        let table = match entity.kind {
            EntityKind::InternalParameter | EntityKind::ExternalParameter => &mut self.pentities,
            _ => &mut self.entities,
        };
        if table.contains_key(&entity.name) {
            if entity.kind != EntityKind::InternalParameter && entity.kind != EntityKind::ExternalParameter {
                self.warning(WAR_ENTITY_REDEFINED, format!("Entity({}) already defined in the internal subset\n", text(&entity.name)));
            }
        } else {
            table.insert(entity.name.clone(), entity.clone());
        }
        if !self.disable_sax {
            sax.entity_decl(self, &entity);
        }
    }

    /// xmlParseEntityDecl.
    fn parse_entity_decl(&mut self, sax: &mut dyn Sax) {
        if !self.starts(b"<!ENTITY") {
            return;
        }
        let input_id = self.input_id();
        self.skip(8);
        if self.skip_blanks() == 0 {
            self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after '<!ENTITY'\n".into());
        }
        let mut is_parameter = false;
        if self.raw() == b'%' {
            self.next();
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after '%%'\n".into());
            }
            is_parameter = true;
        }
        let Some(name) = self.parse_name() else {
            self.fatal_msg(ERR_NAME_REQUIRED, "xmlParseEntityDecl: no name\n".into());
            return;
        };
        if name.contains(&b':') {
            self.ns_error(NS_ERR_COLON, format!("colons are forbidden from entities names '{}'\n", text(&name)));
        }
        if self.skip_blanks() == 0 {
            self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after the entity name\n".into());
        }
        self.instate = State::EntityDecl;
        let entity = |kind, external_id, system_id, content, notation| Entity { name: name.clone(), kind, external_id, system_id, content, notation, checked: 0 };
        if self.raw() == b'"' || self.raw() == b'\'' {
            let value = self.parse_entity_value();
            let kind = if is_parameter { EntityKind::InternalParameter } else { EntityKind::InternalGeneral };
            if value.is_some() || !is_parameter {
                let e = entity(kind, None, None, Some(value.unwrap_or_default()), None);
                self.declare_entity(sax, e);
            }
        } else {
            let (uri, literal) = self.parse_external_id(true);
            if uri.is_none() && literal.is_none() {
                self.fatal(ERR_VALUE_REQUIRED, None);
            }
            if let Some(u) = &uri
                && u.contains(&b'#')
            {
                self.fatal(ERR_URI_FRAGMENT, None);
            }
            if is_parameter {
                if uri.is_some() && !uri.as_ref().is_some_and(|u| u.contains(&b'#')) {
                    let e = entity(EntityKind::ExternalParameter, literal.clone(), uri.clone(), None, None);
                    self.declare_entity(sax, e);
                }
            } else {
                if self.raw() != b'>' && self.skip_blanks() == 0 {
                    self.fatal_msg(ERR_SPACE_REQUIRED, "Space required before 'NDATA'\n".into());
                }
                if self.starts(b"NDATA") {
                    self.skip(5);
                    if self.skip_blanks() == 0 {
                        self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after 'NDATA'\n".into());
                    }
                    let ndata = self.parse_name();
                    let e = entity(EntityKind::ExternalGeneralUnparsed, literal.clone(), uri.clone(), None, ndata.clone());
                    if !self.entities.contains_key(&name) {
                        self.entities.insert(name.clone(), e);
                    }
                    if !self.disable_sax {
                        sax.unparsed_entity_decl(self, &name, literal.as_deref(), uri.as_deref(), ndata.as_deref().unwrap_or(b""));
                    }
                } else {
                    let e = entity(EntityKind::ExternalGeneralParsed, literal.clone(), uri.clone(), None, None);
                    self.declare_entity(sax, e);
                }
            }
        }
        if self.instate == State::Eof {
            return;
        }
        self.skip_blanks();
        if self.raw() != b'>' {
            self.fatal_msg(ERR_ENTITY_NOT_FINISHED, format!("xmlParseEntityDecl: entity {} not terminated\n", text(&name)));
            self.halt();
        } else {
            if input_id != self.input_id() {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "Entity declaration doesn't start and stop in the same entity\n".into());
            }
            self.next();
        }
    }

    /// xmlParseEntityValue: the replacement text (parameter entities and
    /// character references replaced, general references kept).
    fn parse_entity_value(&mut self) -> Option<Vec<u8>> {
        let stop = self.raw();
        if stop != b'"' && stop != b'\'' {
            self.fatal(ERR_ENTITY_NOT_STARTED, None);
            return None;
        }
        self.instate = State::EntityValue;
        let input_depth = self.inputs.len();
        self.next();
        let mut buf = Vec::new();
        let (mut c, mut l) = self.cur_char();
        while is_char(c) && (c != u32::from(stop) || self.inputs.len() != input_depth) && self.instate != State::Eof {
            push_char(&mut buf, c);
            self.nextl(l);
            (c, l) = self.cur_char();
        }
        if c != u32::from(stop) {
            self.fatal(ERR_ENTITY_NOT_FINISHED, None);
        }
        self.next();
        // Each '%' or '&' (not '&#') must start a reference.
        let mut i = 0;
        while i < buf.len() {
            let t = buf[i];
            if t == b'%' || (t == b'&' && buf.get(i + 1) != Some(&b'#')) {
                i += 1;
                let start = i;
                while i < buf.len() && (is_name_char(u32::from(buf[i])) || buf[i] >= 0x80) {
                    i += 1;
                }
                let name_ok = i > start && is_name_start_char(u32::from(buf[start])) | (buf[start] >= 0x80);
                if !name_ok || buf.get(i) != Some(&b';') {
                    self.fatal_msg(ERR_ENTITY_CHAR_ERROR, format!("EntityValue: '{}' forbidden except for entities references\n", t as char));
                }
                if t == b'%' && self.in_subset == 1 && self.inputs.len() == 1 {
                    self.fatal(ERR_ENTITY_PE_INTERNAL, None);
                }
                if i >= buf.len() {
                    break;
                }
            }
            i += 1;
        }
        self.depth += 1;
        let ret = self.decode_value(&buf);
        self.depth -= 1;
        ret
    }

    /// xmlStringDecodeEntities with XML_SUBSTITUTE_PEREF: character and
    /// parameter references replaced, general ones kept.
    fn decode_value(&mut self, s: &[u8]) -> Option<Vec<u8>> {
        if (self.depth > 40 && !self.options.huge) || self.depth > 1024 {
            self.fatal(ERR_ENTITY_LOOP, None);
            return None;
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < s.len() {
            if s[i] == b'&' && s.get(i + 1) == Some(&b'#') {
                let end = s[i..].iter().position(|&c| c == b';').map_or(s.len(), |p| i + p);
                let digits = &s[i + 2..end];
                let v = if digits.first() == Some(&b'x') {
                    u32::from_str_radix(std::str::from_utf8(&digits[1..]).unwrap_or(""), 16).ok()
                } else {
                    std::str::from_utf8(digits).ok().and_then(|d| d.parse().ok())
                };
                match v {
                    Some(v) if is_char(v) => push_char(&mut out, v),
                    _ => {
                        self.fatal_msg(ERR_INVALID_CHAR, format!("xmlParseStringCharRef: invalid xmlChar value {}\n", v.unwrap_or(0)));
                        return Some(out);
                    }
                }
                i = end + 1;
            } else if s[i] == b'%' {
                let end = s[i..].iter().position(|&c| c == b';').map_or(s.len(), |p| i + p);
                let name = s[i + 1..end].to_vec();
                i = end + 1;
                match self.pentities.get(&name).cloned() {
                    Some(ent) => {
                        if let Some(content) = ent.content {
                            self.depth += 1;
                            let rep = self.decode_value(&content);
                            self.depth -= 1;
                            out.extend_from_slice(rep.as_deref().unwrap_or(b""));
                        } else {
                            self.warning(ERR_ENTITY_PROCESSING, format!("not validating will not read content for PE entity {}\n", text(&name)));
                        }
                    }
                    None => {
                        if self.standalone == 1 || (!self.has_external_subset && !self.has_pe_refs) {
                            self.fatal_msg(ERR_UNDECLARED_ENTITY, format!("PEReference: %{}; not found\n", text(&name)));
                        } else {
                            self.warning(WAR_UNDECLARED_ENTITY, format!("PEReference: %{}; not found\n", text(&name)));
                        }
                    }
                }
            } else {
                out.push(s[i]);
                i += 1;
            }
        }
        Some(out)
    }

    /// xmlParseAttributeListDecl.
    fn parse_attribute_list_decl(&mut self, sax: &mut dyn Sax) {
        if !self.starts(b"<!ATTLIST") {
            return;
        }
        let input_id = self.input_id();
        self.skip(9);
        if self.skip_blanks() == 0 {
            self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after '<!ATTLIST'\n".into());
        }
        let Some(elem) = self.parse_name() else {
            self.fatal_msg(ERR_NAME_REQUIRED, "ATTLIST: no name for Element\n".into());
            return;
        };
        self.skip_blanks();
        while self.raw() != b'>' && self.instate != State::Eof {
            let Some(attr) = self.parse_name() else {
                self.fatal_msg(ERR_NAME_REQUIRED, "ATTLIST: no name for Attribute\n".into());
                break;
            };
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after the attribute name\n".into());
                break;
            }
            let kind = self.parse_attribute_type();
            if kind <= 0 {
                break;
            }
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after the attribute type\n".into());
                break;
            }
            let (def, mut value) = self.parse_default_decl();
            if def <= 0 {
                break;
            }
            if kind != ATTRIBUTE_CDATA
                && let Some(v) = &value
            {
                value = Some(normalize_space(v));
            }
            if self.raw() != b'>' && self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after the attribute default value\n".into());
                break;
            }
            if !self.disable_sax {
                sax.attribute_decl(self, &elem, &attr, kind, def, value.as_deref());
            }
            if let Some(v) = &value
                && def != ATTRIBUTE_IMPLIED
                && def != ATTRIBUTE_REQUIRED
            {
                let list = self.atts_default.entry(elem.clone()).or_default();
                if !list.iter().any(|(a, _)| *a == attr) {
                    list.push((attr.clone(), v.clone()));
                }
            }
            if kind != ATTRIBUTE_CDATA {
                self.atts_special.entry((elem.clone(), attr.clone())).or_insert(kind);
            }
        }
        if self.raw() == b'>' {
            if input_id != self.input_id() {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "Attribute list declaration doesn't start and stop in the same entity\n".into());
            }
            self.next();
        }
    }

    fn parse_attribute_type(&mut self) -> i32 {
        for (word, kind) in [
            (&b"CDATA"[..], ATTRIBUTE_CDATA),
            (b"IDREFS", ATTRIBUTE_IDREFS),
            (b"IDREF", ATTRIBUTE_IDREF),
            (b"ID", ATTRIBUTE_ID),
            (b"ENTITY", ATTRIBUTE_ENTITY),
            (b"ENTITIES", ATTRIBUTE_ENTITIES),
            (b"NMTOKENS", ATTRIBUTE_NMTOKENS),
            (b"NMTOKEN", ATTRIBUTE_NMTOKEN),
        ] {
            if self.starts(word) && !(word == b"ENTITY" && self.starts(b"ENTITIES")) {
                self.skip(word.len());
                return kind;
            }
        }
        if self.starts(b"NOTATION") {
            self.skip(8);
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after 'NOTATION'\n".into());
                return 0;
            }
            return if self.parse_enumeration(true) { ATTRIBUTE_NOTATION } else { 0 };
        }
        if self.parse_enumeration(false) { ATTRIBUTE_ENUMERATION } else { 0 }
    }

    /// xmlParseEnumerationType / xmlParseNotationType.
    fn parse_enumeration(&mut self, notation: bool) -> bool {
        if self.raw() != b'(' {
            self.fatal(if notation { ERR_NOTATION_NOT_STARTED } else { ERR_ATTLIST_NOT_STARTED }, None);
            return false;
        }
        let mut seen: Vec<Vec<u8>> = Vec::new();
        loop {
            self.next();
            self.skip_blanks();
            let name = if notation { self.parse_name() } else { self.parse_nmtoken() };
            let Some(name) = name else {
                if notation {
                    self.fatal_msg(ERR_NAME_REQUIRED, "Name expected in NOTATION declaration\n".into());
                } else {
                    self.fatal(ERR_NMTOKEN_REQUIRED, None);
                }
                return false;
            };
            if seen.contains(&name) {
                let what = if notation { "notation" } else { "enumeration" };
                self.validity_error(DTD_DUP_TOKEN, format!("standalone: attribute {what} value token {} duplicated\n", text(&name)));
            } else {
                seen.push(name);
            }
            self.skip_blanks();
            if self.raw() != b'|' {
                break;
            }
        }
        if self.raw() != b')' {
            self.fatal(if notation { ERR_NOTATION_NOT_FINISHED } else { ERR_ATTLIST_NOT_FINISHED }, None);
            return false;
        }
        self.next();
        true
    }

    /// xmlParseDefaultDecl: (default kind, value).
    fn parse_default_decl(&mut self) -> (i32, Option<Vec<u8>>) {
        if self.starts(b"#REQUIRED") {
            self.skip(9);
            return (ATTRIBUTE_REQUIRED, None);
        }
        if self.starts(b"#IMPLIED") {
            self.skip(8);
            return (ATTRIBUTE_IMPLIED, None);
        }
        let mut val = ATTRIBUTE_NONE;
        if self.starts(b"#FIXED") {
            self.skip(6);
            val = ATTRIBUTE_FIXED;
            if self.skip_blanks() == 0 {
                self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after '#FIXED'\n".into());
            }
        }
        let ret = self.parse_att_value(false);
        self.instate = State::Dtd;
        if ret.is_none() {
            let code = self.err_no;
            self.fatal_msg(code, "Attribute default value declaration error\n".into());
        }
        (val, ret)
    }

    /// xmlParseElementDecl: the element type (-1 after an error).
    fn parse_element_decl(&mut self, sax: &mut dyn Sax) -> i32 {
        if !self.starts(b"<!ELEMENT") {
            return -1;
        }
        let input_id = self.input_id();
        self.skip(9);
        if self.skip_blanks() == 0 {
            self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after 'ELEMENT'\n".into());
            return -1;
        }
        let Some(name) = self.parse_name() else {
            self.fatal_msg(ERR_NAME_REQUIRED, "xmlParseElementDecl: no name for Element\n".into());
            return -1;
        };
        if self.skip_blanks() == 0 {
            self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after the element name\n".into());
        }
        let mut content = None;
        let ret = if self.starts(b"EMPTY") {
            self.skip(5);
            ELEMENT_TYPE_EMPTY
        } else if self.starts(b"ANY") {
            self.skip(3);
            ELEMENT_TYPE_ANY
        } else if self.raw() == b'(' {
            let input_id = self.input_id();
            self.next();
            self.skip_blanks();
            if self.starts(b"#PCDATA") {
                content = self.parse_mixed_content_decl(input_id);
                ELEMENT_TYPE_MIXED
            } else {
                content = self.parse_children_content_decl(input_id, 1);
                ELEMENT_TYPE_ELEMENT
            }
        } else {
            if self.raw() == b'%' && self.inputs.len() == 1 {
                self.fatal_msg(ERR_PEREF_IN_INT_SUBSET, "PEReference: forbidden within markup decl in internal subset\n".into());
            } else {
                self.fatal_msg(ERR_ELEMCONTENT_NOT_STARTED, "xmlParseElementDecl: 'EMPTY', 'ANY' or '(' expected\n".into());
            }
            return -1;
        };
        self.skip_blanks();
        if self.raw() != b'>' {
            self.fatal(ERR_GT_REQUIRED, None);
        } else {
            if input_id != self.input_id() {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "Element declaration doesn't start and stop in the same entity\n".into());
            }
            self.next();
            if !self.disable_sax {
                self.element_decls.insert(name.clone(), (ret, content.clone()));
                sax.element_decl(self, &name, ret);
            }
        }
        ret
    }

    fn parse_mixed_content_decl(&mut self, input_id: u32) -> Option<Content> {
        self.skip(7);
        self.skip_blanks();
        if self.raw() == b')' {
            if self.input_id() != input_id {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "Element content declaration doesn't start and stop in the same entity\n".into());
            }
            self.next();
            let occur = if self.raw() == b'*' {
                self.next();
                b'*'
            } else {
                0
            };
            return Some(Content::Or(vec![Content::Pcdata], occur));
        }
        let mut items = vec![Content::Pcdata];
        while self.raw() == b'|' && self.instate != State::Eof {
            self.next();
            self.skip_blanks();
            let Some(elem) = self.parse_name() else {
                self.fatal_msg(ERR_NAME_REQUIRED, "xmlParseElementMixedContentDecl : Name expected\n".into());
                return None;
            };
            items.push(Content::Element(elem, 0));
            self.skip_blanks();
        }
        if self.raw() == b')' && self.nxt(1) == b'*' {
            if self.input_id() != input_id {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "Element content declaration doesn't start and stop in the same entity\n".into());
            }
            self.skip(2);
            Some(Content::Or(items, b'*'))
        } else {
            self.fatal(ERR_MIXED_NOT_STARTED, None);
            None
        }
    }

    fn occurrence(&mut self) -> u8 {
        let c = self.raw();
        if matches!(c, b'?' | b'*' | b'+') {
            self.next();
            c
        } else {
            0
        }
    }

    /// xmlParseElementChildrenContentDeclPriv.
    fn parse_children_content_decl(&mut self, input_id: u32, depth: u32) -> Option<Content> {
        if (depth > 128 && !self.options.huge) || depth > 2048 {
            self.fatal_msg(ERR_ELEMCONTENT_NOT_FINISHED, format!("xmlParseElementChildrenContentDecl : depth {depth} too deep, use XML_PARSE_HUGE\n"));
            return None;
        }
        self.skip_blanks();
        let first = if self.raw() == b'(' {
            let inner = self.input_id();
            self.next();
            self.skip_blanks();
            let c = self.parse_children_content_decl(inner, depth + 1)?;
            self.skip_blanks();
            c
        } else {
            let Some(elem) = self.parse_name() else {
                self.fatal(ERR_ELEMCONTENT_NOT_STARTED, None);
                return None;
            };
            let occ = self.occurrence();
            Content::Element(elem, occ)
        };
        self.skip_blanks();
        let mut items = vec![first];
        let mut sep = 0u8;
        while self.raw() != b')' && self.instate != State::Eof {
            let c = self.raw();
            if c == b',' || c == b'|' {
                if sep == 0 {
                    sep = c;
                } else if sep != c {
                    self.fatal_msg(ERR_SEPARATOR_REQUIRED, format!("xmlParseElementChildrenContentDecl : '{}' expected\n", sep as char));
                    return None;
                }
                self.next();
            } else {
                self.fatal(ERR_ELEMCONTENT_NOT_FINISHED, None);
                return None;
            }
            self.skip_blanks();
            if self.raw() == b'(' {
                let inner = self.input_id();
                self.next();
                self.skip_blanks();
                let c = self.parse_children_content_decl(inner, depth + 1)?;
                items.push(c);
                self.skip_blanks();
            } else {
                let Some(elem) = self.parse_name() else {
                    self.fatal(ERR_ELEMCONTENT_NOT_STARTED, None);
                    return None;
                };
                let occ = self.occurrence();
                items.push(Content::Element(elem, occ));
            }
            self.skip_blanks();
        }
        if self.input_id() != input_id {
            self.fatal_msg(ERR_ENTITY_BOUNDARY, "Element content declaration doesn't start and stop in the same entity\n".into());
        }
        self.next();
        let occ = self.occurrence();
        Some(if sep == b'|' { Content::Or(items, occ) } else { Content::Seq(items, occ) })
    }

    /// xmlParseNotationDecl.
    fn parse_notation_decl(&mut self, sax: &mut dyn Sax) {
        if !self.starts(b"<!NOTATION") {
            return;
        }
        let input_id = self.input_id();
        self.skip(10);
        if self.skip_blanks() == 0 {
            self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after '<!NOTATION'\n".into());
            return;
        }
        let Some(name) = self.parse_name() else {
            self.fatal(ERR_NOTATION_NOT_STARTED, None);
            return;
        };
        if name.contains(&b':') {
            self.ns_error(NS_ERR_COLON, format!("colons are forbidden from notation names '{}'\n", text(&name)));
        }
        if self.skip_blanks() == 0 {
            self.fatal_msg(ERR_SPACE_REQUIRED, "Space required after the NOTATION name'\n".into());
            return;
        }
        let (system, public) = self.parse_external_id(false);
        self.skip_blanks();
        if self.raw() == b'>' {
            if input_id != self.input_id() {
                self.fatal_msg(ERR_ENTITY_BOUNDARY, "Notation declaration doesn't start and stop in the same entity\n".into());
            }
            self.next();
            if !self.disable_sax {
                sax.notation_decl(self, &name, public.as_deref(), system.as_deref());
            }
        } else {
            self.fatal(ERR_NOTATION_NOT_FINISHED, None);
        }
    }
}

/// xmlAttrNormalizeSpace: leading and trailing blanks off, inner runs one space.
pub(crate) fn normalize_space(v: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut space = false;
    for &c in v.iter().skip_while(|&&c| c == 0x20) {
        if c == 0x20 {
            space = true;
        } else {
            if space {
                out.push(0x20);
            }
            space = false;
            out.push(c);
        }
    }
    out
}

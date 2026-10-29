//! DOMDocument::xinclude(): XInclude 1.0 processing as libxml2 2.9.14
//! does it, with PHP's XML_PARSE_NOXINCNODE (no XINCLUDE_START/END nodes).
//!
//! Reference: libxml2 xinclude.c (xmlXIncludeProcessFlags,
//! xmlXIncludeLoadNode, xmlXIncludeBaseFixup), php-src ext/dom/document.c

use super::build;
use super::tree::{DOC, Doc, Id, Kind, Node};
use crate::encoding::{Decoder, decoder};
use crate::parser::{LEVEL_ERROR, LEVEL_WARNING, XML_NS, XmlError};
use crate::xpath::{Eval, XValue, compile};

const XI_2001: &[u8] = b"http://www.w3.org/2001/XInclude";
const XI_2003: &[u8] = b"http://www.w3.org/2003/XInclude";

/// XML_PARSE_NOBASEFIX (no xml:base on included elements).
const NOBASEFIX: i64 = 1 << 18;
/// libxml2's include depth limit (MAX_INCLUDE_DEPTH-ish: 40 nested files).
const MAX_DEPTH: usize = 40;

const FROM_IO: i32 = 8;
const FROM_XINCLUDE: i32 = 11;

const RECURSION: i32 = 1600;
const PARSE_VALUE: i32 = 1601;
const NO_FALLBACK: i32 = 1604;
const INVALID_CHAR: i32 = 1608;
const UNKNOWN_ENCODING: i32 = 1610;
const XPTR_FAILED: i32 = 1612;
const INCLUDE_IN_INCLUDE: i32 = 1614;
const FALLBACKS_IN_INCLUDE: i32 = 1615;
const FALLBACK_NOT_IN_INCLUDE: i32 = 1616;
const IO_LOAD_ERROR: i32 = 1549;

/// One libxml2 error and the PHP warnings it makes (one per line the
/// generic handler printed).
pub struct Diag {
    pub error: XmlError,
    pub warnings: Vec<String>,
}

/// The document's URL for xinclude: its documentURI, or (for a document
/// loaded from a string) the working directory, as PHP's DOM sets it.
fn doc_url(d: &Doc) -> String {
    match &d.settings.url {
        Some(u) if !u.is_empty() => String::from_utf8_lossy(u).into_owned(),
        _ => {
            let mut cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
            if !cwd.ends_with('/') {
                cwd.push('/');
            }
            cwd
        }
    }
}

/// Substitutes the XIncludes of `d`: the number of them done, 0 for none,
/// -1 for an error (the document keeps what was substituted before it).
pub fn run(d: &mut Doc, opts: i64) -> (i64, Vec<Diag>) {
    let url = doc_url(d);
    let Some(root) = d.root() else { return (-1, Vec::new()) };
    let mut xi = Xi { opts, diags: Vec::new(), stack: vec![url.clone()], level: 0, counted: 0 };
    let done = xi.process(d, &url, vec![root]);
    let result = if xi.counted > 0 { -1 } else { done as i64 };
    (result, xi.diags)
}

struct Xi {
    opts: i64,
    diags: Vec<Diag>,
    /// The URLs being included, outermost first.
    stack: Vec<String>,
    /// How deep in included documents and fallbacks: only the top level's
    /// errors make xinclude() return -1 (libxml2 counts them there).
    level: usize,
    counted: usize,
}

fn is_xi(d: &Doc, id: Id, name: &[u8]) -> bool {
    let n = d.node(id);
    n.kind == Kind::Element && n.name == name && n.ns.as_deref().is_some_and(|ns| ns == XI_2001 || ns == XI_2003)
}

/// An attribute of the include element: unqualified, or in an XInclude namespace.
fn prop(d: &Doc, el: Id, name: &[u8]) -> Option<String> {
    d.node(el)
        .attrs
        .iter()
        .map(|&a| d.node(a))
        .find(|a| a.name == name && a.ns.as_deref().is_none_or(|ns| ns == XI_2001 || ns == XI_2003))
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

fn xml_base_attr(d: &Doc, el: Id) -> Option<Id> {
    d.attr_ns(el, Some(XML_NS), b"base").or_else(|| d.attr(el, b"xml:base"))
}

/// xmlNodeGetBase: the xml:base chain of `node`'s ancestors resolved
/// against the document's URL.
fn node_base(d: &Doc, url: &str, node: Id) -> String {
    let mut base: Option<String> = None;
    let mut at = Some(node);
    while let Some(n) = at {
        if d.node(n).kind == Kind::Element
            && let Some(a) = xml_base_attr(d, n)
        {
            let b = String::from_utf8_lossy(&d.node(a).value).into_owned();
            base = Some(match base {
                Some(old) => resolve(&old, &b),
                None => b,
            });
        }
        at = d.parent(n);
    }
    match base {
        Some(b) if !url.is_empty() => resolve(&b, url),
        Some(b) => b,
        None => url.to_string(),
    }
}

impl Xi {
    fn error(&mut self, d: &Doc, url: &str, node: Id, code: i32, message: String) {
        let error = XmlError {
            domain: FROM_XINCLUDE,
            code,
            level: LEVEL_ERROR,
            message: format!("{message}\n"),
            line: d.node(node).line as i32,
            column: 0,
            file: Some(url.to_string()),
            context: Vec::new(),
            context_col: 0,
        };
        self.diags.push(Diag { error, warnings: vec![message] });
        if self.level == 0 {
            self.counted += 1;
        }
    }

    fn io_error(&mut self, url: &str) {
        let message = format!("failed to load external entity \"{url}\"");
        let error = XmlError {
            domain: FROM_IO,
            code: IO_LOAD_ERROR,
            level: LEVEL_WARNING,
            message: format!("{message}\n"),
            line: 0,
            column: 0,
            file: None,
            context: Vec::new(),
            context_col: 0,
        };
        self.diags.push(Diag { error, warnings: vec![format!("I/O warning : {message}")] });
    }

    fn parse_error(&mut self, e: &XmlError) {
        let mut lines = e.report_lines();
        if let Some(file) = &e.file {
            lines[0] = format!("{file}:{}: {}", e.line, lines[0].splitn(3, ": ").nth(2).unwrap_or_default());
        }
        self.diags.push(Diag { error: e.clone(), warnings: lines });
    }

    /// The includes under `start` replaced; how many.
    fn process(&mut self, d: &mut Doc, url: &str, start: Vec<Id>) -> usize {
        let mut found = Vec::new();
        for s in start {
            self.collect(d, url, s, &mut found);
        }
        let mut done = 0;
        for inc in found {
            if self.load(d, url, inc) {
                done += 1;
            }
        }
        done
    }

    fn collect(&mut self, d: &Doc, url: &str, node: Id, out: &mut Vec<Id>) {
        if d.node(node).kind != Kind::Element {
            return;
        }
        if is_xi(d, node, b"include") {
            out.push(node);
        } else if is_xi(d, node, b"fallback") {
            self.error(d, url, node, FALLBACK_NOT_IN_INCLUDE, "fallback is not the child of an 'include'".into());
        } else {
            for c in d.node(node).children.clone() {
                self.collect(d, url, c, out);
            }
        }
    }

    /// One xi:include: replaced by what it names or its fallback (true), or
    /// left in place after an error (false).
    fn load(&mut self, d: &mut Doc, url: &str, inc: Id) -> bool {
        let parse = prop(d, inc, b"parse");
        let text = match parse.as_deref() {
            None | Some("xml") => false,
            Some("text") => true,
            Some(other) => {
                self.error(d, url, inc, PARSE_VALUE, format!("invalid value {other} for 'parse'"));
                return false;
            }
        };
        let base = node_base(d, url, inc);
        let full = resolve(&prop(d, inc, b"href").unwrap_or_default(), &base);
        let (target, fragment) = match full.find('#') {
            Some(i) => (full[..i].to_string(), Some(full[i + 1..].to_string())),
            None => (full.clone(), None),
        };
        let pointer = prop(d, inc, b"xpointer").or(fragment.filter(|f| !f.is_empty()));

        if d.node(inc).children.iter().any(|&c| is_xi(d, c, b"include")) {
            self.error(d, url, inc, INCLUDE_IN_INCLUDE, "include has an 'include' child".into());
            let kids = d.node(inc).children.clone();
            self.level += 1;
            self.process(d, url, kids);
            self.level -= 1;
            return false;
        }
        if d.node(inc).children.iter().filter(|&&c| is_xi(d, c, b"fallback")).count() > 1 {
            self.error(d, url, inc, FALLBACKS_IN_INCLUDE, "include has multiple fallback children".into());
            return false;
        }
        if pointer.is_none() && target == url {
            self.error(d, url, inc, RECURSION, format!("detected a local recursion with no xpointer in {url}"));
            return false;
        }
        let own = target == url;
        if !own && (self.stack.iter().any(|u| *u == target) || self.stack.len() > MAX_DEPTH) {
            self.error(d, url, inc, RECURSION, format!("detected a recursion in {target}"));
            return false;
        }

        let loaded = if text { self.load_text(d, url, inc, &target) } else { self.load_xml(d, url, inc, &target, own, pointer.as_deref(), &base) };
        match loaded {
            Some(nodes) => {
                self.replace(d, inc, nodes);
                true
            }
            None => self.fallback(d, url, inc, &full),
        }
    }

    /// parse="text": one text node, None when it could not be read.
    fn load_text(&mut self, d: &mut Doc, url: &str, inc: Id, target: &str) -> Option<Vec<Id>> {
        let Some(raw) = read_url(target) else {
            self.io_error(target);
            return None;
        };
        let content = match prop(d, inc, b"encoding") {
            None => raw,
            Some(name) => match text_decoder(&name) {
                Some(Some(dec)) => dec.decode(&raw),
                Some(None) => raw,
                None => {
                    self.error(d, url, inc, UNKNOWN_ENCODING, format!("encoding {name} not supported"));
                    return None;
                }
            },
        };
        if let Ok(s) = std::str::from_utf8(&content)
            && s.chars().any(|c| (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r'))
        {
            self.error(d, url, inc, INVALID_CHAR, format!("{target} contains invalid char"));
            return None;
        }
        if content.is_empty() {
            return Some(Vec::new());
        }
        let mut n = Node::new(Kind::Text, Vec::new());
        n.value = content;
        Some(vec![d.add(n)])
    }

    /// parse="xml": copies of the nodes the include selects, in `d`.
    fn load_xml(&mut self, d: &mut Doc, url: &str, inc: Id, target: &str, own: bool, pointer: Option<&str>, base: &str) -> Option<Vec<Id>> {
        let src: Doc;
        let src_url: &str;
        if own {
            src = d.clone_tree();
            src_url = url;
        } else {
            let Some(raw) = read_url(target) else {
                self.io_error(target);
                return None;
            };
            let mut doc = Doc::new(d.class.clone());
            let flags = self.opts & (build::NOENT | build::NOBLANKS | build::NOCDATA | build::HUGE | build::RECOVER);
            let parser = build::parse(&mut doc, raw, Some(target.to_string()), flags);
            for e in &parser.errors {
                self.parse_error(e);
            }
            if !parser.well_formed && flags & build::RECOVER == 0 {
                return None;
            }
            doc.settings.url = Some(target.as_bytes().to_vec());
            self.stack.push(target.to_string());
            self.level += 1;
            if let Some(root) = doc.root() {
                self.process(&mut doc, target, vec![root]);
            }
            self.level -= 1;
            self.stack.pop();
            src = doc;
            src_url = target;
        }

        let picked: Vec<Id> = match pointer {
            Some(p) => match pointer_nodes(&src, p) {
                Ok(v) => v,
                Err(report) => {
                    if report {
                        self.error(d, url, inc, XPTR_FAILED, format!("XPointer evaluation failed: #{p}"));
                    }
                    return None;
                }
            },
            None => src.node(DOC).children.iter().copied().filter(|&c| src.node(c).kind != Kind::DocType).collect(),
        };

        let own_base = xml_base_attr(d, inc).map(|a| d.node(a).value.clone());
        let mut out = Vec::new();
        for sid in picked {
            if !matches!(src.node(sid).kind, Kind::Element | Kind::Text | Kind::CData | Kind::Comment | Kind::Pi) {
                continue;
            }
            let copy = d.copy_from(&src, sid, true);
            if src.node(sid).kind == Kind::Element && self.opts & NOBASEFIX == 0 {
                let value = match &own_base {
                    Some(v) => Some(v.clone()),
                    None => {
                        let rel = relative(&node_base(&src, src_url, sid), base);
                        rel.contains('/').then(|| rel.into_bytes())
                    }
                };
                if let Some(v) = value {
                    set_xml_base(d, copy, v);
                }
            }
            out.push(copy);
        }
        Some(out)
    }

    /// The include's xi:fallback in its place, or the error saying it has none.
    fn fallback(&mut self, d: &mut Doc, url: &str, inc: Id, full: &str) -> bool {
        let fallback = d.node(inc).children.iter().copied().find(|&c| is_xi(d, c, b"fallback"));
        let Some(fb) = fallback else {
            self.error(d, url, inc, NO_FALLBACK, format!("could not load {full}, and no fallback was found"));
            return false;
        };
        let inner = d.node(fb).children.clone();
        self.level += 1;
        self.process(d, url, inner);
        self.level -= 1;
        let kids = d.node(fb).children.clone();
        self.replace(d, inc, kids);
        true
    }

    /// `nodes` in place of the include.
    fn replace(&mut self, d: &mut Doc, inc: Id, nodes: Vec<Id>) {
        let Some(parent) = d.parent(inc) else { return };
        for &n in &nodes {
            d.insert(parent, n, Some(inc));
        }
        d.detach(inc);
        for n in nodes {
            reconcile(d, n);
        }
    }
}

fn set_xml_base(d: &mut Doc, el: Id, value: Vec<u8>) {
    if let Some(a) = xml_base_attr(d, el) {
        d.node_mut(a).value = value;
        return;
    }
    let mut a = Node::new(Kind::Attribute, b"base".to_vec());
    a.prefix = Some(b"xml".to_vec());
    a.ns = Some(XML_NS.to_vec());
    a.value = value;
    let id = d.add(a);
    d.set_attr(el, id);
}

// ---- namespaces of copied nodes ----

/// xmlStaticCopyNode's namespace fix: a copy is made before it has a
/// parent, so a namespace its elements use that is declared above them in
/// the source is declared again on the copy's top element.
fn reconcile(d: &mut Doc, top: Id) {
    if d.node(top).kind != Kind::Element {
        return;
    }
    let mut missing: Vec<(Option<Vec<u8>>, Vec<u8>)> = Vec::new();
    let mut declared: Vec<(Option<Vec<u8>>, Vec<u8>)> = Vec::new();
    needs(d, top, &mut declared, &mut missing);
    d.node_mut(top).ns_defs.extend(missing);
}

fn needs(d: &Doc, el: Id, declared: &mut Vec<(Option<Vec<u8>>, Vec<u8>)>, missing: &mut Vec<(Option<Vec<u8>>, Vec<u8>)>) {
    let mark = declared.len();
    declared.extend(d.node(el).ns_defs.iter().cloned());
    let mut uses = vec![(d.node(el).prefix.clone(), d.node(el).ns.clone())];
    for &a in &d.node(el).attrs {
        let a = d.node(a);
        if a.prefix.is_some() {
            uses.push((a.prefix.clone(), a.ns.clone()));
        }
    }
    for (prefix, ns) in uses {
        let Some(uri) = ns else { continue };
        if prefix.as_deref() == Some(b"xml") || uri == XML_NS {
            continue;
        }
        let seen = declared.iter().rev().find(|(p, _)| *p == prefix).map(|(_, u)| u.clone()).or_else(|| missing.iter().find(|(p, _)| *p == prefix).map(|(_, u)| u.clone()));
        if seen.is_none() {
            missing.push((prefix, uri));
        }
    }
    for c in d.node(el).children.clone() {
        if d.node(c).kind == Kind::Element {
            needs(d, c, declared, missing);
        }
    }
    declared.truncate(mark);
}

// ---- reading ----

/// The bytes a URL names: a local file (plain path, file:, or PHP's
/// compress.zlib: wrapper), None for what libxml2 could not load.
fn read_url(url: &str) -> Option<Vec<u8>> {
    let path = match split(url).scheme {
        None => url.to_string(),
        Some(s) if s.eq_ignore_ascii_case("file") => {
            let rest = url[s.len() + 1..].strip_prefix("//").map(|r| r.strip_prefix("localhost").unwrap_or(r)).unwrap_or(&url[s.len() + 1..]);
            percent_decode(rest)
        }
        Some("compress.zlib") => url["compress.zlib://".len()..].to_string(),
        Some(_) => return None,
    };
    std::fs::read(path).ok()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The encodings xmlParseCharEncoding names and libxml2 can convert:
/// Some(None) for UTF-8 (no conversion), None for unsupported.
fn text_decoder(name: &str) -> Option<Option<Decoder>> {
    let upper = name.to_ascii_uppercase();
    match upper.as_str() {
        "UTF-8" | "UTF8" => Some(None),
        "UTF-16" | "UTF16" => decoder(b"UTF-16LE").map(Some),
        "ISO-8859-1" | "ISO-LATIN-1" | "ISO LATIN-1" => decoder(b"ISO-8859-1").map(Some),
        n if n.strip_prefix("ISO-8859-").is_some_and(|d| matches!(d, "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")) => decoder(upper.as_bytes()).map(Some),
        _ => None,
    }
}

// ---- XPointer ----

/// The nodes an xpointer attribute selects in `doc`: Err(true) when the
/// pointer is bad or selects nothing (libxml2 reports it), Err(false) when
/// a bare name names no element (it does not).
fn pointer_nodes(doc: &Doc, pointer: &str) -> Result<Vec<Id>, bool> {
    let pointer = pointer.trim();
    if !pointer.contains('(') {
        let mut all = Vec::new();
        doc.descendants(DOC, &mut all);
        return all
            .into_iter()
            .find(|&e| doc.node(e).kind == Kind::Element && doc.attr_ns(e, Some(XML_NS), b"id").is_some_and(|a| doc.node(a).value == pointer.as_bytes()))
            .map(|e| vec![e])
            .ok_or(false);
    }
    let mut ns: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for (scheme, data) in parts(pointer).ok_or(true)? {
        match scheme.as_str() {
            "xmlns" => {
                let (prefix, uri) = data.split_once('=').ok_or(true)?;
                ns.push((prefix.trim().as_bytes().to_vec(), uri.trim().as_bytes().to_vec()));
            }
            "xpointer" | "xpath1" => {
                let compiled = compile(&data).map_err(|_| true)?;
                let at = doc.root().unwrap_or(DOC);
                if let Ok(XValue::Nodes(v)) = Eval::new(doc, &ns, None).run(&compiled, at)
                    && !v.is_empty()
                {
                    return Ok(v);
                }
            }
            "element" => {
                if let Some(e) = child_sequence(doc, &data) {
                    return Ok(vec![e]);
                }
            }
            _ => {}
        }
    }
    Err(true)
}

/// A pointer's parts: scheme(data) with ^ escaping ( ) ^ in the data.
fn parts(s: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        chars.peek()?;
        let mut scheme = String::new();
        while let Some(&c) = chars.peek() {
            if c == '(' {
                break;
            }
            scheme.push(c);
            chars.next();
        }
        chars.next()?;
        let mut data = String::new();
        let mut depth = 1;
        loop {
            match chars.next()? {
                '^' => data.push(chars.next()?),
                '(' => {
                    depth += 1;
                    data.push('(');
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    data.push(')');
                }
                c => data.push(c),
            }
        }
        out.push((scheme.trim().to_string(), data));
        if chars.peek().is_none() {
            return Some(out);
        }
    }
}

/// element(/1/2/3) or element(id/1/2): the element a child sequence reaches.
fn child_sequence(doc: &Doc, data: &str) -> Option<Id> {
    let mut steps = data.split('/');
    let first = steps.next()?;
    let mut at = if first.is_empty() {
        DOC
    } else {
        let mut all = Vec::new();
        doc.descendants(DOC, &mut all);
        all.into_iter().find(|&e| doc.node(e).kind == Kind::Element && doc.attr_ns(e, Some(XML_NS), b"id").is_some_and(|a| doc.node(a).value == first.as_bytes()))?
    };
    for step in steps {
        let n: usize = step.parse().ok().filter(|&n| n > 0)?;
        at = doc.node(at).children.iter().copied().filter(|&c| doc.node(c).kind == Kind::Element).nth(n - 1)?;
    }
    (at != DOC).then_some(at)
}

// ---- URIs (xmlBuildURI, xmlBuildRelativeURI) ----

struct Uri<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn split(s: &str) -> Uri<'_> {
    let (s, fragment) = match s.find('#') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (s, query) = match s.find('?') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let mut scheme = None;
    let mut rest = s;
    if let Some(i) = s.find(':')
        && i > 0
        && s[..i].starts_with(|c: char| c.is_ascii_alphabetic())
        && s[..i].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        scheme = Some(&s[..i]);
        rest = &s[i + 1..];
    }
    let mut authority = None;
    if let Some(r) = rest.strip_prefix("//") {
        let end = r.find('/').unwrap_or(r.len());
        authority = Some(&r[..end]);
        rest = &r[end..];
    }
    Uri { scheme, authority, path: rest, query, fragment }
}

fn assemble(scheme: Option<&str>, authority: Option<&str>, path: &str, query: Option<&str>, fragment: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(s) = scheme {
        out.push_str(s);
        out.push(':');
    }
    if let Some(a) = authority {
        out.push_str("//");
        out.push_str(a);
    }
    out.push_str(path);
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Removes . and .. segments.
fn normalize(path: &str) -> String {
    let abs = path.starts_with('/');
    let body = if abs { &path[1..] } else { path };
    let segs: Vec<&str> = body.split('/').collect();
    let mut out: Vec<&str> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match *s {
            "." => {
                if last {
                    out.push("");
                }
            }
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if !abs {
                    out.push("..");
                }
                if last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let joined = out.join("/");
    if abs { format!("/{joined}") } else { joined }
}

/// `reference` resolved against `base` (RFC 3986 as libxml2 does it).
fn resolve(reference: &str, base: &str) -> String {
    if base.is_empty() {
        return reference.to_string();
    }
    let r = split(reference);
    if r.scheme.is_some() {
        return reference.to_string();
    }
    let b = split(base);
    if r.authority.is_some() {
        return assemble(b.scheme, r.authority, &normalize(r.path), r.query, r.fragment);
    }
    if r.path.is_empty() {
        return assemble(b.scheme, b.authority, b.path, r.query.or(b.query), r.fragment);
    }
    if r.path.starts_with('/') {
        return assemble(b.scheme, b.authority, &normalize(r.path), r.query, r.fragment);
    }
    let merged = match b.path.rfind('/') {
        Some(i) => format!("{}{}", &b.path[..=i], r.path),
        None if b.authority.is_some() => format!("/{}", r.path),
        None => r.path.to_string(),
    };
    assemble(b.scheme, b.authority, &normalize(&merged), r.query, r.fragment)
}

/// `uri` relative to `base` where they share scheme and host; else `uri`.
fn relative(uri: &str, base: &str) -> String {
    if uri.is_empty() || base.is_empty() {
        return uri.to_string();
    }
    let (u, b) = (split(uri), split(base));
    if u.scheme != b.scheme || u.authority != b.authority || !u.path.starts_with('/') || !b.path.starts_with('/') {
        return uri.to_string();
    }
    let (ub, bb) = (u.path.as_bytes(), b.path.as_bytes());
    let mut common = 0;
    let mut i = 0;
    while i < ub.len() && i < bb.len() && ub[i] == bb[i] {
        if ub[i] == b'/' {
            common = i + 1;
        }
        i += 1;
    }
    let ups = b.path[common..].matches('/').count();
    let mut out = "../".repeat(ups);
    out.push_str(&u.path[common..]);
    if ups == 0 && out.split('/').next().is_some_and(|s| s.contains(':')) {
        out.insert_str(0, "./");
    }
    if let Some(q) = u.query {
        out.push('?');
        out.push_str(q);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_like_libxml2() {
        assert_eq!(resolve("part.xml", "/work/main.xml"), "/work/part.xml");
        assert_eq!(resolve("../a/b.xml", "/work/sub/main.xml"), "/work/a/b.xml");
        assert_eq!(resolve("/x/y.xml", "http://h/dir/f.xml"), "http://h/x/y.xml");
        assert_eq!(resolve("", "/work/main.xml"), "/work/main.xml");
        assert_eq!(resolve("file:///a", "/b/"), "file:///a");
    }

    #[test]
    fn relative_goes_up_and_down() {
        assert_eq!(relative("/work/part.xml", "/"), "work/part.xml");
        assert_eq!(relative("/work/part.xml", "/work/main.xml"), "part.xml");
        assert_eq!(relative("/work/part.xml", "/work/sub/main.xml"), "../part.xml");
        assert_eq!(relative("/work/part.xml", "/a/b/main.xml"), "../../work/part.xml");
        assert_eq!(relative("file:///w/p.xml", "/"), "file:///w/p.xml");
    }

    #[test]
    fn pointer_parts_unescape() {
        assert_eq!(parts("xmlns(a=b) xpointer(/r/k[^)])"), Some(vec![("xmlns".into(), "a=b".into()), ("xpointer".into(), "/r/k[)]".into())]));
    }
}

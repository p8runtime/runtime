//! Validating a document against a compiled schema, reporting libxml2's
//! XML_FROM_SCHEMASV errors in its wording (xmlschemas.c, xmlSchemaVal*).

use super::content::{AllModel, Expect, Matched, Nfa};
use super::datatypes::{self as dt, B, Ws};
use super::schema::*;
use crate::dom::tree::{Doc, Id, Kind, Node};
use crate::parser::XmlError;
use std::collections::HashMap;
use std::rc::Rc;

pub(crate) const FROM_SCHEMASV: i32 = 17;

// XML_SCHEMAV_* codes (xmlerror.h).
const CVC_DATATYPE_VALID_1_2_1: i32 = 1824;
const CVC_DATATYPE_VALID_1_2_2: i32 = 1825;
const CVC_DATATYPE_VALID_1_2_3: i32 = 1826;
const CVC_TYPE_3_1_1: i32 = 1827;
const CVC_TYPE_3_1_2: i32 = 1828;
const CVC_LENGTH_VALID: i32 = 1830;
const CVC_MINLENGTH_VALID: i32 = 1831;
const CVC_MAXLENGTH_VALID: i32 = 1832;
const CVC_MININCLUSIVE_VALID: i32 = 1833;
const CVC_MAXINCLUSIVE_VALID: i32 = 1834;
const CVC_MINEXCLUSIVE_VALID: i32 = 1835;
const CVC_MAXEXCLUSIVE_VALID: i32 = 1836;
const CVC_TOTALDIGITS_VALID: i32 = 1837;
const CVC_FRACTIONDIGITS_VALID: i32 = 1838;
const CVC_PATTERN_VALID: i32 = 1839;
const CVC_ENUMERATION_VALID: i32 = 1840;
const CVC_COMPLEX_TYPE_2_1: i32 = 1841;
const CVC_COMPLEX_TYPE_2_2: i32 = 1842;
const CVC_COMPLEX_TYPE_3_2_2: i32 = 1867;
const CVC_ELT_4_2: i32 = 1851;
const CVC_COMPLEX_TYPE_2_3: i32 = 1843;
const CVC_ELT_1: i32 = 1845;
const CVC_ELT_2: i32 = 1846;
const CVC_ELT_3_1: i32 = 1847;
const CVC_ELT_3_2_1: i32 = 1848;
const CVC_ELT_4_3: i32 = 1852;
const CVC_ELT_5_2_2_2_2: i32 = 1858;
const CVC_AU: i32 = 1874;
const CVC_COMPLEX_TYPE_3_2_1: i32 = 1866;
const CVC_COMPLEX_TYPE_4: i32 = 1868;
const ELEMENT_CONTENT: i32 = 1871;
const CVC_WILDCARD: i32 = 1878;
const DOCUMENT_ELEMENT_MISSING: i32 = 1872;

pub(crate) struct Validator<'a> {
    schema: &'a Schema,
    pub doc: &'a mut Doc,
    pub errors: Vec<XmlError>,
    create: bool,
    nfas: HashMap<TypeId, Rc<Nfa>>,
    /// The xs:ID attribute values seen.
    ids: std::collections::HashSet<String>,
    /// The element whose attributes or content are being checked (QName values resolve against it).
    cur: std::cell::Cell<Id>,
    file: Option<String>,
}

fn qlabel(ns: &str, local: &str) -> String {
    if ns.is_empty() { local.to_string() } else { format!("{{{ns}}}{local}") }
}

fn st(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn is_blank(t: &[u8]) -> bool {
    t.iter().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
}

impl<'a> Validator<'a> {
    pub(crate) fn new(schema: &'a Schema, doc: &'a mut Doc, create: bool) -> Validator<'a> {
        let file = doc.settings.url.as_ref().map(|u| st(u));
        Validator { schema, doc, errors: Vec::new(), create, nfas: HashMap::new(), ids: std::collections::HashSet::new(), cur: std::cell::Cell::new(0), file }
    }

    pub(crate) fn run(&mut self) {
        let Some(root) = self.doc.root() else {
            self.errors.push(XmlError { domain: FROM_SCHEMASV, code: DOCUMENT_ELEMENT_MISSING, level: 2, message: "The document has no document element.\n".to_string(), line: 0, column: 0, file: self.file.clone(), context: Vec::new(), context_col: 0 });
            return;
        };
        let (ns, local) = self.name_of(root);
        match self.schema.global_elems.get(&(ns, local)) {
            Some(&decl) => self.element(root, decl),
            None => {
                let l = self.elem_label(root);
                self.err(root, CVC_ELT_1, format!("{l}: No matching global declaration available for the validation root."));
            }
        }
    }

    // ---- reporting ------------------------------------------------------------

    fn err(&mut self, node: Id, code: i32, message: String) {
        let line = self.doc.node(node).line as i32;
        self.errors.push(XmlError { domain: FROM_SCHEMASV, code, level: 2, message: format!("{message}\n"), line, column: 0, file: self.file.clone(), context: Vec::new(), context_col: 0 });
    }

    fn name_of(&self, id: Id) -> (String, String) {
        let n = self.doc.node(id);
        (n.ns.as_ref().map(|u| st(u)).unwrap_or_default(), st(&n.name))
    }

    fn elem_label(&self, id: Id) -> String {
        let (ns, local) = self.name_of(id);
        format!("Element '{}'", qlabel(&ns, &local))
    }

    fn attr_label(&self, el: Id, attr: Id) -> String {
        let (ns, local) = self.name_of(attr);
        let (ens, elocal) = self.name_of(el);
        format!("Element '{}', attribute '{}'", qlabel(&ens, &elocal), qlabel(&ns, &local))
    }

    // ---- elements ------------------------------------------------------------

    fn xsi_attr(&self, el: Id, name: &str) -> Option<(Id, String)> {
        let n = self.doc.node(el);
        n.attrs.iter().find(|&&a| self.doc.node(a).ns.as_deref() == Some(XSI_NS.as_bytes()) && self.doc.node(a).name == name.as_bytes()).map(|&a| (a, st(&self.doc.node(a).value)))
    }

    fn element(&mut self, el: Id, decl_id: ElemId) {
        self.cur.set(el);
        let schema = self.schema;
        let decl = &schema.elems[decl_id];
        let label = self.elem_label(el);
        if decl.abstract_ {
            self.err(el, CVC_ELT_2, format!("{label}: The element declaration is abstract."));
            return;
        }
        let mut ty = decl.ty;
        if let Some((attr, xt)) = self.xsi_attr(el, "type") {
            let al = self.attr_label(el, attr);
            let qn = self.resolve_instance_qname(el, &xt);
            match self.find_type(&qn) {
                None => self.err(el, CVC_ELT_4_2, format!("{al}: The QName value '{}' of the xsi:type attribute does not resolve to a type definition.", qname_text(&qn))),
                Some(t) if !schema.derives(t, decl.ty) => {
                    let tl = schema.type_label(t).unwrap_or_default();
                    self.err(el, CVC_ELT_4_3, format!("{al}: The type definition '{tl}', specified by xsi:type, is blocked or not validly derived from the type definition of the element declaration."));
                }
                Some(t) => ty = t,
            }
        }
        let mut nil = matches!(self.xsi_attr(el, "nil").map(|(_, v)| v).as_deref().map(str::trim), Some("true" | "1"));
        if nil && !decl.nillable {
            self.err(el, CVC_ELT_3_1, format!("{label}: The element is not 'nillable'."));
            nil = false;
        }
        match schema.ty(ty).kind.clone() {
            TypeKind::Simple(_) => self.simple_element(el, ty, decl_id, nil),
            TypeKind::Complex(c) => self.complex_element(el, ty, &c, decl_id, nil),
        }
    }

    fn find_type(&self, qn: &QN) -> Option<TypeId> {
        if qn.0 == XSD_NS {
            let b = B::from_name(&qn.1)?;
            return self.schema.builtin_ids.get(&b).copied();
        }
        self.schema.types.iter().position(|t| t.name.as_ref() == Some(qn))
    }

    fn resolve_instance_qname(&self, el: Id, v: &str) -> QN {
        let v = v.trim();
        match v.split_once(':') {
            Some((p, l)) => (self.doc.lookup_ns(el, Some(p.as_bytes())).map(|u| st(&u)).unwrap_or_default(), l.to_string()),
            None => (self.doc.lookup_ns(el, None).map(|u| st(&u)).unwrap_or_default(), v.to_string()),
        }
    }

    /// The text of the element's text children.
    fn text_of(&self, el: Id) -> Vec<u8> {
        let mut out = Vec::new();
        for &c in &self.doc.node(el).children {
            let n = self.doc.node(c);
            if matches!(n.kind, Kind::Text | Kind::CData) {
                out.extend_from_slice(&n.value);
            }
        }
        out
    }

    fn child_elements(&self, el: Id) -> Vec<Id> {
        self.doc.node(el).children.iter().copied().filter(|&c| self.doc.node(c).kind == Kind::Element).collect()
    }

    /// No element or text content at all.
    fn is_empty(&self, el: Id) -> bool {
        !self.doc.node(el).children.iter().any(|&c| matches!(self.doc.node(c).kind, Kind::Element | Kind::Text | Kind::CData | Kind::EntityRef))
    }

    fn simple_element(&mut self, el: Id, ty: TypeId, decl_id: ElemId, nil: bool) {
        let label = self.elem_label(el);
        // Attributes other than the schema-instance ones are not allowed.
        let attrs: Vec<Id> = self.doc.node(el).attrs.clone();
        for a in attrs {
            if self.doc.node(a).ns.as_deref() == Some(XSI_NS.as_bytes()) {
                continue;
            }
            let al = self.attr_label(el, a);
            let (ns, local) = self.name_of(a);
            self.err(el, CVC_TYPE_3_1_1, format!("{al}: The attribute '{}' is not allowed.", qlabel(&ns, &local)));
        }
        if nil {
            if !self.is_empty(el) {
                self.err(el, CVC_ELT_3_2_1, format!("{label}: Neither character nor element content is allowed because the element is 'nilled'."));
            }
            return;
        }
        if !self.child_elements(el).is_empty() {
            self.err(el, CVC_TYPE_3_1_2, format!("{label}: Element content is not allowed, because the type definition is simple."));
        }
        self.element_value(el, ty, decl_id);
    }

    /// Checks the text of `el` against simple type `ty`, with the
    /// declaration's default and fixed values.
    fn element_value(&mut self, el: Id, ty: TypeId, decl_id: ElemId) {
        let decl = &self.schema.elems[decl_id];
        let label = self.elem_label(el);
        let text = st(&self.text_of(el));
        let mut value = text.clone();
        let empty = self.is_empty(el);
        if empty && let Some(d) = decl.default.as_ref().or(decl.fixed.as_ref()) {
            value = d.clone();
            if self.create {
                crate::dom::node::set_text(self.doc, el, d.as_bytes());
            }
        }
        let errs = self.check_simple(ty, &value);
        if !errs.is_empty() {
            for (code, msg) in errs {
                self.err(el, code, format!("{label}: {msg}"));
            }
            return;
        }
        if let Some(fixed) = &decl.fixed && !empty && !self.same_value(ty, &value, fixed) {
            self.err(el, CVC_ELT_5_2_2_2_2, format!("{label}: The actual value '{value}' does not match the fixed value constraint '{fixed}'."));
        }
    }

    fn same_value(&self, ty: TypeId, a: &str, b: &str) -> bool {
        let root = self.schema.root_builtin(ty);
        let ws = self.effective_ws(ty);
        let (a, b) = (dt::normalize(ws, a), dt::normalize(ws, b));
        dt::equal(root, &a, &b)
    }

    fn complex_element(&mut self, el: Id, ty: TypeId, c: &Complex, decl_id: ElemId, nil: bool) {
        let label = self.elem_label(el);
        if c.abstract_ {
            let tl = self.schema.type_label(ty).unwrap_or_default();
            self.err(el, CVC_ELT_2, format!("{label}: The type definition '{tl}' is abstract."));
            return;
        }
        self.attributes(el, c);
        if nil {
            if !self.is_empty(el) {
                self.err(el, CVC_ELT_3_2_1, format!("{label}: Neither character nor element content is allowed because the element is 'nilled'."));
            }
            return;
        }
        match &c.content {
            Content::Empty => {
                for ch in self.doc.node(el).children.clone() {
                    match self.doc.node(ch).kind {
                        Kind::Text | Kind::CData => self.err(el, CVC_COMPLEX_TYPE_2_1, format!("{label}: Character content is not allowed, because the content type is empty.")),
                        Kind::Element => {
                            self.err(el, CVC_COMPLEX_TYPE_2_1, format!("{label}: Element content is not allowed, because the content type is empty."));
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Content::Simple(t) => {
                if !self.child_elements(el).is_empty() {
                    self.err(el, CVC_COMPLEX_TYPE_2_2, format!("{label}: Element content is not allowed, because the content type is a simple type definition."));
                }
                self.element_value(el, *t, decl_id);
            }
            Content::Model { particle, mixed } => self.model_children(el, ty, particle, *mixed),
        }
    }

    // ---- attributes ----------------------------------------------------------------------

    fn attributes(&mut self, el: Id, c: &Complex) {
        let attrs: Vec<Id> = self.doc.node(el).attrs.clone();
        let label = self.elem_label(el);
        let mut unknown: Vec<Id> = Vec::new();
        let mut present = vec![false; c.attrs.len()];
        for &a in &attrs {
            let n = self.doc.node(a);
            let ns = n.ns.as_ref().map(|u| st(u)).unwrap_or_default();
            if ns == XSI_NS {
                continue;
            }
            let local = st(&n.name);
            let value = st(&n.value);
            let al = self.attr_label(el, a);
            match c.attrs.iter().position(|u| u.name == local && u.ns == ns) {
                Some(i) if !c.attrs[i].prohibited => {
                    present[i] = true;
                    let u = &c.attrs[i];
                    let errs = self.check_simple(u.ty, &value);
                    if !errs.is_empty() {
                        for (code, msg) in errs {
                            self.err(el, code, format!("{al}: {msg}"));
                        }
                    } else if let Some(f) = &u.fixed && !self.same_value(u.ty, &value, f) {
                        self.err(el, CVC_AU, format!("{al}: The value '{value}' does not match the fixed value constraint '{f}'."));
                    } else if self.schema.root_builtin(u.ty) == B::Id && !self.ids.insert(value.trim().to_string()) {
                        let label = self.schema.type_label(u.ty).unwrap_or_default();
                        self.err(el, CVC_DATATYPE_VALID_1_2_1, format!("{al}: '{value}' is not a valid value of the atomic type '{label}'."));
                    }
                }
                Some(i) => {
                    present[i] = true;
                    unknown.push(a);
                }
                None => match &c.any_attr {
                    Some(w) if w.allows(&ns) => match self.schema.attrs.get(&(ns.clone(), local.clone())) {
                        Some(u) => {
                            for (code, msg) in self.check_simple(u.ty, &value) {
                                self.err(el, code, format!("{al}: {msg}"));
                            }
                        }
                        None if w.process == Process::Strict => {
                            self.err(el, CVC_WILDCARD, format!("{al}: No matching global attribute declaration available, but demanded by the strict wildcard."));
                        }
                        None => {}
                    },
                    _ => unknown.push(a),
                },
            }
        }
        for a in unknown {
            let al = self.attr_label(el, a);
            let (ns, local) = self.name_of(a);
            let code = if c.any_attr.is_some() { CVC_COMPLEX_TYPE_3_2_2 } else { CVC_COMPLEX_TYPE_3_2_1 };
            self.err(el, code, format!("{al}: The attribute '{}' is not allowed.", qlabel(&ns, &local)));
        }
        for (i, u) in c.attrs.iter().enumerate() {
            if u.required && !present[i] {
                self.err(el, CVC_COMPLEX_TYPE_4, format!("{label}: The attribute '{}' is required but missing.", qlabel(&u.ns, &u.name)));
            }
        }
        if self.create {
            for (i, u) in c.attrs.iter().enumerate() {
                if present[i] || u.prohibited {
                    continue;
                }
                if let Some(v) = u.default.as_ref().or(u.fixed.as_ref()) {
                    let mut node = Node::new(Kind::Attribute, u.name.as_bytes().to_vec());
                    node.value = v.as_bytes().to_vec();
                    if !u.ns.is_empty() {
                        node.ns = Some(u.ns.as_bytes().to_vec());
                    }
                    let id = self.doc.add(node);
                    self.doc.set_attr(el, id);
                }
            }
        }
    }

    // ---- element content --------------------------------------------------------------------------

    fn nfa_for(&mut self, ty: TypeId, p: &Particle) -> Rc<Nfa> {
        if let Some(n) = self.nfas.get(&ty) {
            return n.clone();
        }
        let n = Rc::new(Nfa::new(self.schema, p));
        self.nfas.insert(ty, n.clone());
        n
    }

    fn model_children(&mut self, el: Id, ty: TypeId, particle: &Particle, mixed: bool) {
        let label = self.elem_label(el);
        let children: Vec<Id> = self.doc.node(el).children.clone();
        // `all` content: its own matcher.
        let all_parts = match &particle.term {
            Term::All(parts) => Some(parts.as_slice()),
            _ => None,
        };
        let nfa = if all_parts.is_none() { Some(self.nfa_for(ty, particle)) } else { None };
        let mut set = nfa.as_ref().map(|n| n.begin());
        let mut all = all_parts.map(AllModel::new);
        let mut bad = false;
        for c in children {
            let kind = self.doc.node(c).kind;
            match kind {
                Kind::Text | Kind::CData => {
                    if !mixed && (kind == Kind::CData || !is_blank(&self.doc.node(c).value)) {
                        self.err(el, CVC_COMPLEX_TYPE_2_3, format!("{label}: Character content other than whitespace is not allowed because the content type is 'element-only'."));
                    }
                }
                Kind::Element => {
                    if bad {
                        continue;
                    }
                    let (ns, local) = self.name_of(c);
                    let step = if let Some(a) = all.as_mut() {
                        a.step(self.schema, &ns, &local).map(Matched::Decl)
                    } else if let (Some(n), Some(s)) = (&nfa, &set) {
                        n.step(self.schema, s, &ns, &local).map(|(m, next)| {
                            set = Some(next);
                            m
                        })
                    } else {
                        None
                    };
                    match step {
                        Some(Matched::Decl(d)) => self.element(c, d),
                        Some(Matched::Wild(w)) => self.wildcard_child(c, &w),
                        None => {
                            let cl = self.elem_label(c);
                            let exp = match (&nfa, &set, &all) {
                                (Some(n), Some(s), _) => n.expected(s, false),
                                (_, _, Some(a)) if !a.complete() => a.expected(self.schema),
                                _ => Vec::new(),
                            };
                            let tail = self.expected_text(&exp);
                            self.err(c, ELEMENT_CONTENT, format!("{cl}: This element is not expected.{tail}"));
                            bad = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if bad {
            return;
        }
        let done = match (&nfa, &set, &all) {
            (Some(n), Some(s), _) => n.accepts(s),
            (_, _, Some(a)) => a.complete() || (particle.min == 0 && !a.seen.iter().any(|s| *s)),
            _ => true,
        };
        if !done {
            let exp = match (&nfa, &set, &all) {
                (Some(n), Some(s), _) => n.expected(s, true),
                (_, _, Some(a)) => a.expected(self.schema),
                _ => Vec::new(),
            };
            let tail = self.expected_text(&exp);
            self.err(el, ELEMENT_CONTENT, format!("{label}: Missing child element(s).{tail}"));
        }
    }

    fn expected_text(&self, exp: &[Expect]) -> String {
        let mut names: Vec<String> = Vec::new();
        for e in exp {
            let items: Vec<String> = match e {
                Expect::Elem(id) => {
                    let d = &self.schema.elems[*id];
                    vec![qlabel(&d.ns, &d.name)]
                }
                Expect::Wild(w) => match &w.ns {
                    NsConstraint::Any => vec!["{*}*".to_string(), "*".to_string()],
                    NsConstraint::Other(_) => vec!["##other*".to_string()],
                    NsConstraint::List(l) => l.iter().map(|n| if n.is_empty() { "*".to_string() } else { format!("{{{n}}}*") }).collect(),
                },
            };
            for name in items {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        match names.len() {
            0 => String::new(),
            1 => format!(" Expected is ( {} ).", names[0]),
            _ => format!(" Expected is one of ( {} ).", names.join(", ")),
        }
    }

    fn wildcard_child(&mut self, c: Id, w: &Wildcard) {
        let (ns, local) = self.name_of(c);
        match w.process {
            Process::Skip => {}
            Process::Lax | Process::Strict => match self.schema.global_elems.get(&(ns, local)) {
                Some(&d) => self.element(c, d),
                None if w.process == Process::Strict => {
                    let l = self.elem_label(c);
                    self.err(c, CVC_ELT_1, format!("{l}: No matching global element declaration available, but demanded by the strict wildcard."));
                }
                None => {}
            },
        }
    }

    // ---- simple values -------------------------------------------------------------------------------

    fn effective_ws(&self, mut id: TypeId) -> Ws {
        loop {
            match &self.schema.ty(id).kind {
                TypeKind::Simple(s) => {
                    if let Some(w) = s.facets.ws {
                        return w;
                    }
                    if let Some(b) = s.builtin {
                        return b.ws();
                    }
                    match (&s.variety, s.base) {
                        (Variety::List(_), _) | (Variety::Union(_), _) => return Ws::Collapse,
                        (_, Some(b)) => id = b,
                        _ => return Ws::Preserve,
                    }
                }
                TypeKind::Complex(_) => return Ws::Preserve,
            }
        }
    }

    /// The errors (code, message) of `raw` against simple type `ty`;
    /// empty when valid.
    fn check_simple(&self, ty: TypeId, raw: &str) -> Vec<(i32, String)> {
        let label = self.schema.type_label(ty);
        let Some(variety) = self.variety(ty) else { return Vec::new() };
        match variety {
            Variety::Atomic => match self.atomic(ty, raw) {
                Ok(()) => Vec::new(),
                Err((mut errs, generic)) => {
                    if generic {
                        errs.push((CVC_DATATYPE_VALID_1_2_1, match &label {
                            Some(l) => format!("'{raw}' is not a valid value of the atomic type '{l}'."),
                            None => format!("'{raw}' is not a valid value of the local atomic type."),
                        }));
                    }
                    errs
                }
            },
            Variety::List(item) => {
                let norm = dt::normalize(Ws::Collapse, raw);
                let mut facet_errs = Vec::new();
                let items: Vec<&str> = norm.split(' ').filter(|s| !s.is_empty()).collect();
                let mut ok = true;
                for it in &items {
                    let e = self.check_simple(item, it);
                    if !e.is_empty() {
                        facet_errs = e;
                        ok = false;
                        break;
                    }
                }
                if ok {
                    facet_errs = self.list_facets(ty, &norm, items.len());
                }
                if ok && facet_errs.is_empty() {
                    return Vec::new();
                }
                facet_errs.push((CVC_DATATYPE_VALID_1_2_2, match &label {
                    Some(l) => format!("'{raw}' is not a valid value of the list type '{l}'."),
                    None => format!("'{raw}' is not a valid value of the local list type."),
                }));
                facet_errs
            }
            Variety::Union(members) => {
                let ok = members.iter().any(|&m| self.check_simple(m, raw).is_empty());
                if ok {
                    return Vec::new();
                }
                vec![(CVC_DATATYPE_VALID_1_2_3, match &label {
                    Some(l) => format!("'{raw}' is not a valid value of the union type '{l}'."),
                    None => format!("'{raw}' is not a valid value of the local union type."),
                })]
            }
        }
    }

    fn variety(&self, id: TypeId) -> Option<Variety> {
        match &self.schema.ty(id).kind {
            TypeKind::Simple(s) => Some(s.variety.clone()),
            TypeKind::Complex(_) => None,
        }
    }

    fn list_facets(&self, ty: TypeId, norm: &str, count: usize) -> Vec<(i32, String)> {
        let mut errs = Vec::new();
        let mut at = Some(ty);
        while let Some(id) = at {
            let TypeKind::Simple(s) = &self.schema.ty(id).kind else { break };
            let f = &s.facets;
            if let Some(l) = f.length && count != l {
                errs.push((CVC_LENGTH_VALID, format!("[facet 'length'] The value has a length of '{count}'; this differs from the allowed length of '{l}'.")));
            }
            if let Some(l) = f.min_length && count < l {
                errs.push((CVC_MINLENGTH_VALID, format!("[facet 'minLength'] The value has a length of '{count}'; this underruns the allowed minimum length of '{l}'.")));
            }
            if let Some(l) = f.max_length && count > l {
                errs.push((CVC_MAXLENGTH_VALID, format!("[facet 'maxLength'] The value has a length of '{count}'; this exceeds the allowed maximum length of '{l}'.")));
            }
            if let Some(list) = &f.enums && !list.iter().any(|e| dt::normalize(Ws::Collapse, e) == norm) {
                errs.push((CVC_ENUMERATION_VALID, enum_msg(norm, list)));
            }
            for (p, re) in &f.patterns {
                if let Some(re) = re && !re.is_match(norm) {
                    errs.push((CVC_PATTERN_VALID, format!("[facet 'pattern'] The value '{norm}' is not accepted by the pattern '{p}'.")));
                }
            }
            at = s.base;
        }
        errs
    }

    /// Lexical check against the built-in ancestor, then the facets of the
    /// whole derivation chain.
    fn atomic(&self, ty: TypeId, raw: &str) -> Result<(), (Vec<(i32, String)>, bool)> {
        let root = self.schema.root_builtin(ty);
        let ws = self.effective_ws(ty);
        let norm = dt::normalize(ws, raw);
        // List built-ins (NMTOKENS ...) validate token by token.
        if !dt::lexical_ok(root, &norm) {
            return Err((Vec::new(), true));
        }
        if matches!(root, B::QName | B::Notation) && let Some((prefix, _)) = norm.split_once(':') && self.doc.lookup_ns(self.cur.get(), Some(prefix.as_bytes())).is_none() {
            return Err((vec![(CVC_DATATYPE_VALID_1_2_1, format!("The QName value '{norm}' has no corresponding namespace declaration in scope."))], true));
        }
        let mut errs = Vec::new();
        // Facets from the base up to the type itself.
        let mut chain = Vec::new();
        let mut at = Some(ty);
        while let Some(id) = at {
            match &self.schema.ty(id).kind {
                TypeKind::Simple(s) => {
                    chain.push(&s.facets);
                    at = if s.builtin.is_some() { None } else { s.base };
                }
                TypeKind::Complex(_) => break,
            }
        }
        for f in chain.iter().rev() {
            self.apply_facets(f, root, &norm, &mut errs);
        }
        if errs.is_empty() { Ok(()) } else { Err((errs, false)) }
    }

    fn apply_facets(&self, f: &Facets, root: B, norm: &str, errs: &mut Vec<(i32, String)>) {
        let len = dt::length(root, norm);
        if let Some(l) = f.length && len != l {
            errs.push((CVC_LENGTH_VALID, format!("[facet 'length'] The value has a length of '{len}'; this differs from the allowed length of '{l}'.")));
        }
        if let Some(l) = f.min_length && len < l {
            errs.push((CVC_MINLENGTH_VALID, format!("[facet 'minLength'] The value has a length of '{len}'; this underruns the allowed minimum length of '{l}'.")));
        }
        if let Some(l) = f.max_length && len > l {
            errs.push((CVC_MAXLENGTH_VALID, format!("[facet 'maxLength'] The value has a length of '{len}'; this exceeds the allowed maximum length of '{l}'.")));
        }
        if !f.patterns.is_empty() {
            let ok = f.patterns.iter().any(|(_, re)| re.as_ref().is_none_or(|r| r.is_match(norm)));
            if !ok {
                let (p, _) = &f.patterns[f.patterns.len() - 1];
                errs.push((CVC_PATTERN_VALID, format!("[facet 'pattern'] The value '{norm}' is not accepted by the pattern '{p}'.")));
            }
        }
        if let Some(list) = &f.enums && !list.iter().any(|e| dt::equal(root, &dt::normalize(root.ws(), e), norm)) {
            errs.push((CVC_ENUMERATION_VALID, enum_msg(norm, list)));
        }
        use std::cmp::Ordering::*;
        if let Some(m) = &f.min_incl && dt::compare(root, norm, m) == Some(Less) {
            errs.push((CVC_MININCLUSIVE_VALID, format!("[facet 'minInclusive'] The value '{norm}' is less than the minimum value allowed ('{m}').")));
        }
        if let Some(m) = &f.max_incl && dt::compare(root, norm, m) == Some(Greater) {
            errs.push((CVC_MAXINCLUSIVE_VALID, format!("[facet 'maxInclusive'] The value '{norm}' is greater than the maximum value allowed ('{m}').")));
        }
        if let Some(m) = &f.min_excl && matches!(dt::compare(root, norm, m), Some(Less | Equal)) {
            errs.push((CVC_MINEXCLUSIVE_VALID, format!("[facet 'minExclusive'] The value '{norm}' must be greater than '{m}'.")));
        }
        if let Some(m) = &f.max_excl && matches!(dt::compare(root, norm, m), Some(Greater | Equal)) {
            errs.push((CVC_MAXEXCLUSIVE_VALID, format!("[facet 'maxExclusive'] The value '{norm}' must be less than '{m}'.")));
        }
        if let Some((total, frac)) = dt::digits(norm).filter(|_| root.derives_from(B::Decimal)) {
            if let Some(t) = f.total_digits && total > t {
                errs.push((CVC_TOTALDIGITS_VALID, format!("[facet 'totalDigits'] The value '{norm}' has more digits than are allowed ('{t}').")));
            }
            if let Some(t) = f.fraction_digits && frac > t {
                errs.push((CVC_FRACTIONDIGITS_VALID, format!("[facet 'fractionDigits'] The value '{norm}' has more fractional digits than are allowed ('{t}').")));
            }
        }
    }
}

fn enum_msg(norm: &str, list: &[String]) -> String {
    let set: Vec<String> = list.iter().map(|e| format!("'{e}'")).collect();
    format!("[facet 'enumeration'] The value '{norm}' is not an element of the set {{{}}}.", set.join(", "))
}

fn qname_text(qn: &QN) -> String {
    if qn.0.is_empty() { qn.1.clone() } else { format!("{{{}}}{}", qn.0, qn.1) }
}

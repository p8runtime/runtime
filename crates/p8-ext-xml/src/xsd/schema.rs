//! The schema component model and its construction from schema documents
//! (xmlschemas.c's parser: xmlSchemaParse*, xmlSchemaBuild*).

use super::datatypes::{B, Ws};
use super::regex::Regex;
use crate::dom::tree::{Doc, Id, Kind};
use crate::parser::XmlError;
use std::collections::HashMap;
use std::rc::Rc;

pub(crate) const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema";
pub(crate) const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

pub(crate) type TypeId = usize;
pub(crate) type ElemId = usize;
/// (namespace or "", local name)
pub(crate) type QN = (String, String);

#[derive(Default, Clone)]
pub(crate) struct Facets {
    pub length: Option<usize>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub patterns: Vec<(String, Option<Rc<Regex>>)>,
    pub enums: Option<Vec<String>>,
    pub ws: Option<Ws>,
    pub min_incl: Option<String>,
    pub max_incl: Option<String>,
    pub min_excl: Option<String>,
    pub max_excl: Option<String>,
    pub total_digits: Option<usize>,
    pub fraction_digits: Option<usize>,
}

#[derive(Clone)]
pub(crate) enum Variety {
    Atomic,
    List(TypeId),
    Union(Vec<TypeId>),
}

#[derive(Clone)]
pub(crate) struct Simple {
    pub builtin: Option<B>,
    pub base: Option<TypeId>,
    pub variety: Variety,
    pub facets: Facets,
}

#[derive(Clone)]
pub(crate) enum NsConstraint {
    Any,
    Other(String),
    List(Vec<String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Process {
    Strict,
    Lax,
    Skip,
}

#[derive(Clone)]
pub(crate) struct Wildcard {
    pub ns: NsConstraint,
    pub process: Process,
}

impl Wildcard {
    pub(crate) fn allows(&self, ns: &str) -> bool {
        match &self.ns {
            NsConstraint::Any => true,
            NsConstraint::Other(t) => !ns.is_empty() && ns != t,
            NsConstraint::List(l) => l.iter().any(|n| n == ns),
        }
    }
}

#[derive(Clone)]
pub(crate) enum Term {
    Elem(ElemId),
    Seq(Vec<Particle>),
    Choice(Vec<Particle>),
    All(Vec<Particle>),
    Any(Wildcard),
}

#[derive(Clone)]
pub(crate) struct Particle {
    pub min: u32,
    /// None: unbounded.
    pub max: Option<u32>,
    pub term: Term,
}

#[derive(Clone)]
pub(crate) enum Content {
    Empty,
    Simple(TypeId),
    Model { particle: Particle, mixed: bool },
}

#[derive(Clone)]
pub(crate) struct AttrUse {
    pub ns: String,
    pub name: String,
    pub ty: TypeId,
    pub required: bool,
    pub prohibited: bool,
    pub default: Option<String>,
    pub fixed: Option<String>,
}

#[derive(Clone)]
pub(crate) struct Complex {
    pub abstract_: bool,
    pub base: Option<TypeId>,
    pub content: Content,
    pub attrs: Vec<AttrUse>,
    pub any_attr: Option<Wildcard>,
}

#[derive(Clone)]
pub(crate) enum TypeKind {
    Simple(Simple),
    Complex(Complex),
}

pub(crate) struct TypeDef {
    pub name: Option<QN>,
    pub kind: TypeKind,
}

pub(crate) struct ElemDecl {
    pub name: String,
    pub ns: String,
    pub ty: TypeId,
    pub nillable: bool,
    pub abstract_: bool,
    pub default: Option<String>,
    pub fixed: Option<String>,
    pub head: Option<ElemId>,
}

pub(crate) struct Schema {
    pub types: Vec<TypeDef>,
    pub elems: Vec<ElemDecl>,
    pub global_elems: HashMap<QN, ElemId>,
    pub builtin_ids: HashMap<B, TypeId>,
    pub any_type: TypeId,
    pub any_simple: TypeId,
    /// Substitution group members of each head (transitively), in
    /// declaration order.
    pub members: HashMap<ElemId, Vec<ElemId>>,
    /// Global attribute declarations.
    pub attrs: HashMap<QN, AttrUse>,
}

impl Schema {
    pub(crate) fn ty(&self, id: TypeId) -> &TypeDef {
        &self.types[id]
    }

    /// The built-in type a simple type is ultimately derived from.
    pub(crate) fn root_builtin(&self, mut id: TypeId) -> B {
        loop {
            match &self.types[id].kind {
                TypeKind::Simple(s) => {
                    if let Some(b) = s.builtin {
                        return b;
                    }
                    match s.base {
                        Some(b) => id = b,
                        None => return B::AnySimpleType,
                    }
                }
                TypeKind::Complex(_) => return B::AnyType,
            }
        }
    }

    /// The type name as libxml2 reports it ('xs:integer', '{ns}name', 'name').
    pub(crate) fn type_label(&self, id: TypeId) -> Option<String> {
        let (ns, local) = self.types[id].name.as_ref()?;
        Some(if ns == XSD_NS {
            format!("xs:{local}")
        } else if ns.is_empty() {
            local.clone()
        } else {
            format!("{{{ns}}}{local}")
        })
    }

    pub(crate) fn derives(&self, mut id: TypeId, base: TypeId) -> bool {
        loop {
            if id == base {
                return true;
            }
            let next = match &self.types[id].kind {
                TypeKind::Simple(s) => s.base,
                TypeKind::Complex(c) => c.base,
            };
            match next {
                Some(n) => id = n,
                None => return base == self.any_type,
            }
        }
    }
}

struct SchemaDoc {
    doc: Doc,
    file: String,
    tns: String,
    elem_qualified: bool,
    attr_qualified: bool,
}

type NodeRef = (usize, Id);

pub(crate) struct Compiler {
    docs: Vec<SchemaDoc>,
    pub schema: Schema,
    pub errors: Vec<XmlError>,
    elem_nodes: HashMap<QN, NodeRef>,
    type_nodes: HashMap<QN, NodeRef>,
    group_nodes: HashMap<QN, NodeRef>,
    attr_nodes: HashMap<QN, NodeRef>,
    attr_group_nodes: HashMap<QN, NodeRef>,
    type_ids: HashMap<QN, TypeId>,
    attr_decls: HashMap<QN, AttrUse>,
    in_progress_attr_groups: Vec<QN>,
    in_progress_groups: Vec<QN>,
    elem_order: Vec<QN>,
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// A loader for included and imported documents: (location, base file) to
/// (document, its file name).
pub(crate) type Loader<'a> = &'a mut dyn FnMut(&str, &str) -> Option<(Doc, String)>;

impl Compiler {
    pub(crate) fn new() -> Compiler {
        let mut c = Compiler {
            docs: Vec::new(),
            schema: Schema { types: Vec::new(), elems: Vec::new(), global_elems: HashMap::new(), builtin_ids: HashMap::new(), any_type: 0, any_simple: 0, members: HashMap::new(), attrs: HashMap::new() },
            errors: Vec::new(),
            elem_nodes: HashMap::new(),
            type_nodes: HashMap::new(),
            group_nodes: HashMap::new(),
            attr_nodes: HashMap::new(),
            attr_group_nodes: HashMap::new(),
            type_ids: HashMap::new(),
            attr_decls: HashMap::new(),
            in_progress_attr_groups: Vec::new(),
            in_progress_groups: Vec::new(),
            elem_order: Vec::new(),
        };
        c.install_builtins();
        c
    }

    fn install_builtins(&mut self) {
        // anyType first so every other type can name it.
        let any_type = self.schema.types.len();
        self.schema.types.push(TypeDef {
            name: Some((XSD_NS.to_string(), "anyType".to_string())),
            kind: TypeKind::Complex(Complex {
                abstract_: false,
                base: None,
                content: Content::Model {
                    particle: Particle { min: 1, max: Some(1), term: Term::Seq(vec![Particle { min: 0, max: None, term: Term::Any(Wildcard { ns: NsConstraint::Any, process: Process::Lax }) }]) },
                    mixed: true,
                },
                attrs: Vec::new(),
                any_attr: Some(Wildcard { ns: NsConstraint::Any, process: Process::Lax }),
            }),
        });
        self.schema.any_type = any_type;
        self.schema.builtin_ids.insert(B::AnyType, any_type);
        self.type_ids.insert((XSD_NS.to_string(), "anyType".to_string()), any_type);
        for (b, name) in super::datatypes::ALL {
            if *b == B::AnyType {
                continue;
            }
            let base = b.parent().and_then(|p| self.schema.builtin_ids.get(&p).copied());
            let variety = match b.list_item() {
                Some(item) => {
                    let item_id = self.schema.builtin_ids.get(&item).copied().unwrap_or(0);
                    Variety::List(item_id)
                }
                None => Variety::Atomic,
            };
            let id = self.schema.types.len();
            self.schema.types.push(TypeDef { name: Some((XSD_NS.to_string(), name.to_string())), kind: TypeKind::Simple(Simple { builtin: Some(*b), base, variety, facets: Facets::default() }) });
            self.schema.builtin_ids.insert(*b, id);
            self.type_ids.insert((XSD_NS.to_string(), name.to_string()), id);
        }
        self.schema.any_simple = self.schema.builtin_ids[&B::AnySimpleType];
    }

    // ---- errors -----------------------------------------------------------

    fn err(&mut self, d: usize, node: Option<Id>, code: i32, message: String) {
        let line = node.map_or(0, |n| self.docs[d].doc.node(n).line as i32);
        self.errors.push(XmlError {
            domain: 16,
            code,
            level: 2,
            message: format!("{message}\n"),
            line,
            column: 0,
            file: Some(self.docs[d].file.clone()),
            context: Vec::new(),
            context_col: 0,
        });
    }

    /// A schema-for-schema error about `node`, reported as xmlSchemaPErr does.
    fn node_err(&mut self, d: usize, n: Id, code: i32, text: &str) {
        let name = self.xs_label(d, n);
        self.err(d, Some(n), code, format!("{name}: {text}"));
    }

    fn xs_label(&self, d: usize, n: Id) -> String {
        let node = self.docs[d].doc.node(n);
        match &node.ns {
            Some(ns) => format!("Element '{{{}}}{}'", s(ns), s(&node.name)),
            None => format!("Element '{}'", s(&node.name)),
        }
    }

    fn type_qname(&self, id: TypeId) -> String {
        match &self.schema.types[id].name {
            Some((ns, local)) if ns.is_empty() => local.clone(),
            Some((ns, local)) => format!("{{{ns}}}{local}"),
            None => "(anonymous)".to_string(),
        }
    }

    /// Checks the attributes of every schema element in document order
    /// (xmlSchemaPValAttr*): unknown ones and missing required ones.
    fn check_attributes(&mut self, d: usize, n: Id) {
        let doc = &self.docs[d].doc;
        let node = doc.node(n);
        if node.kind != Kind::Element || node.ns.as_deref() != Some(XSD_NS.as_bytes()) {
            return;
        }
        let name = s(&node.name);
        let global = node.parent.is_some_and(|p| self.is_xs(d, p, "schema"));
        let in_group_def = node.parent.is_some_and(|p| self.is_xs(d, p, "group") && self.attr(d, p, "name").is_some());
        let (allowed, required): (&[&str], &[&str]) = match (name.as_str(), global) {
            ("schema", _) => (&["attributeFormDefault", "blockDefault", "elementFormDefault", "finalDefault", "id", "targetNamespace", "version"], &[]),
            ("element", true) => (&["abstract", "block", "default", "final", "fixed", "id", "name", "nillable", "substitutionGroup", "type"], &["name"]),
            ("element", false) => (&["block", "default", "fixed", "form", "id", "maxOccurs", "minOccurs", "name", "nillable", "ref", "type"], &[]),
            ("attribute", true) => (&["default", "fixed", "id", "name", "type"], &["name"]),
            ("attribute", false) => (&["default", "fixed", "form", "id", "name", "ref", "type", "use"], &[]),
            ("complexType", true) => (&["abstract", "block", "final", "id", "mixed", "name"], &["name"]),
            ("complexType", false) => (&["id", "mixed"], &[]),
            ("simpleType", true) => (&["final", "id", "name"], &["name"]),
            ("simpleType", false) => (&["id"], &[]),
            ("sequence" | "choice" | "all", _) if in_group_def => (&["id"], &[]),
            ("sequence" | "choice" | "all", _) => (&["id", "maxOccurs", "minOccurs"], &[]),
            ("any", _) => (&["id", "maxOccurs", "minOccurs", "namespace", "processContents"], &[]),
            ("anyAttribute", _) => (&["id", "namespace", "processContents"], &[]),
            ("group", true) => (&["id", "name"], &["name"]),
            ("group", false) => (&["id", "maxOccurs", "minOccurs", "ref"], &["ref"]),
            ("attributeGroup", true) => (&["id", "name"], &["name"]),
            ("attributeGroup", false) => (&["id", "ref"], &["ref"]),
            ("restriction", _) | ("extension", _) => (&["base", "id"], &[]),
            ("list", _) => (&["id", "itemType"], &[]),
            ("union", _) => (&["id", "memberTypes"], &[]),
            ("complexContent", _) => (&["id", "mixed"], &[]),
            ("simpleContent", _) => (&["id"], &[]),
            ("annotation", _) => (&["id"], &[]),
            ("documentation", _) | ("appinfo", _) => (&["source"], &[]),
            ("import", _) => (&["id", "namespace", "schemaLocation"], &[]),
            ("include", _) => (&["id", "schemaLocation"], &[]),
            ("enumeration" | "pattern" | "length" | "minLength" | "maxLength" | "minInclusive" | "maxInclusive" | "minExclusive" | "maxExclusive" | "totalDigits" | "fractionDigits" | "whiteSpace", _) => (&["fixed", "id", "value"], &["value"]),
            _ => (&[], &[]),
        };
        let known = !allowed.is_empty() || !required.is_empty();
        let mut found: Vec<String> = Vec::new();
        let mut bad: Vec<String> = Vec::new();
        for &a in &node.attrs {
            let an = doc.node(a);
            if an.ns.is_some() {
                continue;
            }
            let aname = s(&an.name);
            if known && !allowed.contains(&aname.as_str()) {
                bad.push(aname.clone());
            }
            found.push(aname);
        }
        let mut missing: Vec<&str> = required.iter().copied().filter(|r| !found.iter().any(|f| f == r)).collect();
        // A local element or attribute needs a name or a ref.
        let needs_name_or_ref = !global && matches!(name.as_str(), "element" | "attribute") && !found.iter().any(|f| f == "name" || f == "ref");
        let children: Vec<Id> = node.children.clone();
        if matches!(name.as_str(), "element" | "attribute") && found.iter().any(|f| f == "default") && found.iter().any(|f| f == "fixed") {
            self.node_err(d, n, 3038, "The attributes 'default' and 'fixed' are mutually exclusive.");
        }
        for a in bad {
            self.node_err(d, n, 3035, &format!("The attribute '{a}' is not allowed."));
        }
        for m in missing.drain(..) {
            self.node_err(d, n, 3036, &format!("The attribute '{m}' is required but missing."));
        }
        if needs_name_or_ref {
            self.node_err(d, n, 3036, "One of the attributes 'name' or 'ref' must be present.");
        }
        let expected = match name.as_str() {
            "element" => Some((&["annotation", "simpleType", "complexType", "unique", "key", "keyref"][..], "(annotation?, ((simpleType | complexType)?, (unique | key | keyref)*))")),
            "complexType" => Some((&["annotation", "simpleContent", "complexContent", "group", "all", "choice", "sequence", "attribute", "attributeGroup", "anyAttribute"][..], "(annotation?, (simpleContent | complexContent | ((group | all | choice | sequence)?, ((attribute | attributeGroup)*, anyAttribute?))))")),
            "schema" => Some((&["include", "import", "redefine", "annotation", "simpleType", "complexType", "group", "attributeGroup", "element", "attribute", "notation"][..], "((include | import | redefine | annotation)*, (((simpleType | complexType | group | attributeGroup) | element | attribute | notation), annotation*)*)")),
            _ => None,
        };
        if let Some((names, text)) = expected {
            let bad = children.iter().any(|&c| {
                let cn = self.docs[d].doc.node(c);
                cn.kind == Kind::Element && !(cn.ns.as_deref() == Some(XSD_NS.as_bytes()) && names.contains(&s(&cn.name).as_str()))
            });
            if bad {
                self.node_err(d, n, 3033, &format!("The content is not valid. Expected is {text}."));
            }
        }
        for c in children {
            self.check_attributes(d, c);
        }
    }

    // ---- document access ----------------------------------------------------

    fn is_xs(&self, d: usize, n: Id, local: &str) -> bool {
        let node = self.docs[d].doc.node(n);
        node.kind == Kind::Element && node.ns.as_deref() == Some(XSD_NS.as_bytes()) && node.name == local.as_bytes()
    }

    fn xs_children(&self, d: usize, n: Id) -> Vec<Id> {
        let doc = &self.docs[d].doc;
        doc.node(n).children.iter().copied().filter(|&c| doc.node(c).kind == Kind::Element && doc.node(c).ns.as_deref() == Some(XSD_NS.as_bytes())).collect()
    }

    fn attr(&self, d: usize, n: Id, name: &str) -> Option<String> {
        let doc = &self.docs[d].doc;
        doc.node(n).attrs.iter().find(|&&a| doc.node(a).name == name.as_bytes() && doc.node(a).ns.is_none()).map(|&a| s(&doc.node(a).value))
    }

    fn local(&self, d: usize, n: Id) -> String {
        s(&self.docs[d].doc.node(n).name)
    }

    /// A QName attribute value resolved against the namespaces in scope.
    fn resolve_qname(&self, d: usize, n: Id, value: &str) -> QN {
        let doc = &self.docs[d].doc;
        let value = value.trim();
        match value.split_once(':') {
            Some((p, l)) => (doc.lookup_ns(n, Some(p.as_bytes())).map(|u| s(&u)).unwrap_or_default(), l.to_string()),
            None => (doc.lookup_ns(n, None).map(|u| s(&u)).unwrap_or_default(), value.to_string()),
        }
    }

    fn occurs(&self, d: usize, n: Id) -> (u32, Option<u32>) {
        let min = self.attr(d, n, "minOccurs").and_then(|v| v.trim().parse().ok()).unwrap_or(1);
        let max = match self.attr(d, n, "maxOccurs") {
            Some(v) if v.trim() == "unbounded" => None,
            Some(v) => Some(v.trim().parse().unwrap_or(1)),
            None => Some(1),
        };
        (min, max)
    }

    // ---- loading the schema documents -----------------------------------------

    /// Adds the schema document `doc` and everything it includes or imports.
    /// False when `doc` is not a schema document.
    pub(crate) fn add_document(&mut self, doc: Doc, file: String, loader: Loader, inherited_tns: Option<&str>) -> bool {
        let Some(root) = doc.root() else { return false };
        let node = doc.node(root);
        if node.ns.as_deref() != Some(XSD_NS.as_bytes()) || node.name != b"schema" {
            return false;
        }
        let attr = |name: &str| node.attrs.iter().find(|&&a| doc.node(a).name == name.as_bytes() && doc.node(a).ns.is_none()).map(|&a| s(&doc.node(a).value));
        let tns = attr("targetNamespace").or_else(|| inherited_tns.map(str::to_string)).unwrap_or_default();
        let elem_qualified = attr("elementFormDefault").as_deref() == Some("qualified");
        let attr_qualified = attr("attributeFormDefault").as_deref() == Some("qualified");
        let d = self.docs.len();
        self.docs.push(SchemaDoc { doc, file: file.clone(), tns: tns.clone(), elem_qualified, attr_qualified });
        self.check_attributes(d, root);
        let children = self.xs_children(d, root);
        let mut dups: Vec<(Id, String)> = Vec::new();
        for c in children {
            match self.local(d, c).as_str() {
                "include" | "import" | "redefine" => {
                    let is_import = self.local(d, c) == "import";
                    if let Some(loc) = self.attr(d, c, "schemaLocation")
                        && let Some((sub, sub_file)) = loader(&loc, &file)
                    {
                        // Imports keep their own namespace.
                        let inherit = if is_import { None } else { Some(tns.as_str()) };
                        self.add_document(sub, sub_file, loader, inherit);
                    }
                }
                kind @ ("element" | "complexType" | "simpleType" | "group" | "attributeGroup" | "attribute") => {
                    let Some(name) = self.attr(d, c, "name") else { continue };
                    let qn = (tns.clone(), name);
                    let map = match kind {
                        "element" => {
                            self.elem_order.push(qn.clone());
                            &mut self.elem_nodes
                        }
                        "complexType" | "simpleType" => &mut self.type_nodes,
                        "group" => &mut self.group_nodes,
                        "attributeGroup" => &mut self.attr_group_nodes,
                        _ => &mut self.attr_nodes,
                    };
                    match map.entry(qn.clone()) {
                        std::collections::hash_map::Entry::Vacant(slot) => {
                            slot.insert((d, c));
                        }
                        std::collections::hash_map::Entry::Occupied(_) => {
                            let what = match kind {
                                "element" => "element declaration",
                                "simpleType" => "simple type definition",
                                "complexType" => "complex type definition",
                                "group" => "model group definition",
                                "attributeGroup" => "attribute group definition",
                                _ => "attribute declaration",
                            };
                            let shown = if qn.0.is_empty() { qn.1.clone() } else { format!("{{{}}}{}", qn.0, qn.1) };
                            dups.push((c, format!("A global {what} '{shown}' does already exist.")));
                        }
                    }
                }
                _ => {}
            }
        }
        for (c, msg) in dups {
            self.node_err(d, c, 1761, &msg);
        }
        true
    }

    /// Builds every global component, then the substitution groups.
    pub(crate) fn finish(&mut self) {
        let types: Vec<QN> = {
            let mut v: Vec<(NodeRef, QN)> = self.type_nodes.iter().map(|(k, v)| (*v, k.clone())).collect();
            v.sort();
            v.into_iter().map(|(_, k)| k).collect()
        };
        let mut elems: Vec<(NodeRef, QN)> = self.elem_nodes.iter().map(|(k, v)| (*v, k.clone())).collect();
        elems.sort();
        for qn in types {
            self.get_type(&qn);
        }
        for (_, qn) in elems {
            self.get_elem(&qn);
        }
        let attrs: Vec<QN> = self.attr_nodes.keys().cloned().collect();
        for qn in attrs {
            self.get_attr_decl(&qn);
        }
        // Substitution groups.
        let mut direct: HashMap<ElemId, Vec<ElemId>> = HashMap::new();
        for (id, e) in self.schema.elems.iter().enumerate() {
            if let Some(h) = e.head {
                direct.entry(h).or_default().push(id);
            }
        }
        let mut members = HashMap::new();
        for head in direct.keys().copied().collect::<Vec<_>>() {
            let mut out = Vec::new();
            let mut stack = vec![head];
            while let Some(h) = stack.pop() {
                if let Some(ms) = direct.get(&h) {
                    for &m in ms {
                        if !out.contains(&m) && m != head {
                            out.push(m);
                            stack.push(m);
                        }
                    }
                }
            }
            out.sort_unstable();
            members.insert(head, out);
        }
        self.schema.members = members;
        // libxml2 resolves references only when the document parsed cleanly.
        let parse_phase = |e: &XmlError| matches!(e.code, 3033 | 3035 | 3036 | 3037 | 3038 | 3043 | 3091 | 1761 | 1756 | 3087) || e.message.contains("References from this schema");
        if self.errors.iter().any(parse_phase) {
            self.errors.retain(parse_phase);
        }
        self.schema.attrs = self.attr_decls.clone();
    }

    // ---- lookups ---------------------------------------------------------------

    fn get_type(&mut self, qn: &QN) -> Option<TypeId> {
        if let Some(&id) = self.type_ids.get(qn) {
            return Some(id);
        }
        let (d, n) = *self.type_nodes.get(qn)?;
        let id = self.schema.types.len();
        // A placeholder, so a self-reference inside resolves.
        self.schema.types.push(TypeDef { name: Some(qn.clone()), kind: TypeKind::Simple(Simple { builtin: None, base: Some(self.schema.any_simple), variety: Variety::Atomic, facets: Facets::default() }) });
        self.type_ids.insert(qn.clone(), id);
        let kind = if self.is_xs(d, n, "simpleType") { TypeKind::Simple(self.build_simple(d, n)) } else { TypeKind::Complex(self.build_complex(d, n)) };
        self.schema.types[id].kind = kind;
        Some(id)
    }

    fn get_elem(&mut self, qn: &QN) -> Option<ElemId> {
        if let Some(&id) = self.schema.global_elems.get(qn) {
            return Some(id);
        }
        let (d, n) = *self.elem_nodes.get(qn)?;
        let id = self.schema.elems.len();
        self.schema.elems.push(ElemDecl { name: qn.1.clone(), ns: qn.0.clone(), ty: self.schema.any_type, nillable: false, abstract_: false, default: None, fixed: None, head: None });
        self.schema.global_elems.insert(qn.clone(), id);
        self.fill_elem(id, d, n);
        Some(id)
    }

    fn get_attr_decl(&mut self, qn: &QN) -> Option<AttrUse> {
        if let Some(a) = self.attr_decls.get(qn) {
            return Some(a.clone());
        }
        let (d, n) = *self.attr_nodes.get(qn)?;
        let ty = self.attr_type(d, n);
        let u = AttrUse { ns: qn.0.clone(), name: qn.1.clone(), ty, required: false, prohibited: false, default: self.attr(d, n, "default"), fixed: self.attr(d, n, "fixed") };
        self.attr_decls.insert(qn.clone(), u.clone());
        Some(u)
    }

    /// A QName attribute value resolved; None (with libxml2's error) when its
    /// prefix is not bound.
    fn qn_of(&mut self, d: usize, n: Id, attr: &str, v: &str) -> Option<QN> {
        let t = v.trim();
        if let Some((p, _)) = t.split_once(':')
            && self.docs[d].doc.lookup_ns(n, Some(p.as_bytes())).is_none()
        {
            let label = self.xs_label(d, n);
            self.err(d, Some(n), 3037, format!("{label}, attribute '{attr}': The value '{t}' of simple type 'xs:QName' has no corresponding namespace declaration in scope."));
            return None;
        }
        Some(self.resolve_qname(d, n, t))
    }

    /// libxml2 refuses an unprefixed reference to no namespace from a schema that has a target namespace.
    fn no_namespace_ref(&mut self, d: usize, n: Id, attr: &str, qn: &QN) -> bool {
        if qn.0.is_empty() && !self.docs[d].tns.is_empty() {
            let label = self.xs_label(d, n);
            self.err(d, Some(n), 3004, format!("{label}, attribute '{attr}': References from this schema to components in no namespace are not allowed, since not indicated by an import statement."));
            return true;
        }
        false
    }

    fn unresolved(&mut self, d: usize, n: Id, owner: &str, attr: &str, qn: &QN, kind: &str) {
        let shown = if qn.0.is_empty() { qn.1.clone() } else { format!("{{{}}}{}", qn.0, qn.1) };
        self.err(d, Some(n), 3004, format!("{owner}, attribute '{attr}': The QName value '{shown}' does not resolve to a(n) {kind}."));
    }

    /// The type reference of an `element`, `attribute` or the like:
    /// a `type` attribute, or an inline simple/complex type.
    fn resolve_type_ref(&mut self, d: usize, n: Id, attr: &str, owner: &str) -> Option<TypeId> {
        let v = self.attr(d, n, attr)?;
        let is_element = self.is_xs(d, n, "element");
        let Some(qn) = self.qn_of(d, n, attr, &v) else {
            if is_element {
                let local = v.trim().rsplit(':').next().unwrap_or("").to_string();
                self.no_namespace_ref(d, n, attr, &(String::new(), local));
            }
            return Some(self.schema.any_type);
        };
        if is_element && self.no_namespace_ref(d, n, attr, &qn) {
            return Some(self.schema.any_type);
        }
        let found = self.get_type(&qn);
        if found.is_none() {
            let kind = if is_element { "type definition" } else { "simple type definition" };
            self.unresolved(d, n, owner, attr, &qn, kind);
        }
        found.or(Some(self.schema.any_type))
    }

    fn attr_type(&mut self, d: usize, n: Id) -> TypeId {
        if self.attr(d, n, "type").is_some() {
            let owner = self.owner_label(d, n);
            return self.resolve_type_ref(d, n, "type", &owner).unwrap_or(self.schema.any_simple);
        }
        for c in self.xs_children(d, n) {
            if self.is_xs(d, c, "simpleType") {
                return self.build_anon_simple(d, c);
            }
        }
        self.schema.any_simple
    }

    fn owner_label(&self, d: usize, n: Id) -> String {
        let name = self.attr(d, n, "name").unwrap_or_default();
        let global = self.docs[d].doc.node(n).parent.is_some_and(|p| self.is_xs(d, p, "schema"));
        let qualified = if global && !self.docs[d].tns.is_empty() { format!("{{{}}}{name}", self.docs[d].tns) } else { name.clone() };
        match self.local(d, n).as_str() {
            "element" => {
                if global {
                    format!("element decl. '{qualified}'")
                } else {
                    format!("local element decl. '{name}'")
                }
            }
            "attribute" => format!("attribute decl. '{qualified}'"),
            "complexType" => format!("complex type '{name}'"),
            "simpleType" => {
                let kind = self
                    .xs_children(d, n)
                    .into_iter()
                    .find_map(|c| match self.local(d, c).as_str() {
                        "list" => Some("list type"),
                        "union" => Some("union type"),
                        _ => None,
                    })
                    .unwrap_or("simple type");
                format!("{kind} '{name}'")
            }
            other => format!("{other} '{name}'"),
        }
    }

    // ---- elements ------------------------------------------------------------------

    fn fill_elem(&mut self, id: ElemId, d: usize, n: Id) {
        let owner = self.owner_label(d, n);
        let mut ty = None;
        if self.attr(d, n, "type").is_some() {
            ty = self.resolve_type_ref(d, n, "type", &owner);
        } else {
            for c in self.xs_children(d, n) {
                if self.is_xs(d, c, "simpleType") {
                    ty = Some(self.build_anon_simple(d, c));
                } else if self.is_xs(d, c, "complexType") {
                    ty = Some(self.build_anon_complex(d, c));
                }
            }
        }
        let mut head = None;
        if let Some(v) = self.attr(d, n, "substitutionGroup")
            && let Some(qn) = self.qn_of(d, n, "substitutionGroup", &v)
        {
            match self.get_elem(&qn) {
                Some(h) => head = Some(h),
                None => {
                    let shown = if qn.0.is_empty() { qn.1.clone() } else { format!("{{{}}}{}", qn.0, qn.1) };
                    self.err(d, None, 3004, format!("{owner}, attribute 'substitutionGroup': The QName value '{shown}' does not resolve to a(n) element declaration."));
                }
            }
        }
        let ty = ty.unwrap_or_else(|| match head {
            Some(h) => self.schema.elems[h].ty,
            None => self.schema.any_type,
        });
        if let Some(h) = head {
            let head_ty = self.schema.elems[h].ty;
            if !self.schema.derives(ty, head_ty) {
                let name = self.attr(d, n, "name").unwrap_or_default();
                let hname = self.schema.elems[h].name.clone();
                let tl = self.type_qname(ty);
                let hl = self.type_qname(head_ty);
                self.err(d, Some(n), 3047, format!("{owner}: The type definition '{tl}' was either rejected by the substitution group affiliation '{hname}', or not validly derived from its type definition '{hl}'."));
                let _ = name;
            }
        }
        let nillable = matches!(self.attr(d, n, "nillable").as_deref(), Some("true" | "1"));
        let abstract_ = matches!(self.attr(d, n, "abstract").as_deref(), Some("true" | "1"));
        let default = self.attr(d, n, "default");
        let fixed = self.attr(d, n, "fixed");
        let e = &mut self.schema.elems[id];
        e.ty = ty;
        e.head = head;
        e.nillable = nillable;
        e.abstract_ = abstract_;
        e.default = default;
        e.fixed = fixed;
    }

    /// A local element declaration or reference as a particle term.
    fn build_local_element(&mut self, d: usize, n: Id) -> Option<ElemId> {
        if let Some(r) = self.attr(d, n, "ref") {
            let qn = self.qn_of(d, n, "ref", &r)?;
            if self.no_namespace_ref(d, n, "ref", &qn) {
                return None;
            }
            return match self.get_elem(&qn) {
                Some(id) => Some(id),
                None => {
                    let owner = self.xs_label(d, n);
                    self.unresolved(d, n, &owner, "ref", &qn, "element declaration");
                    None
                }
            };
        }
        let name = self.attr(d, n, "name")?;
        let qualified = match self.attr(d, n, "form").as_deref() {
            Some("qualified") => true,
            Some("unqualified") => false,
            _ => self.docs[d].elem_qualified,
        };
        let ns = if qualified { self.docs[d].tns.clone() } else { String::new() };
        let id = self.schema.elems.len();
        self.schema.elems.push(ElemDecl { name, ns, ty: self.schema.any_type, nillable: false, abstract_: false, default: None, fixed: None, head: None });
        self.fill_elem(id, d, n);
        Some(id)
    }

    // ---- particles ---------------------------------------------------------------------

    fn build_particle(&mut self, d: usize, n: Id) -> Option<Particle> {
        let (min, max) = self.occurs(d, n);
        if max.is_some_and(|m| m < min) {
            let label = self.xs_label(d, n);
            self.err(d, Some(n), 3043, format!("{label}, attribute 'minOccurs': The value must not be greater than the value of 'maxOccurs'."));
        }
        let term = match self.local(d, n).as_str() {
            "element" => Term::Elem(self.build_local_element(d, n)?),
            "any" => Term::Any(self.build_wildcard(d, n)),
            "sequence" | "choice" | "all" => {
                let kind = self.local(d, n);
                let mut parts = Vec::new();
                for c in self.xs_children(d, n) {
                    if !self.is_xs(d, c, "annotation")
                        && let Some(p) = self.build_particle(d, c)
                    {
                        if kind == "all" && p.max.is_none_or(|m| m > 1) {
                            self.node_err(d, c, 3091, "Invalid value for maxOccurs (must be 0 or 1).");
                        }
                        parts.push(p);
                    }
                }
                match kind.as_str() {
                    "sequence" => Term::Seq(parts),
                    "choice" => Term::Choice(parts),
                    _ => Term::All(parts),
                }
            }
            "group" => {
                let r = self.attr(d, n, "ref")?;
                let qn = self.qn_of(d, n, "ref", &r)?;
                let Some(&(gd, gn)) = self.group_nodes.get(&qn) else {
                    let owner = self.xs_label(d, n);
                    self.unresolved(d, n, &owner, "ref", &qn, "model group definition");
                    return None;
                };
                if self.in_progress_groups.contains(&qn) {
                    return None;
                }
                self.in_progress_groups.push(qn);
                let mut inner = None;
                for c in self.xs_children(gd, gn) {
                    if matches!(self.local(gd, c).as_str(), "sequence" | "choice" | "all") {
                        inner = self.build_particle(gd, c);
                    }
                }
                self.in_progress_groups.pop();
                let mut p = inner?;
                p.min = min;
                p.max = max;
                return Some(p);
            }
            _ => return None,
        };
        Some(Particle { min, max, term })
    }

    fn build_wildcard(&mut self, d: usize, n: Id) -> Wildcard {
        let tns = self.docs[d].tns.clone();
        let ns = match self.attr(d, n, "namespace").as_deref().map(str::trim) {
            None | Some("##any") => NsConstraint::Any,
            Some("##other") => NsConstraint::Other(tns),
            Some(list) => NsConstraint::List(
                list.split_whitespace()
                    .map(|t| match t {
                        "##targetNamespace" => tns.clone(),
                        "##local" => String::new(),
                        other => other.to_string(),
                    })
                    .collect(),
            ),
        };
        let process = match self.attr(d, n, "processContents").as_deref() {
            Some("lax") => Process::Lax,
            Some("skip") => Process::Skip,
            _ => Process::Strict,
        };
        Wildcard { ns, process }
    }

    // ---- attributes -------------------------------------------------------------------------

    fn build_attr_uses(&mut self, d: usize, parent: Id, attrs: &mut Vec<AttrUse>, any: &mut Option<Wildcard>) {
        for c in self.xs_children(d, parent) {
            match self.local(d, c).as_str() {
                "attribute" => {
                    let use_attr = self.attr(d, c, "use");
                    let prohibited = use_attr.as_deref() == Some("prohibited");
                    let required = use_attr.as_deref() == Some("required");
                    let mut u = if let Some(r) = self.attr(d, c, "ref") {
                        let Some(qn) = self.qn_of(d, c, "ref", &r) else { continue };
                        if qn.0 == XSI_NS {
                            continue;
                        }
                        if self.no_namespace_ref(d, c, "ref", &qn) {
                            continue;
                        }
                        match self.get_attr_decl(&qn) {
                            Some(u) => u,
                            None => {
                                let owner = self.xs_label(d, c);
                                self.unresolved(d, c, &owner, "ref", &qn, "attribute declaration");
                                continue;
                            }
                        }
                    } else {
                        let Some(name) = self.attr(d, c, "name") else { continue };
                        let qualified = match self.attr(d, c, "form").as_deref() {
                            Some("qualified") => true,
                            Some("unqualified") => false,
                            _ => self.docs[d].attr_qualified,
                        };
                        let ns = if qualified { self.docs[d].tns.clone() } else { String::new() };
                        let ty = self.attr_type(d, c);
                        AttrUse { ns, name, ty, required: false, prohibited: false, default: None, fixed: None }
                    };
                    u.required = required;
                    u.prohibited = prohibited;
                    if let Some(v) = self.attr(d, c, "default") {
                        u.default = Some(v);
                    }
                    if let Some(v) = self.attr(d, c, "fixed") {
                        u.fixed = Some(v);
                    }
                    put_attr(attrs, u);
                }
                "attributeGroup" => {
                    let Some(r) = self.attr(d, c, "ref") else { continue };
                    let Some(qn) = self.qn_of(d, c, "ref", &r) else { continue };
                    let Some(&(gd, gn)) = self.attr_group_nodes.get(&qn) else {
                        let owner = self.xs_label(d, c);
                        self.unresolved(d, c, &owner, "ref", &qn, "attribute group definition");
                        continue;
                    };
                    if self.in_progress_attr_groups.contains(&qn) {
                        continue;
                    }
                    self.in_progress_attr_groups.push(qn);
                    self.build_attr_uses(gd, gn, attrs, any);
                    self.in_progress_attr_groups.pop();
                }
                "anyAttribute" => *any = Some(self.build_wildcard(d, c)),
                _ => {}
            }
        }
    }

    // ---- simple types --------------------------------------------------------------------------

    fn build_anon_simple(&mut self, d: usize, n: Id) -> TypeId {
        let simple = self.build_simple(d, n);
        self.schema.types.push(TypeDef { name: None, kind: TypeKind::Simple(simple) });
        self.schema.types.len() - 1
    }

    fn build_simple(&mut self, d: usize, n: Id) -> Simple {
        let any = self.schema.any_simple;
        for c in self.xs_children(d, n) {
            match self.local(d, c).as_str() {
                "restriction" => {
                    let base = if self.attr(d, c, "base").is_some() {
                        let owner = self.owner_label(d, n);
                        self.resolve_type_ref(d, c, "base", &owner)
                    } else {
                        self.xs_children(d, c).into_iter().find(|&k| self.is_xs(d, k, "simpleType")).map(|k| self.build_anon_simple(d, k))
                    }
                    .unwrap_or(any);
                    let facets = self.parse_facets(d, c, base);
                    let variety = match &self.schema.types[base].kind {
                        TypeKind::Simple(b) => b.variety.clone(),
                        TypeKind::Complex(_) => Variety::Atomic,
                    };
                    return Simple { builtin: None, base: Some(base), variety, facets };
                }
                "list" => {
                    let item = if self.attr(d, c, "itemType").is_some() {
                        let owner = self.owner_label(d, n);
                        self.resolve_type_ref(d, c, "itemType", &owner)
                    } else {
                        self.xs_children(d, c).into_iter().find(|&k| self.is_xs(d, k, "simpleType")).map(|k| self.build_anon_simple(d, k))
                    }
                    .unwrap_or(any);
                    return Simple { builtin: None, base: Some(any), variety: Variety::List(item), facets: Facets::default() };
                }
                "union" => {
                    let mut members = Vec::new();
                    if let Some(list) = self.attr(d, c, "memberTypes") {
                        for t in list.split_whitespace() {
                            let Some(qn) = self.qn_of(d, c, "memberTypes", t) else { continue };
                            match self.get_type(&qn) {
                                Some(id) => members.push(id),
                                None => {
                                    let owner = self.owner_label(d, n);
                                    self.unresolved(d, c, &owner, "memberTypes", &qn, "simple type definition");
                                }
                            }
                        }
                    }
                    for k in self.xs_children(d, c) {
                        if self.is_xs(d, k, "simpleType") {
                            members.push(self.build_anon_simple(d, k));
                        }
                    }
                    return Simple { builtin: None, base: Some(any), variety: Variety::Union(members), facets: Facets::default() };
                }
                _ => {}
            }
        }
        Simple { builtin: None, base: Some(any), variety: Variety::Atomic, facets: Facets::default() }
    }

    fn parse_facets(&mut self, d: usize, restriction: Id, _base: TypeId) -> Facets {
        let mut f = Facets::default();
        for c in self.xs_children(d, restriction) {
            let Some(value) = self.attr(d, c, "value") else { continue };
            let num = |v: &str| v.trim().parse::<usize>().ok();
            match self.local(d, c).as_str() {
                "length" => f.length = num(&value),
                "minLength" => f.min_length = num(&value),
                "maxLength" => f.max_length = num(&value),
                "pattern" => {
                    let re = Regex::new(&value).map(Rc::new);
                    if re.is_none() {
                        self.node_err(d, c, 1756, &format!("The value '{value}' of the facet 'pattern' is not a valid regular expression."));
                    }
                    f.patterns.push((value.clone(), re))
                }
                "enumeration" => f.enums.get_or_insert_with(Vec::new).push(value),
                "whiteSpace" => {
                    f.ws = match value.as_str() {
                        "preserve" => Some(Ws::Preserve),
                        "replace" => Some(Ws::Replace),
                        "collapse" => Some(Ws::Collapse),
                        _ => None,
                    }
                }
                "minInclusive" => f.min_incl = Some(value),
                "maxInclusive" => f.max_incl = Some(value),
                "minExclusive" => f.min_excl = Some(value),
                "maxExclusive" => f.max_excl = Some(value),
                "totalDigits" => f.total_digits = num(&value),
                "fractionDigits" => f.fraction_digits = num(&value),
                _ => {}
            }
        }
        f
    }

    // ---- complex types ----------------------------------------------------------------------------

    fn build_anon_complex(&mut self, d: usize, n: Id) -> TypeId {
        let c = self.build_complex(d, n);
        self.schema.types.push(TypeDef { name: None, kind: TypeKind::Complex(c) });
        self.schema.types.len() - 1
    }

    /// "Duplicate attribute use" among the direct attribute children of `parent`.
    fn check_dup_attrs(&mut self, d: usize, parent: Id, complex: Id) {
        let mut seen: Vec<String> = Vec::new();
        let mut dups: Vec<String> = Vec::new();
        for c in self.xs_children(d, parent) {
            if self.is_xs(d, c, "attribute")
                && let Some(name) = self.attr(d, c, "name")
            {
                if seen.contains(&name) {
                    dups.push(name);
                } else {
                    seen.push(name);
                }
            }
        }
        for name in dups {
            let owner = match self.attr(d, complex, "name") {
                Some(n) => format!("complex type '{n}'"),
                None => "local complex type".to_string(),
            };
            self.err(d, Some(complex), 3087, format!("{owner}: Duplicate attribute use '{name}'."));
        }
    }

    fn build_complex(&mut self, d: usize, n: Id) -> Complex {
        let mixed = matches!(self.attr(d, n, "mixed").as_deref(), Some("true" | "1"));
        let abstract_ = matches!(self.attr(d, n, "abstract").as_deref(), Some("true" | "1"));
        let mut c = Complex { abstract_, base: None, content: Content::Empty, attrs: Vec::new(), any_attr: None };
        let children: Vec<Id> = self.xs_children(d, n).into_iter().filter(|&k| !self.is_xs(d, k, "annotation")).collect();
        let mut model: Option<Id> = None;
        for &k in &children {
            match self.local(d, k).as_str() {
                "simpleContent" => {
                    self.build_simple_content(d, k, &mut c);
                    return c;
                }
                "complexContent" => {
                    let cmixed = self.attr(d, k, "mixed").map(|v| matches!(v.as_str(), "true" | "1")).unwrap_or(mixed);
                    self.build_complex_content(d, k, cmixed, &mut c);
                    return c;
                }
                "sequence" | "choice" | "all" | "group" => model = Some(k),
                _ => {}
            }
        }
        if let Some(m) = model {
            if let Some(p) = self.build_particle(d, m) {
                c.content = Content::Model { particle: p, mixed };
            }
        } else if mixed {
            c.content = Content::Model { particle: Particle { min: 1, max: Some(1), term: Term::Seq(Vec::new()) }, mixed: true };
        }
        self.check_dup_attrs(d, n, n);
        self.build_attr_uses(d, n, &mut c.attrs, &mut c.any_attr);
        c
    }

    fn build_complex_content(&mut self, d: usize, content: Id, mixed: bool, c: &mut Complex) {
        let Some(deriv) = self.xs_children(d, content).into_iter().find(|&k| matches!(self.local(d, k).as_str(), "extension" | "restriction")) else { return };
        let extension = self.local(d, deriv) == "extension";
        let owner = format!("complex type '{}'", self.attr(d, self.docs[d].doc.node(content).parent.unwrap_or(content), "name").unwrap_or_default());
        let base = self.resolve_type_ref(d, deriv, "base", &owner).unwrap_or(self.schema.any_type);
        c.base = Some(base);
        let base_complex = match &self.schema.types[base].kind {
            TypeKind::Complex(b) => Some(b.clone()),
            TypeKind::Simple(_) => None,
        };
        let mut own: Option<Particle> = None;
        for k in self.xs_children(d, deriv) {
            if matches!(self.local(d, k).as_str(), "sequence" | "choice" | "all" | "group") {
                own = self.build_particle(d, k);
            }
        }
        let mut own_attrs = Vec::new();
        let mut own_any = None;
        self.build_attr_uses(d, deriv, &mut own_attrs, &mut own_any);
        if extension {
            let base_particle = match base_complex.as_ref().map(|b| &b.content) {
                Some(Content::Model { particle, .. }) => Some(particle.clone()),
                _ => None,
            };
            let particle = match (base_particle, own) {
                (Some(b), Some(o)) => {
                    // An `all` base extended by more content becomes a sequence.
                    Some(Particle { min: 1, max: Some(1), term: Term::Seq(vec![b, o]) })
                }
                (Some(b), None) => Some(b),
                (None, o) => o,
            };
            c.content = match particle {
                Some(p) => Content::Model { particle: p, mixed },
                None if mixed => Content::Model { particle: Particle { min: 1, max: Some(1), term: Term::Seq(Vec::new()) }, mixed: true },
                None => match base_complex.as_ref().map(|b| b.content.clone()) {
                    Some(Content::Simple(t)) => Content::Simple(t),
                    _ => Content::Empty,
                },
            };
            let mut attrs = base_complex.as_ref().map(|b| b.attrs.clone()).unwrap_or_default();
            for u in own_attrs {
                put_attr(&mut attrs, u);
            }
            c.attrs = attrs;
            c.any_attr = own_any.or_else(|| base_complex.and_then(|b| b.any_attr));
        } else {
            c.content = match own {
                Some(p) => Content::Model { particle: p, mixed },
                None if mixed => Content::Model { particle: Particle { min: 1, max: Some(1), term: Term::Seq(Vec::new()) }, mixed: true },
                None => Content::Empty,
            };
            let mut attrs = base_complex.as_ref().map(|b| b.attrs.clone()).unwrap_or_default();
            for u in own_attrs {
                put_attr(&mut attrs, u);
            }
            attrs.retain(|a| !a.prohibited);
            c.attrs = attrs;
            c.any_attr = own_any.or_else(|| base_complex.and_then(|b| b.any_attr));
        }
    }

    fn build_simple_content(&mut self, d: usize, content: Id, c: &mut Complex) {
        let Some(deriv) = self.xs_children(d, content).into_iter().find(|&k| matches!(self.local(d, k).as_str(), "extension" | "restriction")) else { return };
        let extension = self.local(d, deriv) == "extension";
        let owner = format!("complex type '{}'", self.attr(d, self.docs[d].doc.node(content).parent.unwrap_or(content), "name").unwrap_or_default());
        let base = self.resolve_type_ref(d, deriv, "base", &owner).unwrap_or(self.schema.any_simple);
        c.base = Some(base);
        let (base_attrs, base_any, base_simple) = match self.schema.types[base].kind.clone() {
            TypeKind::Complex(bc) => (
                bc.attrs,
                bc.any_attr,
                match bc.content {
                    Content::Simple(t) => t,
                    _ => self.schema.any_simple,
                },
            ),
            TypeKind::Simple(_) => (Vec::new(), None, base),
        };
        let mut simple = base_simple;
        if !extension {
            let facets = self.parse_facets(d, deriv, base_simple);
            let inline = self.xs_children(d, deriv).into_iter().find(|&k| self.is_xs(d, k, "simpleType")).map(|k| self.build_anon_simple(d, k));
            let restricted_base = inline.unwrap_or(base_simple);
            let variety = match &self.schema.types[restricted_base].kind {
                TypeKind::Simple(b) => b.variety.clone(),
                TypeKind::Complex(_) => Variety::Atomic,
            };
            self.schema.types.push(TypeDef { name: None, kind: TypeKind::Simple(Simple { builtin: None, base: Some(restricted_base), variety, facets }) });
            simple = self.schema.types.len() - 1;
        }
        c.content = Content::Simple(simple);
        let mut own_attrs = Vec::new();
        let mut own_any = None;
        self.build_attr_uses(d, deriv, &mut own_attrs, &mut own_any);
        let mut attrs = base_attrs;
        for u in own_attrs {
            put_attr(&mut attrs, u);
        }
        if !extension {
            attrs.retain(|a| !a.prohibited);
        }
        c.attrs = attrs;
        c.any_attr = own_any.or(base_any);
    }
}

/// Adds `u`, replacing an attribute use of the same name.
fn put_attr(attrs: &mut Vec<AttrUse>, u: AttrUse) {
    match attrs.iter_mut().find(|a| a.name == u.name && a.ns == u.ns) {
        Some(slot) => *slot = u,
        None => attrs.push(u),
    }
}

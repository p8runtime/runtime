//! DOM (ext/dom): DOMDocument and its nodes, on the libxml2 parser port.
//! A node's properties are virtual hooked properties backed by native
//! `$name::get` / `$name::set` methods, as PHP's internal DOM classes
//! back theirs with read/write handlers.

pub mod build;
pub mod document;
pub mod lists;
pub mod node;
pub mod serialize;
pub mod tree;
pub mod xinclude;
pub mod xpath;

use document as d;
use lists as l;
use node as n;
use p8_runtime::ext::{ClassSpec, Constant, Registry};
use p8_runtime::object::{CLASS_FINAL, CLASS_INTERFACE, Native, Visibility};

type Prop = (&'static str, Constant, Visibility);
type Meth = (&'static str, Option<Native>);

macro_rules! props {
    ($($name:literal),* $(,)?) => { &[$(($name, Constant::Null, Visibility::Public)),*] };
}

macro_rules! m {
    ($name:literal, $f:path) => {
        ($name, Some(Native::Method($f)))
    };
}

const CHILD_NODE: &[Meth] = &[m!("remove", n::remove), m!("before", n::before), m!("after", n::after), m!("replaceWith", n::replace_with)];
const PARENT_NODE: &[Meth] = &[m!("append", n::append), m!("prepend", n::prepend), m!("replaceChildren", n::replace_children)];

const NODE_PROPS: &[Prop] = props!(
    "nodeName", "nodeValue", "nodeType", "parentNode", "parentElement", "childNodes", "firstChild", "lastChild",
    "previousSibling", "nextSibling", "attributes", "isConnected", "ownerDocument", "namespaceURI", "prefix",
    "localName", "baseURI", "textContent",
);
const NODE_METHODS: &[Meth] = &[
    m!("$nodeName::get", n::node_name_get),
    m!("$nodeValue::get", n::node_value_get),
    m!("$nodeValue::set", n::node_value_set),
    m!("$nodeType::get", n::node_type_get),
    m!("$parentNode::get", n::parent_node_get),
    m!("$parentElement::get", n::parent_element_get),
    m!("$childNodes::get", n::child_nodes_get),
    m!("$firstChild::get", n::first_child_get),
    m!("$lastChild::get", n::last_child_get),
    m!("$previousSibling::get", n::previous_sibling_get),
    m!("$nextSibling::get", n::next_sibling_get),
    m!("$attributes::get", n::attributes_get),
    m!("$isConnected::get", n::is_connected_get),
    m!("$ownerDocument::get", n::owner_document_get),
    m!("$namespaceURI::get", n::namespace_uri_get),
    m!("$prefix::get", n::prefix_get),
    m!("$localName::get", n::local_name_get),
    m!("$baseURI::get", n::base_uri_get),
    m!("$textContent::get", n::text_content_get),
    m!("$textContent::set", n::text_content_set),
    m!("appendChild", n::append_child),
    m!("insertBefore", n::insert_before),
    m!("removeChild", n::remove_child),
    m!("replaceChild", n::replace_child),
    m!("hasChildNodes", n::has_child_nodes),
    m!("hasAttributes", n::has_attributes),
    m!("cloneNode", n::clone_node),
    m!("isSameNode", n::is_same_node),
    m!("contains", n::contains),
    m!("getRootNode", n::get_root_node),
    m!("getLineNo", n::get_line_no),
    m!("normalize", n::normalize),
    m!("lookupNamespaceURI", n::lookup_namespace_uri),
    m!("lookupPrefix", n::lookup_prefix),
    m!("isDefaultNamespace", n::is_default_namespace),
    m!("isEqualNode", n::is_equal_node),
];

const ELEMENT_LIKE_HOOKS: &[Meth] = &[
    m!("$firstElementChild::get", n::first_element_child_get),
    m!("$lastElementChild::get", n::last_element_child_get),
    m!("$childElementCount::get", n::child_element_count_get),
];
const SIBLING_HOOKS: &[Meth] = &[
    m!("$previousElementSibling::get", n::previous_element_sibling_get),
    m!("$nextElementSibling::get", n::next_element_sibling_get),
];

macro_rules! concat_meths {
    ($($part:expr),* $(,)?) => {{
        const PARTS: &[&[Meth]] = &[$($part),*];
        const LEN: usize = {
            let mut n = 0;
            let mut i = 0;
            while i < PARTS.len() {
                n += PARTS[i].len();
                i += 1;
            }
            n
        };
        const OUT: [Meth; LEN] = {
            let mut out: [Meth; LEN] = [("", None); LEN];
            let mut k = 0;
            let mut i = 0;
            while i < PARTS.len() {
                let mut j = 0;
                while j < PARTS[i].len() {
                    out[k] = PARTS[i][j];
                    k += 1;
                    j += 1;
                }
                i += 1;
            }
            out
        };
        &OUT
    }};
}

const ELEMENT_METHODS: &[Meth] = concat_meths!(
    &[
        m!("__construct", d::element_construct),
        m!("$tagName::get", d::tag_name_get),
        m!("$className::get", d::class_name_get),
        m!("$className::set", d::class_name_set),
        m!("$id::get", d::id_get),
        m!("$id::set", d::id_set),
        m!("$schemaTypeInfo::get", d::schema_type_info_get),
        m!("getAttribute", d::get_attribute),
        m!("setAttribute", d::set_attribute),
        m!("removeAttribute", d::remove_attribute),
        m!("hasAttribute", d::has_attribute),
        m!("toggleAttribute", d::toggle_attribute),
        m!("getAttributeNames", d::get_attribute_names),
        m!("getAttributeNode", d::get_attribute_node),
        m!("setAttributeNode", d::set_attribute_node),
        m!("removeAttributeNode", d::remove_attribute_node),
        m!("getAttributeNS", d::get_attribute_ns),
        m!("setAttributeNS", d::set_attribute_ns),
        m!("removeAttributeNS", d::remove_attribute_ns),
        m!("hasAttributeNS", d::has_attribute_ns),
        m!("getAttributeNodeNS", d::get_attribute_node_ns),
        m!("setAttributeNodeNS", d::set_attribute_node),
        m!("getElementsByTagName", l::by_tag_name),
        m!("getElementsByTagNameNS", l::by_tag_name_ns),
    ],
    ELEMENT_LIKE_HOOKS,
    SIBLING_HOOKS,
    CHILD_NODE,
    PARENT_NODE,
);

const DOCUMENT_METHODS: &[Meth] = concat_meths!(
    &[
        m!("__construct", d::document_construct),
        m!("loadXML", d::load_xml),
        m!("load", d::load_file),
        m!("schemaValidate", d::schema_validate),
        m!("schemaValidateSource", d::schema_validate_source),
        m!("saveXML", d::save_xml),
        m!("save", d::save_file),
        m!("createElement", d::create_element),
        m!("createElementNS", d::create_element_ns),
        m!("createTextNode", d::create_text_node),
        m!("createComment", d::create_comment),
        m!("createCDATASection", d::create_cdata_section),
        m!("createProcessingInstruction", d::create_processing_instruction),
        m!("createAttribute", d::create_attribute),
        m!("createAttributeNS", d::create_attribute_ns),
        m!("createDocumentFragment", d::create_document_fragment),
        m!("createEntityReference", d::create_entity_reference),
        m!("getElementsByTagName", l::by_tag_name),
        m!("getElementsByTagNameNS", l::by_tag_name_ns),
        m!("getElementById", d::get_element_by_id),
        m!("importNode", d::import_node),
        m!("adoptNode", d::adopt_node),
        m!("normalizeDocument", d::normalize_document),
        m!("xinclude", d::xinclude),
        m!("$documentElement::get", d::document_element_get),
        m!("$doctype::get", d::doctype_get),
        m!("$implementation::get", d::implementation_get),
        m!("$actualEncoding::get", d::encoding_get),
        m!("$encoding::get", d::encoding_get),
        m!("$encoding::set", d::encoding_set),
        m!("$xmlEncoding::get", d::encoding_get),
        m!("$standalone::get", d::standalone_get),
        m!("$standalone::set", d::standalone_set),
        m!("$xmlStandalone::get", d::standalone_get),
        m!("$xmlStandalone::set", d::standalone_set),
        m!("$version::get", d::version_get),
        m!("$version::set", d::version_set),
        m!("$xmlVersion::get", d::version_get),
        m!("$xmlVersion::set", d::version_set),
        m!("$documentURI::get", d::document_uri_get),
        m!("$documentURI::set", d::document_uri_set),
        m!("$config::get", d::config_get),
        m!("$formatOutput::get", d::format_output_get),
        m!("$formatOutput::set", d::format_output_set),
        m!("$validateOnParse::get", d::validate_on_parse_get),
        m!("$validateOnParse::set", d::validate_on_parse_set),
        m!("$resolveExternals::get", d::resolve_externals_get),
        m!("$resolveExternals::set", d::resolve_externals_set),
        m!("$preserveWhiteSpace::get", d::preserve_white_space_get),
        m!("$preserveWhiteSpace::set", d::preserve_white_space_set),
        m!("$recover::get", d::recover_get),
        m!("$recover::set", d::recover_set),
        m!("$substituteEntities::get", d::substitute_entities_get),
        m!("$substituteEntities::set", d::substitute_entities_set),
        m!("$strictErrorChecking::get", d::strict_error_checking_get),
        m!("$strictErrorChecking::set", d::strict_error_checking_set),
    ],
    ELEMENT_LIKE_HOOKS,
    PARENT_NODE,
);

const CHARDATA_METHODS: &[Meth] = concat_meths!(
    &[
        m!("$data::get", d::data_get),
        m!("$data::set", d::data_set),
        m!("$length::get", d::length_get),
        m!("appendData", d::append_data),
        m!("substringData", d::substring_data),
        m!("insertData", d::insert_data),
        m!("deleteData", d::delete_data),
        m!("replaceData", d::replace_data),
    ],
    SIBLING_HOOKS,
    CHILD_NODE,
);

const FRAGMENT_METHODS: &[Meth] = concat_meths!(
    &[m!("__construct", d::fragment_construct), m!("appendXML", d::append_xml)],
    ELEMENT_LIKE_HOOKS,
    PARENT_NODE,
);

const LIST_METHODS: &[Meth] = &[
    m!("$length::get", l::length_get),
    m!("$[]::get", l::dim_get),
    m!("$[]::has", l::dim_has),
    m!("count", l::count),
    m!("getIterator", l::get_iterator),
    m!("item", l::item),
];

const MAP_METHODS: &[Meth] = &[
    m!("$length::get", l::length_get),
    m!("$[]::get", l::dim_get),
    m!("$[]::has", l::dim_has),
    m!("count", l::count),
    m!("getIterator", l::get_iterator),
    m!("item", l::item),
    m!("getNamedItem", l::get_named_item),
    m!("getNamedItemNS", l::get_named_item_ns),
];

const fn class(name: &'static str, parent: Option<&'static str>, interfaces: &'static [&'static str], properties: &'static [Prop], methods: &'static [Meth]) -> ClassSpec {
    ClassSpec { name, parent, interfaces, flags: 0, properties, methods, constants: &[], debug_info: None }
}

const ELEMENT_PROPS: &[Prop] = props!(
    "tagName", "className", "id", "schemaTypeInfo", "firstElementChild", "lastElementChild", "childElementCount",
    "previousElementSibling", "nextElementSibling",
);
const DOCUMENT_PROPS: &[Prop] = props!(
    "doctype", "implementation", "documentElement", "actualEncoding", "encoding", "xmlEncoding", "standalone",
    "xmlStandalone", "version", "xmlVersion", "strictErrorChecking", "documentURI", "config", "formatOutput",
    "validateOnParse", "resolveExternals", "preserveWhiteSpace", "recover", "substituteEntities",
    "firstElementChild", "lastElementChild", "childElementCount",
);

pub const CLASSES: &[ClassSpec] = &[
    ClassSpec { name: "DOMException", parent: Some("Exception"), interfaces: &[], flags: CLASS_FINAL, properties: &[], methods: &[], constants: &[], debug_info: None },
    ClassSpec { name: "DOMChildNode", parent: None, interfaces: &[], flags: CLASS_INTERFACE, properties: &[], methods: &[("remove", None), ("before", None), ("after", None), ("replaceWith", None)], constants: &[], debug_info: None },
    ClassSpec { name: "DOMParentNode", parent: None, interfaces: &[], flags: CLASS_INTERFACE, properties: &[], methods: &[("append", None), ("prepend", None), ("replaceChildren", None)], constants: &[], debug_info: None },
    class("DOMImplementation", None, &[], &[], &[
        m!("hasFeature", d::implementation_has_feature),
        m!("createDocumentType", d::implementation_create_document_type),
        m!("createDocument", d::implementation_create_document),
    ]),
    class("DOMNode", None, &[], NODE_PROPS, NODE_METHODS),
    class("DOMNameSpaceNode", None, &[], &[], &[]),
    class("DOMDocumentFragment", Some("DOMNode"), &["DOMParentNode"], props!("firstElementChild", "lastElementChild", "childElementCount"), FRAGMENT_METHODS),
    class("DOMDocument", Some("DOMNode"), &["DOMParentNode"], DOCUMENT_PROPS, DOCUMENT_METHODS),
    class("DOMNodeList", None, &["IteratorAggregate", "Countable"], props!("length"), LIST_METHODS),
    class("DOMNamedNodeMap", None, &["IteratorAggregate", "Countable"], props!("length"), MAP_METHODS),
    class("DOMCharacterData", Some("DOMNode"), &["DOMChildNode"], props!("data", "length", "previousElementSibling", "nextElementSibling"), CHARDATA_METHODS),
    class("DOMAttr", Some("DOMNode"), &[], props!("name", "specified", "value", "ownerElement", "schemaTypeInfo"), &[
        m!("__construct", d::attr_construct),
        m!("$name::get", d::attr_name_get),
        m!("$specified::get", d::specified_get),
        m!("$value::get", d::attr_value_get),
        m!("$value::set", d::attr_value_set),
        m!("$ownerElement::get", d::owner_element_get),
        m!("$schemaTypeInfo::get", d::schema_type_info_get),
        m!("isId", d::is_id),
    ]),
    class("DOMElement", Some("DOMNode"), &["DOMParentNode", "DOMChildNode"], ELEMENT_PROPS, ELEMENT_METHODS),
    class("DOMText", Some("DOMCharacterData"), &[], props!("wholeText"), &[
        m!("__construct", d::text_construct),
        m!("$wholeText::get", d::whole_text_get),
        m!("splitText", d::split_text),
        m!("isWhitespaceInElementContent", d::is_whitespace_in_element_content),
        m!("isElementContentWhitespace", d::is_whitespace_in_element_content),
    ]),
    class("DOMComment", Some("DOMCharacterData"), &[], &[], &[m!("__construct", d::comment_construct)]),
    class("DOMCdataSection", Some("DOMText"), &[], &[], &[m!("__construct", d::cdata_construct)]),
    class("DOMDocumentType", Some("DOMNode"), &[], props!("name", "entities", "notations", "publicId", "systemId", "internalSubset"), &[
        m!("$name::get", d::doctype_name_get),
        m!("$entities::get", d::entities_get),
        m!("$notations::get", d::notations_get),
        m!("$publicId::get", d::public_id_get),
        m!("$systemId::get", d::system_id_get),
        m!("$internalSubset::get", d::internal_subset_get),
    ]),
    class("DOMNotation", Some("DOMNode"), &[], props!("publicId", "systemId"), &[
        m!("$publicId::get", d::public_id_get),
        m!("$systemId::get", d::system_id_get),
    ]),
    class("DOMEntity", Some("DOMNode"), &[], props!("publicId", "systemId", "notationName"), &[
        m!("$publicId::get", d::public_id_get),
        m!("$systemId::get", d::system_id_get),
        m!("$notationName::get", d::schema_type_info_get),
    ]),
    class("DOMEntityReference", Some("DOMNode"), &[], &[], &[m!("__construct", d::entity_ref_construct)]),
    class("DOMXPath", None, &[], props!("document", "registerNodeNamespaces"), &[
        m!("__construct", xpath::construct),
        m!("$document::get", xpath::document_get),
        m!("$registerNodeNamespaces::get", xpath::register_node_ns_get),
        m!("$registerNodeNamespaces::set", xpath::register_node_ns_set),
        m!("evaluate", xpath::evaluate),
        m!("query", xpath::query),
        m!("registerNamespace", xpath::register_namespace),
        ("quote", Some(Native::Static(xpath::quote))),
    ]),
    class("DOMProcessingInstruction", Some("DOMNode"), &[], props!("target", "data"), &[
        m!("__construct", d::pi_construct),
        m!("$target::get", d::target_get),
        m!("$data::get", d::data_get),
        m!("$data::set", d::data_set),
    ]),
];

const CONSTANTS: &[(&str, i64)] = &[
    ("XML_ELEMENT_NODE", 1),
    ("XML_ATTRIBUTE_NODE", 2),
    ("XML_TEXT_NODE", 3),
    ("XML_CDATA_SECTION_NODE", 4),
    ("XML_ENTITY_REF_NODE", 5),
    ("XML_ENTITY_NODE", 6),
    ("XML_PI_NODE", 7),
    ("XML_COMMENT_NODE", 8),
    ("XML_DOCUMENT_NODE", 9),
    ("XML_DOCUMENT_TYPE_NODE", 10),
    ("XML_DOCUMENT_FRAG_NODE", 11),
    ("XML_NOTATION_NODE", 12),
    ("XML_HTML_DOCUMENT_NODE", 13),
    ("XML_DTD_NODE", 14),
    ("XML_ELEMENT_DECL_NODE", 15),
    ("XML_ATTRIBUTE_DECL_NODE", 16),
    ("XML_ENTITY_DECL_NODE", 17),
    ("XML_NAMESPACE_DECL_NODE", 18),
    ("XML_LOCAL_NAMESPACE", 18),
    ("XML_ATTRIBUTE_CDATA", 1),
    ("XML_ATTRIBUTE_ID", 2),
    ("XML_ATTRIBUTE_IDREF", 3),
    ("XML_ATTRIBUTE_IDREFS", 4),
    ("XML_ATTRIBUTE_ENTITY", 6),
    ("XML_ATTRIBUTE_NMTOKEN", 7),
    ("XML_ATTRIBUTE_NMTOKENS", 8),
    ("XML_ATTRIBUTE_ENUMERATION", 9),
    ("XML_ATTRIBUTE_NOTATION", 10),
    ("DOM_PHP_ERR", 0),
    ("DOM_INDEX_SIZE_ERR", 1),
    ("DOMSTRING_SIZE_ERR", 2),
    ("DOM_HIERARCHY_REQUEST_ERR", 3),
    ("DOM_WRONG_DOCUMENT_ERR", 4),
    ("DOM_INVALID_CHARACTER_ERR", 5),
    ("DOM_NO_DATA_ALLOWED_ERR", 6),
    ("DOM_NO_MODIFICATION_ALLOWED_ERR", 7),
    ("DOM_NOT_FOUND_ERR", 8),
    ("DOM_NOT_SUPPORTED_ERR", 9),
    ("DOM_INUSE_ATTRIBUTE_ERR", 10),
    ("DOM_INVALID_STATE_ERR", 11),
    ("DOM_SYNTAX_ERR", 12),
    ("DOM_INVALID_MODIFICATION_ERR", 13),
    ("DOM_NAMESPACE_ERR", 14),
    ("DOM_INVALID_ACCESS_ERR", 15),
    ("DOM_VALIDATION_ERR", 16),
];

pub fn register(registry: &mut Registry) {
    registry.add_classes(CLASSES);
    registry.add_data("DOMNode", n::make_node);
    registry.add_data("DOMNodeList", l::make_list);
    registry.add_data("DOMNamedNodeMap", l::make_list);
    registry.add_data("DOMXPath", xpath::make_xpath);
    for (name, value) in CONSTANTS {
        registry.add_constant(name.as_bytes(), Constant::Int(*value));
    }
    registry.add_module("dom");
}

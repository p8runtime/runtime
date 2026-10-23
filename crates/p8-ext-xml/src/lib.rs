#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's XML extensions. So far xmlwriter, over a port of libxml2's
//! writer; the XML parser (and xml, xmlreader, dom, simplexml on it)
//! comes next.

mod constants;
mod encoding;
mod errors;
mod parser;
mod writer;
mod xml;
mod xmlwriter;

use p8_runtime::ext::{Extension, Registry};

pub const EXTENSION: Extension = Extension { name: "xml", register };

fn register(registry: &mut Registry) {
    registry.add_functions(xmlwriter::BUILTINS);
    registry.add_classes(xmlwriter::CLASSES);
    registry.add_data("XMLWriter", xmlwriter::make_state);
    registry.add_dtor("XMLWriter", xmlwriter::destroy);
    registry.add_module("xmlwriter");
    registry.add_functions(xml::BUILTINS);
    registry.add_classes(xml::CLASSES);
    registry.add_data("XMLParser", xml::make_state);
    registry.mark_reenters(xml::REENTER);
    for (_, name, value) in constants::CONSTANTS {
        registry.add_constant(name.as_bytes(), *value);
    }
    registry.add_module("xml");
}

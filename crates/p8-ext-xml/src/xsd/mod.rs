//! XML Schema (XSD 1.0) validation for DOMDocument::schemaValidate*: a
//! schema compiler and a validator that report libxml2 2.9's errors.

mod content;
mod datatypes;
mod regex;
mod schema;
mod validate;

use crate::dom::build;
use crate::dom::tree::Doc;
use crate::parser::XmlError;
pub(crate) use schema::Schema;
use std::path::PathBuf;

const FROM_SCHEMASP: i32 = 16;
pub(crate) const FROM_IO: i32 = 8;

fn error(domain: i32, code: i32, level: i32, message: String) -> XmlError {
    XmlError { domain, code, level, message: format!("{message}\n"), line: 0, column: 0, file: None, context: Vec::new(), context_col: 0 }
}

/// The name libxml2 gives a schema document read from memory.
const IN_MEMORY: &str = "in_memory_buffer";

/// Compiles the schema in `data`; `file` is its full path when it was read
/// from a file. The Err holds everything libxml2 would have reported.
pub(crate) fn compile(data: Vec<u8>, file: Option<String>) -> Result<Schema, Vec<XmlError>> {
    let name = file.clone().unwrap_or_else(|| IN_MEMORY.to_string());
    let mut doc = Doc::new(b"DOMDocument".to_vec());
    let parser = build::parse(&mut doc, data, file.clone(), 0);
    let mut errors: Vec<XmlError> = parser.errors.clone();
    if !parser.well_formed {
        errors.push(error(FROM_SCHEMASP, 3067, 2, format!("Failed to parse the XML resource '{name}'.")));
        return Err(errors);
    }
    let mut compiler = schema::Compiler::new();
    // A schemaLocation is a URI reference, resolved against the schema
    // that names it (one read from memory has no base): a file: URI or
    // a path loads, percent-escapes decoded; other schemes do not.
    let mut loader = |location: &str, base: &str| -> Option<(Doc, String)> {
        let url = if base == IN_MEMORY { location.to_string() } else { crate::uri::resolve(location, base) };
        let path = PathBuf::from(crate::uri::local_path(&url)?);
        let data = std::fs::read(&path).ok()?;
        let full = std::path::absolute(&path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string_lossy().into_owned());
        let mut sub = Doc::new(b"DOMDocument".to_vec());
        let p = build::parse(&mut sub, data, Some(full.clone()), 0);
        if !p.well_formed {
            return None;
        }
        Some((sub, full))
    };
    if !compiler.add_document(doc, name.clone(), &mut loader, None) {
        errors.push(error(FROM_SCHEMASP, 1772, 2, format!("The XML document '{name}' is not a schema document.")));
        return Err(errors);
    }
    compiler.finish();
    if !compiler.errors.is_empty() {
        errors.extend(compiler.errors);
        return Err(errors);
    }
    Ok(compiler.schema)
}

/// An error for a schema file that could not be read.
pub(crate) fn load_errors(path: &str) -> Vec<XmlError> {
    vec![
        error(FROM_IO, 1549, 1, format!("failed to load external entity \"{path}\"")),
        error(FROM_SCHEMASP, 1757, 2, format!("Failed to locate the main schema resource at '{path}'.")),
    ]
}

/// Validates `doc`; with `create`, attribute and element defaults the schema
/// gives are added to it. Returns the errors, none when valid.
pub(crate) fn validate(schema: &Schema, doc: &mut Doc, create: bool) -> Vec<XmlError> {
    let mut v = validate::Validator::new(schema, doc, create);
    v.run();
    v.errors
}

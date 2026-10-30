#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! PHP 8.4's signatures for its builtin functions and methods, generated
//! from php-src's `*.stub.php` files (`examples/gen_signatures.rs`): the
//! parameters' names, types, defaults and by-reference flags that PHP's
//! argument checks and error messages come from. Shared by both engines.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_compile.c - zend_type_to_string

mod modules;
mod table;

pub const T_MIXED: u32 = 1 << 0;
pub const T_STATIC: u32 = 1 << 1;
pub const T_CALLABLE: u32 = 1 << 2;
pub const T_OBJECT: u32 = 1 << 3;
pub const T_ARRAY: u32 = 1 << 4;
pub const T_STRING: u32 = 1 << 5;
pub const T_INT: u32 = 1 << 6;
pub const T_FLOAT: u32 = 1 << 7;
pub const T_BOOL: u32 = 1 << 8;
pub const T_FALSE: u32 = 1 << 9;
pub const T_TRUE: u32 = 1 << 10;
pub const T_VOID: u32 = 1 << 11;
pub const T_NEVER: u32 = 1 << 12;
pub const T_NULL: u32 = 1 << 13;

/// A PHP function's or built-in method's declared signature.
pub struct Sig {
    /// As declared: `strlen`, `ns\\name`, `ArrayAccess::offsetSet`.
    pub name: &'static str,
    pub params: &'static [Param],
    pub ret_mask: u32,
    pub ret_classes: &'static [&'static str],
    /// `@tentative-return-type`: an override without it is only deprecated.
    pub tentative: bool,
    pub returns_ref: bool,
    /// Method modifiers: SIG_FINAL | SIG_STATIC | SIG_ABSTRACT | SIG_PRIVATE | SIG_PROTECTED.
    pub flags: u8,
}

pub const SIG_FINAL: u8 = 1;
pub const SIG_STATIC: u8 = 2;
pub const SIG_ABSTRACT: u8 = 4;
pub const SIG_PRIVATE: u8 = 8;
pub const SIG_PROTECTED: u8 = 16;

/// One declared parameter. `mask` is the type's T_* bits plus the class
/// names in `classes`; an untyped parameter has neither.
pub struct Param {
    pub name: &'static str,
    pub mask: u32,
    pub classes: &'static [&'static str],
    pub by_ref: bool,
    pub variadic: bool,
    /// The default as written in the stub, when the parameter is optional.
    pub default: Option<&'static str>,
}

impl Param {
    pub fn untyped(&self) -> bool {
        self.mask == 0 && self.classes.is_empty()
    }

    pub fn optional(&self) -> bool {
        self.default.is_some() || self.variadic
    }
}

/// The signature of the PHP function `name`, or of `Class::method` (any
/// case), if PHP has one.
/// The extension PHP puts builtin function `name` in ("Core", "standard",
/// "SPL"...), as reflection reports it.
pub fn function_module(name: &[u8]) -> Option<&'static str> {
    module_in(modules::FUNCTION_MODULES, name)
}

/// The extension PHP puts builtin class (or interface, trait) `name` in.
pub fn class_module(name: &[u8]) -> Option<&'static str> {
    module_in(modules::CLASS_MODULES, name)
}

fn module_in(table: &'static [(&'static str, &'static str)], name: &[u8]) -> Option<&'static str> {
    let lower = String::from_utf8_lossy(name.strip_prefix(b"\\").unwrap_or(name)).to_ascii_lowercase();
    table.binary_search_by(|(n, _)| (*n).cmp(lower.as_str())).ok().map(|i| table[i].1)
}

pub fn lookup(name: &[u8]) -> Option<&'static Sig> {
    let name = std::str::from_utf8(name).ok()?;
    let name = name.strip_prefix('\\').unwrap_or(name);
    let table = table::SIGNATURES;
    table
        .binary_search_by(|s| cmp_ignore_case(s.name, name))
        .ok()
        .map(|i| &table[i])
}

fn cmp_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    a.bytes()
        .map(|c| c.to_ascii_lowercase())
        .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
}

/// "Function f() is deprecated since 8.x, <message>" for a function or
/// method PHP 8.4 declares #[\Deprecated] (zend_deprecated_function).
pub fn deprecation(sig: &Sig) -> Option<String> {
    let table = table::DEPRECATED;
    let i = table
        .binary_search_by(|(name, _, _)| cmp_ignore_case(name, sig.name))
        .ok()?;
    let (name, since, message) = table[i];
    let mut text = match name.split_once("::") {
        Some((class, method)) => format!("Method {}::{}() is deprecated", class, method),
        None => format!("Function {}() is deprecated", name),
    };
    if let Some(since) = since {
        text.push_str(&format!(" since {}", since));
    }
    if let Some(message) = message {
        text.push_str(&format!(", {}", message));
    }
    Some(text)
}

/// Every signature: functions, then `Class::method`s, by name.
pub fn all() -> &'static [Sig] {
    table::SIGNATURES
}

/// A declared type as PHP prints it (zend_type_to_string).
pub fn type_to_string(mask: u32, classes: &[&str]) -> String {
    let mut parts: Vec<&str> = classes.to_vec();
    if mask & T_MIXED != 0 {
        parts.push("mixed");
        return parts.join("|");
    }
    for (bit, name) in [
        (T_STATIC, "static"),
        (T_CALLABLE, "callable"),
        (T_OBJECT, "object"),
        (T_ARRAY, "array"),
        (T_STRING, "string"),
        (T_INT, "int"),
        (T_FLOAT, "float"),
    ] {
        if mask & bit != 0 {
            parts.push(name);
        }
    }
    if mask & T_BOOL != 0 || mask & (T_TRUE | T_FALSE) == T_TRUE | T_FALSE {
        parts.push("bool");
    } else if mask & T_FALSE != 0 {
        parts.push("false");
    } else if mask & T_TRUE != 0 {
        parts.push("true");
    }
    if mask & T_VOID != 0 {
        parts.push("void");
    }
    if mask & T_NEVER != 0 {
        parts.push("never");
    }
    if mask & T_NULL != 0 {
        if parts.len() == 1 && !parts[0].contains('&') {
            return format!("?{}", parts[0]);
        }
        parts.push("null");
    }
    parts.join("|")
}

//! ext/libxml: the libxml_* functions and LibXMLError. Every libxml2 error
//! a parse raises goes through [`report`]: it becomes the last error
//! (libxml_get_last_error) and either a PHP warning or, with internal
//! errors on (libxml_use_internal_errors), an entry in the error list.

use crate::parser::XmlError;
use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::Visibility;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

type R = Result<Value, Throw>;

const INTERNAL: &str = "libxml.internal_errors";
/// The errors gathered while internal errors are on, oldest first; each a
/// list [level, code, column, message, file, line].
const ERRORS: &str = "libxml.errors";
const LAST: &str = "libxml.last_error";
const LOADER: &str = "libxml.entity_loader";
const DISABLED: &str = "libxml.entity_loader_disabled";

/// One libxml2 error, as the PHP error handler gets it: a warning made by
/// `warning` (each caller words its own), or kept in the error list.
pub(crate) fn report(ctx: &mut dyn Ctx, error: &XmlError, warning: impl FnOnce(&mut dyn Ctx)) {
    let entry = Value::array(PhpArray::from_list(vec![
        Value::int(i64::from(error.level)),
        Value::int(i64::from(error.code)),
        Value::int(i64::from(error.column)),
        Value::str(PhpStr::from(error.message.as_bytes())),
        Value::str(PhpStr::from(error.file.as_deref().unwrap_or("").as_bytes())),
        Value::int(i64::from(error.line)),
    ]));
    *ctx.state(LAST) = entry.clone();
    if internal(ctx) {
        let mut list = match std::mem::take(ctx.state(ERRORS)).into_array() {
            Ok(a) => Rc::unwrap_or_clone(a),
            Err(_) => PhpArray::new(),
        };
        // A list made here: its next key is always free.
        let _ = list.push(entry);
        *ctx.state(ERRORS) = Value::array(list);
    } else {
        warning(ctx);
    }
}

/// Whether libxml_use_internal_errors(true) is in effect.
pub(crate) fn internal(ctx: &mut dyn Ctx) -> bool {
    ctx.state(INTERNAL).as_bool() == Some(true)
}

fn error_object(ctx: &mut dyn Ctx, entry: &Value) -> Value {
    let Some(fields) = entry.as_array() else { return Value::FALSE };
    let class = ctx.class(b"LibXMLError").expect("LibXMLError");
    let o = ctx.new_object(&class);
    for (name, value) in ["level", "code", "column", "message", "file", "line"].iter().zip(fields.values()) {
        o.class.write(&o, name.as_bytes(), b"LibXMLError", value.clone());
    }
    Value::object(o)
}

fn libxml_use_internal_errors(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let was = internal(ctx);
    match args.first().map(Value::deref) {
        None => {}
        Some(v) if v.is_null() => {}
        Some(v) => {
            let on = v.to_bool();
            *ctx.state(INTERNAL) = Value::bool(on);
            // Turning them off drops what was gathered.
            if !on {
                *ctx.state(ERRORS) = Value::NULL;
            }
        }
    }
    Ok(Value::bool(was))
}

fn libxml_get_errors(ctx: &mut dyn Ctx, _args: &mut [Value]) -> R {
    let entries: Vec<Value> = match ctx.state(ERRORS).as_array() {
        Some(a) => a.values().cloned().collect(),
        None => Vec::new(),
    };
    let objects = entries.iter().map(|e| error_object(ctx, e)).collect();
    Ok(Value::array(PhpArray::from_list(objects)))
}

fn libxml_get_last_error(ctx: &mut dyn Ctx, _args: &mut [Value]) -> R {
    let last = ctx.state(LAST).clone();
    Ok(error_object(ctx, &last))
}

fn libxml_clear_errors(ctx: &mut dyn Ctx, _args: &mut [Value]) -> R {
    *ctx.state(ERRORS) = Value::NULL;
    *ctx.state(LAST) = Value::NULL;
    Ok(Value::NULL)
}

/// Kept for the loaders that would read external resources (p8's never do).
fn libxml_set_streams_context(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    *ctx.state("libxml.streams_context") = args.first().map(Value::deref).unwrap_or_default();
    Ok(Value::NULL)
}

/// Deprecated since 8.0 (the registry warns): returns the previous setting.
fn libxml_disable_entity_loader(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let was = ctx.state(DISABLED).as_bool() == Some(true);
    let disable = args.first().is_none_or(|v| v.deref().to_bool());
    *ctx.state(DISABLED) = Value::bool(disable);
    Ok(Value::bool(was))
}

fn libxml_set_external_entity_loader(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    *ctx.state(LOADER) = args.first().map(Value::deref).unwrap_or_default();
    Ok(Value::TRUE)
}

fn libxml_get_external_entity_loader(ctx: &mut dyn Ctx, _args: &mut [Value]) -> R {
    Ok(ctx.state(LOADER).clone())
}

pub(crate) static BUILTINS: &[Builtin] = &[
    Builtin { name: "libxml_use_internal_errors", run: libxml_use_internal_errors, by_ref: &[] },
    Builtin { name: "libxml_get_errors", run: libxml_get_errors, by_ref: &[] },
    Builtin { name: "libxml_get_last_error", run: libxml_get_last_error, by_ref: &[] },
    Builtin { name: "libxml_clear_errors", run: libxml_clear_errors, by_ref: &[] },
    Builtin { name: "libxml_set_streams_context", run: libxml_set_streams_context, by_ref: &[] },
    Builtin { name: "libxml_disable_entity_loader", run: libxml_disable_entity_loader, by_ref: &[] },
    Builtin { name: "libxml_set_external_entity_loader", run: libxml_set_external_entity_loader, by_ref: &[] },
    Builtin { name: "libxml_get_external_entity_loader", run: libxml_get_external_entity_loader, by_ref: &[] },
];

pub(crate) static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "LibXMLError",
    parent: None,
    interfaces: &[],
    flags: 0,
    properties: &[
        ("level", Constant::Null, Visibility::Public),
        ("code", Constant::Null, Visibility::Public),
        ("column", Constant::Null, Visibility::Public),
        ("message", Constant::Null, Visibility::Public),
        ("file", Constant::Null, Visibility::Public),
        ("line", Constant::Null, Visibility::Public),
    ],
    methods: &[],
    constants: &[],
    debug_info: None,
}];

//! ext/standard's variable functions: types, conversions, print_r and
//! var_export. Arguments arrive already checked and converted to their
//! declared types (p8_runtime::args).
//!
//! Reference: $PHP_SRC_PATH/ext/standard/type.c, var.c;
//! $PHP_SRC_PATH/Zend/zend.c - print_hash, zend_print_zval_r_to_buf

use crate::arg;
use p8_runtime::array::Key;
use p8_runtime::ext::{Builtin, Ctx, Fault, FastBuiltin, Throw};
use p8_runtime::lent;
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::object::{CLASS_ENUM, CLASS_RESOURCE, Object, Visibility};
use p8_runtime::stream::resource;
use std::rc::Rc;
use p8_runtime::numeric::numeric_string;
use p8_runtime::string::PhpStr;
use p8_runtime::value::{Kind, Value};

/// The type tests and conversions, on the borrowed ABI.
pub static FAST: &[FastBuiltin] = &[
    FastBuiltin { name: "gettype", run: gettype },
    FastBuiltin { name: "is_object", run: is_object },
    FastBuiltin { name: "get_debug_type", run: get_debug_type },
    FastBuiltin { name: "is_null", run: is_null },
    FastBuiltin { name: "is_bool", run: is_bool },
    FastBuiltin { name: "is_int", run: is_int },
    FastBuiltin { name: "is_integer", run: is_int },
    FastBuiltin { name: "is_long", run: is_int },
    FastBuiltin { name: "is_float", run: is_float },
    FastBuiltin { name: "is_double", run: is_float },
    FastBuiltin { name: "is_string", run: is_string },
    FastBuiltin { name: "is_array", run: is_array },
    FastBuiltin { name: "is_scalar", run: is_scalar },
    FastBuiltin { name: "is_numeric", run: is_numeric },
    FastBuiltin { name: "is_iterable", run: is_iterable },
    FastBuiltin { name: "is_countable", run: is_countable },
    FastBuiltin { name: "intval", run: intval },
    FastBuiltin { name: "floatval", run: floatval },
    FastBuiltin { name: "doubleval", run: floatval },
    FastBuiltin { name: "boolval", run: boolval },
];

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "is_resource", run: is_resource, by_ref: &[] },
    Builtin { name: "get_resource_type", run: get_resource_type, by_ref: &[] },
    Builtin { name: "get_resource_id", run: get_resource_id, by_ref: &[] },
    Builtin { name: "strval", run: strval, by_ref: &[] },
    Builtin { name: "var_dump", run: var_dump, by_ref: &[] },
    Builtin { name: "print_r", run: print_r, by_ref: &[] },
    Builtin { name: "var_export", run: var_export, by_ref: &[] },
];

fn gettype(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let v = lent::value(args, 0);
    Ok(Value::str(match v.kind() {
        Kind::Null | Kind::Undef => "NULL",
        Kind::Bool => "boolean",
        Kind::Int => "integer",
        Kind::Float => "double",
        Kind::Str => "string",
        Kind::Array => "array",
        Kind::Object => match as_resource(&v) {
            Some(o) => if closed(o) { "resource (closed)" } else { "resource" },
            None => "object",
        },
        Kind::Ref => unreachable!("lent::value looks through references"),
    }))
}

/// Whether a resource has been closed (its type is "Unknown" then).
fn closed(o: &Object) -> bool {
    resource(o).is_some_and(|r| r.closed.get())
}

/// The object of a resource value (an object of a resource class).
fn as_resource(v: &Value) -> Option<&Rc<Object>> {
    v.as_object().filter(|o| o.class.flags & CLASS_RESOURCE != 0)
}

fn is_object(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::value(args, 0).as_object().is_some_and(|o| o.class.flags & CLASS_RESOURCE == 0)))
}

/// is_resource: an open resource.
fn is_resource(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::bool(as_resource(&arg(args, 0)).is_some_and(|o| !closed(o))))
}

fn get_resource_type(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    match as_resource(&v) {
        Some(o) => Ok(Value::str(if closed(o) { PhpStr::from("Unknown") } else { o.class.name.clone() })),
        None => Err(ctx.throw(
            "TypeError",
            format!("get_resource_type(): Argument #1 ($resource) must be of type resource, {} given", v.type_name()),
        )),
    }
}

fn get_resource_id(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    match as_resource(&v) {
        Some(o) => Ok(Value::int(i64::from(o.id))),
        None => Err(ctx.throw(
            "TypeError",
            format!("get_resource_id(): Argument #1 ($resource) must be of type resource, {} given", v.type_name()),
        )),
    }
}

fn get_debug_type(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let v = lent::value(args, 0);
    let name = match v.as_object() {
        Some(o) if o.class.flags & CLASS_RESOURCE != 0 => {
            if closed(o) { "resource (closed)".into() } else { format!("resource ({})", String::from_utf8_lossy(o.class.name.as_bytes())).into() }
        }
        Some(o) => String::from_utf8_lossy(o.class.shown()).into_owned().into(),
        None if v.is_null_or_undef() => "null".into(),
        None if v.is_bool() => "bool".into(),
        None => v.type_name(),
    };
    Ok(Value::str(name.into_owned()))
}

fn is_null(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::value(args, 0).is_null_or_undef()))
}

fn is_bool(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::value(args, 0).is_bool()))
}

fn is_int(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::value(args, 0).is_int()))
}

fn is_float(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::value(args, 0).is_float()))
}

fn is_string(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::value(args, 0).is_str()))
}

fn is_array(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::value(args, 0).is_array()))
}

/// is_iterable(): an array or a Traversable object.
fn is_iterable(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let v = lent::value(args, 0);
    Ok(Value::bool(v.is_array() || v.as_object().is_some_and(|o| o.class.is_a(b"traversable"))))
}

/// is_countable(): an array or a Countable object.
fn is_countable(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let v = lent::value(args, 0);
    Ok(Value::bool(v.is_array() || v.as_object().is_some_and(|o| o.class.is_a(b"countable"))))
}

fn is_scalar(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let v = lent::value(args, 0);
    Ok(Value::bool(v.is_bool() || v.is_number() || v.is_str()))
}

/// An int or float, or a string that is entirely a number (whitespace
/// allowed around it, as of PHP 8).
fn is_numeric(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let v = lent::value(args, 0);
    Ok(Value::bool(v.is_number() || v.as_str().is_some_and(|s| numeric_string(s.as_bytes()).is_some())))
}

/// intval($value, $base = 10): a string in another base is read as
/// strtol does (prefixes 0x, 0o, 0b and a leading 0 for base 0 / 16 / 8 / 2).
fn intval(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let v = lent::value(args, 0);
    let base = if args.len() > 1 { lent::int(args, 1) } else { 10 };
    let Some(s) = v.as_str() else { return Ok(Value::int(v.to_int())) };
    if base == 10 {
        return Ok(Value::int(v.to_int()));
    }
    Ok(Value::int(strtol(s.as_bytes(), base)))
}

/// C's strtol, saturating at the int range, as ZEND_STRTOL.
fn strtol(s: &[u8], base: i64) -> i64 {
    let mut i = s.iter().take_while(|c| c.is_ascii_whitespace()).count();
    let negative = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let rest = &s[i..];
    let prefixed = |p: u8| rest.len() > 2 && rest[0] == b'0' && rest[1].eq_ignore_ascii_case(&p);
    let (base, mut digits) = match base {
        16 if prefixed(b'x') => (16, &rest[2..]),
        8 if prefixed(b'o') => (8, &rest[2..]),
        2 if prefixed(b'b') => (2, &rest[2..]),
        0 if prefixed(b'x') => (16, &rest[2..]),
        0 if prefixed(b'o') => (8, &rest[2..]),
        0 if prefixed(b'b') => (2, &rest[2..]),
        0 if rest.first() == Some(&b'0') => (8, &rest[1..]),
        0 => (10, rest),
        b => (b, rest),
    };
    if !(2..=36).contains(&base) {
        return 0;
    }
    let mut n: i64 = 0;
    let mut overflow = false;
    while let Some((&c, tail)) = digits.split_first() {
        let d = match c {
            b'0'..=b'9' => i64::from(c - b'0'),
            b'a'..=b'z' => i64::from(c - b'a') + 10,
            b'A'..=b'Z' => i64::from(c - b'A') + 10,
            _ => break,
        };
        if d >= base {
            break;
        }
        match n.checked_mul(base).and_then(|n| if negative { n.checked_sub(d) } else { n.checked_add(d) }) {
            Some(next) => n = next,
            None => overflow = true,
        }
        digits = tail;
    }
    if overflow {
        return if negative { i64::MIN } else { i64::MAX };
    }
    n
}

fn floatval(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::float(lent::value(args, 0).to_float()))
}

/// strval(): a string as a string cast makes it (an object's __toString(),
/// an array "Array" with a warning).
fn strval(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str(crate::printf::to_str(ctx, &arg(args, 0))?))
}

fn boolval(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    Ok(Value::bool(lent::value(args, 0).to_bool()))
}

/// print_r($value, $return = false)
fn print_r(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    print_r_to(&arg(args, 0), 0, &mut out, &mut Vec::new());
    if args.len() > 1 && arg(args, 1).to_bool() {
        return Ok(Value::str(PhpStr::new(out)));
    }
    ctx.echo(&out);
    Ok(Value::TRUE)
}

pub(crate) fn print_r_to(v: &Value, indent: usize, out: &mut Vec<u8>, seen: &mut Vec<Rc<Object>>) {
    match v.kind() {
        Kind::Array => {
            let a = v.as_array().expect("an array");
            out.extend_from_slice(b"Array\n");
            out.extend(std::iter::repeat_n(b' ', indent));
            out.extend_from_slice(b"(\n");
            for (k, v) in a.iter() {
                out.extend(std::iter::repeat_n(b' ', indent + 4));
                out.push(b'[');
                match k {
                    Key::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
                    Key::Str(s) => out.extend_from_slice(s.as_bytes()),
                }
                out.extend_from_slice(b"] => ");
                print_r_to(v, indent + 8, out, seen);
                out.push(b'\n');
            }
            out.extend(std::iter::repeat_n(b' ', indent));
            out.extend_from_slice(b")\n");
        }
        Kind::Ref => print_r_to(&v.as_reference().expect("a reference").borrow(), indent, out, seen),
        Kind::Object if as_resource(v).is_some() => {
            let o = v.as_object().expect("an object");
            out.extend_from_slice(format!("Resource id #{}", o.id).as_bytes());
        }
        Kind::Object => {
            let o = v.as_object().expect("an object");
            out.extend_from_slice(o.class.shown());
            // An enum: "Suit Enum" (":string" when backed).
            if o.class.flags & CLASS_ENUM != 0 {
                out.extend_from_slice(b" Enum");
                if let Some(t) = &o.class.enum_backing {
                    out.push(b':');
                    out.extend_from_slice(t.as_bytes());
                }
                out.push(b'\n');
            } else {
                out.extend_from_slice(b" Object\n");
            }
            if seen.iter().any(|s| Rc::ptr_eq(s, o)) {
                out.extend_from_slice(b" *RECURSION*");
                return;
            }
            seen.push(Rc::clone(o));
            out.extend(std::iter::repeat_n(b' ', indent));
            out.extend_from_slice(b"(\n");
            for e in entries(o) {
                let Some(v) = e.value else { continue };
                out.extend(std::iter::repeat_n(b' ', indent + 4));
                out.push(b'[');
                out.extend_from_slice(e.name.as_bytes());
                match e.visibility {
                    Visibility::Public => {}
                    Visibility::Protected => out.extend_from_slice(b":protected"),
                    Visibility::Private => {
                        out.push(b':');
                        out.extend_from_slice(e.declaring.as_bytes());
                        out.extend_from_slice(b":private");
                    }
                }
                out.extend_from_slice(b"] => ");
                print_r_to(&v, indent + 8, out, seen);
                out.push(b'\n');
            }
            out.extend(std::iter::repeat_n(b' ', indent));
            out.extend_from_slice(b")\n");
            seen.pop();
        }
        _ => {
            if let Some(s) = v.to_php_string() {
                out.extend_from_slice(s.as_bytes());
            }
        }
    }
}

/// var_export($value, $return = false)
fn var_export(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    export(ctx, &arg(args, 0), 1, &mut out)?;
    if args.len() > 1 && arg(args, 1).to_bool() {
        return Ok(Value::str(PhpStr::new(out)));
    }
    ctx.echo(&out);
    Ok(Value::NULL)
}

/// php_var_export_ex
fn export(ctx: &mut dyn Ctx, v: &Value, level: usize, out: &mut Vec<u8>) -> Result<(), Throw> {
    // A lazy object is initialized first.
    if let Some(o) = v.as_object()
        && o.lazy.get() != 0
    {
        let real = ctx.lazy_initialize(o)?;
        return export_value(ctx, &Value::object(real), level, out);
    }
    export_value(ctx, v, level, out)
}

fn export_value(ctx: &mut dyn Ctx, v: &Value, level: usize, out: &mut Vec<u8>) -> Result<(), Throw> {
    match v.kind() {
        // PHP also warns "var_export does not handle resources".
        Kind::Null | Kind::Undef => out.extend_from_slice(b"NULL"),
        Kind::Object if as_resource(v).is_some() => out.extend_from_slice(b"NULL"),
        Kind::Bool => out.extend_from_slice(if v.as_bool().expect("a bool") { b"true" } else { b"false" }),
        // PHP_INT_MIN has no literal: written as an expression.
        Kind::Int if v.as_int() == Some(i64::MIN) => out.extend_from_slice(b"-9223372036854775807-1"),
        Kind::Int => out.extend_from_slice(v.as_int().expect("an int").to_string().as_bytes()),
        Kind::Float => {
            let f = v.as_float().expect("a float");
            let text = float_to_string(f, Precision::SERIALIZE, 'E');
            out.extend_from_slice(text.as_bytes());
            if f.is_finite() && !text.contains(['.', 'E', 'e']) {
                out.extend_from_slice(b".0");
            }
        }
        Kind::Str => export_string(v.as_str().expect("a string").as_bytes(), out),
        Kind::Array => {
            let a = v.as_array().expect("an array");
            if level > 1 {
                out.push(b'\n');
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.extend_from_slice(b"array (\n");
            for (k, v) in a.iter() {
                out.extend(std::iter::repeat_n(b' ', level + 1));
                match k {
                    Key::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
                    Key::Str(s) => export_string(s.as_bytes(), out),
                }
                out.extend_from_slice(b" => ");
                export(ctx, v, level + 2, out)?;
                out.extend_from_slice(b",\n");
            }
            if level > 1 {
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.push(b')');
        }
        Kind::Ref => {
            let v = v.as_reference().expect("a reference").borrow().clone();
            export(ctx, &v, level, out)?;
        }
        Kind::Object if v.as_object().is_some_and(|o| o.class.flags & CLASS_ENUM != 0) => {
            let o = v.as_object().expect("an object");
            if level > 1 {
                out.push(b'\n');
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.push(b'\\');
            out.extend_from_slice(o.class.name.as_bytes());
            out.extend_from_slice(b"::");
            out.extend_from_slice(case_name(o).as_bytes());
        }
        Kind::Object => {
            let o = v.as_object().expect("an object");
            if level > 1 {
                out.push(b'\n');
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            let std_class = o.class.lower == b"stdclass";
            if std_class {
                out.extend_from_slice(b"(object) array(\n");
            } else {
                out.push(b'\\');
                out.extend_from_slice(o.class.name.as_bytes());
                out.extend_from_slice(b"::__set_state(array(\n");
            }
            for e in entries_through_hooks(ctx, o)? {
                let Some(v) = e.value else { continue };
                out.extend(std::iter::repeat_n(b' ', level + 2));
                export_string(e.name.as_bytes(), out);
                out.extend_from_slice(b" => ");
                export(ctx, &v, level + 2, out)?;
                out.extend_from_slice(b",\n");
            }
            if level > 1 {
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.extend_from_slice(if std_class { b")" } else { b"))" });
        }
    }
    Ok(())
}

/// The entries as var_export lists them: a hooked property's value
/// through its get hook (virtual ones too).
fn entries_through_hooks(ctx: &mut dyn Ctx, o: &Rc<Object>) -> Result<Vec<Entry>, Throw> {
    if o.class.flags & p8_runtime::object::CLASS_HOOKS == 0 {
        return Ok(entries(o));
    }
    let mut out = Vec::new();
    let mut slot_entries = entries(o).into_iter().peekable();
    for (i, p) in o.class.properties.iter().enumerate() {
        if p.hooks == 0 {
            // The plain ones as entries() made them (in slot order).
            if let Some(e) = slot_entries.next_if(|e| e.name == p.name && e.declaring == p.declaring_name) {
                out.push(e);
            }
            continue;
        }
        if let Some(e) = slot_entries.next_if(|e| e.name == p.name && e.declaring == p.declaring_name) {
            drop(e);
        }
        let value = ctx.prop_value(o, i)?;
        if value.is_some() {
            out.push(Entry { numeric: false, name: p.name.clone(), visibility: p.visibility, declaring: p.declaring_name.clone(), value, ty: p.ty.clone() });
        }
    }
    out.extend(slot_entries);
    Ok(out)
}

/// One of an object's properties as the dump functions list them.
struct Entry {
    name: PhpStr,
    /// An integer key of debug info (SplFixedArray's): `[0]`, not `["0"]`.
    numeric: bool,
    visibility: Visibility,
    declaring: PhpStr,
    /// None: a typed property not yet initialized.
    value: Option<Value>,
    ty: Option<PhpStr>,
}

/// An object's properties in PHP's order: declared slots (the parent's
/// first), then dynamic ones. An unset untyped property is left out.
/// An enum case object's case name.
fn case_name(o: &Object) -> PhpStr {
    o.class.read(o, b"name", &o.class.lower).to_php_string().unwrap_or_else(PhpStr::empty)
}

fn entries(o: &Object) -> Vec<Entry> {
    let mut list = Vec::new();
    // An initialized lazy proxy shows its real instance.
    if let Some(inst) = o.proxy_instance() {
        list.push(Entry { numeric: false, name: PhpStr::from("instance"), visibility: Visibility::Public, declaring: PhpStr::empty(), value: Some(Value::object(inst)), ty: None });
        return list;
    }
    if let Some(info) = o.class.debug_info.or(o.class.get_properties) {
        for (k, v) in info(o).iter() {
            let numeric = matches!(k, Key::Int(_));
            let name = match k {
                Key::Int(i) => PhpStr::from(i.to_string()),
                Key::Str(s) => s.to_php_str(),
            };
            // Mangled names, as PHP's debug info gives them: "\0Class\0name"
            // (private) and "\0*\0name" (protected).
            let (name, visibility, declaring) = match name.as_bytes().strip_prefix(b"\0").and_then(|r| r.iter().position(|&c| c == 0).map(|i| (r[..i].to_vec(), r[i + 1..].to_vec()))) {
                Some((class, prop)) if class == b"*" => (PhpStr::from(prop.as_slice()), Visibility::Protected, PhpStr::empty()),
                Some((class, prop)) => (PhpStr::from(prop.as_slice()), Visibility::Private, PhpStr::from(class.as_slice())),
                None => (name, Visibility::Public, PhpStr::empty()),
            };
            list.push(Entry { numeric, name, visibility, declaring, value: Some(v.clone()), ty: None });
        }
        return list;
    }
    let slots = o.slot_values();
    for (p, v) in o.class.properties.iter().zip(slots.iter()) {
        // A virtual property stores nothing, a builtin's hidden state is
        // not shown: neither is listed.
        if p.hooks & p8_runtime::object::HOOK_VIRTUAL != 0 || p.is_internal() {
            continue;
        }
        let value = match v {
            v if v.is_undef() && p.ty.is_none() => continue,
            v if v.is_undef() => None,
            v => Some(v.clone()),
        };
        list.push(Entry { numeric: false, name: p.name.clone(), visibility: p.visibility, declaring: p.declaring_name.clone(), value, ty: p.ty.clone() });
    }
    for (k, v) in o.dynamic.borrow().iter() {
        let name = match k {
            Key::Int(i) => PhpStr::from(i.to_string()),
            Key::Str(s) => s.to_php_str(),
        };
        list.push(Entry { numeric: false, name, visibility: Visibility::Public, declaring: PhpStr::empty(), value: Some(v.clone()), ty: None });
    }
    list
}

/// var_dump($value, ...$values)
fn var_dump(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    // Each argument is a value (a reference a builtin returned is read through).
    for v in args.iter() {
        dump(&v.deref(), 0, &mut out, &mut seen);
    }
    ctx.echo(&out);
    Ok(Value::NULL)
}

/// A reference inside an array or object: `&` before it when something
/// else holds it too (more than `holders` holders here).
fn dump_ref(cell: &Rc<std::cell::RefCell<Value>>, holders: usize, indent: usize, out: &mut Vec<u8>, seen: &mut Vec<Rc<Object>>) {
    let start = out.len() + indent;
    let inner = cell.borrow().clone();
    dump(&inner, indent, out, seen);
    if Rc::strong_count(cell) > holders && !out[start..].starts_with(b"*RECURSION*") {
        out.insert(start, b'&');
    }
}

fn dump(v: &Value, indent: usize, out: &mut Vec<u8>, seen: &mut Vec<Rc<Object>>) {
    let pad = " ".repeat(indent);
    match v.kind() {
        Kind::Null | Kind::Undef => out.extend_from_slice(format!("{pad}NULL\n").as_bytes()),
        Kind::Bool => out.extend_from_slice(format!("{pad}bool({})\n", v.as_bool().expect("a bool")).as_bytes()),
        Kind::Int => out.extend_from_slice(format!("{pad}int({})\n", v.as_int().expect("an int")).as_bytes()),
        Kind::Float => {
            let s = float_to_string(v.as_float().expect("a float"), Precision::SERIALIZE, 'E');
            out.extend_from_slice(format!("{pad}float({s})\n").as_bytes());
        }
        Kind::Str => {
            let s = v.as_str().expect("a string");
            out.extend_from_slice(format!("{pad}string({}) \"", s.len()).as_bytes());
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(b"\"\n");
        }
        Kind::Array => {
            let a = v.as_array().expect("an array");
            out.extend_from_slice(format!("{pad}array({}) {{\n", a.len()).as_bytes());
            for (k, v) in a.iter() {
                match k {
                    Key::Int(i) => out.extend_from_slice(format!("{pad}  [{i}]=>\n").as_bytes()),
                    Key::Str(s) => {
                        out.extend_from_slice(format!("{pad}  [\"").as_bytes());
                        out.extend_from_slice(s.as_bytes());
                        out.extend_from_slice(b"\"]=>\n");
                    }
                }
                dump(v, indent + 2, out, seen);
            }
            out.extend_from_slice(format!("{pad}}}\n").as_bytes());
        }
        Kind::Ref => dump_ref(v.as_reference().expect("a reference"), 1, indent, out, seen),
        Kind::Object if as_resource(v).is_some() => {
            let o = v.as_object().expect("an object");
            let kind = if closed(o) { "Unknown".to_string() } else { String::from_utf8_lossy(o.class.name.as_bytes()).into_owned() };
            out.extend_from_slice(format!("{pad}resource({}) of type ({kind})\n", o.id).as_bytes());
        }
        Kind::Object if v.as_object().is_some_and(|o| o.class.flags & CLASS_ENUM != 0) => {
            let o = v.as_object().expect("an object");
            out.extend_from_slice(format!("{pad}enum(").as_bytes());
            out.extend_from_slice(o.class.name.as_bytes());
            out.extend_from_slice(b"::");
            out.extend_from_slice(case_name(o).as_bytes());
            out.extend_from_slice(b")\n");
        }
        Kind::Object => {
            let o = v.as_object().expect("an object");
            if seen.iter().any(|s| Rc::ptr_eq(s, o)) {
                out.extend_from_slice(format!("{pad}*RECURSION*\n").as_bytes());
                return;
            }
            seen.push(Rc::clone(o));
            let list = entries(o);
            let count = list.iter().filter(|e| e.value.is_some()).count();
            // A lazy object (PHP 8.4) says so.
            let lazy = match o.lazy.get() {
                0 => "",
                f if f & p8_runtime::object::LAZY_PROXY != 0 => "lazy proxy ",
                _ => "lazy ghost ",
            };
            out.extend_from_slice(format!("{pad}{lazy}object(").as_bytes());
            out.extend_from_slice(o.class.shown());
            out.extend_from_slice(format!(")#{} ({count}) {{\n", o.id).as_bytes());
            for e in list {
                if e.numeric {
                    out.extend_from_slice(format!("{pad}  [").as_bytes());
                    out.extend_from_slice(e.name.as_bytes());
                } else {
                    out.extend_from_slice(format!("{pad}  [\"").as_bytes());
                    out.extend_from_slice(e.name.as_bytes());
                    out.push(b'"');
                }
                match e.visibility {
                    Visibility::Public => {}
                    Visibility::Protected => out.extend_from_slice(b":protected"),
                    Visibility::Private => {
                        out.extend_from_slice(b":\"");
                        out.extend_from_slice(e.declaring.as_bytes());
                        out.extend_from_slice(b"\":private");
                    }
                }
                out.extend_from_slice(b"]=>\n");
                match e.value {
                    // The entry is a copy: one more holder of a reference.
                    Some(v) if v.is_reference() => dump_ref(v.as_reference().expect("a reference"), 2, indent + 2, out, seen),
                    Some(v) => dump(&v, indent + 2, out, seen),
                    None => {
                        out.extend_from_slice(format!("{pad}  uninitialized(").as_bytes());
                        out.extend_from_slice(e.ty.as_ref().map_or(&b""[..], |t| t.as_bytes()));
                        out.extend_from_slice(b")\n");
                    }
                }
            }
            out.extend_from_slice(format!("{pad}}}\n").as_bytes());
            seen.pop();
        }
    }
}

/// A string as var_export writes it: single-quoted with \ and ' escaped,
/// and NUL bytes spliced in as `' . "\0" . '`.
fn export_string(s: &[u8], out: &mut Vec<u8>) {
    out.push(b'\'');
    for &c in s {
        match c {
            b'\'' | b'\\' => {
                out.push(b'\\');
                out.push(c);
            }
            0 => out.extend_from_slice(b"' . \"\\0\" . '"),
            c => out.push(c),
        }
    }
    out.push(b'\'');
}

#[cfg(test)]
mod tests {
    use super::strtol;

    #[test]
    fn strtol_reads_prefixes_and_saturates() {
        assert_eq!(strtol(b"0x1A", 16), 26);
        assert_eq!(strtol(b"0x1A", 0), 26);
        assert_eq!(strtol(b"012", 0), 10);
        assert_eq!(strtol(b"  -101", 2), -5);
        assert_eq!(strtol(b"zz", 36), 1295);
        assert_eq!(strtol(b"99999999999999999999", 10), i64::MAX);
        assert_eq!(strtol(b"12abc", 16), 0x12abc);
    }
}

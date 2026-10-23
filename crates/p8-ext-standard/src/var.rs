//! ext/standard's variable functions: types, conversions, print_r and
//! var_export. Arguments arrive already checked and converted to their
//! declared types (p8_runtime::args).
//!
//! Reference: $PHP_SRC_PATH/ext/standard/type.c, var.c;
//! $PHP_SRC_PATH/Zend/zend.c - print_hash, zend_print_zval_r_to_buf

use crate::{arg, str_arg};
use p8_runtime::array::Key;
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::object::{CLASS_ENUM, CLASS_RESOURCE, Object, Visibility};
use p8_runtime::stream::resource;
use std::rc::Rc;
use p8_runtime::numeric::numeric_string;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "gettype", run: gettype, by_ref: &[] },
    Builtin { name: "is_object", run: is_object, by_ref: &[] },
    Builtin { name: "is_resource", run: is_resource, by_ref: &[] },
    Builtin { name: "get_resource_type", run: get_resource_type, by_ref: &[] },
    Builtin { name: "get_resource_id", run: get_resource_id, by_ref: &[] },
    Builtin { name: "get_debug_type", run: get_debug_type, by_ref: &[] },
    Builtin { name: "is_null", run: is_null, by_ref: &[] },
    Builtin { name: "is_bool", run: is_bool, by_ref: &[] },
    Builtin { name: "is_int", run: is_int, by_ref: &[] },
    Builtin { name: "is_integer", run: is_int, by_ref: &[] },
    Builtin { name: "is_long", run: is_int, by_ref: &[] },
    Builtin { name: "is_float", run: is_float, by_ref: &[] },
    Builtin { name: "is_double", run: is_float, by_ref: &[] },
    Builtin { name: "is_string", run: is_string, by_ref: &[] },
    Builtin { name: "is_array", run: is_array, by_ref: &[] },
    Builtin { name: "is_scalar", run: is_scalar, by_ref: &[] },
    Builtin { name: "is_numeric", run: is_numeric, by_ref: &[] },
    Builtin { name: "is_iterable", run: is_array, by_ref: &[] },
    Builtin { name: "is_countable", run: is_array, by_ref: &[] },
    Builtin { name: "intval", run: intval, by_ref: &[] },
    Builtin { name: "floatval", run: floatval, by_ref: &[] },
    Builtin { name: "doubleval", run: floatval, by_ref: &[] },
    Builtin { name: "strval", run: strval, by_ref: &[] },
    Builtin { name: "boolval", run: boolval, by_ref: &[] },
    Builtin { name: "var_dump", run: var_dump, by_ref: &[] },
    Builtin { name: "print_r", run: print_r, by_ref: &[] },
    Builtin { name: "var_export", run: var_export, by_ref: &[] },
];

fn gettype(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str(match arg(args, 0) {
        Value::Null | Value::Undef => "NULL",
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Float(_) => "double",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Object(o) if o.class.flags & CLASS_RESOURCE != 0 => {
            if closed(&o) { "resource (closed)" } else { "resource" }
        }
        Value::Object(_) => "object",
        Value::Ref(_) => unreachable!("arg() looks through references"),
    }))
}

/// Whether a resource has been closed (its type is "Unknown" then).
fn closed(o: &Object) -> bool {
    resource(o).is_some_and(|r| r.closed.get())
}

fn is_object(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Object(o) if o.class.flags & CLASS_RESOURCE == 0)))
}

/// is_resource: an open resource.
fn is_resource(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Object(o) if o.class.flags & CLASS_RESOURCE != 0 && !closed(&o))))
}

fn get_resource_type(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    match arg(args, 0) {
        Value::Object(o) if o.class.flags & CLASS_RESOURCE != 0 => {
            Ok(Value::Str(if closed(&o) { PhpStr::from("Unknown") } else { o.class.name.clone() }))
        }
        other => Err(ctx.throw(
            "TypeError",
            format!("get_resource_type(): Argument #1 ($resource) must be of type resource, {} given", other.type_name()),
        )),
    }
}

fn get_resource_id(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    match arg(args, 0) {
        Value::Object(o) if o.class.flags & CLASS_RESOURCE != 0 => Ok(Value::Int(i64::from(o.id))),
        other => Err(ctx.throw(
            "TypeError",
            format!("get_resource_id(): Argument #1 ($resource) must be of type resource, {} given", other.type_name()),
        )),
    }
}

fn get_debug_type(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = match arg(args, 0) {
        Value::Null | Value::Undef => "null".into(),
        Value::Bool(_) => "bool".into(),
        Value::Object(o) if o.class.flags & CLASS_RESOURCE != 0 => {
            if closed(&o) { "resource (closed)".into() } else { format!("resource ({})", String::from_utf8_lossy(o.class.name.as_bytes())).into() }
        }
        Value::Object(o) => String::from_utf8_lossy(o.class.shown()).into_owned().into(),
        other => other.type_name(),
    };
    Ok(Value::str(name.into_owned()))
}

fn is_null(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Null | Value::Undef)))
}

fn is_bool(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Bool(_))))
}

fn is_int(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Int(_))))
}

fn is_float(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Float(_))))
}

fn is_string(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Str(_))))
}

fn is_array(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Array(_))))
}

fn is_scalar(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_))))
}

/// An int or float, or a string that is entirely a number (whitespace
/// allowed around it, as of PHP 8).
fn is_numeric(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(match arg(args, 0) {
        Value::Int(_) | Value::Float(_) => true,
        Value::Str(s) => numeric_string(s.as_bytes()).is_some(),
        _ => false,
    }))
}

/// intval($value, $base = 10): a string in another base is read as
/// strtol does (prefixes 0x, 0o, 0b and a leading 0 for base 0 / 16 / 8 / 2).
fn intval(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let base = if args.len() > 1 { arg(args, 1).to_int() } else { 10 };
    let Value::Str(s) = &v else { return Ok(Value::Int(v.to_int())) };
    if base == 10 {
        return Ok(Value::Int(v.to_int()));
    }
    Ok(Value::Int(strtol(s.as_bytes(), base)))
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

fn floatval(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Float(arg(args, 0).to_float()))
}

fn strval(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Str(str_arg(args, 0)))
}

fn boolval(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(arg(args, 0).to_bool()))
}

/// print_r($value, $return = false)
fn print_r(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    print_r_to(&arg(args, 0), 0, &mut out, &mut Vec::new());
    if args.len() > 1 && arg(args, 1).to_bool() {
        return Ok(Value::Str(PhpStr::new(out)));
    }
    ctx.echo(&out);
    Ok(Value::Bool(true))
}

fn print_r_to(v: &Value, indent: usize, out: &mut Vec<u8>, seen: &mut Vec<Rc<Object>>) {
    match v {
        Value::Array(a) => {
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
        Value::Ref(cell) => print_r_to(&cell.borrow(), indent, out, seen),
        Value::Object(o) if o.class.flags & CLASS_RESOURCE != 0 => {
            out.extend_from_slice(format!("Resource id #{}", o.id).as_bytes());
        }
        Value::Object(o) => {
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
        other => {
            if let Some(s) = other.to_php_string() {
                out.extend_from_slice(s.as_bytes());
            }
        }
    }
}

/// var_export($value, $return = false)
fn var_export(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    export(&arg(args, 0), 1, &mut out);
    if args.len() > 1 && arg(args, 1).to_bool() {
        return Ok(Value::Str(PhpStr::new(out)));
    }
    ctx.echo(&out);
    Ok(Value::Null)
}

/// php_var_export_ex
fn export(v: &Value, level: usize, out: &mut Vec<u8>) {
    match v {
        // PHP also warns "var_export does not handle resources".
        Value::Null | Value::Undef => out.extend_from_slice(b"NULL"),
        Value::Object(o) if o.class.flags & CLASS_RESOURCE != 0 => out.extend_from_slice(b"NULL"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        // PHP_INT_MIN has no literal: written as an expression.
        Value::Int(i64::MIN) => out.extend_from_slice(b"-9223372036854775807-1"),
        Value::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Value::Float(f) => {
            let text = float_to_string(*f, Precision::SERIALIZE, 'E');
            out.extend_from_slice(text.as_bytes());
            if f.is_finite() && !text.contains(['.', 'E', 'e']) {
                out.extend_from_slice(b".0");
            }
        }
        Value::Str(s) => export_string(s.as_bytes(), out),
        Value::Array(a) => {
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
                export(v, level + 2, out);
                out.extend_from_slice(b",\n");
            }
            if level > 1 {
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.push(b')');
        }
        Value::Ref(cell) => export(&cell.borrow(), level, out),
        Value::Object(o) if o.class.flags & CLASS_ENUM != 0 => {
            if level > 1 {
                out.push(b'\n');
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.push(b'\\');
            out.extend_from_slice(o.class.name.as_bytes());
            out.extend_from_slice(b"::");
            out.extend_from_slice(case_name(o).as_bytes());
        }
        Value::Object(o) => {
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
            for e in entries(o) {
                let Some(v) = e.value else { continue };
                out.extend(std::iter::repeat_n(b' ', level + 2));
                export_string(e.name.as_bytes(), out);
                out.extend_from_slice(b" => ");
                export(&v, level + 2, out);
                out.extend_from_slice(b",\n");
            }
            if level > 1 {
                out.extend(std::iter::repeat_n(b' ', level - 1));
            }
            out.extend_from_slice(if std_class { b")" } else { b"))" });
        }
    }
}

/// One of an object's properties as the dump functions list them.
struct Entry {
    name: PhpStr,
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
    if let Some(info) = o.class.debug_info {
        for (k, v) in info(o).iter() {
            let name = match k {
                Key::Int(i) => PhpStr::from(i.to_string()),
                Key::Str(s) => s,
            };
            list.push(Entry { name, visibility: Visibility::Public, declaring: PhpStr::empty(), value: Some(v.clone()), ty: None });
        }
        return list;
    }
    let slots = o.slots.borrow();
    for (p, v) in o.class.properties.iter().zip(slots.iter()) {
        let value = match v {
            Value::Undef if p.ty.is_none() => continue,
            Value::Undef => None,
            v => Some(v.clone()),
        };
        list.push(Entry { name: p.name.clone(), visibility: p.visibility, declaring: p.declaring_name.clone(), value, ty: p.ty.clone() });
    }
    for (k, v) in o.dynamic.borrow().iter() {
        let name = match k {
            Key::Int(i) => PhpStr::from(i.to_string()),
            Key::Str(s) => s,
        };
        list.push(Entry { name, visibility: Visibility::Public, declaring: PhpStr::empty(), value: Some(v.clone()), ty: None });
    }
    list
}

/// var_dump($value, ...$values)
fn var_dump(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for v in args.iter() {
        dump(v, 0, &mut out, &mut seen);
    }
    ctx.echo(&out);
    Ok(Value::Null)
}

fn dump(v: &Value, indent: usize, out: &mut Vec<u8>, seen: &mut Vec<Rc<Object>>) {
    let pad = " ".repeat(indent);
    match v {
        Value::Null | Value::Undef => out.extend_from_slice(format!("{pad}NULL\n").as_bytes()),
        Value::Bool(b) => out.extend_from_slice(format!("{pad}bool({b})\n").as_bytes()),
        Value::Int(i) => out.extend_from_slice(format!("{pad}int({i})\n").as_bytes()),
        Value::Float(f) => {
            let s = float_to_string(*f, Precision::SERIALIZE, 'E');
            out.extend_from_slice(format!("{pad}float({s})\n").as_bytes());
        }
        Value::Str(s) => {
            out.extend_from_slice(format!("{pad}string({}) \"", s.len()).as_bytes());
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(b"\"\n");
        }
        Value::Array(a) => {
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
        Value::Ref(cell) => dump(&cell.borrow(), indent, out, seen),
        Value::Object(o) if o.class.flags & CLASS_RESOURCE != 0 => {
            let kind = if closed(o) { "Unknown".to_string() } else { String::from_utf8_lossy(o.class.name.as_bytes()).into_owned() };
            out.extend_from_slice(format!("{pad}resource({}) of type ({kind})\n", o.id).as_bytes());
        }
        Value::Object(o) if o.class.flags & CLASS_ENUM != 0 => {
            out.extend_from_slice(format!("{pad}enum(").as_bytes());
            out.extend_from_slice(o.class.name.as_bytes());
            out.extend_from_slice(b"::");
            out.extend_from_slice(case_name(o).as_bytes());
            out.extend_from_slice(b")\n");
        }
        Value::Object(o) => {
            if seen.iter().any(|s| Rc::ptr_eq(s, o)) {
                out.extend_from_slice(format!("{pad}*RECURSION*\n").as_bytes());
                return;
            }
            seen.push(Rc::clone(o));
            let list = entries(o);
            let count = list.iter().filter(|e| e.value.is_some()).count();
            out.extend_from_slice(format!("{pad}object(").as_bytes());
            out.extend_from_slice(o.class.shown());
            out.extend_from_slice(format!(")#{} ({count}) {{\n", o.id).as_bytes());
            for e in list {
                out.extend_from_slice(format!("{pad}  [\"").as_bytes());
                out.extend_from_slice(e.name.as_bytes());
                out.push(b'"');
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

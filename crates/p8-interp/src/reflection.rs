//! The Reflection classes: ReflectionClass (and ReflectionObject),
//! ReflectionMethod, ReflectionFunction, ReflectionParameter,
//! ReflectionProperty, ReflectionClassConstant, ReflectionNamedType,
//! ReflectionUnionType, ReflectionException.
//!
//! Each keeps what it reflects in hidden properties ("\0..."), shown by
//! var_dump as PHP shows it (name, class). Function declarations come from
//! the engine (Ctx::function_info); classes from their runtime Class.
//!
//! Reference: $PHP_SRC_PATH/ext/reflection/php_reflection.c

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::{
    CLASS_ABSTRACT, CLASS_ENUM, CLASS_FINAL, CLASS_INTERFACE, CLASS_INTERNAL, CLASS_READONLY, CLASS_TRAIT, Class, Native, Object, Visibility,
};
use p8_runtime::reflect::{Callee, FnInfo, ParamInfo, Subject};
use p8_runtime::string::PhpStr;
use p8_runtime::types::TypeDecl;
use p8_runtime::value::Value;
use std::rc::Rc;

const PUBLIC: Visibility = Visibility::Public;

// ---------------------------------------------------------------------------
// Small helpers

fn get(o: &Object, name: &str) -> Value {
    match o.class.slot(name.as_bytes(), None) {
        Some(i) => o.slots.borrow()[i].clone(),
        None => Value::Null,
    }
}

fn set(o: &Object, name: &str, v: Value) {
    if let Some(i) = o.class.slot(name.as_bytes(), None) {
        o.slots.borrow_mut()[i] = v;
    }
}

fn s(text: &[u8]) -> Value {
    Value::Str(PhpStr::from(text))
}

fn text(v: &Value) -> String {
    String::from_utf8_lossy(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned()
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

fn fail(ctx: &mut dyn Ctx, message: String) -> Throw {
    ctx.throw("ReflectionException", message)
}

fn make(ctx: &mut dyn Ctx, class: &[u8]) -> Rc<Object> {
    let class = ctx.class(class).expect("a Reflection class");
    ctx.new_object(&class)
}

fn list(items: Vec<Value>) -> Value {
    Value::Array(Rc::new(PhpArray::from_list(items)))
}

fn shown_class(c: &Class) -> PhpStr {
    c.name.clone()
}

/// The class a ReflectionClass-like argument names: an object's, or a name
/// (after autoloading).
fn class_arg(ctx: &mut dyn Ctx, v: &Value) -> Result<Rc<Class>, Throw> {
    match v {
        Value::Object(o) => Ok(Rc::clone(&o.class)),
        other => {
            let name = text(other);
            match ctx.class_load(name.as_bytes())? {
                Some(c) => Ok(c),
                None => Err(fail(ctx, format!("Class \"{name}\" does not exist"))),
            }
        }
    }
}

/// The class a reflection object holds (its "\0class" name).
fn held_class(ctx: &mut dyn Ctx, o: &Object) -> Result<Rc<Class>, Throw> {
    let name = get(o, "\0class");
    match ctx.class(name.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()) {
        Some(c) => Ok(c),
        None => Err(ctx.throw("Error", "Internal error: Failed to retrieve the reflection object".into())),
    }
}

fn new_class_reflection(ctx: &mut dyn Ctx, class: &Rc<Class>) -> Value {
    let o = make(ctx, if class.flags & CLASS_ENUM != 0 { b"ReflectionEnum" } else { b"ReflectionClass" });
    set(&o, "name", Value::Str(shown_class(class)));
    set(&o, "\0class", Value::Str(class.name.clone()));
    Value::Object(o)
}

// Callees kept in a value: a name, [class, method], or a Closure.
fn encode(c: &Callee) -> Value {
    match c {
        Callee::Function(n) => Value::Str(n.clone()),
        Callee::Method(class, m) => list(vec![Value::Str(class.name.clone()), Value::Str(m.clone())]),
        Callee::Closure(o) => Value::Object(Rc::clone(o)),
    }
}

fn decode(ctx: &mut dyn Ctx, v: &Value) -> Result<Callee, Throw> {
    Ok(match v {
        Value::Object(o) => Callee::Closure(Rc::clone(o)),
        Value::Array(a) => {
            let parts: Vec<Value> = a.values().cloned().collect();
            let class = class_arg(ctx, &parts[0])?;
            Callee::Method(class, parts[1].to_php_string().unwrap_or_else(PhpStr::empty))
        }
        other => Callee::Function(other.to_php_string().unwrap_or_else(PhpStr::empty)),
    })
}

fn callee_of(ctx: &mut dyn Ctx, o: &Object) -> Result<Callee, Throw> {
    decode(ctx, &get(o, "\0callee"))
}

fn info_of(ctx: &mut dyn Ctx, o: &Object) -> Result<FnInfo, Throw> {
    let c = callee_of(ctx, o)?;
    match ctx.function_info(&c) {
        Some(i) => Ok(i),
        None => Err(ctx.throw("Error", "Internal error: Failed to retrieve the reflection object".into())),
    }
}

// ---------------------------------------------------------------------------
// Types

/// The atoms of a type (class names, then builtin names in PHP's order),
/// without null; and whether null is allowed.
fn atoms(t: &TypeDecl) -> (Vec<String>, bool) {
    use p8_runtime::types::*;
    let mut out: Vec<String> = t.written.iter().map(|c| String::from_utf8_lossy(c.as_bytes()).into_owned()).collect();
    if t.mask & T_MIXED != 0 {
        return (vec!["mixed".into()], true);
    }
    for (bit, name) in [(T_STATIC, "static"), (T_CALLABLE, "callable"), (T_OBJECT, "object"), (T_ARRAY, "array"), (T_STRING, "string"), (T_INT, "int"), (T_FLOAT, "float")] {
        if t.mask & bit != 0 {
            out.push(name.into());
        }
    }
    if t.mask & T_BOOL != 0 || t.mask & (T_TRUE | T_FALSE) == T_TRUE | T_FALSE {
        out.push("bool".into());
    } else if t.mask & T_FALSE != 0 {
        out.push("false".into());
    } else if t.mask & T_TRUE != 0 {
        out.push("true".into());
    }
    if t.mask & T_VOID != 0 {
        out.push("void".into());
    }
    if t.mask & T_NEVER != 0 {
        out.push("never".into());
    }
    (out, t.mask & T_NULL != 0)
}

fn named_type(ctx: &mut dyn Ctx, name: &str, nullable: bool, builtin: bool) -> Value {
    let o = make(ctx, b"ReflectionNamedType");
    set(&o, "\0name", s(name.as_bytes()));
    set(&o, "\0null", Value::Bool(nullable));
    set(&o, "\0builtin", Value::Bool(builtin));
    Value::Object(o)
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "mixed" | "callable" | "object" | "array" | "string" | "int" | "float" | "bool" | "false" | "true" | "void" | "never" | "null" | "iterable"
    )
}

/// A ReflectionType for a declared type (None: untyped).
fn type_value(ctx: &mut dyn Ctx, t: &Option<TypeDecl>) -> Value {
    let Some(t) = t else { return Value::Null };
    let (atoms, null) = atoms(t);
    if t.intersection {
        let o = make(ctx, b"ReflectionIntersectionType");
        let types: Vec<Value> = atoms.iter().map(|a| named_type(ctx, a, false, false)).collect();
        set(&o, "\0types", list(types));
        set(&o, "\0shown", s(t.shown().as_bytes()));
        return Value::Object(o);
    }
    if atoms.len() == 1 {
        let name = &atoms[0];
        return named_type(ctx, name, null || name == "mixed", is_builtin(name));
    }
    if atoms.is_empty() {
        return named_type(ctx, "null", true, true);
    }
    let o = make(ctx, b"ReflectionUnionType");
    let mut types: Vec<Value> = atoms.iter().map(|a| named_type(ctx, a, false, is_builtin(a))).collect();
    if null {
        types.push(named_type(ctx, "null", true, true));
    }
    set(&o, "\0types", list(types));
    set(&o, "\0null", Value::Bool(null));
    set(&o, "\0shown", s(t.shown().as_bytes()));
    Value::Object(o)
}

fn nt_get_name(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0name"))
}

fn nt_allows_null(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0null"))
}

fn nt_is_builtin(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0builtin"))
}

fn nt_to_string(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let name = text(&get(this, "\0name"));
    let null = get(this, "\0null").to_bool();
    Ok(s(if null && name != "mixed" && name != "null" { format!("?{name}") } else { name }.as_bytes()))
}

fn ut_get_types(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0types"))
}

fn ut_to_string(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0shown"))
}

// ---------------------------------------------------------------------------
// ReflectionClass

fn rc_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = class_arg(ctx, &arg(args, 0))?;
    set(this, "name", Value::Str(shown_class(&class)));
    set(this, "\0class", Value::Str(class.name.clone()));
    Ok(Value::Null)
}

macro_rules! class_method {
    ($fname:ident, |$ctx:ident, $class:ident, $this:ident, $args:ident| $body:expr) => {
        fn $fname($ctx: &mut dyn Ctx, $this: &Rc<Object>, $args: &mut [Value]) -> Result<Value, Throw> {
            let $class = held_class($ctx, $this)?;
            let _ = (&$args, &$this, &$class);
            $body
        }
    };
}

class_method!(rc_get_name, |ctx, class, this, args| Ok(Value::Str(shown_class(&class))));
class_method!(rc_get_short_name, |ctx, class, this, args| {
    let n = class.name.as_bytes();
    Ok(s(n.iter().rposition(|&c| c == b'\\').map_or(n, |i| &n[i + 1..])))
});
class_method!(rc_get_namespace_name, |ctx, class, this, args| {
    let n = class.name.as_bytes();
    Ok(s(n.iter().rposition(|&c| c == b'\\').map_or(&b""[..], |i| &n[..i])))
});
class_method!(rc_in_namespace, |ctx, class, this, args| Ok(Value::Bool(class.name.as_bytes().contains(&b'\\'))));
class_method!(rc_is_interface, |ctx, class, this, args| Ok(Value::Bool(class.flags & CLASS_INTERFACE != 0)));
class_method!(rc_is_trait, |ctx, class, this, args| Ok(Value::Bool(class.flags & CLASS_TRAIT != 0)));
class_method!(rc_is_enum, |ctx, class, this, args| Ok(Value::Bool(class.flags & CLASS_ENUM != 0)));
class_method!(rc_is_abstract, |ctx, class, this, args| Ok(Value::Bool(
    class.flags & CLASS_ABSTRACT != 0 || class.flags & CLASS_INTERFACE != 0 && !class.methods.is_empty() || class.methods.values().any(|m| m.is_abstract)
)));
class_method!(rc_is_final, |ctx, class, this, args| Ok(Value::Bool(class.flags & CLASS_FINAL != 0)));
class_method!(rc_is_readonly, |ctx, class, this, args| Ok(Value::Bool(class.flags & CLASS_READONLY != 0)));
class_method!(rc_is_internal, |ctx, class, this, args| Ok(Value::Bool(class.flags & CLASS_INTERNAL != 0)));
class_method!(rc_is_user_defined, |ctx, class, this, args| Ok(Value::Bool(class.flags & CLASS_INTERNAL == 0)));
class_method!(rc_is_anonymous, |ctx, class, this, args| Ok(Value::Bool(class.name.as_bytes().windows(10).any(|w| w == b"@anonymous"))));
class_method!(rc_is_instantiable, |ctx, class, this, args| {
    if class.flags & (CLASS_INTERFACE | CLASS_TRAIT | CLASS_ABSTRACT | CLASS_ENUM) != 0 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(class.method(b"__construct").is_none_or(|m| m.visibility == PUBLIC)))
});
class_method!(rc_is_cloneable, |ctx, class, this, args| {
    if class.flags & (CLASS_INTERFACE | CLASS_TRAIT | CLASS_ABSTRACT | CLASS_ENUM) != 0 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(class.method(b"__clone").is_none_or(|m| m.visibility == PUBLIC)))
});
class_method!(rc_is_iterable, |ctx, class, this, args| Ok(Value::Bool(
    class.flags & (CLASS_INTERFACE | CLASS_TRAIT | CLASS_ABSTRACT) == 0 && class.is_a(b"traversable")
)));
class_method!(rc_is_instance, |ctx, class, this, args| match arg(args, 0) {
    Value::Object(o) => Ok(Value::Bool(o.class.is_a(&class.lower))),
    _ => Err(ctx.throw("TypeError", "ReflectionClass::isInstance(): Argument #1 ($object) must be of type object".into())),
});
class_method!(rc_is_subclass_of, |ctx, class, this, args| {
    let other = match arg(args, 0) {
        Value::Object(o) if o.class.is_a(b"reflectionclass") => held_class(ctx, &o)?,
        v => class_arg(ctx, &v)?,
    };
    Ok(Value::Bool(class.lower != other.lower && class.is_a(&other.lower)))
});
class_method!(rc_implements_interface, |ctx, class, this, args| {
    let other = match arg(args, 0) {
        Value::Object(o) if o.class.is_a(b"reflectionclass") => held_class(ctx, &o)?,
        v => match class_arg(ctx, &v) {
            Ok(c) => c,
            Err(_) => return Err(fail(ctx, format!("Interface \"{}\" does not exist", text(&v)))),
        },
    };
    if other.flags & CLASS_INTERFACE == 0 {
        return Err(fail(ctx, format!("{} is not an interface", String::from_utf8_lossy(other.name.as_bytes()))));
    }
    Ok(Value::Bool(class.is_a(&other.lower)))
});
class_method!(rc_get_parent_class, |ctx, class, this, args| Ok(match &class.parent {
    Some(p) => new_class_reflection(ctx, p),
    None => Value::Bool(false),
}));
class_method!(rc_get_interface_names, |ctx, class, this, args| {
    let names: Vec<Value> = class.interfaces.iter().map(|i| ctx.class(i).map_or_else(|| s(i), |c| Value::Str(c.name.clone()))).collect();
    Ok(list(names))
});
class_method!(rc_get_interfaces, |ctx, class, this, args| {
    let mut out = PhpArray::new();
    for i in class.interfaces.clone() {
        if let Some(c) = ctx.class(&i) {
            let r = new_class_reflection(ctx, &c);
            out.insert(Key::from_string(c.name.clone()), r);
        }
    }
    Ok(Value::Array(Rc::new(out)))
});
class_method!(rc_get_modifiers, |ctx, class, this, args| {
    let mut m = 0;
    if class.flags & CLASS_ABSTRACT != 0 {
        m |= 64;
    }
    if class.flags & CLASS_FINAL != 0 {
        m |= 32;
    }
    if class.flags & CLASS_READONLY != 0 {
        m |= 65536;
    }
    Ok(Value::Int(m))
});
class_method!(rc_has_method, |ctx, class, this, args| {
    let name = text(&arg(args, 0)).to_ascii_lowercase();
    Ok(Value::Bool(!name.starts_with('$') && class.method(name.as_bytes()).is_some()))
});
class_method!(rc_get_method, |ctx, class, this, args| {
    let name = text(&arg(args, 0));
    if name.starts_with('$') || class.method(name.to_ascii_lowercase().as_bytes()).is_none() {
        return Err(fail(ctx, format!("Method {}::{}() does not exist", String::from_utf8_lossy(class.name.as_bytes()), name)));
    }
    Ok(new_method(ctx, &class, name.as_bytes()))
});
class_method!(rc_get_methods, |ctx, class, this, args| {
    // `null` (PHPUnit passes it) means no filter, like no argument.
    let filter = args.first().map(Value::deref).filter(|v| !matches!(v, Value::Null | Value::Undef)).map(|v| v.to_int());
    let mut out = Vec::new();
    for m in class.methods.values() {
        if m.name.as_bytes().starts_with(b"$") {
            continue;
        }
        if let Some(f) = filter
            && method_modifiers(m) & f == 0
        {
            continue;
        }
        out.push(new_method(ctx, &class, m.name.as_bytes()));
    }
    Ok(list(out))
});
class_method!(rc_get_constructor, |ctx, class, this, args| Ok(if class.method(b"__construct").is_some() {
    new_method(ctx, &class, b"__construct")
} else {
    Value::Null
}));
class_method!(rc_has_property, |ctx, class, this, args| {
    let name = text(&arg(args, 0));
    if class.slots.contains_key(name.as_bytes()) || class.statics.contains_key(name.as_bytes()) {
        return Ok(Value::Bool(true));
    }
    // A dynamic property of the object it reflects (ReflectionObject).
    Ok(Value::Bool(match get(this, "\0object") {
        Value::Object(o) => o.dynamic.borrow().contains_key(&Key::from_string(PhpStr::from(name.as_str()))),
        _ => false,
    }))
});
class_method!(rc_get_property, |ctx, class, this, args| {
    let name = text(&arg(args, 0));
    if !class.slots.contains_key(name.as_bytes()) && !class.statics.contains_key(name.as_bytes()) {
        let dynamic = match get(this, "\0object") {
            Value::Object(o) => o.dynamic.borrow().contains_key(&Key::from_string(PhpStr::from(name.as_str()))),
            _ => false,
        };
        if !dynamic {
            return Err(fail(ctx, format!("Property {}::${} does not exist", String::from_utf8_lossy(class.name.as_bytes()), name)));
        }
    }
    Ok(new_property(ctx, &class, name.as_bytes()))
});
class_method!(rc_get_properties, |ctx, class, this, args| {
    // `null` (PHPUnit passes it) means no filter, like no argument.
    let filter = args.first().map(Value::deref).filter(|v| !matches!(v, Value::Null | Value::Undef)).map(|v| v.to_int());
    let mut out = Vec::new();
    let mut names: Vec<PhpStr> = Vec::new();
    // Class by class (the class itself, then its parents): its static
    // properties, then its instance ones (private ones only its own).
    let mut chain = vec![Rc::clone(&class)];
    while let Some(p) = chain.last().and_then(|c| c.parent.clone()) {
        chain.push(p);
    }
    for (depth, c) in chain.iter().enumerate() {
        let mut statics: Vec<(&Vec<u8>, &p8_runtime::object::Static)> = class.statics.iter().filter(|(_, (_, vis, d))| *d == c.lower && (depth == 0 || *vis != Visibility::Private)).collect();
        statics.sort_by(|a, b| a.0.cmp(b.0));
        for (n, (_, vis, _)) in statics {
            if filter.is_some_and(|f| (vis_bits(*vis) | 16) & f == 0) || names.iter().any(|x| x.as_bytes() == n.as_slice()) {
                continue;
            }
            names.push(PhpStr::from(n.as_slice()));
        }
        for p in class.properties.iter().filter(|p| p.declaring == c.lower && !p.is_internal() && (depth == 0 || p.visibility != Visibility::Private)) {
            let modifiers = vis_bits(p.visibility) | set_bits(p.set_visibility) | if p.readonly { 128 } else { 0 } | if p.is_final { 32 } else { 0 };
            if filter.is_some_and(|f| modifiers & f == 0) || names.contains(&p.name) {
                continue;
            }
            names.push(p.name.clone());
        }
    }
    for n in names {
        out.push(new_property(ctx, &class, n.as_bytes()));
    }
    Ok(list(out))
});
class_method!(rc_has_constant, |ctx, class, this, args| Ok(Value::Bool(class.constants.contains_key(text(&arg(args, 0)).as_bytes()))));
class_method!(rc_get_constant, |ctx, class, this, args| {
    let name = text(&arg(args, 0));
    if !class.constants.contains_key(name.as_bytes()) {
        return Ok(Value::Bool(false));
    }
    class_constant(ctx, &class, &name)
});
class_method!(rc_get_constants, |ctx, class, this, args| {
    // The class's own, then inherited ones, each in declaration order.
    let own = class.constants.iter().filter(|(_, c)| c.declaring_name == class.name).map(|(k, _)| k.clone());
    let inherited = class.constants.iter().filter(|(_, c)| c.declaring_name != class.name).map(|(k, _)| k.clone());
    let names: Vec<Vec<u8>> = own.chain(inherited).collect();
    let mut out = PhpArray::new();
    for n in names {
        let v = class_constant(ctx, &class, &String::from_utf8_lossy(&n))?;
        out.insert(Key::from_string(PhpStr::from(n.as_slice())), v);
    }
    Ok(Value::Array(Rc::new(out)))
});
fn new_constant(ctx: &mut dyn Ctx, class: &Rc<Class>, name: &[u8]) -> Value {
    let o = make(ctx, b"ReflectionClassConstant");
    set(&o, "name", s(name));
    set(&o, "class", class.constants.get(name).map_or_else(|| Value::Str(class.name.clone()), |c| Value::Str(c.declaring_name.clone())));
    set(&o, "\0class", Value::Str(class.name.clone()));
    Value::Object(o)
}

class_method!(rc_get_reflection_constant, |ctx, class, this, args| {
    let name = text(&arg(args, 0));
    Ok(if class.constants.contains_key(name.as_bytes()) { new_constant(ctx, &class, name.as_bytes()) } else { Value::Bool(false) })
});
class_method!(rc_get_reflection_constants, |ctx, class, this, args| {
    let filter = match arg(args, 0) {
        Value::Null | Value::Undef => None,
        v => Some(v.to_int()),
    };
    let own = class.constants.iter().filter(|(_, c)| c.declaring_name == class.name);
    let inherited = class.constants.iter().filter(|(_, c)| c.declaring_name != class.name);
    let names: Vec<Vec<u8>> = own.chain(inherited).filter(|(_, c)| filter.is_none_or(|f| vis_bits(c.visibility) & f != 0)).map(|(k, _)| k.clone()).collect();
    Ok(list(names.iter().map(|n| new_constant(ctx, &class, n)).collect()))
});
class_method!(rc_new_instance, |ctx, class, this, args| {
    check_constructor(ctx, &class)?;
    let mut list: Vec<Value> = args.iter().map(Value::deref).collect();
    Ok(Value::Object(ctx.new_instance(&class, &mut list, true)?))
});
class_method!(rc_new_instance_args, |ctx, class, this, args| {
    check_constructor(ctx, &class)?;
    let mut list: Vec<Value> = match arg(args, 0) {
        Value::Array(a) => a.values().map(Value::deref).collect(),
        _ => Vec::new(),
    };
    Ok(Value::Object(ctx.new_instance(&class, &mut list, true)?))
});
class_method!(rc_new_instance_without_constructor, |ctx, class, this, args| {
    if class.flags & CLASS_INTERNAL != 0 && class.flags & CLASS_FINAL != 0 {
        return Err(fail(ctx, format!("Class {} is an internal class marked as final that cannot be instantiated without invoking its constructor", String::from_utf8_lossy(class.name.as_bytes()))));
    }
    Ok(Value::Object(ctx.new_instance(&class, &mut [], false)?))
});
class_method!(rc_get_static_property_value, |ctx, class, this, args| {
    let name = text(&arg(args, 0));
    match class.statics.get(name.as_bytes()) {
        Some((cell, _, _)) => Ok(cell.borrow().deref()),
        None if args.len() > 1 => Ok(arg(args, 1)),
        None => Err(fail(ctx, format!("Property {}::${} does not exist", String::from_utf8_lossy(class.name.as_bytes()), name))),
    }
});
class_method!(rc_set_static_property_value, |ctx, class, this, args| {
    let name = text(&arg(args, 0));
    match class.statics.get(name.as_bytes()) {
        Some((cell, _, _)) => {
            let v = arg(args, 1);
            let inner = cell.borrow().clone();
            match inner {
                Value::Ref(r) => *r.borrow_mut() = v,
                _ => *cell.borrow_mut() = v,
            }
            Ok(Value::Null)
        }
        None => Err(fail(ctx, format!("Class {} does not have a property named {}", String::from_utf8_lossy(class.name.as_bytes()), name))),
    }
});
class_method!(rc_get_static_properties, |ctx, class, this, args| {
    let mut names: Vec<&Vec<u8>> = class.statics.keys().collect();
    names.sort();
    let mut out = PhpArray::new();
    for n in names {
        out.insert(Key::from_string(PhpStr::from(n.as_slice())), class.statics[n].0.borrow().deref());
    }
    Ok(Value::Array(Rc::new(out)))
});
class_method!(rc_get_default_properties, |ctx, class, this, args| {
    let mut out = PhpArray::new();
    let mut names: Vec<&Vec<u8>> = class.statics.keys().collect();
    names.sort();
    for n in names {
        out.insert(Key::from_string(PhpStr::from(n.as_slice())), class.statics[n].0.borrow().deref());
    }
    for p in &class.properties {
        if !matches!(p.default, Value::Undef) {
            out.insert(Key::from_string(p.name.clone()), p.default.clone());
        }
    }
    Ok(Value::Array(Rc::new(out)))
});
class_method!(rc_get_file_name, |ctx, class, this, args| Ok(ctx.reflect_meta(&Subject::Class(class)).file.map_or(Value::Bool(false), Value::Str)));
class_method!(rc_get_start_line, |ctx, class, this, args| {
    let m = ctx.reflect_meta(&Subject::Class(class));
    Ok(if m.file.is_some() { Value::Int(i64::from(m.line)) } else { Value::Bool(false) })
});
class_method!(rc_get_end_line, |ctx, class, this, args| {
    let m = ctx.reflect_meta(&Subject::Class(class));
    Ok(if m.file.is_some() { Value::Int(i64::from(m.end_line)) } else { Value::Bool(false) })
});
class_method!(rc_get_doc_comment, |ctx, class, this, args| Ok(doc_value(ctx, Subject::Class(class))));
class_method!(rc_get_attributes, |ctx, class, this, args| attributes_of(ctx, Subject::Class(class), args, "ReflectionClass"));
class_method!(rc_get_trait_names, |ctx, class, this, args| {
    let names: Vec<Value> = ctx.reflect_meta(&Subject::Class(class)).traits.iter().map(|t| ctx.class(t.as_bytes()).map_or_else(|| Value::Str(t.clone()), |c| Value::Str(c.name.clone()))).collect();
    Ok(list(names))
});
class_method!(rc_get_traits, |ctx, class, this, args| {
    let mut out = PhpArray::new();
    for t in ctx.reflect_meta(&Subject::Class(class)).traits {
        if let Some(c) = ctx.class(t.as_bytes()) {
            let r = new_class_reflection(ctx, &c);
            out.insert(Key::from_string(c.name.clone()), r);
        }
    }
    Ok(Value::Array(Rc::new(out)))
});
class_method!(rc_get_extension_name, |ctx, class, this, args| Ok(match class_extension(&class) {
    Some(m) => s(m.as_bytes()),
    None => Value::Bool(false),
}));

fn ro_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(o) = arg(args, 0) else {
        return Err(ctx.throw("TypeError", "ReflectionObject::__construct(): Argument #1 ($object) must be of type object".into()));
    };
    set(this, "name", Value::Str(o.class.name.clone()));
    set(this, "\0class", Value::Str(o.class.name.clone()));
    set(this, "\0object", Value::Object(o));
    Ok(Value::Null)
}

fn check_constructor(ctx: &mut dyn Ctx, class: &Rc<Class>) -> Result<(), Throw> {
    if let Some(m) = class.method(b"__construct")
        && m.visibility != PUBLIC
    {
        return Err(fail(ctx, format!("Access to non-public constructor of class {}", String::from_utf8_lossy(class.name.as_bytes()))));
    }
    Ok(())
}

/// A class constant's value (its code run first, if needed); null for none.
fn class_constant(ctx: &mut dyn Ctx, class: &Rc<Class>, name: &str) -> Result<Value, Throw> {
    Ok(ctx.class_constant(class, name.as_bytes())?.unwrap_or(Value::Null))
}

fn vis_bits(v: Visibility) -> i64 {
    match v {
        Visibility::Public => 1,
        Visibility::Protected => 2,
        Visibility::Private => 4,
    }
}

/// A set visibility's modifier bit (IS_PROTECTED_SET, IS_PRIVATE_SET).
fn set_bits(v: Option<Visibility>) -> i64 {
    match v {
        Some(Visibility::Protected) => 2048,
        Some(Visibility::Private) => 4096,
        _ => 0,
    }
}

fn method_modifiers(m: &p8_runtime::object::Method) -> i64 {
    vis_bits(m.visibility) | if m.is_static { 16 } else { 0 } | if m.is_final { 32 } else { 0 } | if m.is_abstract { 64 } else { 0 }
}

// ---------------------------------------------------------------------------
// ReflectionFunction / ReflectionMethod (ReflectionFunctionAbstract)

fn new_method(ctx: &mut dyn Ctx, class: &Rc<Class>, name: &[u8]) -> Value {
    let o = make(ctx, b"ReflectionMethod");
    fill_method(ctx, &o, class, name);
    Value::Object(o)
}

fn fill_method(ctx: &mut dyn Ctx, o: &Rc<Object>, class: &Rc<Class>, name: &[u8]) {
    let m = class.method(&name.to_ascii_lowercase()).cloned();
    let (shown, declaring) = match &m {
        Some(m) => (m.name.clone(), m.declaring_name.clone()),
        None => (PhpStr::from(name), class.name.clone()),
    };
    let _ = ctx;
    set(o, "name", Value::Str(shown.clone()));
    set(o, "class", Value::Str(declaring));
    set(o, "\0callee", encode(&Callee::Method(Rc::clone(class), shown)));
}

fn rm_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (target, name) = match (arg(args, 0), args.len()) {
        (Value::Str(s), 1) => {
            let t = String::from_utf8_lossy(s.as_bytes()).into_owned();
            match t.split_once("::") {
                Some((c, m)) => (Value::Str(PhpStr::from(c)), m.to_string()),
                None => {
                    return Err(fail(ctx, "ReflectionMethod::__construct(): Argument #1 ($objectOrMethod) must be a valid method name".to_string()));
                }
            }
        }
        (t, _) => (t, text(&arg(args, 1))),
    };
    let class = class_arg(ctx, &target)?;
    if class.method(name.to_ascii_lowercase().as_bytes()).is_none() || name.starts_with('$') {
        return Err(fail(ctx, format!("Method {}::{}() does not exist", String::from_utf8_lossy(class.name.as_bytes()), name)));
    }
    fill_method(ctx, this, &class, name.as_bytes());
    Ok(Value::Null)
}

fn rm_create_from_method_name(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = make(ctx, b"ReflectionMethod");
    let n = args.len().min(1);
    rm_construct(ctx, &o, &mut args[..n])?;
    Ok(Value::Object(o))
}

fn rf_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let callee = match arg(args, 0) {
        Value::Object(o) if o.class.is_a(b"closure") => Callee::Closure(o),
        v => {
            let name = text(&v);
            let c = Callee::Function(PhpStr::from(name.as_str()));
            if ctx.function_info(&c).is_none() {
                return Err(fail(ctx, format!("Function {name}() does not exist")));
            }
            c
        }
    };
    let info = ctx.function_info(&callee).expect("checked");
    // A closure of a builtin (`strlen(...)`, `$doc->createElement(...)`)
    // keeps the builtin's name.
    let shown = if info.is_closure && info.user { PhpStr::from("{closure}") } else { info.name.clone() };
    let shown = if info.is_closure && info.name.as_bytes().starts_with(b"{closure") { info.name.clone() } else { shown };
    set(this, "name", Value::Str(shown));
    set(this, "\0callee", encode(&callee));
    Ok(Value::Null)
}

macro_rules! fn_method {
    ($fname:ident, |$ctx:ident, $info:ident, $this:ident, $args:ident| $body:expr) => {
        fn $fname($ctx: &mut dyn Ctx, $this: &Rc<Object>, $args: &mut [Value]) -> Result<Value, Throw> {
            let $info = info_of($ctx, $this)?;
            let _ = (&$args, &$this, &$info);
            $body
        }
    };
}

fn_method!(fa_get_name, |ctx, info, this, args| Ok(get(this, "name")));
fn_method!(fa_get_short_name, |ctx, info, this, args| {
    let n = text(&get(this, "name"));
    Ok(s(n.rsplit_once('\\').map_or(n.as_str(), |(_, b)| b).as_bytes()))
});
fn_method!(fa_get_namespace_name, |ctx, info, this, args| {
    let n = text(&get(this, "name"));
    Ok(s(n.rsplit_once('\\').map_or("", |(a, _)| a).as_bytes()))
});
fn_method!(fa_in_namespace, |ctx, info, this, args| Ok(Value::Bool(text(&get(this, "name")).contains('\\'))));
fn_method!(fa_get_parameters, |ctx, info, this, args| {
    let callee = get(this, "\0callee");
    let mut out = Vec::new();
    for i in 0..info.params.len() {
        out.push(new_parameter(ctx, &callee, &info, i));
    }
    Ok(list(out))
});
fn_method!(fa_get_number_of_parameters, |ctx, info, this, args| Ok(Value::Int(info.params.len() as i64)));
fn_method!(fa_get_number_of_required_parameters, |ctx, info, this, args| Ok(Value::Int(
    info.params.iter().rposition(|p| !p.optional).map_or(0, |i| i + 1) as i64
)));
fn_method!(fa_get_return_type, |ctx, info, this, args| Ok(type_value(ctx, &info.ret)));
fn_method!(fa_has_return_type, |ctx, info, this, args| Ok(Value::Bool(info.ret.is_some())));
fn_method!(fa_get_tentative_return_type, |ctx, info, this, args| Ok(type_value(ctx, &info.tentative_ret)));
fn_method!(fa_has_tentative_return_type, |ctx, info, this, args| Ok(Value::Bool(info.tentative_ret.is_some())));
fn_method!(fa_is_variadic, |ctx, info, this, args| Ok(Value::Bool(info.params.last().is_some_and(|p| p.variadic))));
fn_method!(fa_returns_reference, |ctx, info, this, args| Ok(Value::Bool(info.returns_ref)));
fn_method!(fa_is_user_defined, |ctx, info, this, args| Ok(Value::Bool(info.user)));
fn_method!(fa_is_internal, |ctx, info, this, args| Ok(Value::Bool(!info.user)));
fn_method!(fa_is_closure, |ctx, info, this, args| Ok(Value::Bool(info.is_closure)));
fn_method!(fa_is_generator, |ctx, info, this, args| Ok(Value::Bool(info.is_generator)));
fn_method!(fa_is_static, |ctx, info, this, args| Ok(Value::Bool(info.is_static)));
fn_method!(fa_is_deprecated, |ctx, info, this, args| Ok(Value::Bool(false)));
fn_method!(fa_get_file_name, |ctx, info, this, args| Ok(info.file.clone().map_or(Value::Bool(false), Value::Str)));
fn_method!(fa_get_start_line, |ctx, info, this, args| Ok(if info.user { Value::Int(i64::from(info.line)) } else { Value::Bool(false) }));
fn_method!(fa_get_end_line, |ctx, info, this, args| {
    let subject = Subject::Function(callee_of(ctx, this)?);
    Ok(if info.user { Value::Int(i64::from(ctx.reflect_meta(&subject).end_line)) } else { Value::Bool(false) })
});
fn_method!(fa_get_doc_comment, |ctx, info, this, args| {
    let subject = Subject::Function(callee_of(ctx, this)?);
    Ok(doc_value(ctx, subject))
});
fn_method!(fa_get_attributes, |ctx, info, this, args| {
    let subject = Subject::Function(callee_of(ctx, this)?);
    attributes_of(ctx, subject, args, "ReflectionFunctionAbstract")
});
fn_method!(fa_get_static_variables, |ctx, info, this, args| {
    let callee = callee_of(ctx, this)?;
    Ok(Value::Array(Rc::new(ctx.static_variables(&callee))))
});
fn_method!(fa_get_closure_used_variables, |ctx, info, this, args| {
    let callee = callee_of(ctx, this)?;
    Ok(Value::Array(Rc::new(ctx.closure_used_variables(&callee))))
});
fn_method!(fa_get_closure_this, |ctx, info, this, args| Ok(info.closure_this.clone().map_or(Value::Null, Value::Object)));
fn_method!(fa_get_closure_scope_class, |ctx, info, this, args| Ok(match &info.closure_scope {
    Some(c) => new_class_reflection(ctx, c),
    None => Value::Null,
}));
fn_method!(fa_get_closure_called_class, |ctx, info, this, args| Ok(match &info.closure_called {
    Some(c) => new_class_reflection(ctx, c),
    None => Value::Null,
}));

fn rf_invoke(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let c = callee_of(ctx, this)?;
    let mut list: Vec<Value> = args.to_vec();
    ctx.invoke(&c, None, &mut list)
}

fn rf_invoke_args(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let c = callee_of(ctx, this)?;
    let mut list: Vec<Value> = match arg(args, 0) {
        Value::Array(a) => a.values().cloned().collect(),
        _ => Vec::new(),
    };
    ctx.invoke(&c, None, &mut list)
}

fn rf_get_closure(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let c = callee_of(ctx, this)?;
    ctx.closure_of(&c, None)
}

fn rf_is_anonymous(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let info = info_of(ctx, this)?;
    Ok(Value::Bool(info.is_closure && info.name.as_bytes().starts_with(b"{closure")))
}

/// The method a ReflectionMethod holds.
fn held_method(ctx: &mut dyn Ctx, this: &Object) -> Result<(Rc<Class>, p8_runtime::object::Method), Throw> {
    match callee_of(ctx, this)? {
        Callee::Method(class, name) => match class.method(&name.as_bytes().to_ascii_lowercase()).cloned() {
            Some(m) => Ok((class, m)),
            None => Err(ctx.throw("Error", "Internal error: Failed to retrieve the reflection object".into())),
        },
        _ => Err(ctx.throw("Error", "Internal error: Failed to retrieve the reflection object".into())),
    }
}

macro_rules! method_flag {
    ($fname:ident, |$m:ident| $body:expr) => {
        fn $fname(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
            let (_, $m) = held_method(ctx, this)?;
            Ok(Value::Bool($body))
        }
    };
}

method_flag!(rm_is_public, |m| m.visibility == Visibility::Public);
method_flag!(rm_is_private, |m| m.visibility == Visibility::Private);
method_flag!(rm_is_protected, |m| m.visibility == Visibility::Protected);
method_flag!(rm_is_abstract, |m| m.is_abstract);
method_flag!(rm_is_final, |m| m.is_final);
method_flag!(rm_is_static, |m| m.is_static);
method_flag!(rm_is_constructor, |m| m.name.as_bytes().eq_ignore_ascii_case(b"__construct"));
method_flag!(rm_is_destructor, |m| m.name.as_bytes().eq_ignore_ascii_case(b"__destruct"));

fn rm_get_modifiers(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (_, m) = held_method(ctx, this)?;
    Ok(Value::Int(method_modifiers(&m)))
}

fn rm_get_declaring_class(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (class, m) = held_method(ctx, this)?;
    let declaring = ctx.class(&m.declaring).unwrap_or(class);
    Ok(new_class_reflection(ctx, &declaring))
}

/// The method `lower` of class `declaring` overrides or implements, as
/// zend_do_inheritance records it: a parent's (non-private) method's own
/// prototype or that method, else an interface's; a constructor has one
/// only when what it overrides is abstract.
fn prototype_of(ctx: &mut dyn Ctx, declaring: &Rc<Class>, lower: &[u8]) -> Option<(Rc<Class>, PhpStr)> {
    if let Some(parent) = &declaring.parent
        && let Some(m) = parent.method(lower).filter(|m| m.visibility != Visibility::Private).cloned()
    {
        let owner = ctx.class(&m.declaring).unwrap_or_else(|| Rc::clone(parent));
        let found = prototype_of(ctx, &owner, lower);
        if lower != b"__construct" || m.is_abstract || found.is_some() {
            return found.or(Some((owner, m.name)));
        }
    }
    for i in &declaring.interfaces {
        if let Some(iface) = ctx.class(i)
            && let Some(m) = iface.method(lower).cloned()
        {
            return Some((ctx.class(&m.declaring).unwrap_or(iface), m.name));
        }
    }
    None
}

fn rm_get_prototype(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (class, m) = held_method(ctx, this)?;
    let declaring = ctx.class(&m.declaring).unwrap_or(class);
    match prototype_of(ctx, &declaring, &m.name.as_bytes().to_ascii_lowercase()) {
        Some((owner, name)) => Ok(new_method(ctx, &owner, name.as_bytes())),
        None => Err(fail(ctx, format!("Method {}::{} does not have a prototype", String::from_utf8_lossy(declaring.name.as_bytes()), String::from_utf8_lossy(m.name.as_bytes())))),
    }
}

fn rm_has_prototype(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (class, m) = held_method(ctx, this)?;
    let declaring = ctx.class(&m.declaring).unwrap_or(class);
    Ok(Value::Bool(prototype_of(ctx, &declaring, &m.name.as_bytes().to_ascii_lowercase()).is_some()))
}

fn rm_invoke(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, m) = held_method(ctx, this)?;
    let target = match arg(args, 0) {
        Value::Object(o) => Some(o),
        _ if m.is_static => None,
        _ => {
            return Err(fail(ctx, format!("Trying to invoke non static method {}::{}() without an object", String::from_utf8_lossy(m.declaring_name.as_bytes()), String::from_utf8_lossy(m.name.as_bytes()))));
        }
    };
    if m.is_abstract {
        return Err(fail(ctx, format!("Trying to invoke abstract method {}::{}()", String::from_utf8_lossy(m.declaring_name.as_bytes()), String::from_utf8_lossy(m.name.as_bytes()))));
    }
    let mut rest: Vec<Value> = args.iter().skip(1).cloned().collect();
    ctx.invoke(&Callee::Method(class, m.name.clone()), target, &mut rest)
}

fn rm_invoke_args(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut all = vec![arg(args, 0)];
    if let Value::Array(a) = arg(args, 1) {
        all.extend(a.values().cloned());
    }
    rm_invoke(ctx, this, &mut all)
}

fn rm_get_closure(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, m) = held_method(ctx, this)?;
    let target = match arg(args, 0) {
        Value::Object(o) => Some(o),
        _ => None,
    };
    ctx.closure_of(&Callee::Method(class, m.name.clone()), target)
}

fn no_op(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// ReflectionParameter

fn new_parameter(ctx: &mut dyn Ctx, callee: &Value, info: &FnInfo, i: usize) -> Value {
    let o = make(ctx, b"ReflectionParameter");
    set(&o, "name", Value::Str(info.params[i].name.clone()));
    set(&o, "\0callee", callee.clone());
    set(&o, "\0pos", Value::Int(i as i64));
    Value::Object(o)
}

fn rp_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let callee = match arg(args, 0) {
        Value::Object(o) if o.class.is_a(b"closure") => Callee::Closure(o),
        Value::Array(a) => {
            let parts: Vec<Value> = a.values().map(Value::deref).collect();
            let class = class_arg(ctx, &parts.first().cloned().unwrap_or(Value::Null))?;
            Callee::Method(class, parts.get(1).cloned().unwrap_or(Value::Null).to_php_string().unwrap_or_else(PhpStr::empty))
        }
        v => Callee::Function(v.to_php_string().unwrap_or_else(PhpStr::empty)),
    };
    let Some(info) = ctx.function_info(&callee) else {
        return Err(fail(ctx, "The parameter specified by its name could not be found".into()));
    };
    let pos = match arg(args, 1) {
        Value::Int(i) if (i as usize) < info.params.len() && i >= 0 => i as usize,
        Value::Int(_) => return Err(fail(ctx, "The parameter specified by its offset could not be found".into())),
        v => {
            let name = text(&v);
            match info.params.iter().position(|p| p.name.as_bytes() == name.as_bytes()) {
                Some(i) => i,
                None => return Err(fail(ctx, "The parameter specified by its name could not be found".into())),
            }
        }
    };
    set(this, "name", Value::Str(info.params[pos].name.clone()));
    set(this, "\0callee", encode(&callee));
    set(this, "\0pos", Value::Int(pos as i64));
    Ok(Value::Null)
}

fn param_of(ctx: &mut dyn Ctx, this: &Object) -> Result<(FnInfo, usize), Throw> {
    let info = info_of(ctx, this)?;
    let pos = get(this, "\0pos").to_int() as usize;
    Ok((info, pos))
}

macro_rules! param_method {
    ($fname:ident, |$ctx:ident, $p:ident, $pos:ident, $info:ident, $this:ident| $body:expr) => {
        fn $fname($ctx: &mut dyn Ctx, $this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
            let ($info, $pos) = param_of($ctx, $this)?;
            let $p: &ParamInfo = &$info.params[$pos];
            let _ = (&$p, &$this);
            $body
        }
    };
}

param_method!(rp_get_name, |ctx, p, pos, info, this| Ok(Value::Str(p.name.clone())));
param_method!(rp_get_position, |ctx, p, pos, info, this| Ok(Value::Int(pos as i64)));
param_method!(rp_is_optional, |ctx, p, pos, info, this| Ok(Value::Bool(
    p.optional && info.params[pos..].iter().all(|q| q.optional)
)));
param_method!(rp_is_default_value_available, |ctx, p, pos, info, this| Ok(Value::Bool(p.optional && !p.variadic)));
param_method!(rp_get_default_value, |ctx, p, pos, info, this| {
    if !p.optional || p.variadic {
        return Err(fail(ctx, "Internal error: Failed to retrieve the default value".into()));
    }
    if p.default.is_none()
        && let Some(name) = &p.default_const
    {
        return default_constant_value(ctx, info.class.as_ref(), name.as_bytes());
    }
    Ok(p.default.clone().unwrap_or(Value::Null))
});
param_method!(rp_is_default_value_constant, |ctx, p, pos, info, this| {
    if !p.optional || p.variadic {
        return Err(fail(ctx, "Internal error: Failed to retrieve the default value".into()));
    }
    Ok(Value::Bool(p.default_const.is_some()))
});
param_method!(rp_get_default_value_constant_name, |ctx, p, pos, info, this| {
    if !p.optional || p.variadic {
        return Err(fail(ctx, "Internal error: Failed to retrieve the default value".into()));
    }
    Ok(p.default_const.clone().map_or(Value::Null, Value::Str))
});

/// The value of a parameter default that is the constant `name`
/// (`self::A` in `class`'s method, `N\X` falling back to the global `X`).
fn default_constant_value(ctx: &mut dyn Ctx, class: Option<&Rc<Class>>, name: &[u8]) -> Result<Value, Throw> {
    let missing = |ctx: &mut dyn Ctx, what: String| ctx.throw("Error", what);
    if let Some(at) = name.windows(2).position(|w| w == b"::") {
        let (owner, constant) = (&name[..at], &name[at + 2..]);
        let owner = match owner.to_ascii_lowercase().as_slice() {
            b"self" | b"static" => class.cloned(),
            b"parent" => class.and_then(|c| c.parent.clone()),
            // Autoloaded if need be, as evaluating the default does.
            _ => ctx.class_load(owner)?,
        };
        let Some(owner) = owner else {
            return Err(missing(ctx, format!("Class \"{}\" not found", String::from_utf8_lossy(&name[..at]))));
        };
        return match ctx.class_constant(&owner, constant)? {
            Some(v) => Ok(v),
            None => Err(missing(ctx, format!("Undefined constant {}::{}", String::from_utf8_lossy(owner.name.as_bytes()), String::from_utf8_lossy(constant)))),
        };
    }
    let global = name.rsplit(|&c| c == b'\\').next().unwrap_or(name);
    match ctx.constant(name).or_else(|| ctx.constant(global)) {
        Some(v) => Ok(v),
        None => Err(missing(ctx, format!("Undefined constant \"{}\"", String::from_utf8_lossy(name)))),
    }
}
param_method!(rp_get_type, |ctx, p, pos, info, this| Ok(type_value(ctx, &p.ty)));
param_method!(rp_has_type, |ctx, p, pos, info, this| Ok(Value::Bool(p.ty.is_some())));
param_method!(rp_allows_null, |ctx, p, pos, info, this| Ok(Value::Bool(
    p.ty.as_ref().is_none_or(|t| t.mask & (p8_runtime::types::T_NULL | p8_runtime::types::T_MIXED) != 0)
)));
param_method!(rp_is_variadic, |ctx, p, pos, info, this| Ok(Value::Bool(p.variadic)));
param_method!(rp_is_passed_by_reference, |ctx, p, pos, info, this| Ok(Value::Bool(p.by_ref)));
param_method!(rp_can_be_passed_by_value, |ctx, p, pos, info, this| Ok(Value::Bool(!p.by_ref)));
param_method!(rp_is_promoted, |ctx, p, pos, info, this| Ok(Value::Bool(p.promoted)));
param_method!(rp_get_declaring_class, |ctx, p, pos, info, this| Ok(match &info.class {
    Some(c) => new_class_reflection(ctx, c),
    None => Value::Null,
}));
// ReflectionParameter::__toString(): "Parameter #0 [ <optional> ?int $a = NULL ]"
// (php_reflection.c _parameter_string).
param_method!(rp_to_string, |ctx, p, pos, info, this| Ok(s(param_text(&info, pos).as_bytes())));

/// Parameter `pos` of a function as reflection prints it (php_reflection.c
/// _parameter_string).
fn param_text(info: &FnInfo, pos: usize) -> String {
    let p = &info.params[pos];
    let required = !(p.optional && info.params[pos..].iter().all(|q| q.optional));
    let mut out = format!("Parameter #{pos} [ {} ", if required { "<required>" } else { "<optional>" });
    if let Some(t) = &p.ty {
        out.push_str(&t.shown());
        out.push(' ');
    }
    if p.by_ref {
        out.push('&');
    }
    if p.variadic {
        out.push_str("...");
    }
    out.push('$');
    out.push_str(&String::from_utf8_lossy(p.name.as_bytes()));
    if !required && !p.variadic {
        if !info.user {
            out.push_str(" = ");
            out.push_str(p.default_text.unwrap_or("<default>"));
        } else if let Some(name) = &p.default_const {
            // A class constant's resolved class is shown fully qualified.
            let name = String::from_utf8_lossy(name.as_bytes());
            let relative = ["self::", "static::", "parent::"].iter().any(|k| name.len() >= k.len() && name[..k.len()].eq_ignore_ascii_case(k));
            out.push_str(if name.contains("::") && !relative { " = \\" } else { " = " });
            out.push_str(&name);
        } else if let Some(v) = &p.default {
            out.push_str(" = ");
            default_text(&mut out, v);
        }
    }
    out.push_str(" ]");
    out
}

/// The extension a builtin class is PHP's (None for a user class).
fn class_extension(class: &Class) -> Option<&'static str> {
    if class.flags & CLASS_INTERNAL == 0 {
        return None;
    }
    p8_stubs::class_module(class.name.as_bytes())
}

/// The extension a builtin function or method is in (a method: its class's).
fn function_extension(info: &FnInfo) -> Option<&'static str> {
    if info.user {
        return None;
    }
    match &info.class {
        Some(c) => class_extension(c),
        None => p8_stubs::function_module(info.name.as_bytes()),
    }
}

fn_method!(fa_get_extension_name, |ctx, info, this, args| Ok(match function_extension(&info) {
    Some(m) => s(m.as_bytes()),
    None => Value::Bool(false),
}));

/// ReflectionFunction / ReflectionMethod::__toString() (php_reflection.c
/// _function_string).
fn fa_to_string(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let info = info_of(ctx, this)?;
    let callee = callee_of(ctx, this)?;
    let meta = ctx.reflect_meta(&Subject::Function(callee.clone()));
    let mut out = String::new();
    if info.user
        && let Some(doc) = &meta.doc
    {
        out.push_str(&String::from_utf8_lossy(doc.as_bytes()));
        out.push('\n');
    }
    let method = match &callee {
        Callee::Method(class, name) => class.method(&name.as_bytes().to_ascii_lowercase()).cloned().map(|m| (Rc::clone(class), m)),
        _ => None,
    };
    out.push_str(if info.is_closure { "Closure [ " } else if method.is_some() { "Method [ " } else { "Function [ " });
    out.push_str(if info.user { "<user" } else { "<internal" });
    if let Some(m) = function_extension(&info) {
        out.push(':');
        out.push_str(m);
    }
    if let Some((class, m)) = &method {
        let lower = m.name.as_bytes().to_ascii_lowercase();
        let declaring = ctx.class(&m.declaring).unwrap_or_else(|| Rc::clone(class));
        if !m.declaring.eq_ignore_ascii_case(class.name.as_bytes()) {
            out.push_str(&format!(", inherits {}", text(&Value::Str(m.declaring_name.clone()))));
        } else if let Some(parent) = &declaring.parent
            && let Some(over) = parent.method(&lower).filter(|o| o.visibility != Visibility::Private)
        {
            out.push_str(&format!(", overwrites {}", text(&Value::Str(over.declaring_name.clone()))));
        }
        if let Some((owner, _)) = prototype_of(ctx, &declaring, &lower) {
            out.push_str(&format!(", prototype {}", text(&Value::Str(owner.name.clone()))));
        }
        if lower == b"__construct" {
            out.push_str(", ctor");
        }
    }
    out.push_str("> ");
    if let Some((_, m)) = &method {
        for (on, word) in [(m.is_abstract, "abstract "), (m.is_final, "final "), (m.is_static, "static ")] {
            if on {
                out.push_str(word);
            }
        }
        out.push_str(match m.visibility {
            Visibility::Public => "public ",
            Visibility::Protected => "protected ",
            Visibility::Private => "private ",
        });
        out.push_str("method ");
    } else {
        if info.is_static && info.is_closure {
            out.push_str("static ");
        }
        out.push_str("function ");
    }
    if info.returns_ref {
        out.push('&');
    }
    out.push_str(&String::from_utf8_lossy(info.name.as_bytes()));
    out.push_str(" ] {\n");
    if info.user {
        let file = info.file.as_ref().map(|f| String::from_utf8_lossy(f.as_bytes()).into_owned()).unwrap_or_default();
        out.push_str(&format!("  @@ {file} {} - {}\n", info.line, meta.end_line));
    }
    if info.is_closure {
        let bound = ctx.static_variables(&callee);
        if !bound.is_empty() {
            out.push_str(&format!("\n  - Bound Variables [{}] {{\n", bound.len()));
            for (i, (k, _)) in bound.iter().enumerate() {
                if let Key::Str(name) = k {
                    out.push_str(&format!("      Variable #{i} [ ${} ]\n", String::from_utf8_lossy(name.as_bytes())));
                }
            }
            out.push_str("  }\n");
        }
    }
    // A user function with neither parameters nor a return type has no
    // argument info, so no list.
    let ret = info.ret.as_ref().map(|t| (t, "Return")).or(info.tentative_ret.as_ref().map(|t| (t, "Tentative return")));
    if !info.user || !info.params.is_empty() || ret.is_some() {
        out.push_str(&format!("\n  - Parameters [{}] {{\n", info.params.len()));
        for pos in 0..info.params.len() {
            out.push_str(&format!("    {}\n", param_text(&info, pos)));
        }
        out.push_str("  }\n");
    }
    if let Some((t, word)) = ret {
        out.push_str(&format!("  - {word} [ {} ]\n", t.shown()));
    }
    out.push_str("}\n");
    Ok(s(out.as_bytes()))
}

/// A default value as reflection prints it (format_default_value).
fn default_text(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("NULL"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Str(t) => {
            out.push('\'');
            for &c in t.as_bytes() {
                match c {
                    b'\\' => out.push_str("\\\\"),
                    b'\n' => out.push_str("\\n"),
                    b'\r' => out.push_str("\\r"),
                    b'\t' => out.push_str("\\t"),
                    0x0b => out.push_str("\\v"),
                    0x0c => out.push_str("\\f"),
                    0x1b => out.push_str("\\e"),
                    c if !(0x20..0x7f).contains(&c) => out.push_str(&format!("\\x{c:02X}")),
                    c => out.push(c as char),
                }
            }
            out.push('\'');
        }
        Value::Array(a) => {
            let is_list = a.iter().enumerate().all(|(i, (k, _))| matches!(k, Key::Int(n) if n == i as i64));
            out.push('[');
            for (i, (k, v)) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if !is_list {
                    match k {
                        Key::Int(n) => out.push_str(&n.to_string()),
                        Key::Str(t) => {
                            out.push('\'');
                            out.push_str(&String::from_utf8_lossy(t.as_bytes()));
                            out.push('\'');
                        }
                    }
                    out.push_str(" => ");
                }
                default_text(out, v);
            }
            out.push(']');
        }
        Value::Object(o) => out.push_str(&format!("object({})", String::from_utf8_lossy(o.class.name.as_bytes()))),
        Value::Ref(r) => default_text(out, &r.borrow()),
        v => out.push_str(&String::from_utf8_lossy(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes())),
    }
}

fn rp_get_attributes(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let subject = Subject::Param(callee_of(ctx, this)?, get(this, "\0pos").to_int() as usize);
    attributes_of(ctx, subject, args, "ReflectionParameter")
}

fn rp_get_declaring_function(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let callee = callee_of(ctx, this)?;
    Ok(match callee {
        Callee::Method(class, name) => new_method(ctx, &class, name.as_bytes()),
        other => {
            let o = make(ctx, b"ReflectionFunction");
            let mut a = [match &other {
                Callee::Closure(c) => Value::Object(Rc::clone(c)),
                Callee::Function(n) => Value::Str(n.clone()),
                Callee::Method(..) => unreachable!("matched above"),
            }];
            rf_construct(ctx, &o, &mut a)?;
            Value::Object(o)
        }
    })
}

// ---------------------------------------------------------------------------
// ReflectionProperty

fn new_property(ctx: &mut dyn Ctx, class: &Rc<Class>, name: &[u8]) -> Value {
    let o = make(ctx, b"ReflectionProperty");
    fill_property(&o, class, name);
    Value::Object(o)
}

fn fill_property(o: &Object, class: &Rc<Class>, name: &[u8]) {
    let declaring = class
        .slot(name, None)
        .map(|i| class.properties[i].declaring_name.clone())
        .or_else(|| {
            // A static's declaring class (lowercase): its name as declared.
            let (_, _, lower) = class.statics.get(name)?;
            let mut c = Some(Rc::clone(class));
            while let Some(k) = c {
                if &k.lower == lower {
                    return Some(k.name.clone());
                }
                c = k.parent.clone();
            }
            None
        })
        .unwrap_or_else(|| class.name.clone());
    set(o, "name", s(name));
    set(o, "class", Value::Str(declaring));
    set(o, "\0class", Value::Str(class.name.clone()));
}

fn rpr_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let target = arg(args, 0);
    let class = class_arg(ctx, &target)?;
    let name = text(&arg(args, 1));
    let dynamic = match &target {
        Value::Object(o) => o.dynamic.borrow().contains_key(&Key::from_string(PhpStr::from(name.as_str()))),
        _ => false,
    };
    if class.slot(name.as_bytes(), None).is_none() && !class.statics.contains_key(name.as_bytes()) && !dynamic {
        return Err(fail(ctx, format!("Property {}::${} does not exist", String::from_utf8_lossy(class.name.as_bytes()), name)));
    }
    fill_property(this, &class, name.as_bytes());
    Ok(Value::Null)
}

/// The property a ReflectionProperty holds: (class, name, declared slot).
fn held_property(ctx: &mut dyn Ctx, this: &Object) -> Result<(Rc<Class>, String, Option<usize>), Throw> {
    let class = held_class(ctx, this)?;
    let name = text(&get(this, "name"));
    let slot = class.slot(name.as_bytes(), None);
    Ok((class, name, slot))
}

fn rpr_get_name(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "name"))
}

fn rpr_get_value(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, name, slot) = held_property(ctx, this)?;
    if let Some((cell, _, _)) = class.statics.get(name.as_bytes()) {
        return Ok(cell.borrow().deref());
    }
    let Value::Object(o) = arg(args, 0) else {
        return Err(ctx.throw("TypeError", "ReflectionProperty::getValue(): Argument #1 ($object) must be provided for instance properties".into()));
    };
    match slot.and_then(|_| o.class.slot(name.as_bytes(), Some(&class.lower))) {
        Some(i) => match o.slots.borrow()[i].deref() {
            Value::Undef => {
                let p = &o.class.properties[i];
                if p.ty.is_some() {
                    Err(ctx.throw("Error", format!("Typed property {}::${} must not be accessed before initialization", String::from_utf8_lossy(p.declaring_name.as_bytes()), name)))
                } else {
                    Ok(Value::Null)
                }
            }
            v => Ok(v),
        },
        None => Ok(o.dynamic.borrow().get(&Key::from_string(PhpStr::from(name.as_str()))).map_or(Value::Null, Value::deref)),
    }
}

fn rpr_set_value(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, name, slot) = held_property(ctx, this)?;
    if let Some((cell, _, _)) = class.statics.get(name.as_bytes()) {
        let v = if args.len() > 1 { arg(args, 1) } else { arg(args, 0) };
        let inner = cell.borrow().clone();
        match inner {
            Value::Ref(r) => *r.borrow_mut() = v,
            _ => *cell.borrow_mut() = v,
        }
        return Ok(Value::Null);
    }
    let Value::Object(o) = arg(args, 0) else {
        return Err(ctx.throw("TypeError", "ReflectionProperty::setValue(): Argument #1 ($objectOrValue) must be of type object".into()));
    };
    let v = arg(args, 1);
    match slot.and_then(|_| o.class.slot(name.as_bytes(), Some(&class.lower))) {
        Some(i) => {
            let mut slots = o.slots.borrow_mut();
            match &slots[i] {
                Value::Ref(cell) => *cell.borrow_mut() = v,
                _ => slots[i] = v,
            }
        }
        None => {
            o.dynamic.borrow_mut().insert(Key::from_string(PhpStr::from(name.as_str())), v);
        }
    }
    Ok(Value::Null)
}

/// ReflectionProperty::getRawValue / setRawValue: the stored value,
/// bypassing hooks (as getValue / setValue already read and write slots);
/// not for static properties.
fn rpr_get_raw_value(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, name, _) = held_property(ctx, this)?;
    if class.statics.contains_key(name.as_bytes()) {
        return Err(fail(ctx, "May not use getRawValue on static properties".into()));
    }
    rpr_get_value(ctx, this, args)
}

fn rpr_set_raw_value(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, name, _) = held_property(ctx, this)?;
    if class.statics.contains_key(name.as_bytes()) {
        return Err(fail(ctx, "May not use setRawValue on static properties".into()));
    }
    rpr_set_value(ctx, this, args)
}

// ---- Lazy objects (PHP 8.4) ----

/// `$object` argument `i` of a lazy-object method: an instance of the class.
fn lazy_object_arg(ctx: &mut dyn Ctx, args: &[Value], i: usize, class: &Rc<Class>, method: &str) -> Result<Rc<Object>, Throw> {
    match arg(args, i) {
        Value::Object(o) if o.class.is_a(&class.lower) => Ok(o),
        v => Err(ctx.throw(
            "TypeError",
            format!(
                "{method}(): Argument #{} ($object) must be of type {}, {} given",
                i + 1,
                String::from_utf8_lossy(class.name.as_bytes()),
                p8_runtime::args::value_name(&v)
            ),
        )),
    }
}

/// The options newLazy*/resetAsLazy* take, checked.
fn lazy_options(ctx: &mut dyn Ctx, v: &Value, method: &str, argn: usize, reset: bool) -> Result<i64, Throw> {
    let options = match v {
        Value::Null | Value::Undef => 0,
        v => v.to_int(),
    };
    if options & !(crate::lazy::SKIP_INITIALIZATION_ON_SERIALIZE | crate::lazy::SKIP_DESTRUCTOR) != 0 {
        return Err(fail(ctx, format!("{method}(): Argument #{argn} ($options) contains invalid flags")));
    }
    if !reset && options & crate::lazy::SKIP_DESTRUCTOR != 0 {
        return Err(fail(ctx, format!("{method}(): Argument #{argn} ($options) does not accept ReflectionClass::SKIP_DESTRUCTOR")));
    }
    Ok(options)
}

fn new_lazy(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], proxy: bool) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    let method = if proxy { "ReflectionClass::newLazyProxy" } else { "ReflectionClass::newLazyGhost" };
    let options = lazy_options(ctx, &arg(args, 1), method, 2, false)?;
    let o = ctx.lazy_make(&class, None, arg(args, 0), options, proxy)?;
    Ok(Value::Object(o))
}

fn rc_new_lazy_ghost(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    new_lazy(ctx, this, args, false)
}

fn rc_new_lazy_proxy(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    new_lazy(ctx, this, args, true)
}

fn reset_lazy(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value], proxy: bool) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    let method = if proxy { "ReflectionClass::resetAsLazyProxy" } else { "ReflectionClass::resetAsLazyGhost" };
    let o = lazy_object_arg(ctx, args, 0, &class, method)?;
    let options = lazy_options(ctx, &arg(args, 2), method, 3, true)?;
    if o.is_uninit_lazy() {
        return Err(fail(ctx, "Object is already lazy".into()));
    }
    ctx.lazy_make(&class, Some(o), arg(args, 1), options, proxy)?;
    Ok(Value::Null)
}

fn rc_reset_as_lazy_ghost(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    reset_lazy(ctx, this, args, false)
}

fn rc_reset_as_lazy_proxy(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    reset_lazy(ctx, this, args, true)
}

fn rc_is_uninitialized_lazy_object(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    let o = lazy_object_arg(ctx, args, 0, &class, "ReflectionClass::isUninitializedLazyObject")?;
    Ok(Value::Bool(o.is_uninit_lazy()))
}

fn rc_initialize_lazy_object(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    let o = lazy_object_arg(ctx, args, 0, &class, "ReflectionClass::initializeLazyObject")?;
    Ok(Value::Object(ctx.lazy_initialize(&o)?))
}

fn rc_mark_lazy_object_as_initialized(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    let o = lazy_object_arg(ctx, args, 0, &class, "ReflectionClass::markLazyObjectAsInitialized")?;
    if o.is_uninit_lazy() {
        ctx.lazy_mark_initialized(&o)?;
    }
    Ok(Value::Object(o.proxy_instance().unwrap_or(o)))
}

fn rc_get_lazy_initializer(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    let o = lazy_object_arg(ctx, args, 0, &class, "ReflectionClass::getLazyInitializer")?;
    if !o.is_uninit_lazy() {
        return Ok(Value::Null);
    }
    Ok(o.lazy_info.borrow().as_ref().map_or(Value::Null, |l| l.initializer.clone()))
}

/// The object a ReflectionProperty lazy method works on (through
/// initialized proxies) and the property's slot there, checked.
fn lazy_property_target(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &[Value], method: &str) -> Result<(Rc<Object>, usize), Throw> {
    let (class, name, _) = held_property(ctx, this)?;
    let mut o = lazy_object_arg(ctx, args, 0, &class, &format!("ReflectionProperty::{method}"))?;
    while let Some(inst) = o.proxy_instance() {
        o = inst;
    }
    if let Some((_, _, _)) = class.statics.get(name.as_bytes()) {
        return Err(fail(ctx, format!("Can not use {method} on static property {}::${name}", String::from_utf8_lossy(class.name.as_bytes()))));
    }
    let Some(slot) = o.class.slot(name.as_bytes(), Some(&class.lower)) else {
        return Err(fail(ctx, format!("Can not use {method} on dynamic property {}::${name}", String::from_utf8_lossy(class.name.as_bytes()))));
    };
    if o.class.properties[slot].hooks & p8_runtime::object::HOOK_VIRTUAL != 0 {
        return Err(fail(ctx, format!("Can not use {method} on virtual property {}::${name}", String::from_utf8_lossy(o.class.properties[slot].declaring_name.as_bytes()))));
    }
    Ok((o, slot))
}

fn rpr_skip_lazy_initialization(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (o, slot) = lazy_property_target(ctx, this, args, "skipLazyInitialization")?;
    if !o.lazy_slot(slot) {
        return Ok(Value::Null);
    }
    let defaults = ctx.class_defaults(&o.class)?;
    o.slots.borrow_mut()[slot] = defaults.get(slot).cloned().unwrap_or(Value::Undef);
    ctx.lazy_slot_done(&o, slot);
    Ok(Value::Null)
}

fn rpr_set_raw_value_without_lazy_initialization(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (o, slot) = lazy_property_target(ctx, this, args, "setRawValueWithoutLazyInitialization")?;
    let was_lazy = o.lazy_slot(slot);
    o.slots.borrow_mut()[slot] = arg(args, 1);
    if was_lazy {
        ctx.lazy_slot_done(&o, slot);
    }
    Ok(Value::Null)
}

fn rpr_is_lazy(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, name, _) = held_property(ctx, this)?;
    let mut o = lazy_object_arg(ctx, args, 0, &class, "ReflectionProperty::isLazy")?;
    while let Some(inst) = o.proxy_instance() {
        o = inst;
    }
    if class.statics.contains_key(name.as_bytes()) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(o.class.slot(name.as_bytes(), Some(&class.lower)).is_some_and(|i| o.lazy_slot(i))))
}

fn rpr_is_initialized(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, name, slot) = held_property(ctx, this)?;
    if class.statics.contains_key(name.as_bytes()) {
        return Ok(Value::Bool(true));
    }
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(match slot.and_then(|_| o.class.slot(name.as_bytes(), Some(&class.lower))) {
        Some(i) => !matches!(o.slots.borrow()[i], Value::Undef),
        None => o.dynamic.borrow().contains_key(&Key::from_string(PhpStr::from(name.as_str()))),
    }))
}

/// What a ReflectionProperty reports: (visibility, static, readonly,
/// default, type, declared).
type PropFacts = (Visibility, bool, bool, Option<Value>, Option<TypeDecl>, bool);

fn prop_facts(ctx: &mut dyn Ctx, this: &Object) -> Result<PropFacts, Throw> {
    let (class, name, slot) = held_property(ctx, this)?;
    if let Some((cell, vis, _)) = class.statics.get(name.as_bytes()) {
        return Ok((*vis, true, false, Some(cell.borrow().deref()), None, true));
    }
    Ok(match slot {
        Some(i) => {
            let p = &class.properties[i];
            let default = (!matches!(p.default, Value::Undef)).then(|| p.default.clone());
            (p.visibility, false, p.readonly, default, p.ty_decl.as_deref().cloned(), true)
        }
        None => (Visibility::Public, false, false, None, None, false),
    })
}

macro_rules! prop_flag {
    ($fname:ident, |$v:ident, $st:ident, $ro:ident, $def:ident, $ty:ident, $declared:ident| $body:expr) => {
        fn $fname(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
            let ($v, $st, $ro, $def, $ty, $declared) = prop_facts(ctx, this)?;
            let _ = (&$v, &$st, &$ro, &$def, &$ty, &$declared);
            $body
        }
    };
}

prop_flag!(rpr_is_public, |v, st, ro, def, ty, declared| Ok(Value::Bool(v == Visibility::Public)));
prop_flag!(rpr_is_private, |v, st, ro, def, ty, declared| Ok(Value::Bool(v == Visibility::Private)));
prop_flag!(rpr_is_protected, |v, st, ro, def, ty, declared| Ok(Value::Bool(v == Visibility::Protected)));
prop_flag!(rpr_is_static, |v, st, ro, def, ty, declared| Ok(Value::Bool(st)));
prop_flag!(rpr_is_readonly, |v, st, ro, def, ty, declared| Ok(Value::Bool(ro)));
prop_flag!(rpr_is_default, |v, st, ro, def, ty, declared| Ok(Value::Bool(declared)));
prop_flag!(rpr_has_default_value, |v, st, ro, def, ty, declared| Ok(Value::Bool(declared && (def.is_some() || ty.is_none()))));
prop_flag!(rpr_get_default_value, |v, st, ro, def, ty, declared| Ok(def.unwrap_or(Value::Null)));
fn rpr_get_type(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (_, _, _, _, ty, _) = prop_facts(ctx, this)?;
    Ok(type_value(ctx, &ty))
}
prop_flag!(rpr_has_type, |v, st, ro, def, ty, declared| Ok(Value::Bool(ty.is_some())));
fn rpr_get_modifiers(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (v, st, ro, _, _, _) = prop_facts(ctx, this)?;
    let h = prop_hooks(ctx, this)?;
    let mut bits = vis_bits(v) | set_bits(prop_set_visibility(ctx, this)?) | if st { 16 } else { 0 } | if ro { 128 } else { 0 };
    bits |= if h.is_final { 32 } else { 0 } | if h.is_abstract { 64 } else { 0 } | if h.virtual_ { 512 } else { 0 };
    Ok(Value::Int(bits))
}

/// Who may write a property, when narrower than its visibility.
fn prop_set_visibility(ctx: &mut dyn Ctx, this: &Object) -> Result<Option<Visibility>, Throw> {
    let (class, name, slot) = held_property(ctx, this)?;
    Ok(slot.filter(|_| !class.statics.contains_key(name.as_bytes())).and_then(|i| class.properties[i].set_visibility))
}

fn rpr_is_private_set(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(prop_set_visibility(ctx, this)? == Some(Visibility::Private)))
}

fn rpr_is_protected_set(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(prop_set_visibility(ctx, this)? == Some(Visibility::Protected)))
}

/// What a property declares beyond its visibility: final, its hooks.
struct PropHooks {
    is_final: bool,
    is_abstract: bool,
    virtual_: bool,
    /// The hook methods it has ("get", "set"), with their method names.
    hooks: Vec<(&'static str, Vec<u8>)>,
    class: Rc<Class>,
}

fn prop_hooks(ctx: &mut dyn Ctx, this: &Object) -> Result<PropHooks, Throw> {
    let (class, name, slot) = held_property(ctx, this)?;
    let (is_final, bits) = match slot.filter(|_| !class.statics.contains_key(name.as_bytes())) {
        Some(i) => (class.properties[i].is_final, class.properties[i].hooks),
        None => (false, 0),
    };
    let mut hooks = Vec::new();
    let mut is_abstract = false;
    for kind in ["get", "set"] {
        let method = format!("${name}::{kind}").into_bytes();
        if let Some(m) = class.method(&method.to_ascii_lowercase()) {
            is_abstract |= m.is_abstract;
            hooks.push((kind, method));
        }
    }
    let virtual_ = bits & p8_runtime::object::HOOK_VIRTUAL != 0 || is_abstract;
    Ok(PropHooks { is_final, is_abstract, virtual_, hooks, class })
}

fn rpr_is_final(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(prop_hooks(ctx, this)?.is_final))
}

fn rpr_is_abstract(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(prop_hooks(ctx, this)?.is_abstract))
}

fn rpr_is_virtual(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(prop_hooks(ctx, this)?.virtual_))
}

fn rpr_has_hooks(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(!prop_hooks(ctx, this)?.hooks.is_empty()))
}

/// getHooks(): ["get" => ReflectionMethod, "set" => ...].
fn rpr_get_hooks(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let h = prop_hooks(ctx, this)?;
    let mut out = PhpArray::new();
    for (kind, method) in &h.hooks {
        let m = new_method(ctx, &h.class, method);
        out.insert(Key::Str(PhpStr::from(*kind)), m);
    }
    Ok(Value::Array(Rc::new(out)))
}

fn rpr_get_declaring_class(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let name = get(this, "class");
    let class = class_arg(ctx, &name)?;
    Ok(new_class_reflection(ctx, &class))
}

fn rpr_is_promoted(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let (class, name, _) = held_property(ctx, this)?;
    let Some(info) = ctx.function_info(&Callee::Method(Rc::clone(&class), PhpStr::from("__construct"))) else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(info.params.iter().any(|p| p.promoted && p.name.as_bytes() == name.as_bytes())))
}

fn property_subject(ctx: &mut dyn Ctx, this: &Object) -> Result<Subject, Throw> {
    let (class, name, _) = held_property(ctx, this)?;
    Ok(Subject::Property(class, PhpStr::from(name.as_str())))
}

fn rpr_get_doc_comment(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let subject = property_subject(ctx, this)?;
    Ok(doc_value(ctx, subject))
}

fn rpr_get_attributes(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let subject = property_subject(ctx, this)?;
    attributes_of(ctx, subject, args, "ReflectionProperty")
}

// ---------------------------------------------------------------------------
// Doc comments and attributes

/// getDocComment: the `/** ... */` before a declaration, or false.
fn doc_value(ctx: &mut dyn Ctx, subject: Subject) -> Value {
    ctx.reflect_meta(&subject).doc.map_or(Value::Bool(false), Value::Str)
}

/// A hidden-property form of a Subject (a ReflectionAttribute keeps the
/// declaration it came from).
fn encode_subject(s: &Subject) -> Value {
    match s {
        Subject::Class(c) => list(vec![Value::Int(0), Value::Str(c.name.clone())]),
        Subject::Function(c) => list(vec![Value::Int(1), encode(c)]),
        Subject::Property(c, n) => list(vec![Value::Int(2), Value::Str(c.name.clone()), Value::Str(n.clone())]),
        Subject::Constant(c, n) => list(vec![Value::Int(3), Value::Str(c.name.clone()), Value::Str(n.clone())]),
        Subject::Param(c, pos) => list(vec![Value::Int(4), encode(c), Value::Int(*pos as i64)]),
    }
}

fn decode_subject(ctx: &mut dyn Ctx, v: &Value) -> Result<Subject, Throw> {
    let parts: Vec<Value> = match v {
        Value::Array(a) => a.values().cloned().collect(),
        _ => Vec::new(),
    };
    let name = |i: usize| parts.get(i).and_then(Value::to_php_string).unwrap_or_else(PhpStr::empty);
    Ok(match parts.first().map(Value::to_int) {
        Some(0) => Subject::Class(class_arg(ctx, &parts[1])?),
        Some(1) => Subject::Function(decode(ctx, &parts[1])?),
        Some(2) => Subject::Property(class_arg(ctx, &parts[1])?, name(2)),
        Some(3) => Subject::Constant(class_arg(ctx, &parts[1])?, name(2)),
        Some(4) => Subject::Param(decode(ctx, &parts[1])?, parts[2].to_int() as usize),
        _ => return Err(ctx.throw("Error", "Internal error: Failed to retrieve the reflection object".into())),
    })
}

const TARGET_CLASS: i64 = 1;
const TARGET_FUNCTION: i64 = 2;
const TARGET_METHOD: i64 = 4;
const TARGET_PROPERTY: i64 = 8;
const TARGET_CLASS_CONSTANT: i64 = 16;
const TARGET_PARAMETER: i64 = 32;
const TARGET_ALL: i64 = 63;
const IS_REPEATABLE: i64 = 64;

/// The attribute target a declaration is.
fn target_of(s: &Subject) -> i64 {
    match s {
        Subject::Class(_) => TARGET_CLASS,
        Subject::Function(Callee::Method(..)) => TARGET_METHOD,
        Subject::Function(_) => TARGET_FUNCTION,
        Subject::Property(..) => TARGET_PROPERTY,
        Subject::Constant(..) => TARGET_CLASS_CONSTANT,
        Subject::Param(..) => TARGET_PARAMETER,
    }
}

/// `getAttributes(?string $name = null, int $flags = 0)`: a
/// ReflectionAttribute for each attribute of `subject` (filtered by name,
/// or with IS_INSTANCEOF by the class they are or extend).
fn attributes_of(ctx: &mut dyn Ctx, subject: Subject, args: &[Value], method: &str) -> Result<Value, Throw> {
    let name = match arg(args, 0) {
        Value::Null | Value::Undef => None,
        v => Some(text(&v)),
    };
    let flags = arg(args, 1).to_int();
    if flags & !2 != 0 {
        return Err(ctx.throw("ValueError", format!("{method}::getAttributes(): Argument #2 ($flags) must be a valid attribute filter flag")));
    }
    let mut base = None;
    let mut name = name;
    if let Some(n) = name.take_if(|_| flags & 2 != 0) {
        match ctx.class_load(n.as_bytes())? {
            Some(c) => base = Some(c),
            None => return Err(ctx.throw("Error", format!("Class \"{n}\" not found"))),
        }
    }
    let names = ctx.reflect_meta(&subject).attrs;
    let target = target_of(&subject);
    let encoded = encode_subject(&subject);
    let mut out = Vec::new();
    for (i, attr) in names.iter().enumerate() {
        let lower = attr.as_bytes().to_ascii_lowercase();
        if let Some(n) = &name
            && n.to_ascii_lowercase().as_bytes() != lower.as_slice()
        {
            continue;
        }
        if let Some(base) = &base {
            match ctx.class_load(attr.as_bytes())? {
                Some(c) if c.is_a(&base.lower) => {}
                _ => continue,
            }
        }
        let repeated = names.iter().filter(|o| o.as_bytes().eq_ignore_ascii_case(attr.as_bytes())).count() > 1;
        let o = make(ctx, b"ReflectionAttribute");
        set(&o, "name", Value::Str(attr.clone()));
        set(&o, "\0subject", encoded.clone());
        set(&o, "\0index", Value::Int(i as i64));
        set(&o, "\0target", Value::Int(target));
        set(&o, "\0repeated", Value::Bool(repeated));
        out.push(Value::Object(o));
    }
    Ok(list(out))
}

/// ReflectionReference::fromArrayElement(array $array, int|string $key):
/// the element's reference, or null when the element is not one.
fn rr_from_array_element(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Array(array) = arg(args, 0) else { return Ok(Value::Null) };
    let key = match Key::from_value(&arg(args, 1)) {
        Ok((key, _)) => key,
        Err(_) => return Err(fail(ctx, "Array key not found".into())),
    };
    match array.get(&key) {
        None => Err(fail(ctx, "Array key not found".into())),
        Some(r @ Value::Ref(_)) => {
            let o = make(ctx, b"ReflectionReference");
            set(&o, "\0ref", r.clone());
            Ok(Value::Object(o))
        }
        Some(_) => Ok(Value::Null),
    }
}

/// ReflectionReference::getId(): 20 bytes that are the same for every
/// ReflectionReference of one reference while it lives.
fn rr_get_id(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let Value::Ref(r) = get(this, "\0ref") else { return Ok(Value::Str(PhpStr::empty())) };
    let mut x = Rc::as_ptr(&r) as usize as u64 ^ 0x9e37_79b9_7f4a_7c15;
    let mut id = Vec::with_capacity(20);
    while id.len() < 20 {
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 29;
        id.extend_from_slice(&x.to_le_bytes());
    }
    id.truncate(20);
    Ok(Value::Str(PhpStr::from(&id[..])))
}

fn rr_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(fail(ctx, "Cannot directly instantiate ReflectionReference".into()))
}

fn ra_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(fail(ctx, "Cannot directly instantiate ReflectionAttribute".into()))
}

fn ra_clone(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(fail(ctx, "Cannot clone object using __clone()".into()))
}

fn ra_get_name(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "name"))
}

fn ra_get_target(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0target"))
}

fn ra_is_repeated(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "\0repeated"))
}

fn ra_get_arguments(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let subject = decode_subject(ctx, &get(this, "\0subject"))?;
    Ok(Value::Array(Rc::new(ctx.reflect_attr_args(&subject, get(this, "\0index").to_int() as usize)?)))
}

/// ReflectionAttribute::__toString(): "Attribute [ Name ]" and its
/// arguments, each as a default value prints (php_reflection.c).
fn ra_to_string(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let subject = decode_subject(ctx, &get(this, "\0subject"))?;
    let args = ctx.reflect_attr_args(&subject, get(this, "\0index").to_int() as usize)?;
    let mut out = format!("Attribute [ {} ]", text(&get(this, "name")));
    if !args.is_empty() {
        out.push_str(&format!(" {{\n  - Arguments [{}] {{\n", args.len()));
        for (i, (k, v)) in args.iter().enumerate() {
            out.push_str(&format!("    Argument #{i} [ "));
            if let Key::Str(name) = k {
                out.push_str(&String::from_utf8_lossy(name.as_bytes()));
                out.push_str(" = ");
            }
            default_text(&mut out, v);
            out.push_str(" ]\n");
        }
        out.push_str("  }\n}");
    }
    out.push('\n');
    Ok(s(out.as_bytes()))
}

/// The attribute names of the builtin attribute classes (they take any target).
const BUILTIN_ATTRIBUTES: [&[u8]; 6] = [b"attribute", b"returntypewillchange", b"allowdynamicproperties", b"sensitiveparameter", b"override", b"deprecated"];

fn target_names(flags: i64) -> String {
    ["class", "function", "method", "property", "class constant", "parameter"]
        .iter()
        .enumerate()
        .filter(|(i, _)| flags & (1 << i) != 0)
        .map(|(_, n)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

/// How a class can be used as an attribute: Err(()) if it is not one;
/// Ok(None) for a builtin one; Ok(Some(flags)) for a user class's
/// `#[Attribute(flags)]`.
fn attribute_flags(ctx: &mut dyn Ctx, class: &Rc<Class>) -> Result<Result<Option<i64>, ()>, Throw> {
    if class.flags & CLASS_INTERNAL != 0 {
        return Ok(if BUILTIN_ATTRIBUTES.contains(&class.lower.as_slice()) { Ok(None) } else { Err(()) });
    }
    let subject = Subject::Class(Rc::clone(class));
    let Some(at) = ctx.reflect_meta(&subject).attrs.iter().position(|a| a.as_bytes().eq_ignore_ascii_case(b"attribute")) else { return Ok(Err(())) };
    let args = ctx.reflect_attr_args(&subject, at)?;
    let Some(flags) = args.get_int(0).or_else(|| args.get(&Key::from_string(PhpStr::from("flags")))).map(Value::deref) else { return Ok(Ok(Some(TARGET_ALL))) };
    let Value::Int(flags) = flags else {
        return Err(ctx.throw("Error", format!("Attribute::__construct(): Argument #1 ($flags) must be of type int, {} given", p8_runtime::args::value_name(&flags))));
    };
    if flags & !(TARGET_ALL | IS_REPEATABLE) != 0 {
        return Err(ctx.throw("Error", "Invalid attribute flags specified".into()));
    }
    Ok(Ok(Some(flags)))
}

fn ra_new_instance(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let name = text(&get(this, "name"));
    let Some(class) = ctx.class_load(name.as_bytes())? else {
        return Err(ctx.throw("Error", format!("Attribute class \"{name}\" not found")));
    };
    let flags = match attribute_flags(ctx, &class)? {
        Ok(flags) => flags,
        Err(()) => return Err(ctx.throw("Error", format!("Attempting to use non-attribute class \"{name}\" as attribute"))),
    };
    if let Some(flags) = flags {
        let target = get(this, "\0target").to_int();
        if flags & target == 0 {
            return Err(ctx.throw("Error", format!("Attribute \"{name}\" cannot target {} (allowed targets: {})", target_names(target), target_names(flags))));
        }
        if flags & IS_REPEATABLE == 0 && get(this, "\0repeated").to_bool() {
            return Err(ctx.throw("Error", format!("Attribute \"{name}\" must not be repeated")));
        }
    }
    let subject = decode_subject(ctx, &get(this, "\0subject"))?;
    let args = ctx.reflect_attr_args(&subject, get(this, "\0index").to_int() as usize)?;
    Ok(Value::Object(ctx.new_instance_named(&class, &args)?))
}

// ---------------------------------------------------------------------------
// ReflectionClassConstant

fn rcc_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = class_arg(ctx, &arg(args, 0))?;
    let name = text(&arg(args, 1));
    let Some(c) = class.constants.get(name.as_bytes()) else {
        return Err(fail(ctx, format!("Constant {}::{} does not exist", String::from_utf8_lossy(class.name.as_bytes()), name)));
    };
    set(this, "name", s(name.as_bytes()));
    set(this, "class", Value::Str(c.declaring_name.clone()));
    set(this, "\0class", Value::Str(class.name.clone()));
    Ok(Value::Null)
}

fn rcc_get_name(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(get(this, "name"))
}

fn rcc_get_value(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    class_constant(ctx, &class, &text(&get(this, "name")))
}

fn rcc_get_declaring_class(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let class = class_arg(ctx, &get(this, "class"))?;
    Ok(new_class_reflection(ctx, &class))
}

fn rcc_visibility(ctx: &mut dyn Ctx, this: &Object) -> Result<Visibility, Throw> {
    let class = held_class(ctx, this)?;
    Ok(class.constants.get(text(&get(this, "name")).as_bytes()).map_or(Visibility::Public, |c| c.visibility))
}

fn rcc_is_public(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(rcc_visibility(ctx, this)? == Visibility::Public))
}

fn rcc_is_private(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(rcc_visibility(ctx, this)? == Visibility::Private))
}

fn rcc_is_protected(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(rcc_visibility(ctx, this)? == Visibility::Protected))
}

fn rcc_get_modifiers(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(vis_bits(rcc_visibility(ctx, this)?)))
}

fn constant_subject(ctx: &mut dyn Ctx, this: &Object) -> Result<Subject, Throw> {
    Ok(Subject::Constant(held_class(ctx, this)?, PhpStr::from(text(&get(this, "name")).as_str())))
}

fn rcc_get_doc_comment(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let subject = constant_subject(ctx, this)?;
    Ok(doc_value(ctx, subject))
}

fn rcc_get_attributes(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let subject = constant_subject(ctx, this)?;
    attributes_of(ctx, subject, args, "ReflectionClassConstant")
}

// ---------------------------------------------------------------------------
// ReflectionEnum, ReflectionEnumUnitCase, ReflectionEnumBackedCase

fn re_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    rc_construct(ctx, this, args)?;
    let class = held_class(ctx, this)?;
    if class.flags & CLASS_ENUM == 0 {
        return Err(fail(ctx, format!("Class \"{}\" is not an enum", String::from_utf8_lossy(class.name.as_bytes()))));
    }
    Ok(Value::Null)
}

fn is_case(class: &Class, name: &str) -> bool {
    class.constants.get(name.as_bytes()).is_some_and(|c| c.is_case)
}

fn new_case(ctx: &mut dyn Ctx, class: &Rc<Class>, name: &str) -> Value {
    let o = make(ctx, if class.enum_backing.is_some() { b"ReflectionEnumBackedCase" } else { b"ReflectionEnumUnitCase" });
    set(&o, "name", s(name.as_bytes()));
    set(&o, "class", Value::Str(class.name.clone()));
    set(&o, "\0class", Value::Str(class.name.clone()));
    Value::Object(o)
}

class_method!(re_has_case, |ctx, class, this, args| Ok(Value::Bool(is_case(&class, &text(&arg(args, 0))))));
class_method!(re_get_case, |ctx, class, this, args| {
    let name = text(&arg(args, 0));
    let shown = String::from_utf8_lossy(class.name.as_bytes()).into_owned();
    match class.constants.get(name.as_bytes()) {
        None => Err(fail(ctx, format!("Case {shown}::{name} does not exist"))),
        Some(c) if !c.is_case => Err(fail(ctx, format!("{shown}::{name} is not a case"))),
        Some(_) => Ok(new_case(ctx, &class, &name)),
    }
});
class_method!(re_get_cases, |ctx, class, this, args| {
    let names: Vec<Vec<u8>> = class.constants.iter().filter(|(_, c)| c.is_case).map(|(k, _)| k.clone()).collect();
    Ok(list(names.iter().map(|n| new_case(ctx, &class, &String::from_utf8_lossy(n))).collect()))
});
class_method!(re_is_backed, |ctx, class, this, args| Ok(Value::Bool(class.enum_backing.is_some())));
class_method!(re_get_backing_type, |ctx, class, this, args| Ok(match &class.enum_backing {
    Some(t) => named_type(ctx, &String::from_utf8_lossy(t.as_bytes()), false, true),
    None => Value::Null,
}));

fn reuc_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    rcc_construct(ctx, this, args)?;
    let class = held_class(ctx, this)?;
    let name = text(&get(this, "name"));
    if !is_case(&class, &name) {
        return Err(fail(ctx, format!("Constant {}::{} is not a case", String::from_utf8_lossy(class.name.as_bytes()), name)));
    }
    Ok(Value::Null)
}

fn reuc_get_enum(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    Ok(new_class_reflection(ctx, &class))
}

fn reuc_get_value(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    Ok(ctx.enum_case(&class, text(&get(this, "name")).as_bytes()))
}

fn rebc_get_backing_value(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let class = held_class(ctx, this)?;
    Ok(class.constants.get(text(&get(this, "name")).as_bytes()).map_or(Value::Null, |c| c.value.clone()))
}

// ---------------------------------------------------------------------------
// Attribute, Deprecated

fn attribute_construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    set(this, "flags", Value::Int(if args.is_empty() { TARGET_ALL } else { arg(args, 0).to_int() }));
    Ok(Value::Null)
}

fn deprecated_construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    set(this, "message", arg(args, 0));
    set(this, "since", arg(args, 1));
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// Reflection

/// Reflection::getModifierNames(int $modifiers): in PHP's order.
fn modifier_names(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let m = arg(args, 0).to_int();
    let mut out = Vec::new();
    if m & 64 != 0 {
        out.push(s(b"abstract"));
    }
    if m & 32 != 0 {
        out.push(s(b"final"));
    }
    if m & 512 != 0 {
        out.push(s(b"virtual"));
    }
    match m & 7 {
        1 => out.push(s(b"public")),
        2 => out.push(s(b"protected")),
        4 => out.push(s(b"private")),
        _ => {}
    }
    if m & 16 != 0 {
        out.push(s(b"static"));
    }
    if m & (128 | 65536) != 0 {
        out.push(s(b"readonly"));
    }
    Ok(list(out))
}

// ---------------------------------------------------------------------------
// var_dump: what PHP shows of them

fn info_name(o: &Object) -> PhpArray {
    let mut out = PhpArray::new();
    out.insert(Key::from_string(PhpStr::from("name")), get(o, "name"));
    out
}

fn info_name_class(o: &Object) -> PhpArray {
    let mut out = info_name(o);
    out.insert(Key::from_string(PhpStr::from("class")), get(o, "class"));
    out
}

fn info_none(_: &Object) -> PhpArray {
    PhpArray::new()
}

// ---------------------------------------------------------------------------
// The class specs

const NAME: (&str, Constant, Visibility) = ("name", Constant::Str(b""), Visibility::Public);
const CLASS: (&str, Constant, Visibility) = ("class", Constant::Str(b""), Visibility::Public);
const H_CLASS: (&str, Constant, Visibility) = ("\0class", Constant::Null, Visibility::Public);
const H_OBJECT: (&str, Constant, Visibility) = ("\0object", Constant::Null, Visibility::Public);
const H_CALLEE: (&str, Constant, Visibility) = ("\0callee", Constant::Null, Visibility::Public);
const H_POS: (&str, Constant, Visibility) = ("\0pos", Constant::Int(0), Visibility::Public);

const FUNCTION_ABSTRACT: &[(&str, Option<Native>)] = &[
    ("getName", Some(Native::Method(fa_get_name))),
    ("getShortName", Some(Native::Method(fa_get_short_name))),
    ("getNamespaceName", Some(Native::Method(fa_get_namespace_name))),
    ("inNamespace", Some(Native::Method(fa_in_namespace))),
    ("getParameters", Some(Native::Method(fa_get_parameters))),
    ("getNumberOfParameters", Some(Native::Method(fa_get_number_of_parameters))),
    ("getNumberOfRequiredParameters", Some(Native::Method(fa_get_number_of_required_parameters))),
    ("getReturnType", Some(Native::Method(fa_get_return_type))),
    ("hasReturnType", Some(Native::Method(fa_has_return_type))),
    ("getTentativeReturnType", Some(Native::Method(fa_get_tentative_return_type))),
    ("hasTentativeReturnType", Some(Native::Method(fa_has_tentative_return_type))),
    ("isVariadic", Some(Native::Method(fa_is_variadic))),
    ("returnsReference", Some(Native::Method(fa_returns_reference))),
    ("isUserDefined", Some(Native::Method(fa_is_user_defined))),
    ("isInternal", Some(Native::Method(fa_is_internal))),
    ("isClosure", Some(Native::Method(fa_is_closure))),
    ("isGenerator", Some(Native::Method(fa_is_generator))),
    ("isStatic", Some(Native::Method(fa_is_static))),
    ("isDeprecated", Some(Native::Method(fa_is_deprecated))),
    ("getFileName", Some(Native::Method(fa_get_file_name))),
    ("getStartLine", Some(Native::Method(fa_get_start_line))),
    ("getEndLine", Some(Native::Method(fa_get_end_line))),
    ("getDocComment", Some(Native::Method(fa_get_doc_comment))),
    ("getAttributes", Some(Native::Method(fa_get_attributes))),
    ("getStaticVariables", Some(Native::Method(fa_get_static_variables))),
    ("getClosureThis", Some(Native::Method(fa_get_closure_this))),
    ("getExtensionName", Some(Native::Method(fa_get_extension_name))),
    ("__toString", Some(Native::Method(fa_to_string))),
    ("getClosureUsedVariables", Some(Native::Method(fa_get_closure_used_variables))),
    ("getClosureScopeClass", Some(Native::Method(fa_get_closure_scope_class))),
    ("getClosureCalledClass", Some(Native::Method(fa_get_closure_called_class))),
];

pub static CLASSES: &[ClassSpec] = &[
    ClassSpec { name: "ReflectionException", parent: Some("Exception"), interfaces: &[], flags: 0, properties: &[], methods: &[], constants: &[], debug_info: None },
    ClassSpec {
        name: "Reflection",
        parent: None,
        interfaces: &[],
        flags: 0,
        properties: &[],
        methods: &[("getModifierNames", Some(Native::Static(modifier_names)))],
        constants: &[],
        debug_info: None,
    },
    ClassSpec { name: "Reflector", parent: None, interfaces: &[], flags: CLASS_INTERFACE, properties: &[], methods: &[], constants: &[], debug_info: None },
    ClassSpec {
        name: "ReflectionType",
        parent: None,
        interfaces: &[],
        flags: CLASS_ABSTRACT,
        properties: &[("\0name", Constant::Str(b""), PUBLIC), ("\0null", Constant::Bool(false), PUBLIC), ("\0builtin", Constant::Bool(false), PUBLIC), ("\0types", Constant::Null, PUBLIC), ("\0shown", Constant::Str(b""), PUBLIC)],
        methods: &[("allowsNull", Some(Native::Method(nt_allows_null)))],
        constants: &[],
        debug_info: Some(info_none),
    },
    ClassSpec {
        name: "ReflectionNamedType",
        parent: Some("ReflectionType"),
        interfaces: &[],
        flags: 0,
        properties: &[],
        methods: &[
            ("getName", Some(Native::Method(nt_get_name))),
            ("isBuiltin", Some(Native::Method(nt_is_builtin))),
            ("__toString", Some(Native::Method(nt_to_string))),
        ],
        constants: &[],
        debug_info: Some(info_none),
    },
    ClassSpec {
        name: "ReflectionUnionType",
        parent: Some("ReflectionType"),
        interfaces: &[],
        flags: 0,
        properties: &[],
        methods: &[("getTypes", Some(Native::Method(ut_get_types))), ("__toString", Some(Native::Method(ut_to_string)))],
        constants: &[],
        debug_info: Some(info_none),
    },
    ClassSpec {
        name: "ReflectionIntersectionType",
        parent: Some("ReflectionType"),
        interfaces: &[],
        flags: 0,
        properties: &[],
        methods: &[("getTypes", Some(Native::Method(ut_get_types))), ("__toString", Some(Native::Method(ut_to_string)))],
        constants: &[],
        debug_info: Some(info_none),
    },
    ClassSpec {
        name: "ReflectionClass",
        parent: None,
        interfaces: &["Reflector"],
        flags: 0,
        properties: &[NAME, H_CLASS, H_OBJECT],
        methods: &[
            ("__construct", Some(Native::Method(rc_construct))),
            ("getName", Some(Native::Method(rc_get_name))),
            ("getShortName", Some(Native::Method(rc_get_short_name))),
            ("getNamespaceName", Some(Native::Method(rc_get_namespace_name))),
            ("inNamespace", Some(Native::Method(rc_in_namespace))),
            ("isInterface", Some(Native::Method(rc_is_interface))),
            ("isTrait", Some(Native::Method(rc_is_trait))),
            ("isEnum", Some(Native::Method(rc_is_enum))),
            ("isAbstract", Some(Native::Method(rc_is_abstract))),
            ("isFinal", Some(Native::Method(rc_is_final))),
            ("isReadOnly", Some(Native::Method(rc_is_readonly))),
            ("isInternal", Some(Native::Method(rc_is_internal))),
            ("isUserDefined", Some(Native::Method(rc_is_user_defined))),
            ("isAnonymous", Some(Native::Method(rc_is_anonymous))),
            ("isInstantiable", Some(Native::Method(rc_is_instantiable))),
            ("isCloneable", Some(Native::Method(rc_is_cloneable))),
            ("isIterable", Some(Native::Method(rc_is_iterable))),
            ("isIterateable", Some(Native::Method(rc_is_iterable))),
            ("isInstance", Some(Native::Method(rc_is_instance))),
            ("isSubclassOf", Some(Native::Method(rc_is_subclass_of))),
            ("implementsInterface", Some(Native::Method(rc_implements_interface))),
            ("getParentClass", Some(Native::Method(rc_get_parent_class))),
            ("getInterfaceNames", Some(Native::Method(rc_get_interface_names))),
            ("getInterfaces", Some(Native::Method(rc_get_interfaces))),
            ("getModifiers", Some(Native::Method(rc_get_modifiers))),
            ("hasMethod", Some(Native::Method(rc_has_method))),
            ("getMethod", Some(Native::Method(rc_get_method))),
            ("getMethods", Some(Native::Method(rc_get_methods))),
            ("getConstructor", Some(Native::Method(rc_get_constructor))),
            ("hasProperty", Some(Native::Method(rc_has_property))),
            ("getProperty", Some(Native::Method(rc_get_property))),
            ("getProperties", Some(Native::Method(rc_get_properties))),
            ("hasConstant", Some(Native::Method(rc_has_constant))),
            ("getConstant", Some(Native::Method(rc_get_constant))),
            ("getConstants", Some(Native::Method(rc_get_constants))),
            ("getReflectionConstant", Some(Native::Method(rc_get_reflection_constant))),
            ("getReflectionConstants", Some(Native::Method(rc_get_reflection_constants))),
            ("newInstance", Some(Native::Method(rc_new_instance))),
            ("newInstanceArgs", Some(Native::Method(rc_new_instance_args))),
            ("newInstanceWithoutConstructor", Some(Native::Method(rc_new_instance_without_constructor))),
            ("newLazyGhost", Some(Native::Method(rc_new_lazy_ghost))),
            ("newLazyProxy", Some(Native::Method(rc_new_lazy_proxy))),
            ("resetAsLazyGhost", Some(Native::Method(rc_reset_as_lazy_ghost))),
            ("resetAsLazyProxy", Some(Native::Method(rc_reset_as_lazy_proxy))),
            ("isUninitializedLazyObject", Some(Native::Method(rc_is_uninitialized_lazy_object))),
            ("initializeLazyObject", Some(Native::Method(rc_initialize_lazy_object))),
            ("markLazyObjectAsInitialized", Some(Native::Method(rc_mark_lazy_object_as_initialized))),
            ("getLazyInitializer", Some(Native::Method(rc_get_lazy_initializer))),
            ("getStaticPropertyValue", Some(Native::Method(rc_get_static_property_value))),
            ("setStaticPropertyValue", Some(Native::Method(rc_set_static_property_value))),
            ("getStaticProperties", Some(Native::Method(rc_get_static_properties))),
            ("getDefaultProperties", Some(Native::Method(rc_get_default_properties))),
            ("getFileName", Some(Native::Method(rc_get_file_name))),
            ("getStartLine", Some(Native::Method(rc_get_start_line))),
            ("getEndLine", Some(Native::Method(rc_get_end_line))),
            ("getDocComment", Some(Native::Method(rc_get_doc_comment))),
            ("getAttributes", Some(Native::Method(rc_get_attributes))),
            ("getTraitNames", Some(Native::Method(rc_get_trait_names))),
            ("getTraits", Some(Native::Method(rc_get_traits))),
            ("getExtensionName", Some(Native::Method(rc_get_extension_name))),
        ],
        constants: &[
            ("IS_IMPLICIT_ABSTRACT", Constant::Int(16)),
            ("IS_EXPLICIT_ABSTRACT", Constant::Int(64)),
            ("IS_FINAL", Constant::Int(32)),
            ("IS_READONLY", Constant::Int(65536)),
            ("SKIP_INITIALIZATION_ON_SERIALIZE", Constant::Int(8)),
            ("SKIP_DESTRUCTOR", Constant::Int(16)),
        ],
        debug_info: Some(info_name),
    },
    ClassSpec { name: "ReflectionObject", parent: Some("ReflectionClass"), interfaces: &[], flags: 0, properties: &[], methods: &[("__construct", Some(Native::Method(ro_construct)))], constants: &[], debug_info: Some(info_name) },
    ClassSpec {
        name: "ReflectionFunctionAbstract",
        parent: None,
        interfaces: &["Reflector"],
        flags: CLASS_ABSTRACT,
        properties: &[NAME, H_CALLEE],
        methods: FUNCTION_ABSTRACT,
        constants: &[],
        debug_info: Some(info_name),
    },
    ClassSpec {
        name: "ReflectionFunction",
        parent: Some("ReflectionFunctionAbstract"),
        interfaces: &[],
        flags: 0,
        properties: &[],
        methods: &[
            ("__construct", Some(Native::Method(rf_construct))),
            ("invoke", Some(Native::Method(rf_invoke))),
            ("invokeArgs", Some(Native::Method(rf_invoke_args))),
            ("getClosure", Some(Native::Method(rf_get_closure))),
            ("isAnonymous", Some(Native::Method(rf_is_anonymous))),
        ],
        constants: &[("IS_DEPRECATED", Constant::Int(2048))],
        debug_info: Some(info_name),
    },
    ClassSpec {
        name: "ReflectionMethod",
        parent: Some("ReflectionFunctionAbstract"),
        interfaces: &[],
        flags: 0,
        properties: &[CLASS],
        methods: &[
            ("__construct", Some(Native::Method(rm_construct))),
            ("createFromMethodName", Some(Native::Static(rm_create_from_method_name))),
            ("invoke", Some(Native::Method(rm_invoke))),
            ("invokeArgs", Some(Native::Method(rm_invoke_args))),
            ("getClosure", Some(Native::Method(rm_get_closure))),
            ("setAccessible", Some(Native::Method(no_op))),
            ("isPublic", Some(Native::Method(rm_is_public))),
            ("isPrivate", Some(Native::Method(rm_is_private))),
            ("isProtected", Some(Native::Method(rm_is_protected))),
            ("isAbstract", Some(Native::Method(rm_is_abstract))),
            ("isFinal", Some(Native::Method(rm_is_final))),
            ("isStatic", Some(Native::Method(rm_is_static))),
            ("isConstructor", Some(Native::Method(rm_is_constructor))),
            ("isDestructor", Some(Native::Method(rm_is_destructor))),
            ("getModifiers", Some(Native::Method(rm_get_modifiers))),
            ("getDeclaringClass", Some(Native::Method(rm_get_declaring_class))),
            ("getPrototype", Some(Native::Method(rm_get_prototype))),
            ("hasPrototype", Some(Native::Method(rm_has_prototype))),
        ],
        constants: &[
            ("IS_STATIC", Constant::Int(16)),
            ("IS_PUBLIC", Constant::Int(1)),
            ("IS_PROTECTED", Constant::Int(2)),
            ("IS_PRIVATE", Constant::Int(4)),
            ("IS_ABSTRACT", Constant::Int(64)),
            ("IS_FINAL", Constant::Int(32)),
        ],
        debug_info: Some(info_name_class),
    },
    ClassSpec {
        name: "ReflectionParameter",
        parent: None,
        interfaces: &["Reflector"],
        flags: 0,
        properties: &[NAME, H_CALLEE, H_POS],
        methods: &[
            ("__construct", Some(Native::Method(rp_construct))),
            ("getName", Some(Native::Method(rp_get_name))),
            ("getPosition", Some(Native::Method(rp_get_position))),
            ("isOptional", Some(Native::Method(rp_is_optional))),
            ("isDefaultValueAvailable", Some(Native::Method(rp_is_default_value_available))),
            ("getDefaultValue", Some(Native::Method(rp_get_default_value))),
            ("isDefaultValueConstant", Some(Native::Method(rp_is_default_value_constant))),
            ("getDefaultValueConstantName", Some(Native::Method(rp_get_default_value_constant_name))),
            ("getType", Some(Native::Method(rp_get_type))),
            ("hasType", Some(Native::Method(rp_has_type))),
            ("allowsNull", Some(Native::Method(rp_allows_null))),
            ("isVariadic", Some(Native::Method(rp_is_variadic))),
            ("isPassedByReference", Some(Native::Method(rp_is_passed_by_reference))),
            ("canBePassedByValue", Some(Native::Method(rp_can_be_passed_by_value))),
            ("isPromoted", Some(Native::Method(rp_is_promoted))),
            ("getDeclaringClass", Some(Native::Method(rp_get_declaring_class))),
            ("getDeclaringFunction", Some(Native::Method(rp_get_declaring_function))),
            ("getAttributes", Some(Native::Method(rp_get_attributes))),
            ("__toString", Some(Native::Method(rp_to_string))),
        ],
        constants: &[],
        debug_info: Some(info_name),
    },
    ClassSpec {
        name: "ReflectionProperty",
        parent: None,
        interfaces: &["Reflector"],
        flags: 0,
        properties: &[NAME, CLASS, H_CLASS],
        methods: &[
            ("__construct", Some(Native::Method(rpr_construct))),
            ("getName", Some(Native::Method(rpr_get_name))),
            ("getValue", Some(Native::Method(rpr_get_value))),
            ("setValue", Some(Native::Method(rpr_set_value))),
            ("getRawValue", Some(Native::Method(rpr_get_raw_value))),
            ("setRawValue", Some(Native::Method(rpr_set_raw_value))),
            ("isInitialized", Some(Native::Method(rpr_is_initialized))),
            ("skipLazyInitialization", Some(Native::Method(rpr_skip_lazy_initialization))),
            ("setRawValueWithoutLazyInitialization", Some(Native::Method(rpr_set_raw_value_without_lazy_initialization))),
            ("isLazy", Some(Native::Method(rpr_is_lazy))),
            ("isPublic", Some(Native::Method(rpr_is_public))),
            ("isPrivate", Some(Native::Method(rpr_is_private))),
            ("isProtected", Some(Native::Method(rpr_is_protected))),
            ("isStatic", Some(Native::Method(rpr_is_static))),
            ("isReadOnly", Some(Native::Method(rpr_is_readonly))),
            ("isPrivateSet", Some(Native::Method(rpr_is_private_set))),
            ("isProtectedSet", Some(Native::Method(rpr_is_protected_set))),
            ("isDefault", Some(Native::Method(rpr_is_default))),
            ("isPromoted", Some(Native::Method(rpr_is_promoted))),
            ("hasDefaultValue", Some(Native::Method(rpr_has_default_value))),
            ("getDefaultValue", Some(Native::Method(rpr_get_default_value))),
            ("getType", Some(Native::Method(rpr_get_type))),
            ("hasType", Some(Native::Method(rpr_has_type))),
            ("getModifiers", Some(Native::Method(rpr_get_modifiers))),
            ("isFinal", Some(Native::Method(rpr_is_final))),
            ("isAbstract", Some(Native::Method(rpr_is_abstract))),
            ("isVirtual", Some(Native::Method(rpr_is_virtual))),
            ("hasHooks", Some(Native::Method(rpr_has_hooks))),
            ("getHooks", Some(Native::Method(rpr_get_hooks))),
            ("getDeclaringClass", Some(Native::Method(rpr_get_declaring_class))),
            ("setAccessible", Some(Native::Method(no_op))),
            ("getDocComment", Some(Native::Method(rpr_get_doc_comment))),
            ("getAttributes", Some(Native::Method(rpr_get_attributes))),
        ],
        constants: &[
            ("IS_STATIC", Constant::Int(16)),
            ("IS_READONLY", Constant::Int(128)),
            ("IS_PUBLIC", Constant::Int(1)),
            ("IS_PROTECTED", Constant::Int(2)),
            ("IS_PRIVATE", Constant::Int(4)),
            ("IS_ABSTRACT", Constant::Int(64)),
            ("IS_PROTECTED_SET", Constant::Int(2048)),
            ("IS_PRIVATE_SET", Constant::Int(4096)),
            ("IS_VIRTUAL", Constant::Int(512)),
            ("IS_FINAL", Constant::Int(32)),
        ],
        debug_info: Some(info_name_class),
    },
    ClassSpec {
        name: "ReflectionClassConstant",
        parent: None,
        interfaces: &["Reflector"],
        flags: 0,
        properties: &[NAME, CLASS, H_CLASS],
        methods: &[
            ("__construct", Some(Native::Method(rcc_construct))),
            ("getName", Some(Native::Method(rcc_get_name))),
            ("getValue", Some(Native::Method(rcc_get_value))),
            ("getDeclaringClass", Some(Native::Method(rcc_get_declaring_class))),
            ("isPublic", Some(Native::Method(rcc_is_public))),
            ("isPrivate", Some(Native::Method(rcc_is_private))),
            ("isProtected", Some(Native::Method(rcc_is_protected))),
            ("getModifiers", Some(Native::Method(rcc_get_modifiers))),
            ("getDocComment", Some(Native::Method(rcc_get_doc_comment))),
            ("getAttributes", Some(Native::Method(rcc_get_attributes))),
        ],
        constants: &[("IS_PUBLIC", Constant::Int(1)), ("IS_PROTECTED", Constant::Int(2)), ("IS_PRIVATE", Constant::Int(4)), ("IS_FINAL", Constant::Int(32))],
        debug_info: Some(info_name_class),
    },
    ClassSpec {
        name: "ReflectionReference",
        parent: None,
        interfaces: &[],
        flags: 0,
        properties: &[("\0ref", Constant::Null, PUBLIC)],
        methods: &[
            ("__construct", Some(Native::Method(rr_construct))),
            ("__clone", Some(Native::Method(ra_clone))),
            ("fromArrayElement", Some(Native::Static(rr_from_array_element))),
            ("getId", Some(Native::Method(rr_get_id))),
        ],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "ReflectionAttribute",
        parent: None,
        interfaces: &["Reflector"],
        flags: 0,
        properties: &[NAME, ("\0subject", Constant::Null, PUBLIC), ("\0index", Constant::Int(0), PUBLIC), ("\0target", Constant::Int(0), PUBLIC), ("\0repeated", Constant::Bool(false), PUBLIC)],
        methods: &[
            ("__construct", Some(Native::Method(ra_construct))),
            ("__clone", Some(Native::Method(ra_clone))),
            ("getName", Some(Native::Method(ra_get_name))),
            ("getTarget", Some(Native::Method(ra_get_target))),
            ("isRepeated", Some(Native::Method(ra_is_repeated))),
            ("getArguments", Some(Native::Method(ra_get_arguments))),
            ("newInstance", Some(Native::Method(ra_new_instance))),
            ("__toString", Some(Native::Method(ra_to_string))),
        ],
        constants: &[("IS_INSTANCEOF", Constant::Int(2))],
        debug_info: Some(info_name),
    },
    ClassSpec {
        name: "ReflectionEnum",
        parent: Some("ReflectionClass"),
        interfaces: &[],
        flags: 0,
        properties: &[],
        methods: &[
            ("__construct", Some(Native::Method(re_construct))),
            ("hasCase", Some(Native::Method(re_has_case))),
            ("getCase", Some(Native::Method(re_get_case))),
            ("getCases", Some(Native::Method(re_get_cases))),
            ("isBacked", Some(Native::Method(re_is_backed))),
            ("getBackingType", Some(Native::Method(re_get_backing_type))),
        ],
        constants: &[],
        debug_info: Some(info_name),
    },
    ClassSpec {
        name: "ReflectionEnumUnitCase",
        parent: Some("ReflectionClassConstant"),
        interfaces: &[],
        flags: 0,
        properties: &[],
        methods: &[
            ("__construct", Some(Native::Method(reuc_construct))),
            ("getEnum", Some(Native::Method(reuc_get_enum))),
            ("getValue", Some(Native::Method(reuc_get_value))),
        ],
        constants: &[],
        debug_info: Some(info_name_class),
    },
    ClassSpec {
        name: "ReflectionEnumBackedCase",
        parent: Some("ReflectionEnumUnitCase"),
        interfaces: &[],
        flags: 0,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(reuc_construct))), ("getBackingValue", Some(Native::Method(rebc_get_backing_value)))],
        constants: &[],
        debug_info: Some(info_name_class),
    },
    ClassSpec {
        name: "Attribute",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL,
        properties: &[("flags", Constant::Int(TARGET_ALL), PUBLIC)],
        methods: &[("__construct", Some(Native::Method(attribute_construct)))],
        constants: &[
            ("TARGET_CLASS", Constant::Int(TARGET_CLASS)),
            ("TARGET_FUNCTION", Constant::Int(TARGET_FUNCTION)),
            ("TARGET_METHOD", Constant::Int(TARGET_METHOD)),
            ("TARGET_PROPERTY", Constant::Int(TARGET_PROPERTY)),
            ("TARGET_CLASS_CONSTANT", Constant::Int(TARGET_CLASS_CONSTANT)),
            ("TARGET_PARAMETER", Constant::Int(TARGET_PARAMETER)),
            ("TARGET_ALL", Constant::Int(TARGET_ALL)),
            ("IS_REPEATABLE", Constant::Int(IS_REPEATABLE)),
        ],
        debug_info: None,
    },
    ClassSpec {
        name: "Deprecated",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL,
        properties: &[("message", Constant::Null, PUBLIC), ("since", Constant::Null, PUBLIC)],
        methods: &[("__construct", Some(Native::Method(deprecated_construct)))],
        constants: &[],
        debug_info: None,
    },
];

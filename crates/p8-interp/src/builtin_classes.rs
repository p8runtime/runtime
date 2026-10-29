//! The classes every request starts with: stdClass, the core interfaces,
//! and the Throwable hierarchy with its methods (in Rust). An isolate
//! builds them once; each context starts from them.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_exceptions.c (+ .stub.php),
//! $PHP_SRC_PATH/Zend/zend_interfaces.c, $PHP_SRC_PATH/ext/spl/spl_exceptions.c

use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::{CLASS_INTERFACE, Class, Method, Native, NativeMethod, Object, Property, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::collections::HashMap;
use std::rc::Rc;

pub(crate) type Classes = HashMap<Vec<u8>, Rc<Class>>;

/// A class: `parent` and `interfaces` already in `classes`.
fn declare(
    classes: &mut Classes,
    name: &str,
    flags: u32,
    parent: Option<&str>,
    interfaces: &[&str],
    properties: Vec<Property>,
    methods: &[(&str, Option<NativeMethod>, bool)],
) {
    let lower = name.to_ascii_lowercase().into_bytes();
    let parent = parent.map(|p| Rc::clone(&classes[p.to_ascii_lowercase().as_bytes()]));
    let (inherited_props, inherited_bool, inherited_string, inherited_clone) = parent.as_ref().map_or((None, None, None, None), |p| (p.get_properties, p.cast_bool, p.cast_string, p.clone_data));
    let inherited_forward = parent.as_ref().and_then(|p| p.forward_calls);
    let mut all_interfaces: Vec<Vec<u8>> = parent.as_ref().map(|p| p.interfaces.clone()).unwrap_or_default();
    let mut table: p8_runtime::object::Methods = parent.as_ref().map(|p| p.methods.clone()).unwrap_or_default();
    // Constants are inherited from the parent and the interfaces, as for
    // a user class (RecursiveRegexIterator::GET_MATCH is RegexIterator's).
    let mut constants: p8_runtime::object::Constants = parent.as_ref().map(|p| p.constants.clone()).unwrap_or_default();
    for i in interfaces {
        let iface = &classes[i.to_ascii_lowercase().as_bytes()];
        for (k, c) in &iface.constants {
            constants.entry(k.clone()).or_insert_with(|| c.clone());
        }
        for x in std::iter::once(iface.lower.clone()).chain(iface.interfaces.iter().cloned()) {
            if !all_interfaces.contains(&x) {
                all_interfaces.push(x);
            }
        }
        for (k, m) in &iface.methods {
            table.entry(k.clone()).or_insert_with(|| m.clone());
        }
    }
    let mut props: Vec<Property> = parent.as_ref().map(|p| p.properties.clone()).unwrap_or_default();
    for p in properties {
        match props.iter().position(|q| q.name == p.name && q.visibility != Visibility::Private) {
            Some(at) => props[at] = p,
            None => props.push(p),
        }
    }
    let mut slots: HashMap<Vec<u8>, Vec<usize>> = HashMap::new();
    for (i, p) in props.iter().enumerate() {
        slots.entry(p.name.as_bytes().to_vec()).or_default().push(i);
    }
    for (m, native, is_final) in methods {
        table.insert(
            m.to_ascii_lowercase().into_bytes(),
            Method {
                name: PhpStr::from(*m),
                func: u32::MAX,
                native: native.map(Native::Method),
                visibility: Visibility::Public,
                is_static: false,
                is_abstract: native.is_none(),
                is_final: *is_final,
                declaring: lower.clone(),
                declaring_name: PhpStr::from(name),
            },
        );
    }
    let class = Class {
        name: PhpStr::from(name),
        lower: lower.clone(),
        flags: flags | p8_runtime::object::CLASS_INTERNAL,
        parent,
        interfaces: all_interfaces,
        properties: props,
        slots,
        methods: table,
        constants,
        statics: HashMap::new(),
        debug_info: None,
        get_properties: inherited_props,
        cast_bool: inherited_bool,
        cast_string: inherited_string,
        clone_data: inherited_clone,
        forward_calls: inherited_forward,
        make_data: None,
        native_dtor: None,
        enum_cases: Vec::new(),
        enum_backing: None,
        static_inits: Vec::new(),
    };
    classes.insert(lower, Rc::new(class));
}

fn interface(classes: &mut Classes, name: &str, extends: &[&str], methods: &[&str]) {
    let methods: Vec<(&str, Option<NativeMethod>, bool)> = methods.iter().map(|m| (*m, None, false)).collect();
    declare(classes, name, CLASS_INTERFACE, None, extends, Vec::new(), &methods);
}

fn prop(name: &str, default: Value, visibility: Visibility, declaring: &str, ty: Option<&str>) -> Property {
    Property {
        name: PhpStr::from(name),
        default,
        visibility,
        declaring: declaring.to_ascii_lowercase().into_bytes(),
        declaring_name: PhpStr::from(declaring),
        readonly: false,
        ty_decl: None,
        init: None,
        hooks: 0,
        ty: ty.map(PhpStr::from),
    }
}

/// Exception's and Error's properties, in PHP's order.
fn throwable_props(root: &str) -> Vec<Property> {
    use Visibility::{Private, Protected};
    let empty = || Value::Str(PhpStr::empty());
    vec![
        prop("message", empty(), Protected, root, None),
        prop("string", empty(), Private, root, Some("string")),
        prop("code", Value::Int(0), Protected, root, None),
        prop("file", empty(), Protected, root, Some("string")),
        prop("line", Value::Int(0), Protected, root, Some("int")),
        prop("trace", Value::Array(Rc::new(PhpArray::new())), Private, root, Some("array")),
        prop("previous", Value::Null, Private, root, Some("?Throwable")),
    ]
}

const THROWABLE_METHODS: &[(&str, Option<NativeMethod>, bool)] = &[
    ("__construct", Some(construct), false),
    ("__wakeup", Some(wakeup), false),
    ("getMessage", Some(get_message), true),
    ("getCode", Some(get_code), true),
    ("getFile", Some(get_file), true),
    ("getLine", Some(get_line), true),
    ("getTrace", Some(get_trace), true),
    ("getPrevious", Some(get_previous), true),
    ("getTraceAsString", Some(get_trace_as_string), true),
    ("__toString", Some(to_string), false),
];

pub(crate) fn build(registry: &p8_runtime::ext::Registry) -> Classes {
    let mut c = Classes::new();
    // #[AllowDynamicProperties], which its subclasses inherit.
    declare(&mut c, "stdClass", p8_runtime::object::CLASS_ALLOW_DYNAMIC, None, &[], Vec::new(), &[]);
    declare(&mut c, "__PHP_Incomplete_Class", p8_runtime::object::CLASS_FINAL, None, &[], Vec::new(), &[]);
    // The builtin attribute classes without state (Attribute and Deprecated: reflection.rs).
    for name in ["ReturnTypeWillChange", "AllowDynamicProperties", "SensitiveParameter", "Override"] {
        declare(&mut c, name, p8_runtime::object::CLASS_FINAL, None, &[], Vec::new(), &[]);
    }
    declare(
        &mut c,
        "Closure",
        p8_runtime::object::CLASS_FINAL,
        None,
        &[],
        Vec::new(),
        &[("bindTo", Some(closure_bind_to), false), ("call", Some(closure_call), false), ("__invoke", Some(closure_invoke), false)],
    );
    let closure = Rc::get_mut(c.get_mut(b"closure".as_slice()).expect("Closure")).expect("not shared yet");
    closure.debug_info = Some(crate::callable::debug_info);
    for (name, f) in [("bind", closure_bind as p8_runtime::object::NativeStatic), ("fromCallable", closure_from_callable)] {
        let lower = name.to_ascii_lowercase().into_bytes();
        let mut m = closure.methods[b"bindto".as_slice()].clone();
        m.name = PhpStr::from(name);
        m.native = Some(Native::Static(f));
        m.is_static = true;
        closure.methods.insert(lower, m);
    }
    interface(&mut c, "Stringable", &[], &["__toString"]);
    interface(&mut c, "Traversable", &[], &[]);
    interface(&mut c, "Iterator", &["Traversable"], &["current", "next", "key", "valid", "rewind"]);
    interface(&mut c, "IteratorAggregate", &["Traversable"], &["getIterator"]);
    interface(&mut c, "ArrayAccess", &[], &["offsetExists", "offsetGet", "offsetSet", "offsetUnset"]);
    interface(&mut c, "Countable", &[], &["count"]);
    interface(&mut c, "JsonSerializable", &[], &["jsonSerialize"]);
    interface(&mut c, "UnitEnum", &[], &["cases"]);
    interface(&mut c, "BackedEnum", &["UnitEnum"], &["from", "tryFrom"]);
    interface(
        &mut c,
        "Throwable",
        &["Stringable"],
        &["getMessage", "getCode", "getFile", "getLine", "getTrace", "getPrevious", "getTraceAsString"],
    );
    declare(
        &mut c,
        "Generator",
        p8_runtime::object::CLASS_FINAL,
        None,
        &["Iterator"],
        Vec::new(),
        &[
            ("current", Some(gen_current), false),
            ("key", Some(gen_key), false),
            ("next", Some(gen_next), false),
            ("send", Some(gen_send), false),
            ("valid", Some(gen_valid), false),
            ("rewind", Some(gen_rewind), false),
            ("getReturn", Some(gen_get_return), false),
            ("throw", Some(gen_throw), false),
        ],
    );
    declare(&mut c, "Exception", 0, None, &["Throwable"], throwable_props("Exception"), THROWABLE_METHODS);
    declare(&mut c, "Error", 0, None, &["Throwable"], throwable_props("Error"), THROWABLE_METHODS);
    declare(
        &mut c,
        "ErrorException",
        0,
        Some("Exception"),
        &[],
        vec![prop("severity", Value::Int(1), Visibility::Protected, "ErrorException", Some("int"))],
        &[("__construct", Some(error_exception_construct), false), ("getSeverity", Some(get_severity), true)],
    );
    for (name, parent) in [
        ("CompileError", "Error"),
        ("ParseError", "CompileError"),
        ("TypeError", "Error"),
        ("ArgumentCountError", "TypeError"),
        ("ValueError", "Error"),
        ("ArithmeticError", "Error"),
        ("DivisionByZeroError", "ArithmeticError"),
        ("AssertionError", "Error"),
        ("UnhandledMatchError", "Error"),
        ("LogicException", "Exception"),
        ("BadFunctionCallException", "LogicException"),
        ("BadMethodCallException", "BadFunctionCallException"),
        ("DomainException", "LogicException"),
        ("InvalidArgumentException", "LogicException"),
        ("LengthException", "LogicException"),
        ("OutOfRangeException", "LogicException"),
        ("RuntimeException", "Exception"),
        ("OutOfBoundsException", "RuntimeException"),
        ("OverflowException", "RuntimeException"),
        ("RangeException", "RuntimeException"),
        ("UnderflowException", "RuntimeException"),
        ("UnexpectedValueException", "RuntimeException"),
        ("JsonException", "Exception"),
    ] {
        declare(&mut c, name, 0, Some(parent), &[], Vec::new(), &[]);
    }
    // Reflection's and the extensions' classes are declared when first
    // wanted (Context::builtin_on_demand).
    let _ = registry;
    c
}

/// Declare a class an extension (or the engine's Reflection) specifies.
pub(crate) fn declare_spec(
    c: &mut Classes,
    spec: &p8_runtime::ext::ClassSpec,
    make_data: Option<p8_runtime::object::MakeData>,
    dtor: Option<p8_runtime::object::NativeMethod>,
) {
    let props = spec.properties.iter().map(|(n, d, v)| prop(n, d.to_value(), *v, spec.name, None)).collect();
    let methods: Vec<(&str, Option<NativeMethod>, bool)> = spec
        .methods
        .iter()
        .filter_map(|(n, m)| match m {
            Some(Native::Method(f)) => Some((*n, Some(*f), false)),
            None => Some((*n, None, false)),
            Some(Native::Static(_)) => None,
        })
        .collect();
    declare(c, spec.name, spec.flags, spec.parent, spec.interfaces, props, &methods);
    let class = Rc::get_mut(c.get_mut(spec.name.to_ascii_lowercase().as_bytes()).expect("just declared")).expect("not shared yet");
    class.debug_info = spec.debug_info;
    // A property with a `$name::get` / `$name::set` native method is a
    // virtual hooked property: reads and writes run that native code, as
    // PHP's internal classes (DOM) back their properties.
    use p8_runtime::object::{CLASS_HOOKS, HOOK_GET, HOOK_SET, HOOK_VIRTUAL};
    for (n, _, _) in spec.properties {
        let has = |kind: &str| spec.methods.iter().any(|(m, _)| m.eq_ignore_ascii_case(&format!("${n}::{kind}")));
        let hooks = if has("get") { HOOK_GET } else { 0 } | if has("set") { HOOK_SET } else { 0 };
        if hooks != 0
            && let Some(p) = class.properties.iter_mut().find(|p| p.name.as_bytes() == n.as_bytes())
        {
            p.hooks = hooks | HOOK_VIRTUAL;
        }
    }
    if class.properties.iter().any(|p| p.hooks != 0) {
        class.flags |= CLASS_HOOKS;
    }
    if [&b"__get"[..], b"__set", b"__isset", b"__unset", b"$*::get", b"$*::set", b"$*::isset", b"$*::unset"].iter().any(|m| class.methods.contains_key(*m)) {
        class.flags |= p8_runtime::object::CLASS_MAGIC_PROPS;
    }
    class.make_data = make_data.or_else(|| class.parent.as_ref().and_then(|p| p.make_data));
    class.native_dtor = dtor.or_else(|| class.parent.as_ref().and_then(|p| p.native_dtor));
    if class.native_dtor.is_some() {
        class.flags |= p8_runtime::object::CLASS_DESTRUCTOR;
    }
    for (n, value) in spec.constants {
        class.constants.insert(
            n.as_bytes().to_vec(),
            p8_runtime::object::Constant { value: value.to_value(), visibility: Visibility::Public, declaring_name: PhpStr::from(spec.name), is_case: false, init: None },
        );
    }
    for (n, m) in spec.methods {
        if let Some(Native::Static(f)) = m {
            class.methods.insert(
                n.to_ascii_lowercase().into_bytes(),
                Method {
                    name: PhpStr::from(*n),
                    func: u32::MAX,
                    native: Some(Native::Static(*f)),
                    visibility: Visibility::Public,
                    is_static: true,
                    is_abstract: false,
                    is_final: false,
                    declaring: spec.name.to_ascii_lowercase().into_bytes(),
                    declaring_name: PhpStr::from(spec.name),
                },
            );
        }
    }
}

fn gen_current(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.generator(this, "current", Value::Null)
}

fn gen_key(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.generator(this, "key", Value::Null)
}

fn gen_next(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.generator(this, "next", Value::Null)
}

fn gen_send(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let v = args.first().map(Value::deref).unwrap_or(Value::Null);
    ctx.generator(this, "send", v)
}

fn gen_valid(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.generator(this, "valid", Value::Null)
}

fn gen_rewind(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.generator(this, "rewind", Value::Null)
}

fn gen_get_return(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.generator(this, "getreturn", Value::Null)
}

fn gen_throw(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    match args.first().map(Value::deref) {
        Some(e @ Value::Object(_)) => ctx.generator(this, "throw", e),
        _ => Err(ctx.throw("TypeError", "Generator::throw(): Argument #1 ($exception) must be of type Throwable".into())),
    }
}

/// An enum's cases(), in order.
pub(crate) fn enum_cases(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let Some(class) = ctx.native_class() else { return Ok(Value::Null) };
    let cases: Vec<Value> = class.enum_cases.iter().map(|n| ctx.enum_case(&class, n.as_bytes())).collect();
    Ok(Value::Array(Rc::new(PhpArray::from_list(cases))))
}

/// The case backed by `v` (converted to the backing type), if any.
fn backed_case(ctx: &mut dyn Ctx, args: &[Value]) -> Result<(Rc<Class>, Value, Option<Value>), Throw> {
    let class = ctx.native_class().expect("an enum's method");
    let v = args.first().map(Value::deref).unwrap_or(Value::Null);
    let int = class.enum_backing.as_ref().is_some_and(|t| t.as_bytes() == b"int");
    let v = if int { Value::Int(v.to_int()) } else { Value::Str(v.to_php_string().unwrap_or_else(PhpStr::empty)) };
    let found = class.enum_cases.iter().find(|n| class.constants.get(n.as_bytes()).is_some_and(|c| c.value.identical(&v))).cloned();
    let case = found.map(|n| ctx.enum_case(&class, n.as_bytes()));
    Ok((class, v, case))
}

/// BackedEnum::from(int|string $value)
pub(crate) fn enum_from(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (class, v, case) = backed_case(ctx, args)?;
    match case {
        Some(c) => Ok(c),
        None => {
            let shown = match &v {
                Value::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
                other => String::from_utf8_lossy(other.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned(),
            };
            Err(ctx.throw("ValueError", format!("{shown} is not a valid backing value for enum {}", String::from_utf8_lossy(class.name.as_bytes()))))
        }
    }
}

/// BackedEnum::tryFrom(int|string $value)
pub(crate) fn enum_try_from(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(backed_case(ctx, args)?.2.unwrap_or(Value::Null))
}

/// Closure::bindTo(?object $newThis, object|string|null $newScope = "static")
fn closure_bind_to(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let new_this = args.first().map(Value::deref).unwrap_or(Value::Null);
    let scope = args.get(1).map(Value::deref);
    ctx.bind_closure(this, new_this, scope)
}

/// Closure::bind(Closure $closure, ?object $newThis, object|string|null $newScope = "static")
fn closure_bind(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(Value::Object(closure)) = args.first().map(Value::deref) else {
        return Err(ctx.throw("TypeError", "Closure::bind(): Argument #1 ($closure) must be of type Closure".into()));
    };
    let new_this = args.get(1).map(Value::deref).unwrap_or(Value::Null);
    let scope = args.get(2).map(Value::deref);
    ctx.bind_closure(&closure, new_this, scope)
}

/// Closure::call(object $newThis, mixed ...$args): bound to the object
/// and its class for this one call.
fn closure_call(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let new_this = args.first().map(Value::deref).unwrap_or(Value::Null);
    let scope = match &new_this {
        Value::Object(o) => Value::Str(o.class.name.clone()),
        _ => Value::Null,
    };
    let bound = ctx.bind_closure(this, new_this, Some(scope))?;
    let rest = if args.is_empty() { &mut [][..] } else { &mut args[1..] };
    ctx.call(&bound, rest)
}

fn closure_invoke(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    ctx.call(&Value::Object(Rc::clone(this)), args)
}

/// Closure::fromCallable(callable $callback)
fn closure_from_callable(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let callable = args.first().map(Value::deref).unwrap_or(Value::Null);
    ctx.closure_from(&callable)
}

/// The class Throwable's private properties belong to: Exception or Error.
fn root(o: &Object) -> &'static [u8] {
    if o.class.is_a(b"exception") { b"exception" } else { b"error" }
}

fn read(o: &Object, name: &[u8]) -> Value {
    match o.class.read(o, name, root(o)) {
        Value::Undef => Value::Null,
        v => v,
    }
}

fn write(o: &Object, name: &[u8], value: Value) {
    o.class.write(o, name, root(o), value);
}

/// Chains `previous` under `e`, at the end of e's own previous chain (as
/// zend_exception_set_previous; nothing if it is in that chain already).
pub(crate) fn set_previous(e: &Rc<Object>, previous: Rc<Object>) {
    let mut last = Rc::clone(e);
    loop {
        if Rc::ptr_eq(&last, &previous) {
            return;
        }
        match read(&last, b"previous") {
            Value::Object(p) => last = p,
            _ => break,
        }
    }
    write(&last, b"previous", Value::Object(previous));
}

/// Exception::__construct(string $message = "", int $code = 0, ?Throwable $previous = null)
fn construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let class = String::from_utf8_lossy(this.class.name.as_bytes()).into_owned();
    if args.len() > 3 {
        return Err(ctx.throw("ArgumentCountError", format!("{class}::__construct() expects at most 3 arguments, {} given", args.len())));
    }
    if let Some(m) = args.first().map(Value::deref) {
        let message = match m {
            Value::Array(_) | Value::Object(_) => {
                return Err(ctx.throw(
                    "TypeError",
                    format!("{class}::__construct(): Argument #1 ($message) must be of type string, {} given", m.type_name()),
                ));
            }
            v => Value::Str(v.to_php_string().unwrap_or_else(PhpStr::empty)),
        };
        write(this, b"message", message);
    }
    if let Some(c) = args.get(1).map(Value::deref) {
        let code = match c {
            Value::Int(i) => i,
            Value::Bool(b) => i64::from(b),
            Value::Float(f) if f.fract() == 0.0 => f as i64,
            other => {
                return Err(ctx.throw(
                    "TypeError",
                    format!("{class}::__construct(): Argument #2 ($code) must be of type int, {} given", other.type_name()),
                ));
            }
        };
        write(this, b"code", Value::Int(code));
    }
    if let Some(p) = args.get(2).map(Value::deref) {
        match &p {
            Value::Null => {}
            Value::Object(o) if o.class.is_a(b"throwable") => write(this, b"previous", p.clone()),
            other => {
                return Err(ctx.throw(
                    "TypeError",
                    format!("{class}::__construct(): Argument #3 ($previous) must be of type ?Throwable, {} given", other.type_name()),
                ));
            }
        }
    }
    Ok(Value::Null)
}

/// ErrorException::__construct($message = "", $code = 0, $severity = E_ERROR,
/// $filename = null, $line = null, $previous = null)
fn error_exception_construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let mut base: Vec<Value> = args.iter().take(2).cloned().collect();
    if let Some(p) = args.get(5) {
        base.resize(2, Value::Int(0));
        base.push(p.clone());
    }
    construct(ctx, this, &mut base)?;
    if let Some(s) = args.get(2) {
        this.class.write(this, b"severity", b"errorexception", Value::Int(s.to_int()));
    }
    if let Some(f) = args.get(3).map(Value::deref).filter(|v| !matches!(v, Value::Null)) {
        write(this, b"file", Value::Str(f.to_php_string().unwrap_or_else(PhpStr::empty)));
    }
    if let Some(l) = args.get(4).map(Value::deref).filter(|v| !matches!(v, Value::Null)) {
        write(this, b"line", Value::Int(l.to_int()));
    }
    Ok(Value::Null)
}

fn get_severity(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(this.class.read(this, b"severity", b"errorexception"))
}

fn wakeup(_: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

fn get_message(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(read(this, b"message"))
}

fn get_code(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(read(this, b"code"))
}

fn get_file(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(read(this, b"file"))
}

fn get_line(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(read(this, b"line"))
}

fn get_trace(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(read(this, b"trace"))
}

fn get_previous(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(read(this, b"previous"))
}

fn get_trace_as_string(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Str(PhpStr::from(p8_runtime::trace::trace_string(&read(this, b"trace")))))
}

fn to_string(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Str(PhpStr::from(describe(this))))
}

/// What Exception::__toString gives: the exception and its previous ones,
/// the innermost first, each with its trace (zend_exceptions.c).
pub(crate) fn describe(this: &Rc<Object>) -> String {
    let mut out = String::new();
    let mut current = Some(Rc::clone(this));
    while let Some(e) = current {
        let class = String::from_utf8_lossy(e.class.name.as_bytes()).into_owned();
        let mut message = text(&read(&e, b"message"));
        // An argument error raised for a call names where the function is
        // defined next (the " in file:line" that follows).
        if matches!(e.class.lower.as_slice(), b"typeerror" | b"argumentcounterror") && message.contains(", called in ") {
            message.push_str(" and defined");
        }
        let file = text(&read(&e, b"file"));
        let line = read(&e, b"line").to_int();
        let trace = p8_runtime::trace::trace_string(&read(&e, b"trace"));
        let next = if out.is_empty() { String::new() } else { format!("\n\nNext {out}") };
        out = if message.is_empty() {
            format!("{class} in {file}:{line}\nStack trace:\n{trace}{next}")
        } else {
            format!("{class}: {message} in {file}:{line}\nStack trace:\n{trace}{next}")
        };
        current = match read(&e, b"previous") {
            Value::Object(p) => Some(p),
            _ => None,
        };
    }
    out
}

fn text(v: &Value) -> String {
    String::from_utf8_lossy(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned()
}


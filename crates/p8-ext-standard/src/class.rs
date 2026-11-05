//! Class and object information: get_class, method_exists, class_exists
//! and the like. Arguments arrive already checked and converted
//! (p8_runtime::args).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_builtin_functions.c,
//! $PHP_SRC_PATH/ext/spl/php_spl.c (spl_object_id, spl_object_hash)

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::object::{CLASS_ENUM, CLASS_INTERFACE, CLASS_TRAIT, Class, visible};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "clone", run: clone, by_ref: &[] },
    Builtin { name: "get_class", run: get_class, by_ref: &[] },
    Builtin { name: "get_parent_class", run: get_parent_class, by_ref: &[] },
    Builtin { name: "get_object_vars", run: get_object_vars, by_ref: &[] },
    Builtin { name: "get_mangled_object_vars", run: get_mangled_object_vars, by_ref: &[] },
    Builtin { name: "get_class_vars", run: get_class_vars, by_ref: &[] },
    Builtin { name: "method_exists", run: method_exists, by_ref: &[] },
    Builtin { name: "property_exists", run: property_exists, by_ref: &[] },
    Builtin { name: "class_exists", run: class_exists, by_ref: &[] },
    Builtin { name: "get_declared_classes", run: get_declared_classes, by_ref: &[] },
    Builtin { name: "get_declared_interfaces", run: get_declared_interfaces, by_ref: &[] },
    Builtin { name: "get_declared_traits", run: get_declared_traits, by_ref: &[] },
    Builtin { name: "interface_exists", run: interface_exists, by_ref: &[] },
    Builtin { name: "trait_exists", run: trait_exists, by_ref: &[] },
    Builtin { name: "enum_exists", run: enum_exists, by_ref: &[] },
    Builtin { name: "is_a", run: is_a, by_ref: &[] },
    Builtin { name: "is_subclass_of", run: is_subclass_of, by_ref: &[] },
    Builtin { name: "spl_object_id", run: spl_object_id, by_ref: &[] },
    Builtin { name: "spl_object_hash", run: spl_object_hash, by_ref: &[] },
    Builtin { name: "get_called_class", run: get_called_class, by_ref: &[] },
    Builtin { name: "get_class_methods", run: get_class_methods, by_ref: &[] },
    Builtin { name: "class_alias", run: class_alias, by_ref: &[] },
    Builtin { name: "constant", run: constant, by_ref: &[] },
    Builtin { name: "set_error_handler", run: set_error_handler, by_ref: &[] },
    Builtin { name: "restore_error_handler", run: restore_error_handler, by_ref: &[] },
    Builtin { name: "set_exception_handler", run: set_exception_handler, by_ref: &[] },
    Builtin { name: "restore_exception_handler", run: restore_exception_handler, by_ref: &[] },
    Builtin { name: "spl_autoload_register", run: spl_autoload_register, by_ref: &[] },
    Builtin { name: "spl_autoload_unregister", run: spl_autoload_unregister, by_ref: &[] },
    Builtin { name: "spl_autoload_functions", run: spl_autoload_functions, by_ref: &[] },
    Builtin { name: "spl_autoload_call", run: spl_autoload_call, by_ref: &[] },
    Builtin { name: "register_shutdown_function", run: register_shutdown_function, by_ref: &[] },
];

/// Builtins that run PHP code: constant() computes a class constant
/// written as an expression; those that look a class up by name run the
/// autoloaders (zend_lookup_class).
pub const REENTER: &[&str] = &[
    "constant", "get_class_methods", "class_alias", "get_parent_class", "get_class_vars", "method_exists", "property_exists", "class_exists",
    "interface_exists", "trait_exists", "enum_exists", "is_a", "is_subclass_of", "spl_autoload_call",
];

/// get_class_methods(object|string $object_or_class): the methods the
/// calling code can see, as declared.
fn get_class_methods(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let class = match &v {
        Value::Object(o) => Some(Rc::clone(&o.class)),
        Value::Str(s) => ctx.class_load(s.as_bytes())?,
        _ => None,
    };
    let Some(class) = class else {
        return Err(ctx.throw(
            "TypeError",
            format!("get_class_methods(): Argument #1 ($object_or_class) must be an object or a valid class name, {} given", v.type_name()),
        ));
    };
    let scope = ctx.scope();
    let names: Vec<Value> = class
        .methods
        .values()
        // A property hook (`$x::get`) is not a method PHP lists.
        .filter(|m| !m.name.as_bytes().starts_with(b"$"))
        .filter(|m| visible(m.visibility, declaring(&class, &m.declaring), scope.as_deref()))
        .map(|m| Value::Str(m.name.clone()))
        .collect();
    Ok(Value::Array(Rc::new(PhpArray::from_list(names))))
}

fn get_called_class(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    match ctx.called_class() {
        Some(c) => Ok(name(&c)),
        None => Err(ctx.throw("Error", "get_called_class() must be called from within a class".into())),
    }
}

/// class_alias(string $class, string $alias, bool $autoload = true)
fn class_alias(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let original = str_arg(args, 0);
    let alias = str_arg(args, 1);
    let class = if args.len() < 3 || arg(args, 2).to_bool() { ctx.class_load(original.as_bytes())? } else { ctx.class(original.as_bytes()) };
    let Some(class) = class else {
        ctx.warn(&format!("Class \"{}\" not found", String::from_utf8_lossy(original.as_bytes())));
        return Ok(Value::Bool(false));
    };
    if !ctx.class_alias(class, alias.as_bytes()) {
        ctx.warn(&format!("Cannot declare class {}, because the name is already in use", String::from_utf8_lossy(alias.as_bytes())));
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

/// constant(string $name): a constant, or a class constant ("C::NAME").
fn constant(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let full = str_arg(args, 0);
    let text = full.as_bytes();
    if let Some(at) = text.windows(2).position(|w| w == b"::") {
        let (class_name, name) = (&text[..at], &text[at + 2..]);
        let class = match class_named(ctx, class_name)? {
            Some(c) => c,
            None => return Err(ctx.throw("Error", format!("Class \"{}\" not found", String::from_utf8_lossy(class_name)))),
        };
        if name.eq_ignore_ascii_case(b"class") {
            return Ok(Value::Str(class.name.clone()));
        }
        return match class.constants.get(name) {
            Some(c) => {
                let declaring = ctx.class(c.declaring_name.as_bytes()).unwrap_or_else(|| Rc::clone(&class));
                if !visible(c.visibility, &declaring, ctx.scope().as_deref()) {
                    return Err(ctx.throw(
                        "Error",
                        format!("Cannot access {} constant {}::{}", c.visibility.word(), class.name.to_lossy(), String::from_utf8_lossy(name)),
                    ));
                }
                // An enum case's object; a constant computed from an expression.
                Ok(ctx.class_constant(&class, name)?.unwrap_or(Value::Null))
            }
            None => Err(ctx.throw("Error", format!("Undefined constant {}::{}", class.name.to_lossy(), String::from_utf8_lossy(name)))),
        };
    }
    match ctx.constant(text) {
        Some(v) => Ok(v),
        None => Err(ctx.throw("Error", format!("Undefined constant \"{}\"", String::from_utf8_lossy(text)))),
    }
}

/// Whether class constant `class_name::name` exists and the running code
/// can see it (defined()): no error when the class or constant is missing.
/// The class a constant name's class part names: self, parent and static
/// are the caller's (zend_get_class_constant_ex; an Error where there is
/// none), any other is looked up (autoloaded); None when there is no such
/// class.
fn class_named(ctx: &mut dyn Ctx, class_name: &[u8]) -> Result<Option<Rc<Class>>, Throw> {
    let (word, class) = match class_name.to_ascii_lowercase().as_slice() {
        b"self" => ("self", ctx.scope()),
        b"parent" => ("parent", ctx.scope().and_then(|c| c.parent.clone())),
        b"static" => ("static", ctx.called_class()),
        _ => return ctx.class_load(class_name),
    };
    match class {
        Some(c) => Ok(Some(c)),
        None => {
            let message = match (word, ctx.scope()) {
                ("parent", Some(_)) => "Cannot access \"parent\" when current class scope has no parent".to_string(),
                _ => format!("Cannot access \"{word}\" when no class scope is active"),
            };
            Err(ctx.throw("Error", message))
        }
    }
}

pub(crate) fn class_constant_defined(ctx: &mut dyn Ctx, class_name: &[u8], name: &[u8]) -> Result<bool, Throw> {
    let Some(class) = class_named(ctx, class_name)? else { return Ok(false) };
    // A trait's constants are its users' (read through the trait, never).
    if class.flags & CLASS_TRAIT != 0 {
        return Ok(false);
    }
    Ok(match class.constants.get(name) {
        Some(c) => {
            let declaring = ctx.class(c.declaring_name.as_bytes()).unwrap_or_else(|| Rc::clone(&class));
            visible(c.visibility, &declaring, ctx.scope().as_deref())
        }
        None => false,
    })
}

trait Lossy {
    fn to_lossy(&self) -> String;
}

impl Lossy for PhpStr {
    fn to_lossy(&self) -> String {
        String::from_utf8_lossy(self.as_bytes()).into_owned()
    }
}

/// E_ALL
const E_ALL: i64 = 30719;

/// set_error_handler(?callable $callback, int $error_levels = E_ALL)
fn set_error_handler(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let levels = if args.len() > 1 { arg(args, 1).to_int() } else { E_ALL };
    Ok(ctx.error_handler(Some((arg(args, 0), levels))))
}

fn restore_error_handler(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.error_handler(None);
    Ok(Value::Bool(true))
}

fn set_exception_handler(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(ctx.exception_handler(Some(arg(args, 0))))
}

fn restore_exception_handler(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    ctx.exception_handler(None);
    Ok(Value::Bool(true))
}

/// spl_autoload_register(?callable $callback = null, bool $throw = true, bool $prepend = false)
fn spl_autoload_register(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let callable = arg(args, 0);
    if matches!(callable, Value::Null) {
        // The default (spl_autoload: lowercase file names in include_path) is not here yet.
        return Ok(Value::Bool(true));
    }
    let prepend = args.len() > 2 && arg(args, 2).to_bool();
    Ok(Value::Bool(ctx.autoloader(callable, Some(prepend))))
}

fn spl_autoload_unregister(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let callable = arg(args, 0);
    // spl_autoload_call itself: every autoloader goes (php_spl.c),
    // deprecated since PHP 8.5.
    if let Value::Str(s) = &callable
        && s.as_bytes().strip_prefix(b"\\").unwrap_or(s.as_bytes()).eq_ignore_ascii_case(b"spl_autoload_call")
    {
        ctx.deprecated(
            "spl_autoload_unregister(): Using spl_autoload_call() as a callback for spl_autoload_unregister() is deprecated, to remove all registered autoloaders, call spl_autoload_unregister() for all values returned from spl_autoload_functions()",
        );
        for loader in ctx.autoloaders() {
            ctx.autoloader(loader, None);
        }
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(ctx.autoloader(callable, None)))
}

/// spl_autoload_call(string $class): the autoloaders run for `$class`
/// (unless it is declared already), as a lookup of it runs them.
fn spl_autoload_call(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    ctx.class_load(name.as_bytes())?;
    Ok(Value::Null)
}

fn spl_autoload_functions(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Array(Rc::new(PhpArray::from_list(ctx.autoloaders()))))
}

/// register_shutdown_function(callable $callback, mixed ...$args)
fn register_shutdown_function(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let callable = arg(args, 0);
    let rest = args[1..].iter().map(Value::deref).collect();
    ctx.on_shutdown(callable, rest);
    Ok(Value::Null)
}

/// The class an `$object_or_class` argument names: an object's, or a
/// declared class's (None for an unknown name).
fn class_of(ctx: &mut dyn Ctx, v: &Value) -> Option<Rc<Class>> {
    match v {
        Value::Object(o) => Some(Rc::clone(&o.class)),
        Value::Str(s) => ctx.class(s.as_bytes()),
        _ => None,
    }
}

/// class_of, autoloading a class given by name (zend_lookup_class).
fn class_loaded(ctx: &mut dyn Ctx, v: &Value) -> Result<Option<Rc<Class>>, Throw> {
    match v {
        Value::Str(s) => ctx.class_load(s.as_bytes()),
        other => Ok(class_of(ctx, other)),
    }
}

fn name(class: &Class) -> Value {
    Value::Str(class.name.clone())
}

/// get_class($object = ?): the object's class, or with no argument the
/// class the calling code is in.
fn get_class(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    match args.first().map(Value::deref) {
        Some(Value::Object(o)) => Ok(name(&o.class)),
        _ => match ctx.scope() {
            Some(c) => Ok(name(&c)),
            None => Err(ctx.throw("Error", "get_class() without arguments must be called from within a class".into())),
        },
    }
}

/// get_parent_class($object_or_class = ?): the parent's name, or false.
fn get_parent_class(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let class = match args.first().map(Value::deref) {
        Some(v) => match class_loaded(ctx, &v)? {
            Some(c) => Some(c),
            None => {
                return Err(ctx.throw(
                    "TypeError",
                    format!("get_parent_class(): Argument #1 ($object_or_class) must be an object or a valid class name, {} given", v.type_name()),
                ));
            }
        },
        None => ctx.scope(),
    };
    Ok(class.and_then(|c| c.parent.as_deref().map(name)).unwrap_or(Value::Bool(false)))
}

/// get_mangled_object_vars($object): every initialized property under its
/// mangled name ("\0A\0p" private, "\0*\0p" protected), then the dynamic
/// ones, as (array) gives them (but no class's own view of its properties).
fn get_mangled_object_vars(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Null) };
    let o = if o.lazy.get() != 0 { ctx.lazy_initialize(&o)? } else { o };
    let mut out = PhpArray::new();
    let slots = o.slots.borrow();
    for (p, v) in o.class.properties.iter().zip(slots.iter()) {
        if matches!(v, Value::Undef) || p.is_internal() {
            continue;
        }
        let mut name = Vec::new();
        match p.visibility {
            p8_runtime::object::Visibility::Public => {}
            p8_runtime::object::Visibility::Protected => name.extend_from_slice(b"\0*\0"),
            p8_runtime::object::Visibility::Private => {
                name.push(0);
                name.extend_from_slice(p.declaring_name.as_bytes());
                name.push(0);
            }
        }
        name.extend_from_slice(p.name.as_bytes());
        out.insert(Key::from_string(PhpStr::new(name)), v.clone());
    }
    for (k, v) in o.dynamic.borrow().iter() {
        out.insert(k, v.clone());
    }
    Ok(Value::Array(Rc::new(out)))
}

/// get_object_vars($object): the properties the calling code can see,
/// initialized ones only, by their plain names.
/// clone(object $object, array $withProperties = []): what `clone $object`
/// makes, with the properties updated (ZEND_FUNCTION(clone)).
fn clone(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Null) };
    let with = match arg(args, 1) {
        Value::Array(a) => a,
        _ => Rc::new(PhpArray::new()),
    };
    ctx.clone_with(&o, &with)
}

fn get_object_vars(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Null) };
    let o = if o.lazy.get() != 0 { ctx.lazy_initialize(&o)? } else { o };
    if let Some(props) = o.class.get_properties {
        return Ok(Value::Array(Rc::new(props(&o))));
    }
    let scope = ctx.scope();
    let mut out = PhpArray::new();
    for (i, p) in o.class.properties.iter().enumerate() {
        if p.is_internal() || p.hooks == 0 && matches!(o.slots.borrow()[i], Value::Undef) {
            continue;
        }
        let declaring = declaring(&o.class, &p.declaring);
        if !visible(p.visibility, declaring, scope.as_deref()) {
            continue;
        }
        // A parent's private property hidden by the scope's own: the
        // scope sees its own.
        if o.class.slot(p.name.as_bytes(), scope.as_ref().map(|s| s.lower.as_slice())) != Some(i) {
            continue;
        }
        // Through its get hook, as `$o->name` reads it.
        if let Some(v) = ctx.prop_value(&o, i)? {
            out.insert(Key::from_string(p.name.clone()), v);
        }
    }
    for (k, v) in o.dynamic.borrow().iter() {
        out.insert(k, v.deref());
    }
    Ok(Value::Array(Rc::new(out)))
}

/// get_class_vars(string $class): the default values of the properties
/// the calling code can see (the class's own first, then each parent's;
/// a typed one with no default as null), then the static ones' values
/// (by name, as Reflection lists them).
fn get_class_vars(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0);
    let Some(class) = ctx.class_load(name.as_bytes())? else {
        return Err(ctx.throw("TypeError", format!("get_class_vars(): Argument #1 ($class) must be a valid class name, {} given", String::from_utf8_lossy(name.as_bytes()))));
    };
    let defaults = ctx.class_defaults(&class)?;
    let scope = ctx.scope();
    // The declaring classes, the class itself first.
    let mut chain = Vec::new();
    let mut c = Some(&*class);
    while let Some(k) = c {
        chain.push(k.lower.clone());
        c = k.parent.as_deref();
    }
    let mut out = PhpArray::new();
    for lower in &chain {
        for (i, p) in class.properties.iter().enumerate() {
            if &p.declaring != lower || p.is_internal() || p.hooks & p8_runtime::object::HOOK_VIRTUAL != 0 {
                continue;
            }
            if !visible(p.visibility, declaring(&class, &p.declaring), scope.as_deref()) {
                continue;
            }
            if class.slot(p.name.as_bytes(), scope.as_ref().map(|s| s.lower.as_slice())) != Some(i) {
                continue;
            }
            let v = match &defaults[i] {
                Value::Undef => Value::Null,
                v => v.clone(),
            };
            out.insert(Key::from_string(p.name.clone()), v);
        }
    }
    // In declaration order (the class's own, its parents', its traits').
    for (name, s) in &class.statics {
        if visible(s.visibility, declaring(&class, &s.declaring), scope.as_deref()) {
            // An uninitialized typed static is listed as null, as an
            // instance property is above (add_class_vars: ZVAL_NULL).
            let cell = ctx.static_cell(s);
            let v = match cell.borrow().deref() {
                Value::Undef => Value::Null,
                v => v,
            };
            out.insert(Key::from_string(PhpStr::from(name.as_slice())), v);
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

fn declaring<'c>(class: &'c Class, lower: &[u8]) -> &'c Class {
    let mut c = Some(class);
    while let Some(k) = c {
        if k.lower == lower {
            return k;
        }
        c = k.parent.as_deref();
    }
    class
}

/// method_exists($object_or_class, $method)
fn method_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let target = arg(args, 0);
    if !matches!(target, Value::Object(_) | Value::Str(_)) {
        return Err(ctx.throw(
            "TypeError",
            format!("method_exists(): Argument #1 ($object_or_class) must be of type object|string, {} given", target.type_name()),
        ));
    }
    let method = str_arg(args, 1).as_bytes().to_ascii_lowercase();
    if method.starts_with(b"$") {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(class_loaded(ctx, &target)?.is_some_and(|c| c.method(&method).is_some())))
}

/// property_exists($object_or_class, $property): declared (any
/// visibility, static too), or dynamic on the object.
fn property_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let target = arg(args, 0);
    let property = str_arg(args, 1);
    let Some(class) = class_loaded(ctx, &target)? else { return Ok(Value::Bool(false)) };
    if class.slots.contains_key(property.as_bytes()) || class.statics.contains_key(property.as_bytes()) {
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(match &target {
        Value::Object(o) => o.dynamic.borrow().contains_key(&Key::from_string(property)),
        _ => false,
    }))
}

/// The class `$class` names, autoloaded when `$autoload` (argument 1) is.
fn lookup(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Option<Rc<Class>>, Throw> {
    let name = str_arg(args, 0);
    if args.len() < 2 || arg(args, 1).to_bool() { ctx.class_load(name.as_bytes()) } else { Ok(ctx.class(name.as_bytes())) }
}

/// class_exists($class, $autoload = true): a class (not an interface or trait).
fn class_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let class = lookup(ctx, args)?;
    Ok(Value::Bool(class.is_some_and(|c| c.flags & (CLASS_INTERFACE | CLASS_TRAIT) == 0)))
}

fn interface_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let class = lookup(ctx, args)?;
    Ok(Value::Bool(class.is_some_and(|c| c.flags & CLASS_INTERFACE != 0)))
}

fn trait_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let class = lookup(ctx, args)?;
    Ok(Value::Bool(class.is_some_and(|c| c.flags & CLASS_TRAIT != 0)))
}

fn enum_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let class = lookup(ctx, args)?;
    Ok(Value::Bool(class.is_some_and(|c| c.flags & CLASS_ENUM != 0)))
}

/// is_a($object_or_class, $class, $allow_string = false)
fn is_a(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let allow_string = args.len() > 2 && arg(args, 2).to_bool();
    Ok(Value::Bool(instance(ctx, args, allow_string, false)?))
}

/// is_subclass_of($object_or_class, $class, $allow_string = true)
fn is_subclass_of(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let allow_string = args.len() <= 2 || arg(args, 2).to_bool();
    Ok(Value::Bool(instance(ctx, args, allow_string, true)?))
}

/// A class named by a string is autoloaded (when strings are allowed).
fn instance(ctx: &mut dyn Ctx, args: &[Value], allow_string: bool, strict: bool) -> Result<bool, Throw> {
    let target = arg(args, 0);
    let class = match &target {
        Value::Str(_) if !allow_string => return Ok(false),
        Value::Str(s) => ctx.class_load(s.as_bytes())?,
        other => class_of(ctx, other),
    };
    let Some(class) = class else { return Ok(false) };
    let wanted = str_arg(args, 1);
    let wanted = wanted.as_bytes();
    let wanted = wanted.strip_prefix(b"\\").unwrap_or(wanted).to_ascii_lowercase();
    // An alias (class_alias) names its class (looked up, not autoloaded).
    let wanted = ctx.class(&wanted).map_or(wanted, |c| c.lower.clone());
    if strict && class.lower == wanted {
        return Ok(false);
    }
    Ok(class.is_a(&wanted))
}

fn spl_object_id(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Null) };
    Ok(Value::Int(i64::from(o.id)))
}

/// spl_object_hash: the id in 16 hex digits, then 16 zeros (PHP 8.1+).
fn spl_object_hash(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Null) };
    Ok(Value::Str(PhpStr::from(format!("{:016x}0000000000000000", o.id))))
}

/// The names of the declared classes whose kind `pick` accepts.
fn declared(ctx: &mut dyn Ctx, pick: impl Fn(u32) -> bool) -> Value {
    let names = ctx.declared_classes().into_iter().filter(|(_, c)| pick(c.flags)).map(|(name, _)| Value::Str(name)).collect();
    Value::Array(Rc::new(PhpArray::from_list(names)))
}

/// Classes and enums: not interfaces or traits.
fn get_declared_classes(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(declared(ctx, |f| f & (CLASS_INTERFACE | CLASS_TRAIT) == 0))
}

fn get_declared_interfaces(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(declared(ctx, |f| f & CLASS_INTERFACE != 0))
}

fn get_declared_traits(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(declared(ctx, |f| f & CLASS_TRAIT != 0))
}

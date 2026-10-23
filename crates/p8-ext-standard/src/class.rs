//! Class and object information: get_class, method_exists, class_exists
//! and the like. Arguments arrive already checked and converted
//! (p8_runtime::args).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_builtin_functions.c,
//! $PHP_SRC_PATH/ext/spl/php_spl.c (spl_object_id, spl_object_hash)

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::object::{CLASS_INTERFACE, CLASS_TRAIT, Class, visible};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "get_class", run: get_class, by_ref: &[] },
    Builtin { name: "get_parent_class", run: get_parent_class, by_ref: &[] },
    Builtin { name: "get_object_vars", run: get_object_vars, by_ref: &[] },
    Builtin { name: "method_exists", run: method_exists, by_ref: &[] },
    Builtin { name: "property_exists", run: property_exists, by_ref: &[] },
    Builtin { name: "class_exists", run: class_exists, by_ref: &[] },
    Builtin { name: "interface_exists", run: interface_exists, by_ref: &[] },
    Builtin { name: "is_a", run: is_a, by_ref: &[] },
    Builtin { name: "is_subclass_of", run: is_subclass_of, by_ref: &[] },
    Builtin { name: "spl_object_id", run: spl_object_id, by_ref: &[] },
    Builtin { name: "spl_object_hash", run: spl_object_hash, by_ref: &[] },
    Builtin { name: "get_called_class", run: get_called_class, by_ref: &[] },
    Builtin { name: "class_alias", run: class_alias, by_ref: &[] },
    Builtin { name: "constant", run: constant, by_ref: &[] },
    Builtin { name: "set_error_handler", run: set_error_handler, by_ref: &[] },
    Builtin { name: "restore_error_handler", run: restore_error_handler, by_ref: &[] },
    Builtin { name: "set_exception_handler", run: set_exception_handler, by_ref: &[] },
    Builtin { name: "restore_exception_handler", run: restore_exception_handler, by_ref: &[] },
    Builtin { name: "spl_autoload_register", run: spl_autoload_register, by_ref: &[] },
    Builtin { name: "spl_autoload_unregister", run: spl_autoload_unregister, by_ref: &[] },
    Builtin { name: "spl_autoload_functions", run: spl_autoload_functions, by_ref: &[] },
    Builtin { name: "register_shutdown_function", run: register_shutdown_function, by_ref: &[] },
];

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
        let Some(class) = ctx.class_load(class_name)? else {
            return Err(ctx.throw("Error", format!("Class \"{}\" not found", String::from_utf8_lossy(class_name))));
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
                Ok(c.value.clone())
            }
            None => Err(ctx.throw("Error", format!("Undefined constant {}::{}", class.name.to_lossy(), String::from_utf8_lossy(name)))),
        };
    }
    match ctx.constant(text) {
        Some(v) => Ok(v),
        None => Err(ctx.throw("Error", format!("Undefined constant \"{}\"", String::from_utf8_lossy(text)))),
    }
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
    Ok(Value::Bool(ctx.autoloader(arg(args, 0), None)))
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
fn class_of(ctx: &dyn Ctx, v: &Value) -> Option<Rc<Class>> {
    match v {
        Value::Object(o) => Some(Rc::clone(&o.class)),
        Value::Str(s) => ctx.class(s.as_bytes()),
        _ => None,
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
        Some(v) => class_of(ctx, &v),
        None => ctx.scope(),
    };
    Ok(class.and_then(|c| c.parent.as_deref().map(name)).unwrap_or(Value::Bool(false)))
}

/// get_object_vars($object): the properties the calling code can see,
/// initialized ones only, by their plain names.
fn get_object_vars(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Null) };
    let scope = ctx.scope();
    let mut out = PhpArray::new();
    let slots = o.slots.borrow();
    for (i, p) in o.class.properties.iter().enumerate() {
        if matches!(slots[i], Value::Undef) {
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
        out.insert(Key::from_string(p.name.clone()), slots[i].deref());
    }
    for (k, v) in o.dynamic.borrow().iter() {
        out.insert(k, v.deref());
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
    Ok(Value::Bool(class_of(ctx, &target).is_some_and(|c| c.method(&method).is_some())))
}

/// property_exists($object_or_class, $property): declared (any
/// visibility, static too), or dynamic on the object.
fn property_exists(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let target = arg(args, 0);
    let property = str_arg(args, 1);
    let Some(class) = class_of(ctx, &target) else { return Ok(Value::Bool(false)) };
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

/// is_a($object_or_class, $class, $allow_string = false)
fn is_a(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let allow_string = args.len() > 2 && arg(args, 2).to_bool();
    Ok(Value::Bool(instance(ctx, args, allow_string, false)))
}

/// is_subclass_of($object_or_class, $class, $allow_string = true)
fn is_subclass_of(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let allow_string = args.len() <= 2 || arg(args, 2).to_bool();
    Ok(Value::Bool(instance(ctx, args, allow_string, true)))
}

fn instance(ctx: &dyn Ctx, args: &[Value], allow_string: bool, strict: bool) -> bool {
    let target = arg(args, 0);
    if matches!(target, Value::Str(_)) && !allow_string {
        return false;
    }
    let Some(class) = class_of(ctx, &target) else { return false };
    let wanted = str_arg(args, 1);
    let wanted = wanted.as_bytes();
    let wanted = wanted.strip_prefix(b"\\").unwrap_or(wanted).to_ascii_lowercase();
    if strict && class.lower == wanted {
        return false;
    }
    class.is_a(&wanted)
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

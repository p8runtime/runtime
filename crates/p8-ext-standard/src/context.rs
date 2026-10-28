//! Stream contexts: stream_context_create() and friends. A context is a
//! "stream-context" resource holding its options (wrapper => [option =>
//! value]) and params; the default context is made once, when first asked
//! for.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/streamsfuncs.c (stream_context_*)

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::object::Object;
use p8_runtime::stream::{Resource, Stream, Target, resource, resource_of};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "stream_context_create", run: create, by_ref: &[] },
    Builtin { name: "stream_context_get_options", run: get_options, by_ref: &[] },
    Builtin { name: "stream_context_set_option", run: set_option, by_ref: &[] },
    Builtin { name: "stream_context_set_options", run: set_options, by_ref: &[] },
    Builtin { name: "stream_context_get_params", run: get_params, by_ref: &[] },
    Builtin { name: "stream_context_set_params", run: set_params, by_ref: &[] },
    Builtin { name: "stream_context_get_default", run: get_default, by_ref: &[] },
    Builtin { name: "stream_context_set_default", run: set_default, by_ref: &[] },
];

// Where a context keeps them: properties no PHP code can name.
const OPTIONS: &[u8] = b"\0options";
const NOTIFICATION: &[u8] = b"\0notification";

fn key(k: &[u8]) -> Key {
    Key::Str(PhpStr::from(k))
}

fn options(o: &Object) -> PhpArray {
    match o.dynamic.borrow().get(&key(OPTIONS)) {
        Some(Value::Array(a)) => (**a).clone(),
        _ => PhpArray::new(),
    }
}

fn store(o: &Object, options: PhpArray) {
    o.dynamic.borrow_mut().insert(key(OPTIONS), Value::Array(Rc::new(options)));
}

/// `wrapper => [option => value]` merged into the context's options.
fn merge(ctx: &mut dyn Ctx, o: &Object, given: &PhpArray) -> Result<(), Throw> {
    let mut all = options(o);
    for (wrapper, opts) in given.iter() {
        let Value::Array(opts) = opts.deref() else {
            return Err(ctx.throw("ValueError", "Options should have the form [\"wrappername\"][\"optionname\"] = $value".into()));
        };
        let mut w = match all.get(&wrapper) {
            Some(Value::Array(a)) => (**a).clone(),
            _ => PhpArray::new(),
        };
        for (k, v) in opts.iter() {
            w.insert(k, v.deref());
        }
        all.insert(wrapper, Value::Array(Rc::new(w)));
    }
    store(o, all);
    Ok(())
}

fn new_context(ctx: &mut dyn Ctx) -> Value {
    ctx.new_resource(Resource {
        kind: "stream-context",
        closed: std::cell::Cell::new(false),
        stream: std::cell::RefCell::new(Stream::new(Target::Memory(Vec::new()), "", "r")),
    })
}

/// The context argument `n`: a stream-context resource (or a stream, for
/// the functions that take either).
fn context_arg(ctx: &mut dyn Ctx, args: &[Value], n: usize, function: &str, streams: bool) -> Result<Rc<Object>, Throw> {
    let what = if streams { "($stream_or_context) must be a valid stream/context" } else { "($context) must be a valid stream/context" };
    match resource_of(&arg(args, n)) {
        Some(o) if resource(&o).is_some_and(|r| r.kind == "stream-context" || (streams && r.kind == "stream")) => Ok(o),
        _ => Err(ctx.throw("TypeError", format!("{function}(): Argument #{} {what}", n + 1))),
    }
}

fn set_params_on(ctx: &mut dyn Ctx, o: &Object, params: &PhpArray) -> Result<(), Throw> {
    if let Some(n) = params.get(&key(b"notification")) {
        o.dynamic.borrow_mut().insert(key(NOTIFICATION), n.deref());
    }
    if let Some(Value::Array(opts)) = params.get(&key(b"options")).map(Value::deref) {
        merge(ctx, o, &opts)?;
    }
    Ok(())
}

/// stream_context_create(?array $options = null, ?array $params = null)
fn create(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = new_context(ctx);
    let o = resource_of(&v).expect("a resource");
    if let Value::Array(opts) = arg(args, 0) {
        merge(ctx, &o, &opts)?;
    }
    if let Value::Array(params) = arg(args, 1) {
        set_params_on(ctx, &o, &params)?;
    }
    Ok(v)
}

fn get_options(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, 0, "stream_context_get_options", true)?;
    Ok(Value::Array(Rc::new(options(&o))))
}

/// stream_context_set_option($context, string $wrapper, string $option, $value),
/// or (deprecated) with an array of options in place of the last three.
fn set_option(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, 0, "stream_context_set_option", true)?;
    match arg(args, 1) {
        Value::Array(opts) => {
            if args.len() > 2 {
                return Err(ctx.throw("ArgumentCountError", "stream_context_set_option(): Argument #3 ($option_name) must be null when argument #2 ($wrapper_or_options) is an array".into()));
            }
            merge(ctx, &o, &opts)?;
        }
        wrapper => {
            if args.len() < 3 || matches!(arg(args, 2), Value::Null) {
                return Err(ctx.throw("ValueError", "stream_context_set_option(): Argument #3 ($option_name) cannot be null when argument #2 ($wrapper_or_options) is a string".into()));
            }
            if args.len() < 4 {
                return Err(ctx.throw("ArgumentCountError", "stream_context_set_option(): Argument #4 ($value) must be provided when argument #2 ($wrapper_or_options) is a string".into()));
            }
            let mut opt = PhpArray::new();
            opt.insert(Key::from_value(&arg(args, 2)).map(|(k, _)| k).unwrap_or_else(|_| key(b"")), arg(args, 3));
            let mut given = PhpArray::new();
            given.insert(Key::from_value(&wrapper).map(|(k, _)| k).unwrap_or_else(|_| key(b"")), Value::Array(Rc::new(opt)));
            merge(ctx, &o, &given)?;
        }
    }
    Ok(Value::Bool(true))
}

fn set_options(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, 0, "stream_context_set_options", false)?;
    if let Value::Array(opts) = arg(args, 1) {
        merge(ctx, &o, &opts)?;
    }
    Ok(Value::Bool(true))
}

/// The params: its notification callback, if any, and its options.
fn get_params(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, 0, "stream_context_get_params", true)?;
    let mut out = PhpArray::new();
    if let Some(n) = o.dynamic.borrow().get(&key(NOTIFICATION)) {
        out.insert(key(b"notification"), n.clone());
    }
    out.insert(key(b"options"), Value::Array(Rc::new(options(&o))));
    Ok(Value::Array(Rc::new(out)))
}

fn set_params(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, 0, "stream_context_set_params", true)?;
    if let Value::Array(params) = arg(args, 1) {
        set_params_on(ctx, &o, &params)?;
    }
    Ok(Value::Bool(true))
}

fn default_context(ctx: &mut dyn Ctx) -> Value {
    if let v @ Value::Object(_) = ctx.state("stream_context_default").clone() {
        return v;
    }
    let v = new_context(ctx);
    *ctx.state("stream_context_default") = v.clone();
    v
}

/// stream_context_get_default(?array $options = null)
fn get_default(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = default_context(ctx);
    if let Value::Array(opts) = arg(args, 0) {
        merge(ctx, &resource_of(&v).expect("a resource"), &opts)?;
    }
    Ok(v)
}

fn set_default(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = default_context(ctx);
    if let Value::Array(opts) = arg(args, 0) {
        merge(ctx, &resource_of(&v).expect("a resource"), &opts)?;
    }
    Ok(v)
}

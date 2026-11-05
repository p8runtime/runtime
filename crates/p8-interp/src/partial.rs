//! Partial function application (PHP 8.6): `f(?, $x)`, `$o->m(?, ...)`,
//! `C::m(a: ?)`. The compiler leaves the arguments' values in registers and
//! a PartialSite saying which are placeholders (Rare::Partial); here the
//! function applied is resolved and a Closure is made: its code is a
//! closure written for this site and this function (its parameters the
//! placeholders, its `use` variables the bound arguments, its body the
//! forwarding call), compiled once for the isolate and shared by every
//! request (program memory); a call of the site only binds the values.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_partial.c (zp_compile, zp_bind,
//! zend_partial_create), $PHP_SRC_PATH/Zend/zend_vm_def.h
//! (ZEND_CALLABLE_CONVERT_PARTIAL, ZEND_SEND_PLACEHOLDER)

use crate::callable::{ClosureData, ClosureFunc, closure_data};
use crate::{Closure, Context};
use p8_bytecode::{Function, PartialSite};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::frozen::Frozen;
use p8_runtime::object::Class;
use p8_runtime::string::PhpStr;
use p8_runtime::types::TypeDecl;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// A compiled partial application's closure code, by (the code making it,
/// its site there, the function applied, whether it is applied to an
/// object): what zend_accel_pfa_cache_get keys on. The Arcs keep the
/// addresses in the key from being reused.
pub(crate) struct Compiled {
    pub(crate) func: Arc<Function>,
    _site: Arc<Function>,
    _applied: Option<Arc<Function>>,
}

pub(crate) type CacheKey = (usize, u16, usize, bool);

/// A parameter of the function applied, as zend_partial.c reads its
/// arg_info.
struct Param {
    name: Vec<u8>,
    by_ref: bool,
    ty: Option<TypeDecl>,
    /// None: required. Some(None): optional, its default not known (a
    /// builtin whose stub gives none, an expression p8 cannot write).
    default: Option<Option<Vec<u8>>>,
    /// `#[\SensitiveParameter]` (inherited).
    sensitive: bool,
}

/// How the closure calls the function (zp_compile_forwarding_call).
enum Call {
    /// `\name(…)`.
    Function(Vec<u8>),
    /// `$this->name(…)`.
    Method(Vec<u8>),
    /// `static::name(…)`.
    Static(Vec<u8>),
    /// `$fn(…)`: the function is a Closure, bound as the first `use`.
    Closure,
    /// `\call_user_func('assert', …)` (assert() would show the generated
    /// code in its message).
    Assert,
}

/// The function applied, as zend_partial.c reads its zend_function.
struct Applied {
    /// get_function_or_method_name: `f`, `C::m`.
    display: String,
    params: Vec<Param>,
    variadic: Option<Param>,
    required: usize,
    ret: Option<TypeDecl>,
    returns_ref: bool,
    no_discard: bool,
    call: Call,
    /// The namespace the parameters' defaults are written in.
    ns: Option<Vec<u8>>,
    /// Its code (user code): what the cache keys on.
    code: Option<Arc<Function>>,
    /// The key for a builtin.
    builtin_key: usize,
    /// Never cached (ZEND_ACC_NEVER_CACHE).
    never_cache: bool,
    /// Called through a trampoline (__call, __callStatic,
    /// Closure::__invoke): the closure takes the caller's scope, not the
    /// function's (ZEND_ACC_CALL_VIA_TRAMPOLINE).
    trampoline: bool,
}

/// One argument slot as the partial application's call frame has it.
#[derive(Clone)]
enum Slot {
    /// Not passed (a named argument skipped it).
    Undef,
    Placeholder,
    Value(Value),
}

fn text(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

/// A constant's value as PHP source (a parameter's default).
fn frozen_src(f: &Frozen, out: &mut Vec<u8>) {
    match f {
        Frozen::Null => out.extend_from_slice(b"null"),
        Frozen::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Frozen::Int(i) if *i == i64::MIN => out.extend_from_slice(b"\\PHP_INT_MIN"),
        Frozen::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Frozen::Float(x) if x.is_nan() => out.extend_from_slice(b"\\NAN"),
        Frozen::Float(x) if x.is_infinite() => out.extend_from_slice(if *x > 0.0 { b"\\INF" } else { b"-\\INF" }),
        Frozen::Float(x) => out.extend_from_slice(format!("{x:?}").as_bytes()),
        Frozen::Str(s) => {
            out.push(b'\'');
            for &b in s.as_bytes() {
                if b == b'\'' || b == b'\\' {
                    out.push(b'\\');
                }
                out.push(b);
            }
            out.push(b'\'');
        }
        Frozen::Array(entries) => {
            out.push(b'[');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(b", ");
                }
                if let Some(k) = k {
                    frozen_src(k, out);
                    out.extend_from_slice(b" => ");
                }
                frozen_src(v, out);
            }
            out.push(b']');
        }
    }
}

/// A declared type as PHP source (zp_type_to_ast): class names fully
/// qualified, `self` and `parent` as they are.
fn type_src(ty: &TypeDecl) -> String {
    let names: Vec<String> = ty
        .classes
        .iter()
        .map(|c| {
            let c = text(c.as_bytes());
            if c.eq_ignore_ascii_case("self") || c.eq_ignore_ascii_case("parent") { c } else { format!("\\{c}") }
        })
        .collect();
    if ty.intersection {
        let joined = names.join("&");
        return p8_runtime::stubs::type_to_string(ty.mask, &[joined.as_str()]);
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    p8_runtime::stubs::type_to_string(ty.mask, &refs)
}

/// Whether a type admits void (a function returning nothing: the
/// forwarding call is not returned).
fn is_void(ty: &Option<TypeDecl>) -> bool {
    ty.as_ref().is_some_and(|t| t.mask & p8_runtime::types::T_VOID != 0)
}

impl Context<'_> {
    /// Rare::Partial: the Closure of a partial application of `callee`
    /// with `values` (one per argument written; placeholders' unused),
    /// made by `code`'s site `index` (zend_partial_create).
    pub(crate) fn partial(&mut self, code: &Arc<Function>, index: u16, callee: Value, values: Vec<Value>, strict: bool) -> Result<Value, Throw> {
        let site = &code.partials[index as usize];
        // A Closure is applied as it is; another callable as the closure
        // `f(...)` would make of it (no object made: only the partial
        // application's own is).
        let target = match callee.deref() {
            Value::Object(o) if closure_data(&o).is_some() => Some(o),
            _ => None,
        };
        let is_closure = target.is_some();
        let resolved = match target {
            Some(_) => None,
            None => Some(self.callable_data(&callee)?),
        };
        let Some(data) = target.as_deref().and_then(closure_data).or(resolved.as_ref()) else { return Ok(Value::Null) };
        let (this, scope, called) = (data.this.clone(), data.scope.clone(), data.called.clone());
        let applied = self.applied(data, is_closure, this.is_some());
        let (mut argv, extra, named_positions) = self.send_args(site, &applied, values)?;
        // zend_partial_create: the object (none for a static closure), the
        // called scope, and the scope: the function's (its constant
        // expressions' self:: and parent:: mean what they mean there);
        // through a trampoline, the caller's. An object bound with no
        // scope gets Closure's (zend_create_closure_ex).
        let is_static_callee = data.func.user().is_some() && is_closure && data.is_static;
        let object = if is_static_callee { None } else { this.clone() };
        let called = called.or_else(|| object.as_ref().map(|o| Rc::clone(&o.class)));
        let scope = if applied.trampoline { self.frames.last().and_then(|f| f.scope.clone()) } else { scope };
        let scope = match scope {
            None if object.is_some() => Some(Rc::clone(&self.classes[&self.isolate.known.closure])),
            s => s,
        };
        // A static closure unless applied to an object, or to a closure
        // that is not static.
        let is_static = this.is_none() && (data.is_static || !matches!(applied.call, Call::Closure));
        let key: CacheKey = (Arc::as_ptr(code) as usize, index, applied.code.as_ref().map_or(applied.builtin_key, |c| Arc::as_ptr(c) as usize), this.is_some());
        let cached = if applied.never_cache { None } else { self.isolate.partials.borrow().get(&key).map(|c| Arc::clone(&c.func)) };
        let func = match cached {
            Some(f) => {
                if site.variadic {
                    let n = applied.params.len();
                    if argv.len() < n {
                        argv.resize(n, Slot::Undef);
                    }
                    for s in argv.iter_mut() {
                        if matches!(s, Slot::Undef) {
                            *s = Slot::Placeholder;
                        }
                    }
                }
                f
            }
            None => {
                let class = scope.as_ref().map(|c| c.name.as_bytes().to_vec());
                let f = self.compile_partial(site, &applied, &mut argv, extra.as_ref(), &named_positions, is_static, class.as_deref(), &code.file)?;
                if !applied.never_cache {
                    let compiled = Compiled { func: Arc::clone(&f), _site: Arc::clone(code), _applied: applied.code.clone() };
                    let mut partials = self.isolate.partials.borrow_mut();
                    // (Code an isolate compiles again and again, eval's,
                    // would grow this without end: past a bound it starts over.)
                    if partials.len() >= 1 << 14 {
                        partials.clear();
                    }
                    partials.insert(key, compiled);
                }
                f
            }
        };
        // zp_bind: the inner closure, the bound arguments (their types
        // checked as the code making the closure would pass them), the
        // extra named arguments.
        let mut captured = Vec::new();
        if matches!(applied.call, Call::Closure)
            && let Some(target) = &target
        {
            captured.push(Value::Object(Rc::clone(target)));
        }
        for (offset, slot) in argv.iter().enumerate() {
            let Slot::Value(v) = slot else { continue };
            let param = applied.params.get(offset).or(applied.variadic.as_ref());
            let mut v = v.clone();
            // A value (not a variable) for a parameter taking a reference
            // (ZEND_SEND_VAL_EX: zend_cannot_pass_by_reference).
            if param.is_some_and(|p| p.by_ref) && !matches!(v, Value::Ref(_)) {
                return Err(self.cannot_pass_by_ref(&applied, offset));
            }
            if let Some(ty) = param.and_then(|p| p.ty.as_ref())
                && !matches!(v, Value::Ref(_))
            {
                match self.check_type(ty, &v, strict)? {
                    Ok(Some(converted)) => v = converted,
                    Ok(None) => {}
                    Err(()) => {
                        let name = param.map_or_else(String::new, |p| text(&p.name));
                        return Err(self.throw(
                            "TypeError",
                            format!("{}(): Argument #{} (${name}) must be of type {}, {} given", applied.display, offset + 1, ty.shown_resolved(), p8_runtime::args::value_name(&v)),
                        ));
                    }
                }
            }
            captured.push(v);
        }
        if let Some(extra) = extra {
            captured.push(Value::Array(Rc::new(extra)));
        }
        let closure = {
            let key = Arc::as_ptr(&func) as usize;
            match self.lambdas.get(&key) {
                Some(c) => Rc::clone(c),
                None => {
                    let c = Closure::new(Arc::clone(&func), self.isolate);
                    self.lambdas.insert(key, Rc::clone(&c));
                    c
                }
            }
        };
        let data = ClosureData { func: ClosureFunc::User(closure), is_static, this: object, scope, called, captured: RefCell::new(captured), trampoline: None, fake: true };
        let class = Rc::clone(&self.classes[&self.isolate.known.closure]);
        Ok(Value::Object(p8_runtime::object::Object::with_data(class, &self.ids, Some(Rc::new(data)))))
    }

    /// The function a closure of the callable runs, as zend_partial.c
    /// reads it.
    fn applied(&self, data: &ClosureData, is_closure: bool, has_this: bool) -> Applied {
        let class_name = |c: &Option<Rc<Class>>| c.as_ref().map(|c| text(c.name.as_bytes()));
        match &data.func {
            ClosureFunc::User(clo) if data.trampoline.is_some() => {
                // A __call / __callStatic trampoline: `mixed ...$arguments`
                // (zend_get_call_trampoline_func).
                let name = data.trampoline.clone().unwrap_or_else(PhpStr::empty).as_bytes().to_vec();
                let class = class_name(&data.scope).or_else(|| clo.code.class.as_ref().map(|c| text(c.as_bytes()))).unwrap_or_default();
                Applied {
                    display: format!("{class}::{}", text(&name)),
                    params: Vec::new(),
                    variadic: Some(Param { name: b"arguments".to_vec(), by_ref: false, ty: Some(TypeDecl::new(p8_runtime::types::T_MIXED, Vec::new(), Vec::new(), false)), default: None, sensitive: false }),
                    required: 0,
                    ret: None,
                    returns_ref: false,
                    no_discard: false,
                    call: if has_this { Call::Method(name) } else { Call::Static(name) },
                    ns: None,
                    code: None,
                    builtin_key: 0,
                    never_cache: true,
                    trampoline: true,
                }
            }
            ClosureFunc::User(clo) => {
                let code = &clo.code;
                let mut params = Vec::new();
                let mut variadic = None;
                let mut ns = None;
                for p in &code.params {
                    let default = if !p.has_default {
                        None
                    } else if let Some(d) = &p.default {
                        let mut out = Vec::new();
                        frozen_src(d, &mut out);
                        Some(Some(out))
                    } else if let Some(src) = &p.default_src {
                        if let Some(n) = &p.default_ns {
                            ns = Some(n.as_bytes().to_vec());
                        }
                        Some(Some(src.as_bytes().to_vec()))
                    } else {
                        Some(None)
                    };
                    let param = Param {
                        name: p.name.as_bytes().to_vec(),
                        by_ref: p.by_ref,
                        ty: p.ty.clone(),
                        default,
                        sensitive: p.attrs.iter().any(|a| a.name.as_bytes().eq_ignore_ascii_case(b"SensitiveParameter")),
                    };
                    if p.variadic {
                        variadic = Some(param);
                    } else {
                        params.push(param);
                    }
                }
                let name = code.name.as_bytes().to_vec();
                let display = match &code.class {
                    Some(c) if !name.starts_with(b"{closure") => format!("{}::{}", text(c.as_bytes()), text(&name)),
                    _ => text(&name),
                };
                let call = if is_closure {
                    Call::Closure
                } else if code.class.is_some() {
                    if has_this && !code.is_static { Call::Method(name) } else { Call::Static(name) }
                } else if name.eq_ignore_ascii_case(b"assert") {
                    Call::Assert
                } else {
                    Call::Function(name)
                };
                Applied {
                    display,
                    params,
                    variadic,
                    required: code.required,
                    ret: code.ret_type.clone(),
                    returns_ref: code.returns_ref,
                    no_discard: code.meta.attrs.iter().any(|a| a.name.as_bytes().eq_ignore_ascii_case(b"NoDiscard")),
                    call,
                    ns,
                    code: Some(Arc::clone(code)),
                    builtin_key: 0,
                    never_cache: false,
                    trampoline: false,
                }
            }
            ClosureFunc::Builtin(i) => {
                let b = self.isolate.registry.function(*i);
                let call = if is_closure {
                    Call::Closure
                } else if b.name.eq_ignore_ascii_case("assert") {
                    Call::Assert
                } else {
                    Call::Function(b.name.as_bytes().to_vec())
                };
                self.stub_applied(b.name, b.name.to_string(), call, *i * 2 + 1)
            }
            ClosureFunc::Method(_, name)
                if name.as_bytes().rsplit(|&b| b == b':').next().is_some_and(|m| m.eq_ignore_ascii_case(b"__invoke"))
                    && let Some(inner) = data.this.as_deref().and_then(closure_data) =>
            {
                // `$closure->__invoke(?)`: the closure's own parameters,
                // their defaults not known (zend_get_closure_invoke_method:
                // ZEND_ACC_USER_ARG_INFO), never cached.
                let mut applied = self.applied(inner, true, true);
                for p in &mut applied.params {
                    if p.default.is_some() {
                        p.default = Some(None);
                    }
                }
                applied.display = "Closure::__invoke".into();
                applied.call = Call::Method(b"__invoke".to_vec());
                applied.ns = None;
                applied.code = None;
                applied.never_cache = true;
                applied.trampoline = true;
                applied
            }
            ClosureFunc::Method(native, name) => {
                let class = class_name(&data.scope).unwrap_or_default();
                let method = name.as_bytes().rsplit(|&b| b == b':').next().unwrap_or_default().to_vec();
                let stub = format!("{class}::{}", text(&method));
                let call = if is_closure {
                    Call::Closure
                } else if has_this {
                    Call::Method(method)
                } else {
                    Call::Static(method)
                };
                let _ = native;
                // Keyed by its stub's name (a builtin method's code has no address).
                let key = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    stub.to_ascii_lowercase().hash(&mut h);
                    (h.finish() as usize) | 1
                };
                self.stub_applied(&stub, stub.clone(), call, key)
            }
        }
    }

    /// A builtin as its stub declares it.
    fn stub_applied(&self, stub: &str, display: String, call: Call, key: usize) -> Applied {
        let sig = p8_runtime::stubs::lookup(stub.as_bytes());
        let mut params = Vec::new();
        let mut variadic = None;
        for p in sig.map_or(&[][..], |s| s.params) {
            let param = Param {
                name: p.name.as_bytes().to_vec(),
                by_ref: p.by_ref,
                ty: (p.mask != 0 || !p.classes.is_empty()).then(|| {
                    let names: Vec<p8_runtime::frozen::Text> = p.classes.iter().map(|c| p8_runtime::frozen::Text::from(*c)).collect();
                    TypeDecl::new(p.mask, names.clone(), names, false)
                }),
                // A stub's UNKNOWN: optional, its default not known.
                default: if p.optional() && !p.variadic { Some(p.default.filter(|d| *d != "UNKNOWN").map(|d| d.as_bytes().to_vec())) } else { None },
                sensitive: false,
            };
            if p.variadic {
                variadic = Some(param);
            } else {
                params.push(param);
            }
        }
        let required = params.iter().rposition(|p| p.default.is_none()).map_or(0, |i| i + 1);
        Applied {
            display,
            params,
            variadic,
            required,
            ret: sig.filter(|s| !s.tentative).and_then(|s| {
                (s.ret_mask != 0 || !s.ret_classes.is_empty()).then(|| {
                    let names: Vec<p8_runtime::frozen::Text> = s.ret_classes.iter().map(|c| p8_runtime::frozen::Text::from(*c)).collect();
                    TypeDecl::new(s.ret_mask, names.clone(), names, false)
                })
            }),
            returns_ref: sig.is_some_and(|s| s.returns_ref),
            no_discard: false,
            call,
            ns: None,
            code: None,
            builtin_key: key,
            never_cache: false,
            trampoline: false,
        }
    }

    /// The arguments as the call frame has them after their SEND opcodes:
    /// by position (a named one where its parameter is), the extra named
    /// ones of a variadic function, and the named placeholders' order
    /// (zend_handle_named_arg's errors).
    #[allow(clippy::type_complexity)] // (frame, extra named arguments, named placeholders' order), as ZEND_CALLABLE_CONVERT_PARTIAL reads them
    fn send_args(&mut self, site: &PartialSite, applied: &Applied, values: Vec<Value>) -> Result<(Vec<Slot>, Option<PhpArray>, Vec<Vec<u8>>), Throw> {
        let mut argv: Vec<Slot> = Vec::new();
        let mut extra: Option<PhpArray> = None;
        let mut named_positions = Vec::new();
        let mut positional = 0;
        for (arg, v) in site.args.iter().zip(values) {
            let slot = if arg.placeholder { Slot::Placeholder } else { Slot::Value(v) };
            let Some(name) = &arg.name else {
                argv.push(slot);
                positional += 1;
                continue;
            };
            let name = name.as_bytes();
            match applied.params.iter().position(|p| p.name == name) {
                Some(at) => {
                    if argv.len() <= at {
                        argv.resize(at + 1, Slot::Undef);
                    }
                    match &argv[at] {
                        Slot::Undef => {}
                        Slot::Placeholder => return Err(self.throw("Error", format!("Named parameter ${} overwrites previous placeholder", text(name)))),
                        Slot::Value(_) => return Err(self.throw("Error", format!("Named parameter ${} overwrites previous argument", text(name)))),
                    }
                    if matches!(slot, Slot::Placeholder) {
                        named_positions.push(name.to_vec());
                    }
                    argv[at] = slot;
                }
                None if applied.variadic.is_some() => {
                    let extra = extra.get_or_insert_with(PhpArray::new);
                    let key = Key::from_string(PhpStr::from(name));
                    if extra.get(&key).is_some() {
                        return Err(self.throw("Error", format!("Named parameter ${} overwrites previous argument", text(name))));
                    }
                    let v = match slot {
                        // A placeholder for no parameter: zp_args_check's error.
                        Slot::Placeholder => return Err(self.throw("Error", format!("Cannot use named placeholder for unknown or variadic parameter ${}", text(name)))),
                        Slot::Value(v) if applied.variadic.as_ref().is_some_and(|p| p.by_ref) => {
                            if !matches!(v, Value::Ref(_)) {
                                return Err(self.cannot_pass_by_ref(applied, applied.params.len()));
                            }
                            v
                        }
                        Slot::Value(v) => v.deref(),
                        Slot::Undef => Value::Null,
                    };
                    extra.insert(key, v);
                }
                None => return Err(self.throw("Error", format!("Unknown named parameter ${}", text(name)))),
            }
        }
        let _ = positional;
        Ok((argv, extra, named_positions))
    }

    /// zp_args_check, then zp_compile: the closure's code written as PHP
    /// and compiled. Leaves `argv` with the implicit placeholders `...`
    /// adds.
    #[allow(clippy::too_many_arguments)] // zp_compile's own parameters: the site, the function, the frame
    fn compile_partial(
        &mut self,
        site: &PartialSite,
        applied: &Applied,
        argv: &mut Vec<Slot>,
        extra: Option<&PhpArray>,
        named_positions: &[Vec<u8>],
        is_static_closure: bool,
        class: Option<&[u8]>,
        file: &str,
    ) -> Result<Arc<Function>, Throw> {
        let num_args = applied.params.len();
        let argc = argv.len();
        let variadic_fn = applied.variadic.is_some();
        // zp_args_check
        if argc < applied.required && !site.variadic {
            let limit = if num_args <= applied.required { "exactly" } else { "at least" };
            return Err(self.throw("ArgumentCountError", format!("Partial application of {}() expects {limit} {} arguments, {argc} given", applied.display, applied.required)));
        }
        if argc > num_args && !variadic_fn {
            return Err(self.throw("ArgumentCountError", format!("Partial application of {}() expects at most {num_args} arguments, {argc} given", applied.display)));
        }
        // Implicit placeholders for the parameters `...` leaves out.
        let new_argc = if site.variadic { argc.max(num_args) } else { argc };
        argv.resize(new_argc, Slot::Undef);
        // Parameter order: positional placeholders, then named ones in the
        // order written, then the implicit ones.
        let mut param_of = vec![usize::MAX; new_argc];
        let mut num_params = 0;
        let mut num_positional = 0;
        for offset in 0..argc {
            if !matches!(argv[offset], Slot::Placeholder) {
                continue;
            }
            num_params += 1;
            let name = &self.param_at(applied, offset).name;
            param_of[offset] = match named_positions.iter().position(|n| n == name) {
                Some(at) => num_positional + at,
                None => {
                    num_positional += 1;
                    num_positional - 1
                }
            };
        }
        // Every explicit placeholder is required.
        let mut num_required = num_params;
        if site.variadic {
            for offset in 0..new_argc {
                if offset < argc && !matches!(argv[offset], Slot::Undef) {
                    continue;
                }
                argv[offset] = Slot::Placeholder;
                num_params += 1;
                param_of[offset] = num_params - 1;
                // zp_compute_num_required
                if offset < num_args {
                    if offset < applied.required {
                        num_required = num_required.max(num_params);
                    }
                } else {
                    num_required = num_params;
                }
            }
        }
        let names = assign_names(argv, applied, site.variadic, extra.is_some());
        // The parameters, in their order.
        let mut params: Vec<Option<Vec<u8>>> = vec![None; num_params];
        let mut uses: Vec<Vec<u8>> = Vec::new();
        if matches!(applied.call, Call::Closure) {
            uses.push([b"$".as_slice(), &names.inner_closure].concat());
        }
        for offset in 0..new_argc {
            match &argv[offset] {
                Slot::Placeholder => {
                    let p = self.param_at(applied, offset);
                    let at = param_of[offset];
                    let mut src = Vec::new();
                    if p.sensitive {
                        src.extend_from_slice(b"#[\\SensitiveParameter] ");
                    }
                    if let Some(ty) = &p.ty {
                        src.extend_from_slice(type_src(ty).as_bytes());
                        src.push(b' ');
                    }
                    if p.by_ref {
                        src.push(b'&');
                    }
                    src.push(b'$');
                    src.extend_from_slice(&names.params[offset]);
                    if at >= num_required {
                        src.extend_from_slice(b" = ");
                        src.extend_from_slice(&self.default_of(applied, offset)?);
                    }
                    params[at] = Some(src);
                }
                Slot::Value(_) => {
                    let p = self.param_at(applied, offset);
                    let amp: &[u8] = if p.by_ref { b"&" } else { b"" };
                    uses.push([amp, b"$", &names.params[offset]].concat());
                }
                Slot::Undef => {}
            }
        }
        if extra.is_some() {
            uses.push([b"$".as_slice(), &names.extra_named_params].concat());
        }
        let mut params: Vec<Vec<u8>> = params.into_iter().flatten().collect();
        if site.variadic
            && let Some(v) = &applied.variadic
        {
            let mut src = Vec::new();
            if v.sensitive {
                src.extend_from_slice(b"#[\\SensitiveParameter] ");
            }
            if let Some(ty) = &v.ty {
                src.extend_from_slice(type_src(ty).as_bytes());
                src.push(b' ');
            }
            if v.by_ref {
                src.push(b'&');
            }
            src.extend_from_slice(b"...$");
            src.extend_from_slice(&names.variadic_param);
            params.push(src);
        }
        // The body: the forwarding call; with `...` and a function that is
        // not variadic, superfluous arguments are forwarded too (only when
        // there are some: func_num_args() is one instruction).
        let body = if site.variadic && !variadic_fn {
            let plain = self.forwarding_call(applied, argv, &names, site.variadic, num_params, false, extra.is_some())?;
            let forwarding = self.forwarding_call(applied, argv, &names, site.variadic, num_params, true, extra.is_some())?;
            format!("if (\\func_num_args() <= {num_params}) {{ {plain} }} else {{ {forwarding} }}").into_bytes()
        } else {
            self.forwarding_call(applied, argv, &names, site.variadic, num_params, false, extra.is_some())?.into_bytes()
        };
        let mut src = Vec::new();
        // On the line the partial application is written on.
        src.extend_from_slice(b"<?php");
        src.resize(src.len() + site.line.max(1) as usize - 1, b'\n');
        src.push(b' ');
        if let Some(ns) = &applied.ns {
            src.extend_from_slice(b"namespace ");
            src.extend_from_slice(ns);
            src.extend_from_slice(b"; ");
        }
        // An expression (`function` starting a statement declares one).
        src.extend_from_slice(b"return ");
        if applied.no_discard {
            src.extend_from_slice(b"#[\\NoDiscard] ");
        }
        if is_static_closure {
            src.extend_from_slice(b"static ");
        }
        src.extend_from_slice(b"function ");
        if applied.returns_ref {
            src.push(b'&');
        }
        src.push(b'(');
        src.extend_from_slice(&params.join(b", ".as_slice()));
        src.push(b')');
        if !uses.is_empty() {
            src.extend_from_slice(b" use (");
            src.extend_from_slice(&uses.join(b", ".as_slice()));
            src.push(b')');
        }
        if let Some(ret) = &applied.ret {
            src.extend_from_slice(b": ");
            src.extend_from_slice(type_src(ret).as_bytes());
        }
        src.extend_from_slice(b" { ");
        src.extend_from_slice(&body);
        src.extend_from_slice(b" };");
        let Some(loader) = self.loader.clone() else {
            return Err(self.throw("Error", String::from("Partial application needs a compiler")));
        };
        match loader.load_partial(&src, file, site.name.as_bytes(), class, is_static_closure) {
            Ok(f) => Ok(f),
            Err(e) => Err(Throw::Fatal { message: e.message, file: Rc::from(file), line: site.line }),
        }
    }

    /// zend_cannot_pass_by_reference: the parameter named when it is a
    /// declared one.
    fn cannot_pass_by_ref(&self, applied: &Applied, offset: usize) -> Throw {
        let name = applied.params.get(offset).map_or_else(String::new, |p| format!(" (${})", text(&p.name)));
        self.throw("Error", format!("{}(): Argument #{}{name} could not be passed by reference", applied.display, offset + 1))
    }

    /// The parameter an argument slot binds to (a variadic one past the
    /// declared ones).
    fn param_at<'p>(&self, applied: &'p Applied, offset: usize) -> &'p Param {
        applied.params.get(offset).or(applied.variadic.as_ref()).unwrap_or_else(|| &applied.params[applied.params.len() - 1])
    }

    /// A parameter's default as PHP source (zp_get_param_default_value).
    fn default_of(&self, applied: &Applied, offset: usize) -> Result<Vec<u8>, Throw> {
        let p = &applied.params[offset];
        match &p.default {
            Some(Some(src)) => Ok(src.clone()),
            _ => Err(self.throw(
                "ArgumentCountError",
                format!("{}(): Argument #{} (${}) must be passed explicitly, because the default value is not known", applied.display, offset + 1, text(&p.name)),
            )),
        }
    }

    /// zp_compile_forwarding_call: the statement calling the function.
    #[allow(clippy::too_many_arguments)] // zp_compile_forwarding_call's own parameters
    fn forwarding_call(&self, applied: &Applied, argv: &[Slot], names: &Names, variadic: bool, num_params: usize, superfluous: bool, extra: bool) -> Result<String, Throw> {
        let mut args: Vec<String> = Vec::new();
        if matches!(applied.call, Call::Assert) {
            args.push("'assert'".into());
        }
        for (offset, slot) in argv.iter().enumerate() {
            match slot {
                Slot::Undef => {
                    if offset < applied.required {
                        return Err(self.throw("ArgumentCountError", format!("{}(): Argument #{} (${}) not passed", applied.display, offset + 1, text(&applied.params[offset].name))));
                    }
                    args.push(text(&self.default_of(applied, offset)?));
                }
                _ => args.push(format!("${}", text(&names.params[offset]))),
            }
        }
        if extra {
            args.push(format!("...${}", text(&names.extra_named_params)));
        }
        if variadic {
            if applied.variadic.is_some() {
                args.push(format!("...${}", text(&names.variadic_param)));
            } else if superfluous {
                args.push(format!("...\\array_slice(\\func_get_args(), {num_params})"));
            }
        }
        let args = args.join(", ");
        let call = match &applied.call {
            Call::Assert => format!("\\call_user_func({args})"),
            Call::Closure => format!("${}({args})", text(&names.inner_closure)),
            Call::Method(m) => format!("$this->{}({args})", text(m)),
            Call::Static(m) => format!("static::{}({args})", text(m)),
            Call::Function(f) => format!("\\{}({args})", text(f)),
        };
        Ok(if is_void(&applied.ret) { format!("{call};") } else { format!("return {call};") })
    }
}

/// The names of the closure's variables (zp_names).
struct Names {
    params: Vec<Vec<u8>>,
    variadic_param: Vec<u8>,
    extra_named_params: Vec<u8>,
    inner_closure: Vec<u8>,
}

/// zp_assign_names: a name for every variable of the closure: the
/// placeholders' (their parameters' names, never renamed), the variadic
/// parameter's, placeholders running into it (`$args0`, `$args1`…), the
/// bound arguments' (renamed on a clash: `$a2`), `$extra_named_params`
/// and `$fn`.
fn assign_names(argv: &[Slot], applied: &Applied, variadic: bool, extra: bool) -> Names {
    let argc = argv.len();
    let num_args = applied.params.len();
    let param_name = |offset: usize| -> Vec<u8> { applied.params.get(offset).or(applied.variadic.as_ref()).map_or_else(Vec::new, |p| p.name.clone()) };
    let mut names = Names { params: vec![Vec::new(); argc], variadic_param: Vec::new(), extra_named_params: Vec::new(), inner_closure: Vec::new() };
    let exists = |names: &Names, name: &[u8]| -> bool {
        names.params.iter().any(|p| !p.is_empty() && p.as_slice() == name)
            || names.variadic_param.as_slice() == name
            || names.extra_named_params.as_slice() == name
            || names.inner_closure.as_slice() == name
    };
    for (offset, slot) in argv.iter().enumerate().take(num_args) {
        if matches!(slot, Slot::Placeholder) {
            names.params[offset] = param_name(offset);
        }
    }
    if variadic && applied.variadic.is_some() {
        names.variadic_param = param_name(num_args);
    }
    for (offset, slot) in argv.iter().enumerate().skip(num_args) {
        if !matches!(slot, Slot::Placeholder) {
            continue;
        }
        let orig = param_name(num_args);
        let mut n = 0;
        let name = loop {
            let candidate = [orig.as_slice(), n.to_string().as_bytes()].concat();
            if !exists(&names, &candidate) {
                break candidate;
            }
            n += 1;
        };
        names.params[offset] = name;
    }
    for (offset, slot) in argv.iter().enumerate() {
        if !matches!(slot, Slot::Value(_)) {
            continue;
        }
        let orig = param_name(offset.min(num_args));
        let mut name = orig.clone();
        let mut n = 2;
        while exists(&names, &name) {
            name = [orig.as_slice(), n.to_string().as_bytes()].concat();
            n += 1;
        }
        names.params[offset] = name;
    }
    let fresh = |names: &Names, base: &[u8]| -> Vec<u8> {
        let mut name = base.to_vec();
        let mut n = 2;
        while exists(names, &name) {
            name = [base, n.to_string().as_bytes()].concat();
            n += 1;
        }
        name
    };
    if extra {
        names.extra_named_params = fresh(&names, b"extra_named_params");
    }
    if matches!(applied.call, Call::Closure) {
        names.inner_closure = fresh(&names, b"fn");
    }
    names
}

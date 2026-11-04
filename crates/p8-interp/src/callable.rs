//! Closures and callables: a Closure object's function and bindings, what
//! a callable value (a closure, a function name, `[$obj, 'method']`,
//! `"Class::method"`, an invokable object) calls, and calls of PHP code
//! from a builtin (array_map, usort).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_closures.c,
//! $PHP_SRC_PATH/Zend/zend_API.c (zend_is_callable_ex, zend_call_function)

use crate::classes::{Code, MethodFrame};
use crate::{Closure, Context, Next};
use p8_bytecode::Function;
use p8_runtime::ext::Throw;
use p8_runtime::object::{Class, Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// What a Closure object holds.
pub(crate) struct ClosureData {
    pub func: ClosureFunc,
    pub is_static: bool,
    pub this: Option<Rc<Object>>,
    pub scope: Option<Rc<Class>>,
    pub called: Option<Rc<Class>>,
    /// `use` values (references for `&$x`), in the function's `bound` order.
    pub captured: RefCell<Vec<Value>>,
    /// A Closure of a __call / __callStatic trampoline: the method name.
    pub trampoline: Option<PhpStr>,
}

/// What a Closure runs.
pub(crate) enum ClosureFunc {
    User(Rc<Closure>),
    /// A builtin function (its index), from its name.
    Builtin(usize),
    /// A builtin class's method (`$doc->createElement(...)`), as its
    /// MethodFrame holds it.
    Method(Native, PhpStr),
}

impl ClosureFunc {
    /// The user code it runs, if it is user code.
    pub(crate) fn user(&self) -> Option<&Rc<Closure>> {
        match self {
            ClosureFunc::User(c) => Some(c),
            _ => None,
        }
    }
}

impl Clone for ClosureFunc {
    fn clone(&self) -> ClosureFunc {
        match self {
            ClosureFunc::User(c) => ClosureFunc::User(Rc::clone(c)),
            ClosureFunc::Builtin(i) => ClosureFunc::Builtin(*i),
            ClosureFunc::Method(n, name) => ClosureFunc::Method(*n, name.clone()),
        }
    }
}

/// The closure data of a Closure object.
pub(crate) fn closure_data(o: &Object) -> Option<&ClosureData> {
    o.data.as_ref().and_then(|d| d.0.downcast_ref::<ClosureData>())
}

/// What a callable value calls.
pub(crate) enum Target {
    Builtin(usize),
    Frame(MethodFrame),
}

/// A parameter as named arguments see it: its name, whether it may be
/// left out, whether it is variadic, and a builtin's default (as the stub
/// writes it).
struct NamedParam {
    name: String,
    optional: bool,
    variadic: bool,
    default: Option<&'static str>,
}

/// The native stack nested PHP calls may use (the CLI's interpreter
/// thread has 64 MiB).
const STACK_LIMIT: usize = 48 * 1024 * 1024;

fn text(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

impl Context<'_> {
    /// MakeClosure: a Closure of `func`, bound to the running frame's
    /// `$this` (unless static) and class.
    pub(crate) fn make_closure(&mut self, func: &Arc<Function>, is_static: bool) -> Rc<Object> {
        let key = Arc::as_ptr(func) as usize;
        let closure = match self.lambdas.get(&key) {
            Some(c) => Rc::clone(c),
            None => {
                let c = Closure::new(Arc::clone(func));
                self.lambdas.insert(key, Rc::clone(&c));
                c
            }
        };
        let (this, scope, called) = match self.frames.last() {
            Some(f) => (if is_static { None } else { f.this.clone() }, f.scope.clone(), f.called.clone()),
            None => (None, None, None),
        };
        let data = ClosureData { func: ClosureFunc::User(closure), is_static, this, scope, called, captured: RefCell::new(Vec::new()), trampoline: None };
        let class = Rc::clone(&self.classes[&self.isolate.known.closure]);
        Object::with_data(class, &self.ids, Some(Rc::new(data)))
    }

    /// What calling a Closure object calls.
    fn closure_target(&self, o: &Object) -> Option<Target> {
        let data = closure_data(o)?;
        Some(match &data.func {
            ClosureFunc::User(func) => Target::Frame(MethodFrame {
                func: Code::User(Rc::clone(func)),
                this: data.this.clone(),
                scope: data.scope.clone(),
                called: data.called.clone(),
                captured: data.captured.borrow().clone(),
                trampoline: data.trampoline.clone(),
            }),
            ClosureFunc::Method(native, name) => Target::Frame(MethodFrame {
                func: Code::Native(*native, name.clone()),
                this: data.this.clone(),
                scope: data.scope.clone(),
                called: data.called.clone(),
                captured: Vec::new(),
                trampoline: None,
            }),
            ClosureFunc::Builtin(builtin) => Target::Builtin(*builtin),
        })
    }

    fn new_closure(&self, data: ClosureData) -> Value {
        let class = Rc::clone(&self.classes[&self.isolate.known.closure]);
        Value::Object(Object::with_data(class, &self.ids, Some(Rc::new(data))))
    }

    /// Closure::bind / bindTo: a copy bound to `new_this` and the scope
    /// `scope` names ("static" or none: its own).
    pub(crate) fn bind(&mut self, closure: &Rc<Object>, new_this: Value, scope: Option<Value>) -> Result<Value, Throw> {
        let Some(data) = closure_data(closure) else { return Ok(Value::Null) };
        let this = match new_this {
            Value::Object(o) => Some(o),
            _ => None,
        };
        if this.is_some() && data.is_static {
            self.warn("Cannot bind an instance to a static closure, this will be an error in PHP 9");
            return Ok(Value::Null);
        }
        let scope = match scope {
            None => data.scope.clone(),
            Some(Value::Object(o)) => Some(Rc::clone(&o.class)),
            Some(Value::Null) => None,
            Some(v) => {
                let name = v.to_php_string().unwrap_or_else(PhpStr::empty);
                if name.as_bytes() == b"static" {
                    data.scope.clone()
                } else {
                    match self.class_named(name.as_bytes()) {
                        Some(c) => Some(Rc::clone(c)),
                        None => {
                            self.warn(format!("Class \"{}\" not found", text(name.as_bytes())));
                            return Ok(Value::Null);
                        }
                    }
                }
            }
        };
        let called = this.as_ref().map(|t| Rc::clone(&t.class)).or_else(|| scope.clone());
        let copy = ClosureData {
            func: data.func.clone(),
            is_static: data.is_static,
            this,
            scope,
            called,
            captured: RefCell::new(data.captured.borrow().clone()),
            trampoline: data.trampoline.clone(),
        };
        Ok(self.new_closure(copy))
    }

    /// Closure::fromCallable, and `f(...)`: a Closure calling `callable`.
    pub(crate) fn closure_from_callable(&mut self, callable: &Value) -> Result<Value, Throw> {
        if let Value::Object(o) = callable.deref()
            && closure_data(&o).is_some()
        {
            return Ok(Value::Object(o));
        }
        let data = match self.resolve_callable(callable)? {
            Target::Builtin(i) => ClosureData {
                func: ClosureFunc::Builtin(i),
                is_static: true,
                this: None,
                scope: None,
                called: None,
                captured: RefCell::new(Vec::new()),
                trampoline: None,
            },
            Target::Frame(MethodFrame { func: Code::User(func), this, scope, called, captured, trampoline }) => ClosureData {
                func: ClosureFunc::User(func),
                is_static: this.is_none(),
                this,
                scope,
                called,
                captured: RefCell::new(captured),
                trampoline,
            },
            Target::Frame(MethodFrame { func: Code::Native(native, name), this, scope, called, .. }) => ClosureData {
                func: ClosureFunc::Method(native, name),
                is_static: this.is_none(),
                this,
                scope,
                called,
                captured: RefCell::new(Vec::new()),
                trampoline: None,
            },
        };
        Ok(self.new_closure(data))
    }

    /// Whether `target` takes its argument `n` by reference.
    pub(crate) fn takes_ref(&self, target: &Target, n: usize) -> bool {
        match target {
            Target::Builtin(i) => self.isolate.registry.by_ref(*i, n),
            Target::Frame(MethodFrame { func: Code::User(f), .. }) => f.code.param_by_ref(n),
            Target::Frame(MethodFrame { func: Code::Native(..), .. }) => false,
        }
    }

    /// An argument as parameter `n` gets it: a reference (an argument
    /// compiled as one) only where the parameter takes one.
    fn arg_or_ref(&self, target: &Target, n: usize, v: &Value) -> Value {
        if matches!(v, Value::Ref(_)) && self.takes_ref(target, n) { v.clone() } else { v.deref() }
    }

    /// Whether the parameter of `target` named `name` takes a reference.
    /// An unknown name answers yes, so the variable is not read (and
    /// warned about) before the call reports the name, as in PHP.
    pub(crate) fn named_takes_ref(&self, target: &Target, name: &[u8]) -> bool {
        let (_, params) = self.named_params(target);
        let name = String::from_utf8_lossy(name);
        params.iter().position(|p| p.name == name && !p.variadic).is_none_or(|i| self.takes_ref(target, i))
    }

    /// What `callable` calls, with PHP's error when it is not callable.
    pub(crate) fn resolve_callable(&mut self, callable: &Value) -> Result<Target, Throw> {
        match callable.deref() {
            Value::Object(o) => {
                if let Some(target) = self.closure_target(&o) {
                    return Ok(target);
                }
                if o.class.method(b"__invoke").is_some() {
                    return self.method_frame(&Value::Object(Rc::clone(&o)), Some(p8_runtime::symbol::known::INVOKE), b"__invoke").map(Target::Frame);
                }
                Err(self.throw("Error", "Object of type ".to_string() + &o.class.name.to_string_lossy() + " is not callable"))
            }
            Value::Str(name) => self.resolve_name(name.as_bytes()),
            Value::Array(a) if a.len() == 2 => {
                let (Some(target), Some(Value::Str(method))) = (a.get_int(0).map(Value::deref), a.get_int(1).map(Value::deref)) else {
                    return Err(self.throw("Error", "Array callback must have exactly two elements"));
                };
                let key = self.method_key(method.as_bytes());
                match target {
                    Value::Object(_) => self.method_frame(&target, key, method.as_bytes()).map(Target::Frame),
                    Value::Str(class) => self.static_by_name(class.as_bytes(), key, method.as_bytes()),
                    _ => Err(self.throw("Error", "Array callback must have exactly two elements")),
                }
            }
            Value::Array(_) => Err(self.throw("Error", "Array callback must have exactly two elements")),
            _ => Err(self.throw("Error", "Value not callable")),
        }
    }

    /// A callable string: a function, or "Class::method".
    fn resolve_name(&mut self, name: &[u8]) -> Result<Target, Throw> {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        if let Some(at) = name.windows(2).position(|w| w == b"::") {
            let method = &name[at + 2..];
            return self.static_by_name(&name[..at], self.method_key(method), method);
        }
        match self.function_named(name) {
            Some(crate::sites::Callee::User(f)) => {
                let func = Rc::clone(&self.closures[f as usize]);
                Ok(Target::Frame(MethodFrame { func: Code::User(func), this: None, scope: None, called: None, captured: Vec::new(), trampoline: None }))
            }
            Some(crate::sites::Callee::Builtin(i)) => Ok(Target::Builtin(i)),
            _ => Err(self.throw("Error", format!("Call to undefined function {}()", text(name)))),
        }
    }

    /// `"Class::method"` or `['Class', 'method']`.
    fn static_by_name(&mut self, class: &[u8], key: Option<p8_runtime::symbol::Sym>, display: &[u8]) -> Result<Target, Throw> {
        let class = class.strip_prefix(b"\\").unwrap_or(class);
        // `self`, `parent` and `static` name classes from where the call is
        // made, and pass its called class on (late static binding).
        let (scope, called_here) = self.scope();
        let keyword = match class.to_ascii_lowercase().as_slice() {
            b"self" => Some(scope.clone()),
            b"parent" => Some(scope.as_ref().and_then(|s| s.parent.clone())),
            b"static" => Some(called_here.clone().or_else(|| scope.clone())),
            _ => None,
        };
        let is_keyword = keyword.is_some();
        let target = match keyword {
            Some(Some(c)) => c,
            Some(None) => return Err(self.throw("Error", format!("Cannot use \"{}\" when no class scope is active", text(&class.to_ascii_lowercase())))),
            None => self.find_class(class)?,
        };
        let forwarded = is_keyword.then_some(called_here).flatten().filter(|c| c.is_a(&target.lower));
        let Some(method) = crate::classes::method_of(&target, key).cloned() else {
            if let Some(m) = self.static_magic_call(&target, display) {
                return Ok(Target::Frame(m));
            }
            return Err(self.throw("Error", format!("Call to undefined method {}::{}()", target.name.to_string_lossy(), text(display))));
        };
        let this = self.frames.last().and_then(|f| f.this.clone()).filter(|t| t.class.is_a(&target.lower));
        if !method.is_static && this.is_none() {
            return Err(self.throw(
                "Error",
                format!(
                    "Non-static method {}::{}() cannot be called statically",
                    method.declaring_name.to_string_lossy(),
                    method.name.to_string_lossy()
                ),
            ));
        }
        let scope = self.class_named(&method.declaring).cloned().unwrap_or_else(|| Rc::clone(&target));
        let called = this.as_ref().map(|t| Rc::clone(&t.class)).or(forwarded).unwrap_or(target);
        let this = if method.is_static { None } else { this };
        Ok(Target::Frame(MethodFrame::method(self.code_of(&method), this, scope, called)))
    }

    /// CallValue: call `callee` with the `argc` arguments at `args`
    /// (absolute registers), its result to `ret`.
    pub(crate) fn call_value(&mut self, regs: &mut Vec<Value>, callee: &Value, args: usize, argc: u8, ret: u32, pc: usize) -> Result<Next, Throw> {
        match self.resolve_callable(callee)? {
            Target::Frame(m) => {
                // Arguments arrive by value: a reference is read through,
                // unless its parameter takes one (a spread call's).
                let by_ref = |n: usize| matches!(&m.func, Code::User(f) if f.code.param_by_ref(n));
                for (n, r) in regs[args..args + argc as usize].iter_mut().enumerate() {
                    if let Value::Ref(cell) = r
                        && !by_ref(n)
                    {
                        let v = cell.borrow().clone();
                        *r = v;
                    }
                }
                self.call_method(regs, m, args, argc, ret, pc)
            }
            Target::Builtin(i) => {
                let base = self.frames.last().expect("a frame").base as usize;
                self.call_builtin(regs, base, i, (args - base) as p8_bytecode::Reg, argc, (ret as usize - base) as p8_bytecode::Reg)?;
                Ok(Next::Go(pc))
            }
        }
    }

    /// Call PHP code from a builtin (it holds the lent registers): run
    /// `callable` on `args` to its return, above the running frames.
    pub(crate) fn call_from_builtin(&mut self, callable: &Value, args: &mut [Value]) -> Result<Value, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = self.call_with_regs(&mut regs, callable, args);
        self.isolate.regs = regs;
        r
    }

    /// Call `callable` on `args` above the running frames, whose
    /// registers `regs` is.
    pub(crate) fn call_with_regs(&mut self, regs: &mut Vec<Value>, callable: &Value, args: &mut [Value]) -> Result<Value, Throw> {
        // Lent while the callable is resolved: a class it names is
        // autoloaded there (zend_is_callable_ex), not by running it again.
        std::mem::swap(&mut self.isolate.regs, regs);
        let target = self.resolve_callable(callable);
        std::mem::swap(&mut self.isolate.regs, regs);
        let target = target?;
        let m = match target {
            Target::Builtin(i) => {
                // A builtin that calls back finds the registers lent; it is
                // running, for its messages' prefix and stack traces.
                for (n, a) in args.iter_mut().enumerate() {
                    if matches!(a, Value::Ref(_)) && !self.isolate.registry.by_ref(i, n) {
                        *a = a.deref();
                    }
                }
                std::mem::swap(&mut self.isolate.regs, regs);
                let name = self.isolate.registry.function(i).name;
                self.push_native(name, None, args, false);
                let r = self.run_builtin(i, args);
                self.pop_native();
                std::mem::swap(&mut self.isolate.regs, regs);
                return r;
            }
            Target::Frame(MethodFrame { func: Code::Native(native, name), this, scope, called, .. }) => {
                let mut passed: Vec<Value> = args.iter().map(Value::deref).collect();
                let saved = std::mem::replace(&mut self.native_class, called);
                // Lent and running, as a method the code calls is.
                std::mem::swap(&mut self.isolate.regs, regs);
                let r = self.run_native_method(native, &name, scope.as_ref(), this.as_ref(), &mut passed);
                std::mem::swap(&mut self.isolate.regs, regs);
                self.native_class = saved;
                return r;
            }
            Target::Frame(m) => m,
        };
        self.run_nested(regs, m, args)
    }

    pub(crate) fn run_nested(&mut self, regs: &mut Vec<Value>, m: MethodFrame, args: &mut [Value]) -> Result<Value, Throw> {
        let at = match self.frames.last() {
            Some(top) => top.base as usize + top.func.code.nregs as usize,
            None => 0,
        };
        self.run_nested_at(regs, m, args, at.max(self.result_reg))
    }

    fn run_nested_at(&mut self, regs: &mut Vec<Value>, mut m: MethodFrame, args: &mut [Value], at: usize) -> Result<Value, Throw> {
        // __call / __callStatic: the arguments become (name, [arguments]).
        if let Some(name) = m.trampoline.take() {
            let packed: Vec<Value> = args.iter().map(Value::deref).collect();
            let mut repacked = [Value::Str(name), Value::Array(Rc::new(p8_runtime::array::PhpArray::from_list(packed)))];
            return self.run_nested_at(regs, m, &mut repacked, at);
        }
        // Each nested run takes native stack: stop before it runs out.
        let here = 0u8;
        let depth = self.stack_base.saturating_sub(std::ptr::addr_of!(here) as usize);
        if self.stack_base != 0 && depth > STACK_LIMIT {
            return Err(self.throw(
                "Error",
                format!("Maximum call stack size of {STACK_LIMIT} bytes (zend.max_allowed_stack_size - zend.reserved_stack_size) reached. Infinite recursion?"),
            ));
        }
        if regs.len() < at + args.len() + 1 {
            regs.resize(at + args.len() + 1, Value::Undef);
        }
        let by_ref = |n: usize| match &m.func {
            Code::User(f) => f.code.param_by_ref(n),
            Code::Native(..) => false,
        };
        // A value where the parameter takes a reference: PHP warns
        // (zend_call_function) and passes the value.
        if let Code::User(f) = &m.func {
            let code = &f.code;
            for (n, a) in args.iter().enumerate() {
                if by_ref(n) && !matches!(a, Value::Ref(_)) {
                    let name = match &code.class {
                        Some(class) => format!("{}::{}", text(class.as_bytes()), text(code.name.as_bytes())),
                        None => text(code.name.as_bytes()),
                    };
                    let param = code.params.get(n).or(code.params.last()).map(|p| text(p.name.as_bytes())).unwrap_or_default();
                    self.warn(format!("{name}(): Argument #{} (${param}) must be passed by reference, value given", n + 1));
                }
            }
        }
        for (n, a) in args.iter().enumerate() {
            regs[at + n] = match a {
                Value::Ref(_) if by_ref(n) => a.clone(),
                other => other.deref(),
            };
        }
        let argc = args.len() as u8;
        let (saved_pc, saved_floor) = (self.pc, self.frames.floor);
        let caller_pc = self.frames.last().map_or(0, |f| f.pc);
        // The callee's result lands in its first register (written after
        // its window is cleared).
        let next = self.call_method(regs, m, at, argc, at as u32, caller_pc as usize)?;
        if let Next::Go(_) = next {
            // Nothing to run (a generator function made its Generator).
            return Ok(std::mem::replace(&mut regs[at], Value::Undef));
        }
        self.frames.floor = self.frames.len() - 1;
        let saved_result_reg = std::mem::replace(&mut self.result_reg, at + 1);
        let r = self.execute(regs);
        self.result_reg = saved_result_reg;
        self.frames.floor = saved_floor;
        self.pc = saved_pc;
        if let Some(caller) = self.frames.last_mut() {
            caller.pc = caller_pc;
        }
        if r.is_err() {
            // A result the callee returned before a destructor threw is dropped with the frame.
            regs[at] = Value::Undef;
        }
        r?;
        let v = std::mem::replace(&mut regs[at], Value::Undef);
        Ok(v)
    }

    /// A call's argument array (`...`, named arguments) in parameter
    /// order: string keys are named arguments. A skipped optional
    /// parameter is Undef for a user function (its default runs) or its
    /// default for a builtin.
    pub(crate) fn place_args(&mut self, target: &Target, list: &PhpArray) -> Result<Vec<Value>, Throw> {
        if !list.iter().any(|(k, _)| matches!(k, Key::Str(_))) {
            return Ok(list.values().map(Value::deref).collect());
        }
        let (fname, params) = self.named_params(target);
        let mut extra = PhpArray::new();
        let mut out: Vec<Value> = Vec::new();
        let mut set: Vec<bool> = Vec::new();
        for (k, v) in list.iter() {
            match k {
                Key::Int(_) => {
                    out.push(self.arg_or_ref(target, out.len(), v));
                    set.push(true);
                }
                Key::Str(key) => {
                    let name = String::from_utf8_lossy(key.as_bytes()).into_owned();
                    let Some(i) = params.iter().position(|p| p.name == name && !p.variadic) else {
                        // A variadic user function collects it by name.
                        if params.last().is_some_and(|p| p.variadic) && matches!(target, Target::Frame(_)) {
                            extra.insert(Key::Str(key.clone()), v.deref());
                            continue;
                        }
                        return Err(self.throw("Error", format!("Unknown named parameter ${name}")));
                    };
                    if i < set.len() && set[i] {
                        return Err(self.throw("Error", format!("Named parameter ${name} overwrites previous argument")));
                    }
                    if out.len() <= i {
                        out.resize(i + 1, Value::Undef);
                        set.resize(i + 1, false);
                    }
                    out[i] = self.arg_or_ref(target, i, v);
                    set[i] = true;
                }
            }
        }
        // The holes: optional parameters left out.
        for i in 0..out.len() {
            if set[i] {
                continue;
            }
            let p = &params[i];
            if !p.optional {
                return Err(self.throw("ArgumentCountError", format!("{fname}(): Argument #{} (${}) not passed", i + 1, p.name)));
            }
            if let Some(text) = p.default {
                out[i] = match self.stub_default(text) {
                    Some(v) => v,
                    None => return Err(self.throw("Error", format!("p8 cannot pass named arguments to {fname}() past ${} yet", p.name))),
                };
            }
        }
        if !extra.is_empty() {
            // For the callee's frame, one above this one.
            self.named_extra = Some((self.frames.len() + 1, extra));
        }
        Ok(out)
    }

    /// The parameters of a call's target, and its name for messages.
    fn named_params(&self, target: &Target) -> (String, Vec<NamedParam>) {
        match target {
            Target::Builtin(i) => {
                let name = self.isolate.registry.function(*i).name.to_string();
                let params = self
                    .isolate
                    .registry
                    .sig(*i)
                    .map(|s| s.params.iter().map(|p| NamedParam { name: p.name.to_string(), optional: p.optional(), variadic: p.variadic, default: p.default }).collect())
                    .unwrap_or_default();
                (name, params)
            }
            Target::Frame(m) => match &m.func {
                Code::User(clo) => {
                    let code = &clo.code;
                    let name = match (&code.class, &m.scope) {
                        (Some(_), Some(scope)) => format!("{}::{}", text(scope.name.as_bytes()), text(code.name.as_bytes())),
                        _ => text(code.name.as_bytes()),
                    };
                    let params = code
                        .params
                        .iter()
                        .map(|p| NamedParam { name: text(p.name.as_bytes()), optional: p.has_default || p.variadic, variadic: p.variadic, default: None })
                        .collect();
                    (name, params)
                }
                Code::Native(native, _) => {
                    // A builtin class's method: its stub signature, found by name.
                    let Some(scope) = &m.scope else { return (String::new(), Vec::new()) };
                    let same = |other: &Option<p8_runtime::object::Native>| match (other, native) {
                        (Some(p8_runtime::object::Native::Method(a)), p8_runtime::object::Native::Method(b)) => std::ptr::fn_addr_eq(*a, *b),
                        (Some(p8_runtime::object::Native::Static(a)), p8_runtime::object::Native::Static(b)) => std::ptr::fn_addr_eq(*a, *b),
                        _ => false,
                    };
                    let Some(method) = scope.methods.values().find(|meth| same(&meth.native)) else { return (String::new(), Vec::new()) };
                    let name = format!("{}::{}", text(method.declaring_name.as_bytes()), text(method.name.as_bytes()));
                    let params = p8_runtime::stubs::lookup(name.as_bytes())
                        .map(|s| s.params.iter().map(|p| NamedParam { name: p.name.to_string(), optional: p.optional(), variadic: p.variadic, default: p.default }).collect())
                        .unwrap_or_default();
                    (name, params)
                }
            },
        }
    }

    /// A builtin parameter's default as the stub writes it: literals,
    /// constants, `A | B`, `[]`.
    pub(crate) fn stub_default(&self, text: &str) -> Option<Value> {
        let t = text.trim();
        if t.contains('|') {
            let mut acc = 0i64;
            for part in t.split('|') {
                acc |= self.stub_default(part)?.to_int();
            }
            return Some(Value::Int(acc));
        }
        Some(match t {
            "null" | "NULL" => Value::Null,
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "[]" => Value::Array(Rc::new(PhpArray::new())),
            _ if (t.starts_with('"') && t.ends_with('"') || t.starts_with('\'') && t.ends_with('\'')) && t.len() >= 2 => Value::Str(PhpStr::from(&unquote(t)[..])),
            _ => {
                if let Ok(i) = t.parse::<i64>() {
                    Value::Int(i)
                } else if let Ok(x) = t.parse::<f64>() {
                    Value::Float(x)
                } else {
                    let c = self.isolate.registry.constant(t.trim_start_matches('\\').as_bytes())?;
                    c.to_value()
                }
            }
        })
    }

    /// Whether a value is callable (is_callable): a class it names is
    /// autoloaded where the registers are lent.
    pub(crate) fn callable(&mut self, v: &Value) -> bool {
        let saved = std::mem::take(&mut self.failed_call);
        let r = self.resolve_callable(v);
        self.failed_call = saved;
        // Not lent: the class stays unloaded, and a later lookup asks the
        // autoloaders again.
        if let Err(Throw::Autoload(name)) = &r {
            self.autoload_tried.remove(&name.as_bytes().to_ascii_lowercase());
        }
        r.is_ok()
    }
}

trait Lossy {
    fn to_string_lossy(&self) -> String;
}

impl Lossy for PhpStr {
    fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(self.as_bytes()).into_owned()
    }
}

/// What var_dump shows of a Closure (zend_closure_get_debug_info): its
/// name, where it is declared, its captured values, `$this`, parameters.
pub(crate) fn debug_info(o: &Object) -> p8_runtime::array::PhpArray {
    use p8_runtime::array::{Key, PhpArray};
    let key = |k: &str| Key::from_string(PhpStr::from(k));
    let mut out = PhpArray::new();
    let Some(data) = closure_data(o) else { return out };
    let Some(func) = data.func.user().map(|f| &f.code) else { return out };
    out.insert(key("name"), Value::Str(PhpStr::from(func.name.as_bytes())));
    out.insert(key("file"), Value::Str(PhpStr::from(&*func.file)));
    out.insert(key("line"), Value::Int(i64::from(func.line)));
    let captured = data.captured.borrow();
    if !captured.is_empty() {
        let mut vars = PhpArray::new();
        for (r, v) in func.bound.iter().zip(captured.iter()) {
            if matches!(v, Value::Undef) {
                continue;
            }
            let name = func.cv_names[*r as usize].clone();
            vars.insert(Key::from_string(PhpStr::from(name.as_bytes())), v.clone());
        }
        out.insert(key("static"), Value::Array(Rc::new(vars)));
    }
    if let Some(this) = &data.this {
        out.insert(key("this"), Value::Object(Rc::clone(this)));
    }
    if !func.params.is_empty() {
        let mut params = PhpArray::new();
        for p in &func.params {
            let name = format!("{}${}", if p.by_ref { "&" } else { "" }, String::from_utf8_lossy(p.name.as_bytes()));
            let kind = if p.has_default || p.variadic { "<optional>" } else { "<required>" };
            params.insert(Key::from_string(PhpStr::from(name)), Value::str(kind));
        }
        out.insert(key("parameter"), Value::Array(Rc::new(params)));
    }
    out
}

/// A stub default's string literal as its value: `"\""` is `"`, `'\\'` is `\`
/// (the escapes stubs use).
fn unquote(t: &str) -> Vec<u8> {
    let double = t.starts_with('"');
    let body = &t.as_bytes()[1..t.len() - 1];
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        let c = body[i];
        if c == b'\\' && i + 1 < body.len() {
            let n = body[i + 1];
            let v = match (double, n) {
                (_, b'\\') => Some(b'\\'),
                (true, b'"') => Some(b'"'),
                (false, b'\'') => Some(b'\''),
                (true, b'n') => Some(b'\n'),
                (true, b't') => Some(b'\t'),
                (true, b'r') => Some(b'\r'),
                (true, b'0') => Some(0),
                (true, b'$') => Some(b'$'),
                _ => None,
            };
            if let Some(v) = v {
                out.push(v);
                i += 2;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

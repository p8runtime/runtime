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
use p8_runtime::value::{Kind, Value};
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
    /// Made from a callable (`f(...)`, Closure::fromCallable) or by a
    /// partial application, not written as a closure: its $this and scope
    /// are the method's (ZEND_ACC_FAKE_CLOSURE, ZEND_PARTIAL).
    pub fake: bool,
}

/// What a Closure runs.
pub(crate) enum ClosureFunc {
    User(Rc<Closure>),
    /// A builtin function (its index, and the function: var_dump shows
    /// its name and parameters), from its name.
    Builtin(usize, p8_runtime::ext::Entry),
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
            ClosureFunc::Builtin(i, b) => ClosureFunc::Builtin(*i, *b),
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

/// Whether `call_with_regs` enters `func` itself on `args`
/// (`run_direct`): every required argument there and none extra the
/// frame cannot count (`fast::direct`: not a generator's code either),
/// and no reference parameter among those `args` fill (their warning
/// and passing are `run_nested_at`'s).
#[inline(always)]
fn enters_directly(func: &Closure, args: &[Value]) -> bool {
    let n = args.len();
    n <= usize::from(u8::MAX) && crate::fast::direct(func.shape, &func.code, n as u8) && !(0..n).any(|i| func.code.param_by_ref(i))
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

/// Whether call_user_func_array's `list` names arguments: a string key,
/// a named argument (a packed list has none).
#[inline(always)]
fn names_arguments(list: &PhpArray) -> bool {
    !list.is_packed() && list.iter().any(|(k, _)| matches!(k, Key::Str(_)))
}

impl Context<'_> {
    /// Whether a nested run (a callback a builtin makes, a call native code
    /// makes, #444) would take the native stack past `STACK_LIMIT` from
    /// where this request's run started.
    pub(crate) fn stack_exhausted(&self) -> bool {
        let here = 0u8;
        let depth = self.stack_base.saturating_sub(std::ptr::addr_of!(here) as usize);
        self.stack_base != 0 && depth > STACK_LIMIT
    }

    /// MakeClosure: a Closure of `func`, bound to the running frame's
    /// `$this` (unless static) and class.
    pub(crate) fn make_closure(&mut self, func: &Arc<Function>, is_static: bool) -> Rc<Object> {
        let key = Arc::as_ptr(func) as usize;
        let closure = match self.lambdas.get(&key) {
            Some(c) => Rc::clone(c),
            None => {
                let c = Closure::new(Arc::clone(func), self.isolate);
                self.lambdas.insert(key, Rc::clone(&c));
                c
            }
        };
        let (this, scope, called) = match self.frames.last() {
            Some(f) => (if is_static { None } else { f.this.clone() }, f.scope.clone(), f.called_class().cloned()),
            None => (None, None, None),
        };
        let data = ClosureData { func: ClosureFunc::User(closure), is_static, this, scope, called, captured: RefCell::new(Vec::new()), trampoline: None, fake: false };
        let class = Rc::clone(&self.classes[&self.isolate.known.closure]);
        Object::with_data(class, &self.ids, Some(Rc::new(data)))
    }

    /// What calling a Closure object calls. (In line: resolve_callable's
    /// first case, on every callback's call.)
    #[inline(always)]
    fn closure_target(&self, o: &Rc<Object>) -> Option<Target> {
        let data = closure_data(o)?;
        Some(match &data.func {
            ClosureFunc::User(func) => Target::Frame(MethodFrame {
                func: Code::User(Rc::clone(func)),
                this: data.this.clone(),
                scope: data.scope.clone(),
                called: data.called.clone(),
                trampoline: data.trampoline.clone(),
                closure: Some(Rc::clone(o)),
            }),
            ClosureFunc::Method(native, name) => Target::Frame(MethodFrame {
                func: Code::Native(*native, name.clone()),
                this: data.this.clone(),
                scope: data.scope.clone(),
                called: data.called.clone(),
                trampoline: None,
                closure: None,
            }),
            ClosureFunc::Builtin(builtin, _) => Target::Builtin(*builtin),
        })
    }

    fn new_closure(&self, data: ClosureData) -> Value {
        let class = Rc::clone(&self.classes[&self.isolate.known.closure]);
        Value::object(Object::with_data(class, &self.ids, Some(Rc::new(data))))
    }

    /// Closure::bind / bindTo: a copy bound to `new_this` and the scope
    /// `scope` names ("static" or none: its own).
    pub(crate) fn bind(&mut self, closure: &Rc<Object>, new_this: Value, scope: Option<Value>) -> Result<Value, Throw> {
        let Some(data) = closure_data(closure) else { return Ok(Value::NULL) };
        let this = new_this.into_object().ok();
        // zend_valid_closure_binding: a closure made from a method (or by
        // a partial application) keeps its method's $this and scope.
        let method_name = || match &data.func {
            ClosureFunc::User(c) => text(c.code.name.as_bytes()),
            ClosureFunc::Builtin(_, b) => b.name.to_string(),
            ClosureFunc::Method(_, n) => text(n.as_bytes().rsplit(|&b| b == b':').next().unwrap_or_default()),
        };
        match &this {
            Some(_) if data.is_static => {
                self.warn("Cannot bind an instance to a static closure, this will be an error in PHP 9");
                return Ok(Value::NULL);
            }
            Some(t) if data.fake && data.scope.as_ref().is_some_and(|s| !t.class.is_a(&s.name.as_bytes().to_ascii_lowercase())) => {
                let scope = data.scope.as_ref().map_or_else(String::new, |s| text(s.name.as_bytes()));
                let message = format!("Cannot bind method {scope}::{}() to object of class {}, this will be an error in PHP 9", method_name(), text(t.class.name.as_bytes()));
                self.warn(message);
                return Ok(Value::NULL);
            }
            None if data.fake && data.scope.is_some() && !data.is_static => {
                self.warn("Cannot unbind $this of method, this will be an error in PHP 9");
                return Ok(Value::NULL);
            }
            _ => {}
        }
        let scope = match scope {
            None => data.scope.clone(),
            Some(v) if v.is_null() => None,
            Some(v) => {
                if let Some(o) = v.as_object() {
                    Some(Rc::clone(&o.class))
                } else {
                    let name = v.to_php_string().unwrap_or_else(PhpStr::empty);
                    if name.as_bytes() == b"static" {
                        data.scope.clone()
                    } else {
                        match self.class_named(name.as_bytes()) {
                            Some(c) => Some(Rc::clone(c)),
                            None => {
                                self.warn(format!("Class \"{}\" not found", text(name.as_bytes())));
                                return Ok(Value::NULL);
                            }
                        }
                    }
                }
            }
        };
        let same_scope = match (&scope, &data.scope) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b) || a.name.as_bytes().eq_ignore_ascii_case(b.name.as_bytes()),
            (None, None) => true,
            _ => false,
        };
        if data.fake && !same_scope {
            let what = if data.scope.is_none() { "function" } else { "method" };
            self.warn(format!("Cannot rebind scope of closure created from {what}, this will be an error in PHP 9"));
            return Ok(Value::NULL);
        }
        let called = this.as_ref().map(|t| Rc::clone(&t.class)).or_else(|| scope.clone());
        let copy = ClosureData {
            func: data.func.clone(),
            is_static: data.is_static,
            this,
            scope,
            called,
            captured: RefCell::new(data.captured.borrow().clone()),
            trampoline: data.trampoline.clone(),
            fake: data.fake,
        };
        Ok(self.new_closure(copy))
    }

    /// Closure::fromCallable, and `f(...)`: a Closure calling `callable`.
    pub(crate) fn closure_from_callable(&mut self, callable: &Value) -> Result<Value, Throw> {
        if let Ok(o) = callable.deref().into_object()
            && closure_data(&o).is_some()
        {
            return Ok(Value::object(o));
        }
        let data = self.callable_data(callable)?;
        Ok(self.new_closure(data))
    }

    /// `C::m(...)`: a Closure of the static method "C::m" in `callable`,
    /// looked up as zend_std_get_static_method looks a static call's up
    /// (not visible from here: __callStatic, else an error; abstract: an
    /// error; a trait's own: deprecated). `in_const`: one in a constant
    /// expression, as zend_ast_evaluate looks it up (no __callStatic, only
    /// a static method). Any other callable is closure_from_callable's.
    pub(crate) fn static_callable(&mut self, callable: &Value, in_const: bool) -> Result<Value, Throw> {
        let Ok(s) = callable.deref().into_str() else { return self.closure_from_callable(callable) };
        let name = s.as_bytes();
        let Some(at) = name.windows(2).position(|w| w == b"::") else { return self.closure_from_callable(callable) };
        let (class, method) = (&name[..at], &name[at + 2..]);
        let scope = self.scope().0;
        let target = match class.to_ascii_lowercase().as_slice() {
            b"self" => scope.clone(),
            b"parent" => scope.as_ref().and_then(|s| s.parent.clone()),
            b"static" => None,
            _ => Some(self.find_class(class.strip_prefix(b"\\").unwrap_or(class))?),
        };
        // (`static`, or no class scope: resolve_callable says.)
        let Some(target) = target else { return self.closure_from_callable(callable) };
        let key = self.method_key(method);
        let call_static = target.method_sym(p8_runtime::symbol::known::CALL_STATIC).is_some();
        const MAGIC: &str = "Creating a callable for the magic __callStatic() method is not supported in constant expressions";
        let Some(m) = crate::classes::method_of(&target, key).cloned() else {
            if in_const && call_static {
                return Err(self.throw("Error", MAGIC));
            }
            return self.closure_from_callable(callable);
        };
        if m.visibility != p8_runtime::object::Visibility::Public {
            let declaring = self.declaring(&target, &m.declaring);
            let access = self.access_class(&declaring, &m, key);
            if !p8_runtime::object::visible(m.visibility, &access, scope.as_deref()) {
                if in_const && call_static {
                    return Err(self.throw("Error", MAGIC));
                }
                if !in_const && let Some(frame) = self.static_magic_call(&target, method) {
                    let data = self.target_data(Target::Frame(frame));
                    return Ok(self.new_closure(data));
                }
                let message = format!("Call to {} method {}::{}() from {}", m.visibility.word(), text(m.declaring_name.as_bytes()), text(method), self.scope_name());
                return Err(self.throw("Error", message));
            }
        }
        if in_const && !m.is_static {
            return Err(self.throw("Error", format!("Non-static method {}::{}() cannot be called statically", text(m.declaring_name.as_bytes()), text(m.name.as_bytes()))));
        }
        if m.is_abstract {
            return Err(self.throw("Error", format!("Cannot call abstract method {}::{}()", text(m.declaring_name.as_bytes()), text(m.name.as_bytes()))));
        }
        self.trait_static_deprecated(&target, &m);
        self.closure_from_callable(callable)
    }

    /// What a Closure of `callable` (not a Closure itself) holds, no
    /// object made (a partial application makes its own).
    pub(crate) fn callable_data(&mut self, callable: &Value) -> Result<ClosureData, Throw> {
        let target = self.resolve_callable(callable)?;
        Ok(self.target_data(target))
    }

    /// What a Closure calling `target` holds.
    fn target_data(&self, target: Target) -> ClosureData {
        match target {
            Target::Builtin(i) => ClosureData {
                func: ClosureFunc::Builtin(i, self.isolate.registry.function(i)),
                is_static: true,
                this: None,
                scope: None,
                called: None,
                captured: RefCell::new(Vec::new()),
                trampoline: None,
                fake: true,
            },
            Target::Frame(MethodFrame { func: Code::User(func), this, scope, called, trampoline, closure }) => ClosureData {
                func: ClosureFunc::User(func),
                is_static: this.is_none(),
                this,
                scope,
                called,
                // A Closure's target: its `use` values too.
                captured: RefCell::new(closure.as_deref().and_then(closure_data).map(|d| d.captured.borrow().clone()).unwrap_or_default()),
                trampoline,
                fake: true,
            },
            Target::Frame(MethodFrame { func: Code::Native(native, name), this, scope, called, .. }) => ClosureData {
                func: ClosureFunc::Method(native, name),
                is_static: this.is_none(),
                this,
                scope,
                called,
                captured: RefCell::new(Vec::new()),
                trampoline: None,
                fake: true,
            },
        }
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
        if v.is_reference() && self.takes_ref(target, n) { v.clone() } else { v.deref() }
    }

    /// Whether the parameter of `target` named `name` takes a reference.
    /// An unknown name answers yes, so the variable is not read (and
    /// warned about) before the call reports the name, as in PHP.
    /// Whether a Closure of PHP code takes its argument `n` by reference,
    /// read from the closure itself (no frame made); None for any other
    /// callable.
    pub(crate) fn closure_takes_ref(callee: &Value, n: usize) -> Option<bool> {
        let o = callee.deref().into_object().ok()?;
        match &closure_data(&o)?.func {
            ClosureFunc::User(clo) => Some(clo.code.param_by_ref(n)),
            _ => None,
        }
    }

    pub(crate) fn named_takes_ref(&self, target: &Target, name: &[u8]) -> bool {
        self.named_position(target, name).is_none_or(|i| self.takes_ref(target, i))
    }

    /// The position of the (non-variadic) parameter named `name` of
    /// `target`. A user function's are compared where they are: no list
    /// of its parameters is made.
    fn named_position(&self, target: &Target, name: &[u8]) -> Option<usize> {
        match target {
            Target::Frame(MethodFrame { func: Code::User(clo), .. }) => clo.code.params.iter().position(|p| p.name.as_bytes() == name && !p.variadic),
            _ => {
                let (_, params) = self.named_params(target);
                let name = String::from_utf8_lossy(name);
                params.iter().position(|p| p.name == name && !p.variadic)
            }
        }
    }

    /// What `callable` calls, with PHP's error when it is not callable.
    pub(crate) fn resolve_callable(&mut self, callable: &Value) -> Result<Target, Throw> {
        let callable = callable.deref();
        match callable.kind() {
            Kind::Object => {
                let o = callable.as_object().expect("an object");
                if let Some(target) = self.closure_target(o) {
                    return Ok(target);
                }
                if o.class.method(b"__invoke").is_some() {
                    return self.method_frame(&Value::object(Rc::clone(o)), Some(p8_runtime::symbol::known::INVOKE), b"__invoke").map(Target::Frame);
                }
                Err(self.throw("Error", "Object of type ".to_string() + &o.class.name.to_string_lossy() + " is not callable"))
            }
            Kind::Str => self.resolve_name(callable.as_str().expect("a string").as_bytes()),
            Kind::Array => {
                let a = callable.as_array().expect("an array");
                if a.len() != 2 {
                    return Err(self.throw("Error", "Array callback must have exactly two elements"));
                }
                let (Some(target), Some(method)) = (a.get_int(0).map(Value::deref), a.get_int(1).map(Value::deref).and_then(|v| v.into_str().ok())) else {
                    return Err(self.throw("Error", "Array callback must have exactly two elements"));
                };
                if target.is_object() {
                    self.method_frame(&target, self.method_key(method.as_bytes()), method.as_bytes()).map(Target::Frame)
                } else if let Some(class) = target.as_str() {
                    self.static_by_name(class.as_bytes(), method.as_bytes())
                } else {
                    Err(self.throw("Error", "Array callback must have exactly two elements"))
                }
            }
            // ZEND_INIT_DYNAMIC_CALL's message.
            _ => Err(self.throw("Error", format!("Value of type {} is not callable", callable.type_name()))),
        }
    }

    /// A callable string: a function, or "Class::method".
    fn resolve_name(&mut self, name: &[u8]) -> Result<Target, Throw> {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        if let Some(at) = name.windows(2).position(|w| w == b"::") {
            let method = &name[at + 2..];
            return self.static_by_name(&name[..at], method);
        }
        match self.function_named(name) {
            Some(crate::sites::Callee::User(f)) => {
                let func = self.closure_at(f);
                Ok(Target::Frame(MethodFrame { func: Code::User(func), this: None, scope: None, called: None, trampoline: None, closure: None }))
            }
            Some(crate::sites::Callee::Builtin(i)) => Ok(Target::Builtin(i)),
            _ => Err(self.throw("Error", format!("Call to undefined function {}()", text(name)))),
        }
    }

    /// `"Class::method"` or `['Class', 'method']`.
    fn static_by_name(&mut self, class: &[u8], display: &[u8]) -> Result<Target, Throw> {
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
        // Looked up once the class is there: declaring it (on demand, by
        // an autoloader) is what interns its members' names.
        let key = self.method_key(display);
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
        // Among the class and its parents first, as zend_function's scope
        // points at it: no lookup by name.
        let scope = self.declaring(&target, &method.declaring);
        let called = this.as_ref().map(|t| Rc::clone(&t.class)).or(forwarded).unwrap_or(target);
        let this = if method.is_static { None } else { this };
        self.method_deprecated(&method);
        Ok(Target::Frame(MethodFrame::method(self.code_of(&method), this, scope, called)))
    }

    /// CallValue: call `callee` with the `argc` arguments at `args`
    /// (absolute registers), its result to `ret`.
    pub(crate) fn call_value(&mut self, regs: &mut Vec<Value>, callee: &Value, args: usize, argc: u8, ret: u32, pc: usize) -> Result<Next, Throw> {
        // A Closure of PHP code: its frame made from the closure itself, and
        // its captured values put straight into the callee's variables, no
        // list of them made per call (ZEND_BIND_LEXICAL bound them once, at
        // creation).
        let closure = match callee.as_reference() {
            Some(cell) => cell.borrow().cloned_object(),
            None => callee.cloned_object(),
        };
        if let Some(o) = &closure
            && let Some(data) = closure_data(o)
            && let ClosureFunc::User(clo) = &data.func
            && !clo.code.is_generator
            && data.trampoline.is_none()
        {
            // Its `use` values go in as its frame starts (call_method).
            let m = MethodFrame { func: Code::User(Rc::clone(clo)), this: data.this.clone(), scope: data.scope.clone(), called: data.called.clone(), trampoline: None, closure: Some(Rc::clone(o)) };
            return self.call_target(regs, Target::Frame(m), args, argc, ret, pc);
        }
        let target = self.resolve_callable(callee)?;
        self.call_target(regs, target, args, argc, ret, pc)
    }

    /// `call_value` of a callable already resolved.
    pub(crate) fn call_target(&mut self, regs: &mut Vec<Value>, target: Target, args: usize, argc: u8, ret: u32, pc: usize) -> Result<Next, Throw> {
        match target {
            Target::Frame(m) => {
                // Arguments arrive by value: a reference is read through,
                // unless its parameter takes one (a spread call's).
                let by_ref = |n: usize| matches!(&m.func, Code::User(f) if f.code.param_by_ref(n));
                for (n, r) in regs[args..args + argc as usize].iter_mut().enumerate() {
                    if let Some(cell) = r.as_reference()
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
    /// `callable` on `args` to its return, above the running frames. (In
    /// line: Ctx::call is this, on every callback a builtin makes.)
    #[inline(always)]
    pub(crate) fn call_from_builtin(&mut self, callable: &Value, args: &mut [Value]) -> Result<Value, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = self.call_with_regs(&mut regs, callable, args);
        self.isolate.regs = regs;
        r
    }

    /// The function of PHP code the string `v` names (no `::`), if it
    /// names one. A name found before, by the string's address: the entry
    /// (`named_calls`) holds its string (an interned one lives for good),
    /// so no other string has that address. A function, once declared,
    /// stays: what a name found is found again.
    #[inline(always)]
    fn named_function(&mut self, v: &Value) -> Option<u32> {
        let head = v.as_str()?;
        let slot = (std::ptr::from_ref(head) as usize >> 4) % crate::isolate::NAMED_CALLS;
        let (held, found) = &self.named_calls[slot];
        if held.as_str().is_some_and(|s| std::ptr::eq(s, head)) {
            return Some(*found);
        }
        let name = head.as_bytes();
        if name.windows(2).any(|w| w == b"::") {
            return None;
        }
        let Some(crate::sites::Callee::User(f)) = self.function_named(name) else { return None };
        // (Its closure made now: a name found here has one in `closures`.)
        self.closure_at(f);
        self.named_calls[slot] = (v.clone(), f);
        Some(f)
    }

    /// `user_call` from the loop's call of a builtin (`builtin_call`):
    /// the pc to go on at, the callee's first when its frame was pushed.
    /// (Out of line: every other builtin's call keeps `builtin_call`'s
    /// frame as small as it was.)
    #[inline(never)]
    pub(crate) fn user_call_at(&mut self, regs: &mut Vec<Value>, base: usize, i: usize, call: (p8_bytecode::Reg, u8, p8_bytecode::Reg), pc: usize) -> Result<usize, Throw> {
        Ok(match self.user_call(regs, base, i, call, pc)? {
            Next::Go(next) => next,
            Next::Switch => self.frames.last().map_or(pc, |f| f.pc as usize),
        })
    }

    /// A call site PHP compiles to ZEND_INIT_USER_CALL
    /// (`Site::user_call`): call_user_func(callback, ...) or
    /// call_user_func_array(callback, args), `i` the builtin's twin, its
    /// `argc` arguments at `args` above `base`, its result to `dst`, the
    /// caller going on at `pc`. The callback is the caller's own call
    /// (INIT_USER_CALL, SEND_USER or SEND_ARRAY, DO_FCALL): no
    /// call_user_func frame, in a trace or anywhere. A function of PHP
    /// code gets its frame on the arguments where they are, after the
    /// callback's register, and the caller's loop runs it (Next::Switch),
    /// as it runs a plain call; a builtin is called from here. (In line
    /// in its two callers, `user_call_at` and `call_site`, both out of
    /// line themselves.)
    #[inline(always)]
    pub(crate) fn user_call(&mut self, regs: &mut Vec<Value>, base: usize, i: usize, (args, argc, dst): (p8_bytecode::Reg, u8, p8_bytecode::Reg), pc: usize) -> Result<Next, Throw> {
        // (As the loop's call of a function: an interrupt is raised here.)
        if self.stop.load(std::sync::atomic::Ordering::Relaxed) != crate::isolate::NO_INTERRUPT {
            return Err(self.interrupted());
        }
        let array = i == self.user_call_twins[1];
        let (start, argc) = (base + args as usize, argc as usize);
        let at = start + 1;
        // The common case, call_user_func of a function's name found
        // before, entered as the loop's Call arm enters a function (none
        // of its parameters by reference, every required one passed): the
        // arguments read through a reference where one is, the callback's
        // register cleared, the frame pushed and bound.
        if !array
            && let Some(f) = self.named_function(&regs[start])
        {
            let target = &self.closures[f as usize];
            let n = argc - 1;
            if crate::fast::direct(target.shape, &target.code, n as u8) && !target.code.params.iter().any(|p| p.by_ref) {
                for r in &mut regs[at..at + n] {
                    if r.is_reference() {
                        *r = r.deref();
                    }
                }
                // (A string: nothing it frees has a destructor.)
                regs[start] = Value::UNDEF;
                let target = &self.closures[f as usize];
                self.frames.call(Some(target), pc, at, n, (base + dst as usize) as u32);
                crate::fast::bind(&mut self.frames, regs, target.shape, at, n);
                return Ok(Next::Switch);
            }
        }
        self.user_call_frame(regs, base, i, (args, argc as u8, dst), pc)
    }

    /// `user_call` of call_user_func_array (its two arguments at `args`
    /// above `base`, its result to `dst`), its common case (a hook's
    /// callback: WP_Hook runs its filters so): a function's name found
    /// before and a list with no string key, as many values as the
    /// function takes (or more, or enough for its required parameters),
    /// none of its parameters by reference. The list's values are placed
    /// by value after the callback's register, as SEND_ARRAY places them,
    /// and the frame pushed and bound as the loop's Call arm does. None:
    /// anything else, which is the rest of `user_call_frame`'s. (In line
    /// there: out of `user_call`, whose callers it would grow.)
    #[inline(always)]
    fn user_call_list_direct(&mut self, regs: &mut Vec<Value>, base: usize, (args, dst): (p8_bytecode::Reg, p8_bytecode::Reg), pc: usize) -> Option<Result<Next, Throw>> {
        let start = base + args as usize;
        let at = start + 1;
        if let Some(f) = self.named_function(&regs[start])
            && let Some(list) = regs[at].as_array()
            && list.len() <= usize::from(u8::MAX)
            && !names_arguments(list)
        {
            let target = &self.closures[f as usize];
            let n = list.len();
            if crate::fast::direct(target.shape, &target.code, n as u8) && !target.code.params.iter().any(|p| p.by_ref) {
                let held = std::mem::replace(&mut regs[at], Value::UNDEF);
                let list = held.as_array().expect("an array (just matched)");
                if regs.len() < at + n + 1 {
                    regs.resize(at + n + 1, Value::UNDEF);
                }
                for (r, v) in regs[at..at + n].iter_mut().zip(list.values()) {
                    *r = if v.is_reference() { v.deref() } else { v.clone() };
                }
                // The list let go, as SEND_ARRAY lets go of it: a
                // destructor that makes due runs before the call, in the
                // caller (as `user_call_frame` runs it). (The callback is
                // a string: nothing it frees has a destructor.)
                drop(held);
                regs[start] = Value::UNDEF;
                if self.ids.has_pending()
                    && let Err(t) = self.run_destructors(regs)
                {
                    return Some(Err(t));
                }
                let target = &self.closures[f as usize];
                self.frames.call(Some(target), pc, at, n, (base + dst as usize) as u32);
                crate::fast::bind(&mut self.frames, regs, target.shape, at, n);
                return Some(Ok(Next::Switch));
            }
        }
        None
    }

    /// `user_call` past its common case (out of line: that case keeps a
    /// small frame of its own): the callback found and checked, its
    /// arguments placed, its frame pushed (call_method), or a builtin
    /// called.
    #[inline(never)]
    fn user_call_frame(&mut self, regs: &mut Vec<Value>, base: usize, i: usize, (args, argc, dst): (p8_bytecode::Reg, u8, p8_bytecode::Reg), pc: usize) -> Result<Next, Throw> {
        let array = i == self.user_call_twins[1];
        if array && let Some(next) = self.user_call_list_direct(regs, base, (args, dst), pc) {
            return next;
        }
        let (start, argc) = (base + args as usize, argc as usize);
        let at = start + 1;
        // The callback, as INIT_USER_CALL finds it (zend_is_callable_ex).
        let mut target = match self.named_function(&regs[start]) {
            Some(f) => {
                let func = self.closure_at(f);
                Target::Frame(MethodFrame { func: Code::User(func), this: None, scope: None, called: None, trampoline: None, closure: None })
            }
            None => self.user_call_target(regs, i, start)?,
        };
        // Its arguments, by value after the callback's register: what
        // SEND_USER sends (a reference read through), or the list's values
        // (SEND_ARRAY: a reference stays one where the parameter takes
        // one). A value where a parameter of PHP code takes a reference
        // is passed, after a warning (zend_call_function's).
        let takes_ref = |cx: &Self, n: usize| match &target {
            Target::Frame(MethodFrame { func: Code::User(f), .. }) => f.code.param_by_ref(n),
            Target::Frame(_) => false,
            Target::Builtin(b) => cx.isolate.registry.by_ref(*b, n),
        };
        let n = if array {
            if regs[at].as_array().is_none() {
                self.user_call_list(regs, at)?;
            }
            let held = std::mem::replace(&mut regs[at], Value::UNDEF);
            let list = held.as_array().expect("an array (user_call_list checked)");
            if names_arguments(list) {
                let placed = self.user_call_named(&mut target, list)?;
                if placed.len() > usize::from(u8::MAX) {
                    return self.user_call_nested(regs, start, target, placed, base + dst as usize, pc);
                }
                if regs.len() < at + placed.len() + 1 {
                    regs.resize(at + placed.len() + 1, Value::UNDEF);
                }
                let n = placed.len();
                for (k, v) in placed.into_iter().enumerate() {
                    regs[at + k] = v;
                }
                n
            } else {
                if list.len() > usize::from(u8::MAX) {
                    return self.user_call_nested(regs, start, target, list.values().map(Value::deref).collect(), base + dst as usize, pc);
                }
                if regs.len() < at + list.len() + 1 {
                    regs.resize(at + list.len() + 1, Value::UNDEF);
                }
                for (n, a) in list.values().enumerate() {
                    let by_ref = takes_ref(self, n);
                    if by_ref && !a.is_reference() {
                        self.user_call_by_value(&target, n);
                    }
                    regs[at + n] = if a.is_reference() && by_ref { a.clone() } else { a.deref() };
                }
                list.len()
            }
        } else {
            for n in 0..argc - 1 {
                if takes_ref(self, n) {
                    self.user_call_by_value(&target, n);
                }
                let r = &mut regs[at + n];
                if r.is_reference() {
                    *r = r.deref();
                }
            }
            argc - 1
        };
        regs[start] = Value::UNDEF;
        // What sending raised reaches the error handler before the call,
        // in the caller (and a destructor due runs there).
        if !self.pending_errors.is_empty() {
            self.dispatch_errors(regs)?;
        }
        if self.ids.has_pending() {
            self.run_destructors(regs)?;
        }
        match target {
            Target::Frame(m) => self.call_method(regs, m, at, n as u8, (base + dst as usize) as u32, pc),
            Target::Builtin(b) => {
                self.call_builtin(regs, base, b, args + 1, n as u8, dst)?;
                Ok(Next::Go(pc))
            }
        }
    }

    /// `user_call`'s callback when it is not a function's name found
    /// before: a Closure's function, else the callback checked as
    /// INIT_USER_CALL checks it (its TypeError, the builtin's message) and
    /// resolved; then, for call_user_func_array, its list checked
    /// (SEND_ARRAY's TypeError). The registers lent meanwhile: an
    /// autoloader may run.
    #[cold]
    #[inline(never)]
    fn user_call_target(&mut self, regs: &mut Vec<Value>, i: usize, start: usize) -> Result<Target, Throw> {
        let callable = regs[start].deref();
        let target = match callable.as_object().and_then(|o| self.closure_target(o)) {
            Some(target) => target,
            None => {
                std::mem::swap(&mut self.isolate.regs, regs);
                let target = match self.why_not_callable(&callable, true) {
                    Ok(None) => self.resolve_callable(&callable),
                    Ok(Some(why)) => {
                        let sig = self.isolate.registry.sig(i).expect("call_user_func's signature");
                        let (class, message) = p8_runtime::args::callback_error(sig, 0, &why);
                        Err(self.throw(class, message))
                    }
                    Err(t) => Err(t),
                };
                std::mem::swap(&mut self.isolate.regs, regs);
                target?
            }
        };
        Ok(target)
    }

    /// call_user_func_array's list, in register `at`, as SEND_ARRAY
    /// checks it once the callback is found: an array (read through a
    /// reference), else its TypeError.
    #[cold]
    #[inline(never)]
    fn user_call_list(&mut self, regs: &mut [Value], at: usize) -> Result<(), Throw> {
        let list = regs[at].deref();
        if list.as_array().is_none() {
            let given = p8_runtime::args::value_name(&list);
            return Err(self.throw("TypeError", format!("call_user_func_array(): Argument #2 ($args) must be of type array, {given} given")));
        }
        regs[at] = list;
        Ok(())
    }

    /// The warning for a value `user_call` sends where parameter `n` of
    /// the callback takes a reference.
    #[cold]
    fn user_call_by_value(&mut self, target: &Target, n: usize) {
        if let Target::Frame(MethodFrame { func: Code::User(f), .. }) = target {
            self.by_value_warning(&f.code, n);
        }
    }

    /// `user_call` of a list too long for a frame's argument count: the
    /// callback run nested, to its return (into register `ret`), on
    /// `args` (the list's values).
    #[cold]
    #[inline(never)]
    fn user_call_nested(&mut self, regs: &mut Vec<Value>, start: usize, target: Target, mut args: Vec<Value>, ret: usize, pc: usize) -> Result<Next, Throw> {
        regs[start] = Value::UNDEF;
        let v = match target {
            Target::Frame(m) => self.run_nested(regs, m, &mut args)?,
            Target::Builtin(b) => {
                std::mem::swap(&mut self.isolate.regs, regs);
                let r = self.run_builtin(b, &mut args);
                std::mem::swap(&mut self.isolate.regs, regs);
                r?
            }
        };
        regs[ret] = v;
        Ok(Next::Go(pc))
    }

    /// A call site's call_user_func("f", ...) or call_user_func_array("f",
    /// [...]) (builtin `i`, its `argc` arguments at absolute register
    /// `start`, its result to `dst` above `base`) when "f" names a function
    /// of PHP code: every check the builtin's path makes (its arguments'
    /// count and types, zend_is_callable) passes for it, so the function is
    /// looked up once and run on the arguments as they come out of the
    /// registers, with no list made for the builtin. The builtin runs as
    /// on that path (its entry in traces, its name for messages). None:
    /// anything else, which goes the builtin's way. (Out of line: the
    /// path every other builtin call takes keeps its own, smaller frame.)
    #[inline(never)]
    pub(crate) fn call_by_name(&mut self, regs: &mut Vec<Value>, base: usize, i: usize, start: usize, argc: usize, dst: p8_bytecode::Reg) -> Option<Result<(), Throw>> {
        let array = i == self.user_calls[1];
        if argc == 0 || (array && (argc != 2 || regs[start + 1].as_array().is_none())) {
            return None;
        }
        let f = self.named_function(&regs[start])?;
        p8_runtime::counts::bump(p8_runtime::counts::Count::BuiltinLent);
        let func = self.closure_at(f);
        // Its frames show this call's line.
        if let Some(top) = self.frames.last_mut() {
            top.pc = self.pc as u32;
        }
        self.push_native(if array { "call_user_func_array" } else { "call_user_func" }, None, &regs[start..start + argc], false);
        let entry = self.natives.len - 1;
        let outer = self.running_builtin.replace(i);
        let r = self.run_by_name(regs, func, array, start, argc);
        self.running_builtin = outer;
        // The arguments as the call was given them, for the trace of an
        // error it raised: its entry's (read through, as recorded anyway).
        let mut given = Vec::new();
        if self.natives.len == entry + 1 {
            std::mem::swap(&mut given, &mut self.natives.slots[entry].args);
        }
        let r = self.builtin_done(i, &given, r);
        if self.natives.len == entry + 1 {
            std::mem::swap(&mut given, &mut self.natives.slots[entry].args);
        }
        self.pop_native();
        // What the builtin's path held until now: the callable (and the list).
        for r in &mut regs[start..start + argc] {
            *r = Value::UNDEF;
        }
        Some(r.map(|v| regs[base + dst as usize] = v))
    }

    /// `call_by_name`'s run of `func`: the arguments after the callable
    /// (`argc` with it, at `start`) moved into the callee's registers by
    /// value (call_user_func), or the list's values as they are
    /// (call_user_func_array: a reference stays one where the parameter
    /// takes one), as `run_nested_at` places them.
    fn run_by_name(&mut self, regs: &mut Vec<Value>, func: Rc<Closure>, array: bool, start: usize, argc: usize) -> Result<Value, Throw> {
        let at = self.nested_at();
        if array {
            let list = regs[start + 1].clone();
            let list = list.as_array().expect("an array");
            if names_arguments(list) {
                let mut target = Target::Frame(MethodFrame { func: Code::User(func), this: None, scope: None, called: None, trampoline: None, closure: None });
                let placed = self.user_call_named(&mut target, list)?;
                let Target::Frame(m) = target else { unreachable!("a function's frame") };
                self.nested_room(regs, at, placed.len())?;
                let n = placed.len();
                for (k, v) in placed.into_iter().enumerate() {
                    regs[at + k] = v;
                }
                return self.run_placed(regs, m, n, at);
            }
            self.nested_room(regs, at, list.len())?;
            for (n, a) in list.values().enumerate() {
                let by_ref = func.code.param_by_ref(n);
                if by_ref && !a.is_reference() {
                    self.by_value_warning(&func.code, n);
                }
                regs[at + n] = if a.is_reference() && by_ref { a.clone() } else { a.deref() };
            }
            let m = MethodFrame { func: Code::User(func), this: None, scope: None, called: None, trampoline: None, closure: None };
            return self.run_placed(regs, m, list.len(), at);
        }
        self.nested_room(regs, at, argc - 1)?;
        for n in 0..argc - 1 {
            let a = std::mem::replace(&mut regs[start + 1 + n], Value::UNDEF);
            if func.code.param_by_ref(n) {
                self.by_value_warning(&func.code, n);
            }
            regs[at + n] = if a.is_reference() { a.deref() } else { a };
        }
        let m = MethodFrame { func: Code::User(func), this: None, scope: None, called: None, trampoline: None, closure: None };
        self.run_placed(regs, m, argc - 1, at)
    }

    /// Call `callable` on `args` above the running frames, whose
    /// registers `regs` is.
    pub(crate) fn call_with_regs(&mut self, regs: &mut Vec<Value>, callable: &Value, args: &mut [Value]) -> Result<Value, Throw> {
        // The common case (array_map's, usort's, a filter's callback): a
        // Closure of PHP code (not a __call trampoline's) or the name of a
        // function of PHP code found before, entered here (`run_direct`).
        // Anything else goes the general way (`resolve_callable`,
        // `run_nested_at`).
        match callable.as_object() {
            Some(o) => {
                if let Some(data) = closure_data(o)
                    && let ClosureFunc::User(func) = &data.func
                    && data.trampoline.is_none()
                    && enters_directly(func, args)
                {
                    return self.run_direct(regs, func, Some((o, data)), args);
                }
            }
            None => {
                if let Some(f) = self.named_function(callable) {
                    let func = Rc::clone(&self.closures[f as usize]);
                    if enters_directly(&func, args) {
                        return self.run_direct(regs, &func, None, args);
                    }
                }
            }
        }
        let target = self.resolve_lent(regs, callable)?;
        self.call_target_with_regs(regs, target, args)
    }

    /// `func` run nested on `args` to its return, as `call_with_regs`'
    /// common case (`enters_directly`): the arguments placed by value
    /// above the running frame, the frame pushed and entered as
    /// call_method pushes a function's (from `closure`, the Closure
    /// called and what it holds: its `$this`, scope, called class and
    /// `use` values), and the loop run on it (`run_entered`).
    #[inline(never)]
    fn run_direct(&mut self, regs: &mut Vec<Value>, func: &Rc<Closure>, closure: Option<(&Rc<Object>, &ClosureData)>, args: &[Value]) -> Result<Value, Throw> {
        let n = args.len();
        let at = self.nested_at();
        self.nested_room(regs, at, n)?;
        for (r, a) in regs[at..at + n].iter_mut().zip(args) {
            *r = if a.is_reference() { a.deref() } else { a.clone() };
        }
        let (saved_pc, saved_floor) = (self.pc, self.frames.floor);
        let caller_pc = self.frames.last().map_or(0, |f| f.pc);
        let frame = self.frames.push(func, at, n, at as u32);
        match closure {
            Some((o, data)) => (frame.this, frame.scope, frame.called, frame.owner) = (data.this.clone(), data.scope.clone(), data.called.clone(), Some(Rc::clone(o))),
            None => (frame.this, frame.scope, frame.called, frame.owner) = (None, None, None, None),
        }
        crate::fast::bind(&mut self.frames, regs, func.shape, at, n);
        if let Some((_, data)) = closure
            && !func.code.bound.is_empty()
        {
            for (r, v) in func.code.bound.iter().zip(data.captured.borrow().iter()) {
                regs[at + *r as usize] = v.clone();
            }
        }
        self.run_entered(regs, at, (saved_pc, saved_floor, caller_pc))
    }

    /// call_user_func_array(`callable`, `list`) from a builtin (it holds
    /// the lent registers): the list's values the arguments, its string
    /// keys named ones (`user_call_named`).
    pub(crate) fn call_list_from_builtin(&mut self, callable: &Value, list: &PhpArray) -> Result<Value, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = if names_arguments(list) {
            self.call_named_with_regs(&mut regs, callable, list)
        } else {
            // References in the list stay ones: the callee takes them where
            // its parameters do.
            let mut args: Vec<Value> = list.values().cloned().collect();
            self.call_with_regs(&mut regs, callable, &mut args)
        };
        self.isolate.regs = regs;
        r
    }

    /// `call_list_from_builtin` of a list that names arguments.
    #[cold]
    #[inline(never)]
    fn call_named_with_regs(&mut self, regs: &mut Vec<Value>, callable: &Value, list: &PhpArray) -> Result<Value, Throw> {
        let mut target = self.resolve_lent(regs, callable)?;
        let mut args = self.user_call_named(&mut target, list)?;
        self.call_target_with_regs(regs, target, &mut args)
    }

    /// What `callable` calls, the registers lent meanwhile: a class it
    /// names is autoloaded there (zend_is_callable_ex), not by running it
    /// again.
    #[inline(always)]
    fn resolve_lent(&mut self, regs: &mut Vec<Value>, callable: &Value) -> Result<Target, Throw> {
        std::mem::swap(&mut self.isolate.regs, regs);
        let target = self.resolve_callable(callable);
        std::mem::swap(&mut self.isolate.regs, regs);
        target
    }

    /// `target` called on `args` above the running frames, whose
    /// registers `regs` is.
    #[inline(always)]
    fn call_target_with_regs(&mut self, regs: &mut Vec<Value>, target: Target, args: &mut [Value]) -> Result<Value, Throw> {
        let m = match target {
            Target::Builtin(i) => {
                // A builtin that calls back finds the registers lent; it is
                // running, for its messages' prefix and stack traces.
                for (n, a) in args.iter_mut().enumerate() {
                    if a.is_reference() && !self.isolate.registry.by_ref(i, n) {
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
        let at = self.nested_at();
        self.run_nested_at(regs, m, args, at)
    }

    /// Where a nested run's registers start: above the running frame's
    /// and any nested run's result.
    fn nested_at(&self) -> usize {
        let at = match self.frames.last() {
            Some(top) => top.base as usize + top.func.code.nregs as usize,
            None => 0,
        };
        at.max(self.result_reg)
    }

    fn run_nested_at(&mut self, regs: &mut Vec<Value>, mut m: MethodFrame, args: &mut [Value], at: usize) -> Result<Value, Throw> {
        // __call / __callStatic: the arguments become (name, [arguments]).
        if let Some(name) = m.trampoline.take() {
            let packed: Vec<Value> = args.iter().map(Value::deref).collect();
            let mut repacked = [Value::str(name), Value::array(p8_runtime::array::PhpArray::from_list(packed))];
            return self.run_nested_at(regs, m, &mut repacked, at);
        }
        self.nested_room(regs, at, args.len())?;
        let by_ref = |n: usize| match &m.func {
            Code::User(f) => f.code.param_by_ref(n),
            Code::Native(..) => false,
        };
        // A value where the parameter takes a reference: PHP warns
        // (zend_call_function) and passes the value. (Undef: a parameter
        // named arguments skipped, its default to run.)
        if let Code::User(f) = &m.func {
            for (n, a) in args.iter().enumerate() {
                if by_ref(n) && !a.is_reference() && !a.is_undef() {
                    self.by_value_warning(&f.code, n);
                }
            }
        }
        for (n, a) in args.iter().enumerate() {
            regs[at + n] = if a.is_reference() && by_ref(n) { a.clone() } else { a.deref() };
        }
        self.run_placed(regs, m, args.len(), at)
    }

    /// Room for a nested run at `at` with `argc` arguments: Err when the
    /// native stack is near its end (each nested run takes some).
    #[inline(always)]
    fn nested_room(&mut self, regs: &mut Vec<Value>, at: usize, argc: usize) -> Result<(), Throw> {
        if self.stack_exhausted() {
            return Err(self.throw(
                "Error",
                format!("Maximum call stack size of {STACK_LIMIT} bytes (zend.max_allowed_stack_size - zend.reserved_stack_size) reached. Infinite recursion?"),
            ));
        }
        if regs.len() < at + argc + 1 {
            regs.resize(at + argc + 1, Value::UNDEF);
        }
        Ok(())
    }

    /// A value given for `code`'s parameter `n`, which takes a reference:
    /// PHP's warning (zend_call_function).
    #[cold]
    fn by_value_warning(&mut self, code: &Function, n: usize) {
        let name = match &code.class {
            Some(class) => format!("{}::{}", text(class.as_bytes()), text(code.name.as_bytes())),
            None => text(code.name.as_bytes()),
        };
        let param = code.params.get(n).or(code.params.last()).map(|p| text(p.name.as_bytes())).unwrap_or_default();
        self.warn(format!("{name}(): Argument #{} (${param}) must be passed by reference, value given", n + 1));
    }

    /// The nested run of `m` on the `argc` arguments placed at `at` (see
    /// `run_nested_at`), to its return: its result. (In line: it is
    /// `run_nested_at`'s body, which every callback's run takes.)
    #[inline(always)]
    fn run_placed(&mut self, regs: &mut Vec<Value>, m: MethodFrame, argc: usize, at: usize) -> Result<Value, Throw> {
        let argc = argc as u8;
        let (saved_pc, saved_floor) = (self.pc, self.frames.floor);
        let caller_pc = self.frames.last().map_or(0, |f| f.pc);
        // The callee's result lands in its first register (written after
        // its window is cleared).
        let next = self.call_method(regs, m, at, argc, at as u32, caller_pc as usize)?;
        if let Next::Go(_) = next {
            // Nothing to run (a generator function made its Generator).
            return Ok(std::mem::replace(&mut regs[at], Value::UNDEF));
        }
        self.run_entered(regs, at, (saved_pc, saved_floor, caller_pc))
    }

    /// The loop run on the frame just pushed for a nested run at `at`, to
    /// its return (its result in register `at`), then the caller's pc,
    /// the loop's floor and `self.pc` (`saved`, as they were before the
    /// push) put back.
    #[inline(always)]
    fn run_entered(&mut self, regs: &mut Vec<Value>, at: usize, (saved_pc, saved_floor, caller_pc): (usize, usize, u32)) -> Result<Value, Throw> {
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
            regs[at] = Value::UNDEF;
        }
        r?;
        let v = std::mem::replace(&mut regs[at], Value::UNDEF);
        Ok(v)
    }

    /// call_user_func_array's list when it names arguments
    /// ([`names_arguments`]): the arguments in `target`'s parameter order,
    /// as zend_call_function places fci->named_params (`place_args`, its
    /// errors a call's, not a spread's), a value given where a parameter
    /// of PHP code takes a reference warned about and passed in a
    /// reference of its own. For __call / __callStatic (the target's
    /// trampoline taken): the method's name and the list, keys kept.
    #[cold]
    #[inline(never)]
    fn user_call_named(&mut self, target: &mut Target, list: &PhpArray) -> Result<Vec<Value>, Throw> {
        if let Target::Frame(m) = target
            && let Some(name) = m.trampoline.take()
        {
            let mut packed = PhpArray::new();
            for (k, v) in list.iter() {
                packed.insert(k, v.deref());
            }
            return Ok(vec![Value::str(name), Value::array(packed)]);
        }
        let mut placed = self.place_args(target, list, false)?;
        if let Target::Frame(MethodFrame { func: Code::User(f), .. }) = &*target {
            let mut position = 0;
            for (k, v) in list.iter() {
                let n = match k {
                    Key::Int(_) => {
                        position += 1;
                        position - 1
                    }
                    Key::Str(name) => match self.named_position(target, name.as_bytes()) {
                        Some(n) => n,
                        None => continue,
                    },
                };
                if f.code.param_by_ref(n) && !v.is_reference() {
                    self.by_value_warning(&f.code, n);
                    if let Some(p) = placed.get_mut(n) {
                        *p = Value::new_reference(std::mem::replace(p, Value::UNDEF));
                    }
                }
            }
        }
        Ok(placed)
    }

    /// A call's argument array (`...`, named arguments) in parameter
    /// order: string keys are named arguments. A skipped optional
    /// parameter is Undef for a user function (its default runs) or its
    /// default for a builtin. `unpacking`: the list is a `...` spread (its
    /// errors say so).
    pub(crate) fn place_args(&mut self, target: &Target, list: &PhpArray, unpacking: bool) -> Result<Vec<Value>, Throw> {
        if !list.iter().any(|(k, _)| matches!(k, Key::Str(_))) {
            return Ok(list.values().map(Value::deref).collect());
        }
        let mut extra = PhpArray::new();
        let mut out: Vec<Value> = Vec::new();
        let mut set: Vec<bool> = Vec::new();
        let mut named = false;
        for (k, v) in list.iter() {
            match k {
                // (ZEND_SEND_UNPACK, zend_call_function: a position means
                // nothing after a name.)
                Key::Int(_) if named => {
                    let message = format!("Cannot use positional argument after named argument{}", if unpacking { " during unpacking" } else { "" });
                    return Err(self.throw("Error", message));
                }
                Key::Int(_) => {
                    out.push(self.arg_or_ref(target, out.len(), v));
                    set.push(true);
                }
                Key::Str(key) => {
                    named = true;
                    let Some(i) = self.named_position(target, key.as_bytes()) else {
                        let (fname, params) = self.named_params(target);
                        if params.last().is_some_and(|p| p.variadic) {
                            match target {
                                // A variadic user function collects it by name.
                                Target::Frame(_) => {
                                    extra.insert(Key::Str(key.clone()), v.deref());
                                    continue;
                                }
                                // A variadic builtin takes none by a name of its
                                // own (Z_PARAM_VARIADIC), but those that pass
                                // them on (Z_PARAM_VARIADIC_WITH_NAMED).
                                Target::Builtin(_) if !matches!(fname.as_str(), "call_user_func" | "forward_static_call") => {
                                    return Err(self.throw("ArgumentCountError", format!("Internal function {fname}() does not accept named variadic arguments")));
                                }
                                Target::Builtin(_) => {}
                            }
                        }
                        return Err(self.throw("Error", format!("Unknown named parameter ${}", text(key.as_bytes()))));
                    };
                    if i < set.len() && set[i] {
                        return Err(self.throw("Error", format!("Named parameter ${} overwrites previous argument", text(key.as_bytes()))));
                    }
                    if out.len() <= i {
                        out.resize(i + 1, Value::UNDEF);
                        set.resize(i + 1, false);
                    }
                    out[i] = self.arg_or_ref(target, i, v);
                    set[i] = true;
                }
            }
        }
        // The holes: optional parameters left out (their list made only
        // when there is one).
        if set.iter().all(|&s| s) {
            if !extra.is_empty() {
                self.named_extra = Some((self.frames.len() + 1, extra));
            }
            return Ok(out);
        }
        // A user function's: a constant default put in place (it has no code
        // to run it), another left to its code; looked at in its parameters.
        if let Target::Frame(MethodFrame { func: Code::User(clo), .. }) = target {
            for i in 0..out.len() {
                if set[i] {
                    continue;
                }
                let p = &clo.code.params[i];
                if !(p.has_default || p.variadic) {
                    continue;
                }
                if let Some(Some(d)) = clo.defaults.get(i) {
                    out[i] = d.clone();
                }
                set[i] = true;
            }
            if set.iter().all(|&s| s) {
                if !extra.is_empty() {
                    self.named_extra = Some((self.frames.len() + 1, extra));
                }
                return Ok(out);
            }
        }
        let (fname, params) = self.named_params(target);
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
    /// What messages call `target` (`f`, `C::m`) and its parameters' names.
    pub(crate) fn callable_params(&self, target: &Target) -> (String, Vec<String>) {
        let (name, params) = self.named_params(target);
        (name, params.into_iter().map(|p| p.name).collect())
    }

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
            return Some(Value::int(acc));
        }
        Some(match t {
            "null" | "NULL" => Value::NULL,
            "true" => Value::TRUE,
            "false" => Value::FALSE,
            "[]" => Value::array(PhpArray::new()),
            _ if (t.starts_with('"') && t.ends_with('"') || t.starts_with('\'') && t.ends_with('\'')) && t.len() >= 2 => Value::str(PhpStr::from(&unquote(t)[..])),
            _ => {
                if let Ok(i) = t.parse::<i64>() {
                    Value::int(i)
                } else if let Ok(x) = t.parse::<f64>() {
                    Value::float(x)
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
        let r = self.why_not_callable(v, true);
        self.failed_call = saved;
        // Not lent: the class stays unloaded, and a later lookup asks the
        // autoloaders again.
        if let Err(Throw::Autoload(name)) = &r {
            self.autoload_tried.remove(&name.as_bytes().to_ascii_lowercase());
        }
        matches!(r, Ok(None))
    }

    /// Whether `v` can be called from the running code, as
    /// zend_is_callable_at_frame decides: None if so, else its reason
    /// (`function "f" not found or invalid function name`, `class "C"
    /// not found`, `class C does not have a method "m"`, `non-static
    /// method C::m() cannot be called statically`, `cannot access private
    /// method C::m()`, …), which TypeErrors quote. A class it names is
    /// looked up as `find_class` does (an autoloader's exception, or
    /// Throw::Autoload where the registers are not lent, comes back as Err).
    /// `warn`: report the deprecations of `self`/`parent`/`static` and of
    /// `["C", "P::m"]` callables (as is_callable() and the checks of
    /// arguments do; a call itself does not).
    pub(crate) fn why_not_callable(&mut self, v: &Value, warn: bool) -> Result<Option<String>, Throw> {
        let v = v.deref();
        match v.kind() {
            Kind::Str => self.callable_func(v.as_str().expect("a string").as_bytes(), None, warn),
            Kind::Array => {
                let a = v.as_array().expect("an array");
                if a.len() != 2 {
                    return Ok(Some("array callback must have exactly two members".into()));
                }
                let (Some(holder), Some(method)) = (a.get_int(0).map(Value::deref), a.get_int(1).map(Value::deref)) else {
                    return Ok(Some("array callback has to contain indices 0 and 1".into()));
                };
                let holder = if let Some(class) = holder.as_str() {
                    match self.callable_class(class.as_bytes(), None, warn)? {
                        Ok(holder) => holder,
                        Err(why) => return Ok(Some(why)),
                    }
                } else {
                    match holder.into_object() {
                        Ok(o) => (Rc::clone(&o.class), Some(o)),
                        Err(_) => return Ok(Some("first array member is not a valid class name or object".into())),
                    }
                };
                let Ok(method) = method.into_str() else {
                    return Ok(Some("second array member is not a valid method".into()));
                };
                self.callable_func(method.as_bytes(), Some(holder), warn)
            }
            // A Closure, or an object with __invoke (zend_std_get_closure).
            Kind::Object if v.as_object().is_some_and(|o| closure_data(o).is_some() || o.class.method(b"__invoke").is_some()) => Ok(None),
            _ => Ok(Some("no array or string given".into())),
        }
    }

    /// The class a callable names (zend_is_callable_check_class): `self`,
    /// `parent`, `static` from the running code, or a class by name; with
    /// the object the call would have (the running `$this`, when it is of
    /// that class from code of that class). Err: why not.
    #[allow(clippy::type_complexity)] // (class, object) or the reason; used here only
    fn callable_class(&mut self, name: &[u8], within: Option<&Rc<Class>>, warn: bool) -> Result<Result<(Rc<Class>, Option<Rc<Object>>), String>, Throw> {
        // `self` and `parent` are of the array's class in `["C", "parent::m"]`.
        let (scope, called) = match (within, self.scope()) {
            (Some(c), (_, called)) => (Some(Rc::clone(c)), called),
            (None, found) => found,
        };
        let this = self.frames.last().and_then(|f| f.this.clone());
        let lower = name.to_ascii_lowercase();
        let keyword = matches!(lower.as_slice(), b"self" | b"parent" | b"static");
        let found = match lower.as_slice() {
            b"self" => match scope {
                Some(s) => Ok((s, this)),
                None => Err("cannot access \"self\" when no class scope is active".into()),
            },
            b"parent" => match scope {
                None => Err("cannot access \"parent\" when no class scope is active".into()),
                Some(s) => match s.parent.clone() {
                    Some(p) => Ok((p, this)),
                    None => Err("cannot access \"parent\" when current class scope has no parent".into()),
                },
            },
            b"static" => match called {
                Some(c) => Ok((c, this)),
                None => Err("cannot access \"static\" when no class scope is active".into()),
            },
            _ => match self.load_class(name)? {
                Some(class) => {
                    // `$this` goes with it from code of a class that is (or
                    // extends) the one named, on an instance of that code's class.
                    let object = match &scope {
                        Some(s) if s.is_a(&class.lower) => this.filter(|t| t.class.is_a(&s.lower)),
                        _ => None,
                    };
                    Ok((class, object))
                }
                None => Err(format!("class \"{}\" not found", text(name))),
            },
        };
        if warn && keyword && found.is_ok() {
            self.diags.push(p8_runtime::ops::Diag::Deprecated(format!("Use of \"{}\" in callables is deprecated", text(&lower))));
            self.report_diags();
        }
        Ok(found)
    }

    /// A callable's function or method (zend_is_callable_check_func):
    /// `name` alone (a function, or "Class::method"), or the method `name`
    /// of the class (and object) an array callable holds.
    fn callable_func(&mut self, name: &[u8], holder: Option<(Rc<Class>, Option<Rc<Object>>)>, warn: bool) -> Result<Option<String>, Throw> {
        if holder.is_none() && self.function_named(name.strip_prefix(b"\\").unwrap_or(name)).is_some() {
            return Ok(None);
        }
        // "Class::method" (the last `::`), else the holder's method.
        let split = name.windows(2).rposition(|w| w == b"::");
        let (class, object, method, strict) = match (split, holder) {
            (Some(0), _) => return Ok(Some("invalid function name".into())),
            (Some(at), holder) => {
                // (Under an array's class, `self` and kin are not reported.)
                let within = holder.as_ref().map(|(c, _)| Rc::clone(c));
                let (class, object) = match self.callable_class(&name[..at], within.as_ref(), warn && holder.is_none())? {
                    Ok(found) => found,
                    Err(why) => return Ok(Some(why)),
                };
                if let Some((org, _)) = &holder {
                    if !org.is_a(&class.lower) {
                        return Ok(Some(format!("class {} is not a subclass of {}", text(org.name.as_bytes()), text(class.name.as_bytes()))));
                    }
                    if warn {
                        self.diags.push(p8_runtime::ops::Diag::Deprecated(format!("Callables of the form [\"{}\", \"{}\"] are deprecated", text(org.name.as_bytes()), text(name))));
                        self.report_diags();
                    }
                }
                let object = holder.and_then(|(_, o)| o).or(object);
                (class, object, &name[at + 2..], true)
            }
            (None, Some((class, object))) => (class, object, name, false),
            (None, None) => return Ok(Some(format!("function \"{}\" not found or invalid function name", text(name)))),
        };
        let key = self.method_key(method);
        let found = if strict {
            crate::classes::method_of(&class, key).cloned()
        } else {
            // Code of a class naming its own private method on one of its
            // instances means that one: the method a call would run.
            crate::classes::called_method(&class, self.frames.last().and_then(|f| f.scope.as_deref()), key).cloned()
        };
        // __call / __callStatic answer what is not there, or not visible.
        let magic = |c: &Class, object: bool| c.method(if object { b"__call" } else { b"__callstatic" }).is_some();
        let Some(m) = found else {
            let this_fits = || self.frames.last().and_then(|f| f.this.as_ref()).is_some_and(|t| t.class.is_a(&class.lower));
            let via = if object.is_some() { magic(&class, true) } else { magic(&class, false) || (magic(&class, true) && this_fits()) };
            return Ok(if via { None } else { Some(format!("class {} does not have a method \"{}\"", text(class.name.as_bytes()), text(method))) });
        };
        let shown = || format!("{}::{}()", text(class.name.as_bytes()), text(m.name.as_bytes()));
        // A public method needs no visibility check (the common case).
        let accessible = m.visibility == p8_runtime::object::Visibility::Public || {
            let declaring = self.declaring(&class, &m.declaring);
            let root = self.access_class(&declaring, &m, key);
            let scope = self.scope().0;
            p8_runtime::object::visible(m.visibility, &root, scope.as_deref())
        };
        if !accessible && magic(&class, object.is_some()) {
            return Ok(None);
        }
        if m.is_abstract {
            return Ok(Some(format!("cannot call abstract method {}", shown())));
        }
        if object.is_none() && !m.is_static {
            return Ok(Some(format!("non-static method {} cannot be called statically", shown())));
        }
        if !accessible {
            return Ok(Some(format!("cannot access {} method {}", m.visibility.word(), shown())));
        }
        Ok(None)
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
    if let ClosureFunc::Builtin(_, b) = &data.func {
        // A closure of a builtin function: its name as declared, and its
        // parameters as its stub has them.
        let sig = p8_stubs::lookup(b.name.as_bytes());
        out.insert(key("function"), Value::str(sig.map_or(b.name, |s| s.name)));
        if let Some(sig) = sig.filter(|s| !s.params.is_empty()) {
            let mut params = PhpArray::new();
            for p in sig.params {
                let name = format!("{}${}", if p.by_ref { "&" } else { "" }, p.name);
                params.insert(Key::from_string(PhpStr::from(name)), Value::str(if p.optional() { "<optional>" } else { "<required>" }));
            }
            out.insert(key("parameter"), Value::array(params));
        }
        return out;
    }
    let Some(func) = data.func.user().map(|f| &f.code) else { return out };
    if data.fake && data.trampoline.is_none() && !func.name.as_bytes().starts_with(b"{closure") {
        // A closure of a named function or method (ZEND_ACC_FAKE_CLOSURE).
        let name = match &data.scope {
            Some(scope) => format!("{}::{}", scope.name.to_string_lossy(), String::from_utf8_lossy(func.name.as_bytes())),
            None => String::from_utf8_lossy(func.name.as_bytes()).into_owned(),
        };
        out.insert(key("function"), Value::str(PhpStr::from(name)));
    } else {
        out.insert(key("name"), Value::str(PhpStr::from(func.name.as_bytes())));
        out.insert(key("file"), Value::str(PhpStr::from(&*func.file)));
        out.insert(key("line"), Value::int(i64::from(func.line)));
    }
    let captured = data.captured.borrow();
    if !captured.is_empty() {
        let mut vars = PhpArray::new();
        for (r, v) in func.bound.iter().zip(captured.iter()) {
            if v.is_undef() {
                continue;
            }
            let name = func.cv_names[*r as usize].clone();
            vars.insert(Key::from_string(PhpStr::from(name.as_bytes())), v.clone());
        }
        out.insert(key("static"), Value::array(vars));
    }
    if let Some(this) = &data.this {
        out.insert(key("this"), Value::object(Rc::clone(this)));
    }
    if !func.params.is_empty() {
        let mut params = PhpArray::new();
        for p in &func.params {
            let name = format!("{}${}", if p.by_ref { "&" } else { "" }, String::from_utf8_lossy(p.name.as_bytes()));
            let kind = if p.has_default || p.variadic { "<optional>" } else { "<required>" };
            params.insert(Key::from_string(PhpStr::from(name)), Value::str(kind));
        }
        out.insert(key("parameter"), Value::array(params));
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

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
use p8_runtime::object::{Class, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// What a Closure object holds.
pub(crate) struct ClosureData {
    /// What it runs: user code, or (from a builtin's name) a builtin.
    pub func: Result<Rc<Closure>, usize>,
    pub is_static: bool,
    pub this: Option<Rc<Object>>,
    pub scope: Option<Rc<Class>>,
    pub called: Option<Rc<Class>>,
    /// `use` values (references for `&$x`), in the function's `bound` order.
    pub captured: RefCell<Vec<Value>>,
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
        let data = ClosureData { func: Ok(closure), is_static, this, scope, called, captured: RefCell::new(Vec::new()) };
        let class = Rc::clone(&self.classes[b"closure".as_slice()]);
        Object::with_data(class, &self.ids, Some(Rc::new(data)))
    }

    /// What calling a Closure object calls.
    fn closure_target(&self, o: &Object) -> Option<Target> {
        let data = closure_data(o)?;
        Some(match &data.func {
            Ok(func) => Target::Frame(MethodFrame {
                func: Code::User(Rc::clone(func)),
                this: data.this.clone(),
                scope: data.scope.clone(),
                called: data.called.clone(),
                captured: data.captured.borrow().clone(),
            }),
            Err(builtin) => Target::Builtin(*builtin),
        })
    }

    fn new_closure(&self, data: ClosureData) -> Value {
        let class = Rc::clone(&self.classes[b"closure".as_slice()]);
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
                    match self.classes.get(&name.as_bytes().to_ascii_lowercase()) {
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
                func: Err(i),
                is_static: true,
                this: None,
                scope: None,
                called: None,
                captured: RefCell::new(Vec::new()),
            },
            Target::Frame(MethodFrame { func: Code::User(func), this, scope, called, captured }) => ClosureData {
                func: Ok(func),
                is_static: this.is_none(),
                this,
                scope,
                called,
                captured: RefCell::new(captured),
            },
            Target::Frame(MethodFrame { func: Code::Native(_), .. }) => {
                return Err(self.throw("Error", "Cannot make a closure of a builtin class's method yet"));
            }
        };
        Ok(self.new_closure(data))
    }

    /// What `callable` calls, with PHP's error when it is not callable.
    pub(crate) fn resolve_callable(&mut self, callable: &Value) -> Result<Target, Throw> {
        match callable.deref() {
            Value::Object(o) => {
                if let Some(target) = self.closure_target(&o) {
                    return Ok(target);
                }
                if o.class.method(b"__invoke").is_some() {
                    return self.method_frame(&Value::Object(Rc::clone(&o)), b"__invoke", b"__invoke").map(Target::Frame);
                }
                Err(self.throw("Error", "Object of type ".to_string() + &o.class.name.to_string_lossy() + " is not callable"))
            }
            Value::Str(name) => self.resolve_name(name.as_bytes()),
            Value::Array(a) if a.len() == 2 => {
                let (Some(target), Some(Value::Str(method))) = (a.get_int(0).map(Value::deref), a.get_int(1).map(Value::deref)) else {
                    return Err(self.throw("Error", "Array callback must have exactly two elements"));
                };
                let lower = method.as_bytes().to_ascii_lowercase();
                match target {
                    Value::Object(_) => self.method_frame(&target, &lower, method.as_bytes()).map(Target::Frame),
                    Value::Str(class) => self.static_by_name(class.as_bytes(), &lower, method.as_bytes()),
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
            return self.static_by_name(&name[..at], &method.to_ascii_lowercase(), method);
        }
        let lower = name.to_ascii_lowercase();
        if let Some(&f) = self.functions.get(&lower) {
            let func = Rc::clone(&self.closures[f as usize]);
            return Ok(Target::Frame(MethodFrame { func: Code::User(func), this: None, scope: None, called: None, captured: Vec::new() }));
        }
        match self.isolate.registry.lookup(&lower) {
            Some(i) => Ok(Target::Builtin(i)),
            None => Err(self.throw("Error", format!("Call to undefined function {}()", text(name)))),
        }
    }

    /// `"Class::method"` or `['Class', 'method']`.
    fn static_by_name(&mut self, class: &[u8], lower: &[u8], display: &[u8]) -> Result<Target, Throw> {
        let class = class.strip_prefix(b"\\").unwrap_or(class);
        let target = self.find_class(class)?;
        let Some(method) = target.method(lower).cloned() else {
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
        let scope = self.classes.get(&method.declaring).cloned().unwrap_or_else(|| Rc::clone(&target));
        let called = this.as_ref().map(|t| Rc::clone(&t.class)).unwrap_or(target);
        let this = if method.is_static { None } else { this };
        Ok(Target::Frame(MethodFrame::method(self.code_of(&method), this, scope, called)))
    }

    /// CallValue: call `callee` with the `argc` arguments at `args`
    /// (absolute registers), its result to `ret`.
    pub(crate) fn call_value(&mut self, regs: &mut Vec<Value>, callee: &Value, args: usize, argc: u8, ret: u32, pc: usize) -> Result<Next, Throw> {
        match self.resolve_callable(callee)? {
            Target::Frame(m) => {
                // Arguments arrive by value: a reference is read through.
                for r in &mut regs[args..args + argc as usize] {
                    if let Value::Ref(cell) = r {
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
        let target = self.resolve_callable(callable)?;
        let m = match target {
            Target::Builtin(i) => {
                // A builtin that calls back finds the registers lent.
                std::mem::swap(&mut self.isolate.regs, regs);
                let r = self.run_builtin(i, args);
                std::mem::swap(&mut self.isolate.regs, regs);
                return r;
            }
            Target::Frame(MethodFrame { func: Code::Native(native), this, .. }) => {
                let mut passed: Vec<Value> = args.iter().map(Value::deref).collect();
                return self.run_native(native, this.as_ref(), &mut passed);
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
        self.run_nested_at(regs, m, args, at)
    }

    fn run_nested_at(&mut self, regs: &mut Vec<Value>, m: MethodFrame, args: &mut [Value], at: usize) -> Result<Value, Throw> {
        if regs.len() < at + args.len() + 1 {
            regs.resize(at + args.len() + 1, Value::Undef);
        }
        let by_ref = |n: usize| match &m.func {
            Code::User(f) => f.code.param_by_ref(n),
            Code::Native(_) => false,
        };
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
        let r = self.execute(regs);
        self.frames.floor = saved_floor;
        self.pc = saved_pc;
        if let Some(caller) = self.frames.last_mut() {
            caller.pc = caller_pc;
        }
        r?;
        let v = std::mem::replace(&mut regs[at], Value::Undef);
        Ok(v)
    }

    /// Whether a value is callable (is_callable, no autoloading).
    pub(crate) fn callable(&mut self, v: &Value) -> bool {
        let saved = std::mem::take(&mut self.failed_call);
        let ok = self.resolve_callable(v).is_ok();
        self.failed_call = saved;
        ok
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
    let func = match &data.func {
        Ok(f) => &f.code,
        Err(_) => return out,
    };
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

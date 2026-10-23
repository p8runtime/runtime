//! How builtins plug into p8: an [`Extension`] registers its functions and
//! constants into a [`Registry`] once per process (the Platform's), and a
//! builtin reaches the running request only through [`Ctx`].
//!
//! The registry is immutable after startup and `Send + Sync`, so every
//! isolate on every thread shares one.

use crate::object::Class;
use crate::string::PhpStr;
use crate::value::Value;
use std::collections::HashMap;
use std::rc::Rc;

/// Something that ends the normal flow: a thrown error, or exit().
#[derive(Debug, Clone)]
pub enum Throw {
    Error { class: &'static str, message: String, file: Rc<str>, line: u32 },
    /// A fatal error: not an exception, nothing catches it.
    Fatal { message: String, file: Rc<str>, line: u32 },
    /// A class that is not declared, which autoloaders may declare: the
    /// engine runs them and the instruction again.
    Autoload(PhpStr),
    /// A file that does not parse (an included one).
    Parse { message: String, file: Rc<str>, line: u32 },
    Exit(i32),
    /// A thrown object (a Throwable): what `throw` throws, and what an
    /// Error becomes once the engine makes its object.
    Object(Rc<crate::object::Object>),
}

/// What a builtin can see and do of the request running it.
pub trait Ctx {
    /// Write to the output (through any output buffers).
    fn echo(&mut self, bytes: &[u8]);
    /// ob_start(): start an output buffer.
    fn ob_start(&mut self);
    /// ob_end_clean(): drop the innermost buffer; false if there is none.
    fn ob_end_clean(&mut self) -> bool;
    /// Report a warning at the current line.
    fn warn(&mut self, message: &str);
    /// Report a deprecation at the current line.
    fn deprecated(&mut self, message: &str);
    /// Report a diagnostic of PHP error `level` (E_USER_NOTICE, E_WARNING,
    /// ...) at the current line, if error_reporting includes it.
    fn report(&mut self, level: i64, message: &str);
    /// A fatal error at the current line: the request ends with it.
    fn fatal(&self, message: String) -> Throw;
    /// ini_get(): the setting's current value as a string (null when PHP
    /// has no value for it), or None for an unknown directive.
    fn ini_get(&self, name: &[u8]) -> Option<Value>;
    /// ini_set(): set a known directive for this request; its old value,
    /// or None for an unknown directive.
    fn ini_set(&mut self, name: &[u8], value: Value) -> Option<Value>;
    /// error_reporting(): the current level, after setting `new` if given.
    fn error_level(&mut self, new: Option<i64>) -> i64;
    /// The constant `name` (as written), defined by the script or an extension.
    fn constant(&self, name: &[u8]) -> Option<Value>;
    /// define(): false when `name` is already defined.
    fn define(&mut self, name: &[u8], value: Value) -> bool;
    /// Whether a function `name` exists (any case, optional leading `\`).
    fn function_exists(&self, name: &[u8]) -> bool;
    /// The running user function's arguments past its parameters.
    fn extra_args(&self) -> &[Value];
    /// A Throw with PHP's error `class` at the current line.
    fn throw(&self, class: &'static str, message: String) -> Throw;
    /// The class `name` (any case, optional leading `\`), if declared.
    fn class(&self, name: &[u8]) -> Option<Rc<Class>>;
    /// The class the running code is in (None outside any class).
    fn scope(&self) -> Option<Rc<Class>>;
    /// Call a PHP callable (only from a builtin that takes a callable:
    /// the engine lends it what running PHP code needs).
    fn call(&mut self, callable: &Value, args: &mut [Value]) -> Result<Value, Throw>;
    /// is_callable(): whether `v` names something that can be called.
    fn is_callable(&mut self, v: &Value) -> bool;
    /// A copy of a Closure bound to `new_this` (null: unbound) and the
    /// class scope `scope` names (None: its own).
    fn bind_closure(&mut self, closure: &Rc<crate::object::Object>, new_this: Value, scope: Option<Value>) -> Result<Value, Throw>;
    /// Closure::fromCallable: a Closure calling `callable`.
    fn closure_from(&mut self, callable: &Value) -> Result<Value, Throw>;
    /// A new object of class `class` (declared), its constructor not run
    /// (unserialize, json_decode's stdClass).
    fn new_object(&mut self, class: &Rc<Class>) -> Rc<crate::object::Object>;
    /// The calling code's arguments (func_get_args).
    fn call_args(&self) -> Vec<Value>;
    /// The calling code's variable `name` (None: not set).
    fn local(&self, name: &[u8]) -> Option<Value>;
    /// Set the calling code's variable `name`; false when it has none of
    /// that name (a function's variables are known when it is compiled).
    fn set_local(&mut self, name: &[u8], value: Value) -> bool;
    /// The calling code's set variables, in order (get_defined_vars).
    fn locals(&self) -> Vec<(PhpStr, Value)>;
    /// The calls that led here, as debug_backtrace() gives them.
    fn backtrace(&self) -> crate::array::PhpArray;
    /// get_included_files(): every file run, in order.
    fn included_files(&self) -> Vec<String>;
    /// Per-request state an extension keeps (json_last_error, ...).
    fn state(&mut self, key: &'static str) -> &mut Value;
    /// A new resource (its number is the next one).
    fn new_resource(&mut self, resource: crate::stream::Resource) -> Value;
    /// Write to the host directly, past output buffers (PHP's STDOUT).
    fn write_stdout(&mut self, bytes: &[u8]);
    /// The class `name`, after the autoloaders had their go (an Err of
    /// Throw::Autoload asks the engine to run them and the call again).
    fn class_load(&mut self, name: &[u8]) -> Result<Option<Rc<Class>>, Throw>;
    /// The class `static::` names in the running code.
    fn called_class(&self) -> Option<Rc<Class>>;
    /// The class a static builtin method (an enum's cases()) was called on.
    fn native_class(&self) -> Option<Rc<Class>>;
    /// An enum's case object.
    fn enum_case(&mut self, class: &Rc<Class>, case: &[u8]) -> Value;
    /// set_error_handler / restore_error_handler (`handler` None): the
    /// handler before.
    fn error_handler(&mut self, handler: Option<(Value, i64)>) -> Value;
    /// set_exception_handler / restore_exception_handler (None).
    fn exception_handler(&mut self, handler: Option<Value>) -> Value;
    /// spl_autoload_register (`Some(prepend)`) or unregister (None):
    /// whether it was done.
    fn autoloader(&mut self, callable: Value, register: Option<bool>) -> bool;
    /// spl_autoload_functions()
    fn autoloaders(&self) -> Vec<Value>;
    /// register_shutdown_function()
    fn on_shutdown(&mut self, callable: Value, args: Vec<Value>);
    /// class_alias(): false when the alias is taken.
    fn class_alias(&mut self, class: Rc<Class>, alias: &[u8]) -> bool;
    /// Every (key, value) going through `v` gives (an array, a generator,
    /// an Iterator or IteratorAggregate), as foreach does.
    fn iterate(&mut self, v: &Value) -> Result<Vec<(Value, Value)>, Throw>;
    /// A Generator method (`op`: current, key, next, send, valid, rewind,
    /// getreturn, throw).
    fn generator(&mut self, generator: &Rc<crate::object::Object>, op: &str, arg: Value) -> Result<Value, Throw>;
}

/// A builtin function: arguments arrive by value (moved out of the caller's
/// registers); a by-reference parameter receives the variable's reference cell.
pub type Run = fn(&mut dyn Ctx, &mut [Value]) -> Result<Value, Throw>;

pub struct Builtin {
    /// Lowercase.
    pub name: &'static str,
    pub run: Run,
    /// Parameters passed by reference.
    pub by_ref: &'static [usize],
}

/// A constant's value as an extension declares it.
#[derive(Debug, Clone, Copy)]
pub enum Constant {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'static [u8]),
}

impl Constant {
    pub fn to_value(self) -> Value {
        match self {
            Constant::Null => Value::Null,
            Constant::Bool(b) => Value::Bool(b),
            Constant::Int(i) => Value::Int(i),
            Constant::Float(f) => Value::Float(f),
            Constant::Str(s) => Value::Str(PhpStr::from(s)),
        }
    }
}

/// A set of builtins: `register` adds them to a registry at startup.
pub struct Extension {
    pub name: &'static str,
    pub register: fn(&mut Registry),
}

/// Every builtin function and constant of the process's extensions.
#[derive(Default)]
pub struct Registry {
    /// ini directives PHP knows, with their built-in defaults.
    ini: HashMap<&'static [u8], Option<&'static [u8]>>,
    functions: Vec<&'static Builtin>,
    /// Each function's stub signature (its argument checks), by index.
    sigs: Vec<Option<&'static p8_stubs::Sig>>,
    /// "Function f() is deprecated…" for the functions PHP deprecates.
    deprecations: Vec<Option<String>>,
    /// Whether each function takes a callable (and so may run PHP code).
    reenters: Vec<bool>,
    index: HashMap<&'static str, usize>,
    constants: HashMap<&'static [u8], Constant>,
}

impl Registry {
    pub fn new(extensions: &[Extension]) -> Registry {
        let mut registry = Registry::default();
        for extension in extensions {
            (extension.register)(&mut registry);
        }
        registry
    }

    pub fn add_functions(&mut self, functions: &'static [Builtin]) {
        for f in functions {
            let i = self.functions.len();
            self.functions.push(f);
            let sig = p8_stubs::lookup(f.name.as_bytes());
            self.sigs.push(sig);
            self.deprecations.push(sig.and_then(p8_stubs::deprecation));
            // A callable, or a Traversable (gone through by running its code).
            self.reenters.push(sig.is_some_and(|s| {
                s.params.iter().any(|p| p.mask & p8_stubs::T_CALLABLE != 0 || p.classes.iter().any(|c| c.eq_ignore_ascii_case("Traversable")))
            }));
            self.index.insert(f.name, i);
        }
    }

    /// An ini directive and its default (None: PHP has no value for it).
    pub fn add_ini(&mut self, name: &'static str, default: Option<&'static [u8]>) {
        self.ini.insert(name.as_bytes(), default);
    }

    /// An ini directive's default: None if PHP has no such directive.
    pub fn ini_default(&self, name: &[u8]) -> Option<Option<&'static [u8]>> {
        self.ini.get(name).copied()
    }

    pub fn add_constant(&mut self, name: &'static [u8], value: Constant) {
        self.constants.insert(name, value);
    }

    /// The builtin named `name` (lowercase).
    pub fn lookup(&self, name: &[u8]) -> Option<usize> {
        let name = std::str::from_utf8(name).ok()?;
        self.index.get(name).copied()
    }

    pub fn function(&self, i: usize) -> &'static Builtin {
        self.functions[i]
    }

    /// Builtin `i`'s signature from PHP's stubs, which its arguments are
    /// checked against before it runs.
    pub fn sig(&self, i: usize) -> Option<&'static p8_stubs::Sig> {
        self.sigs[i]
    }

    /// The deprecation PHP reports on every call of builtin `i`, if any.
    pub fn deprecation(&self, i: usize) -> Option<&str> {
        self.deprecations[i].as_deref()
    }

    /// Whether parameter `n` of builtin `i` is passed by reference.
    /// Whether builtin `i` may call back into PHP code (it takes a
    /// callable): the engine lends it the registers.
    pub fn reenters(&self, i: usize) -> bool {
        self.reenters[i]
    }

    /// Builtins whose stub signature is not checked (assert: the compiler
    /// passes it the assertion's source as an extra argument).
    pub fn unchecked(&mut self, names: &[&str]) {
        for n in names {
            if let Some(&i) = self.index.get(n) {
                self.sigs[i] = None;
            }
        }
    }

    /// Builtins that run PHP code or reach the caller's variables though
    /// they take no callable (serialize, compact): the registers are lent.
    pub fn mark_reenters(&mut self, names: &[&str]) {
        for n in names {
            if let Some(&i) = self.index.get(n) {
                self.reenters[i] = true;
            }
        }
    }

    pub fn by_ref(&self, i: usize, n: usize) -> bool {
        self.functions[i].by_ref.contains(&n)
    }

    pub fn constant(&self, name: &[u8]) -> Option<Constant> {
        self.constants.get(name).copied()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn registry_is_shared_across_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<super::Registry>();
    }
}

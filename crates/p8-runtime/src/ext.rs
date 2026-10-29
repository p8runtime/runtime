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
    /// An exception the engine makes: `props` are what the object gets
    /// besides its message, file, line and trace (a code, a subclass's own
    /// property), set as code in the class sees them.
    Error { class: &'static str, message: String, file: Rc<str>, line: u32, props: Vec<(&'static str, Value)> },
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

/// The output buffer operations (ob_*).
pub enum Ob {
    /// ob_start($callback, $chunk_size): a new buffer (its handler, if any).
    Start { handler: Value, chunk: i64 },
    /// ob_get_contents()
    Contents,
    /// ob_get_length()
    Length,
    /// ob_get_level()
    Level,
    /// ob_clean(): drop the innermost buffer's contents.
    Clean,
    /// ob_flush(): pass them on (through the handler).
    Flush,
    /// ob_end_flush() / ob_end_clean(): remove the innermost buffer,
    /// passing its contents on or not.
    End { flush: bool },
    /// ob_list_handlers()
    Handlers,
    /// ob_get_status($full_status)
    Status { full: bool },
}

/// A response's head as the script builds it (header(), setcookie(),
/// http_response_code()), for a host that sends it (a web server). A host
/// that does not (the CLI) leaves `server` false: the header functions
/// still check what they are given and keep the response code, but keep no
/// header.
#[derive(Debug, Clone, Default)]
pub struct Response {
    /// The host sends headers (PHP's cli-server SAPI, not the CLI one).
    pub server: bool,
    /// The response code; 0 until set (the CLI has none until a script sets one).
    pub status: i64,
    /// A status line the script gave (`header("HTTP/1.1 404 Not Found")`),
    /// used as it is instead of the default for `status`.
    pub status_line: Option<Vec<u8>>,
    /// The header lines (`Name: value`), in the order set.
    pub headers: Vec<Vec<u8>>,
    /// No Content-Type header was set: the default one goes out with the
    /// head (`text/html; charset=UTF-8`; None: none).
    pub default_content_type: Option<Vec<u8>>,
    /// The `default_charset` setting, added to a `text/*` Content-Type
    /// that has no charset.
    pub default_charset: Vec<u8>,
    /// A `Location` header under an HTTP/1.1 request that is not GET or
    /// HEAD answers 303 rather than 302.
    pub redirect_see_other: bool,
}

/// What a builtin can see and do of the request running it.
pub trait Ctx {
    /// The response being built (see [`Response`]).
    fn response(&mut self) -> &mut Response;
    /// flush(): what was echoed reaches the host now, and with it the head.
    fn flush_output(&mut self) {}
    /// Write to the output (through any output buffers).
    fn echo(&mut self, bytes: &[u8]);
    /// An output buffer operation (the ob_* functions); false when there
    /// is no buffer to act on.
    fn ob(&mut self, op: Ob) -> Result<Value, Throw>;
    /// Report a warning at the current line.
    fn warn(&mut self, message: &str);
    /// Report a deprecation at the current line.
    fn deprecated(&mut self, message: &str);
    /// Report a notice at the current line.
    fn notice(&mut self, message: &str);
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
    /// error_get_last(): the last error not handled by a user handler, as
    /// (type, message, file, line); error_clear_last() when `clear`.
    fn last_error(&mut self, _clear: bool) -> Option<(i64, String, String, u32)> {
        None
    }
    /// Where output first reached the host (file, line; an empty file when
    /// no code was running), once it has: the headers are sent then.
    fn output_started(&self) -> Option<(Rc<str>, u32)> {
        None
    }
    /// error_reporting(): the current level, after setting `new` if given.
    fn error_level(&mut self, new: Option<i64>) -> i64;
    /// The constant `name` (as written), defined by the script or an extension.
    fn constant(&self, name: &[u8]) -> Option<Value>;
    /// define(): false when `name` is already defined.
    fn define(&mut self, name: &[u8], value: Value) -> bool;
    /// Whether a function `name` exists (any case, optional leading `\`).
    fn function_exists(&self, name: &[u8]) -> bool;
    /// The name the running builtin was called by (an alias's own name:
    /// gzread, not fread), for its messages.
    fn builtin_name(&self) -> &'static str;
    /// The PHP modules the process's extensions provide (extension_loaded,
    /// get_loaded_extensions), in the order they registered.
    fn modules(&self) -> &[&'static str];
    /// The running user function's arguments past its parameters.
    fn extra_args(&self) -> &[Value];
    /// A Throw with PHP's error `class` at the current line.
    fn throw(&self, class: &'static str, message: String) -> Throw;
    /// The class `name` (any case, optional leading `\`), if declared.
    fn class(&mut self, name: &[u8]) -> Option<Rc<Class>>;
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
    /// The value of `o`'s property at `slot` as `$o->name` reads it here:
    /// through its get hook (from a builtin marked as reentering); None
    /// when it has none (unset, or virtual without a get hook).
    fn prop_value(&mut self, o: &Rc<crate::object::Object>, slot: usize) -> Result<Option<Value>, Throw>;
    /// Reflection: a function's or method's declaration (None: none such).
    fn function_info(&mut self, callee: &crate::reflect::Callee) -> Option<crate::reflect::FnInfo>;
    /// Reflection: call it (whatever its visibility), with `this` for a method.
    fn invoke(&mut self, callee: &crate::reflect::Callee, this: Option<Rc<crate::object::Object>>, args: &mut [Value]) -> Result<Value, Throw>;
    /// `new C(...args)` as the engine makes it (the instantiation checks,
    /// computed defaults), its constructor run when `construct`.
    fn new_instance(&mut self, class: &Rc<Class>, args: &mut [Value], construct: bool) -> Result<Rc<crate::object::Object>, Throw>;
    /// The value each property slot of `class` starts with in a new
    /// object, constant-expression defaults computed (Undef: none).
    fn class_defaults(&mut self, class: &Rc<Class>) -> Result<Vec<Value>, Throw>;
    /// Reflection: a Closure of it (bound to `this` for a method).
    fn closure_of(&mut self, callee: &crate::reflect::Callee, this: Option<Rc<crate::object::Object>>) -> Result<Value, Throw>;
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
    /// Every class, interface, trait and enum, as get_declared_classes()
    /// lists them: the built-in ones (by name), then the script's in the
    /// order it declared them; no aliases.
    fn declared_classes(&self) -> Vec<Rc<Class>>;
    /// class_alias(): false when the alias is taken.
    fn class_alias(&mut self, class: Rc<Class>, alias: &[u8]) -> bool;
    /// Every (key, value) going through `v` gives (an array, a generator,
    /// an Iterator or IteratorAggregate), as foreach does.
    fn iterate(&mut self, v: &Value) -> Result<Vec<(Value, Value)>, Throw>;
    /// A Generator method (`op`: current, key, next, send, valid, rewind,
    /// getreturn, throw).
    fn generator(&mut self, generator: &Rc<crate::object::Object>, op: &str, arg: Value) -> Result<Value, Throw>;

    /// gc_collect_cycles(): garbage cycles freed; how many values they held.
    fn gc_collect(&mut self) -> Result<i64, Throw> {
        Ok(0)
    }

    /// gc_status()'s runs and collected.
    fn gc_counts(&self) -> (i64, i64) {
        (0, 0)
    }

    /// How long the request has been running (gc_status()'s application_time).
    fn running_for(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    /// The host's configuration (`php -d`) as get_cfg_var() sees it: every
    /// setting given, known to p8 or not, as given.
    fn cfg(&self) -> Vec<(Vec<u8>, String)> {
        Vec::new()
    }

    /// A global variable's value (for a builtin marked as reentering:
    /// the registers are lent to the context then).
    fn global_value(&self, _name: &[u8]) -> Option<Value> {
        None
    }

    /// The request's own $_SERVER['argv'] (not the script's copy of
    /// $_SERVER), when the host gave a command line and registers it.
    fn request_argv(&self) -> Option<Vec<String>> {
        None
    }

    /// Whether `source` parses: its parse error (message, line) if not.
    fn parse_check(&self, _source: &[u8]) -> Option<(String, u32)> {
        None
    }

    /// Fibers: start, resume, throw, suspend, getCurrent (op), for
    /// `fiber` (None for the static ones).
    fn fiber_op(&mut self, fiber: Option<&Rc<crate::object::Object>>, op: u8, args: &mut [Value]) -> Result<Value, Throw> {
        let _ = (fiber, op, args);
        Err(self.throw("Error", "Fibers are not supported here".into()))
    }

    /// Lazy objects (ReflectionClass::newLazyGhost and the rest): `obj`, or
    /// a new instance of `class`, made lazy.
    fn lazy_make(&mut self, class: &Rc<Class>, obj: Option<Rc<crate::object::Object>>, initializer: Value, options: i64, proxy: bool) -> Result<Rc<crate::object::Object>, Throw> {
        let _ = (class, obj, initializer, options, proxy);
        Err(self.throw("Error", "Lazy objects are not supported here".into()))
    }

    /// A lazy object initialized: the object its accesses go to.
    fn lazy_initialize(&mut self, o: &Rc<crate::object::Object>) -> Result<Rc<crate::object::Object>, Throw> {
        Ok(Rc::clone(o))
    }

    /// A lazy object marked initialized (lazy properties take defaults).
    fn lazy_mark_initialized(&mut self, o: &Rc<crate::object::Object>) -> Result<(), Throw> {
        let _ = o;
        Ok(())
    }

    /// A lazy object's slot stops being lazy (skipLazyInitialization).
    fn lazy_slot_done(&mut self, o: &Rc<crate::object::Object>, slot: usize) {
        let _ = (o, slot);
    }
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
    /// Classes extensions declare, in order (a parent before its children).
    classes: Vec<&'static ClassSpec>,
    /// Native classes' per-object state makers, by class name.
    data: HashMap<&'static str, crate::object::MakeData>,
    /// Native classes' cleanups on destruction, by class name.
    dtors: HashMap<&'static str, crate::object::NativeMethod>,
    /// Native classes' object handlers, by class name.
    hooks: HashMap<&'static str, ClassHooks>,
    /// The PHP modules provided ("standard", "json", ...), in order.
    modules: Vec<&'static str>,
}

/// A class an extension declares: its methods are Rust. The engine links
/// it with the core classes, after them (so it may extend Exception).
/// A native class's object handlers (zend_object_handlers' get_properties
/// and cast_object): see [`crate::object::Class`].
#[derive(Clone, Copy, Default)]
pub struct ClassHooks {
    pub get_properties: Option<fn(&crate::object::Object) -> crate::array::PhpArray>,
    pub cast_bool: Option<fn(&crate::object::Object) -> bool>,
    pub cast_string: Option<fn(&crate::object::Object) -> crate::string::PhpStr>,
    pub clone_data: Option<crate::object::CloneData>,
    pub forward_calls: Option<crate::object::ForwardCalls>,
}

pub struct ClassSpec {
    pub name: &'static str,
    pub parent: Option<&'static str>,
    pub interfaces: &'static [&'static str],
    /// CLASS_FINAL, CLASS_INTERFACE, ... (crate::object)
    pub flags: u32,
    /// Declared properties: name, default, visibility.
    pub properties: &'static [(&'static str, Constant, crate::object::Visibility)],
    /// Methods (None: abstract, for an interface).
    pub methods: &'static [(&'static str, Option<crate::object::Native>)],
    pub constants: &'static [(&'static str, Constant)],
    /// What var_dump shows instead of the properties.
    pub debug_info: Option<fn(&crate::object::Object) -> crate::array::PhpArray>,
}

impl Registry {
    pub fn new(extensions: &[Extension]) -> Registry {
        let mut registry = Registry::default();
        for extension in extensions {
            (extension.register)(&mut registry);
        }
        registry
    }

    pub fn add_classes(&mut self, classes: &'static [ClassSpec]) {
        self.classes.extend(classes.iter());
    }

    /// Declare a PHP module (as extension_loaded names it) provided.
    pub fn add_module(&mut self, name: &'static str) {
        self.modules.push(name);
    }

    /// The modules provided, in the order they were added.
    pub fn modules(&self) -> &[&'static str] {
        &self.modules
    }

    /// Give every new object of native class `class` (and its
    /// subclasses) the state `make` makes.
    pub fn add_data(&mut self, class: &'static str, make: crate::object::MakeData) {
        self.data.insert(class, make);
    }

    /// Run `dtor` when an object of native class `class` (or a
    /// subclass) is destroyed, before any __destruct.
    pub fn add_dtor(&mut self, class: &'static str, dtor: crate::object::NativeMethod) {
        self.dtors.insert(class, dtor);
    }

    /// Object handlers for native class `class` (and its subclasses).
    pub fn add_hooks(&mut self, class: &'static str, hooks: ClassHooks) {
        self.hooks.insert(class, hooks);
    }

    /// The handlers `add_hooks` registered for `class`.
    pub fn class_hooks(&self, class: &str) -> Option<ClassHooks> {
        self.hooks.get(class).copied()
    }

    /// The cleanup `add_dtor` registered for `class`.
    pub fn native_dtor(&self, class: &str) -> Option<crate::object::NativeMethod> {
        self.dtors.get(class).copied()
    }

    /// The state maker `add_data` registered for `class`.
    pub fn data_maker(&self, class: &str) -> Option<crate::object::MakeData> {
        self.data.get(class).copied()
    }

    /// The classes extensions declare, in order.
    pub fn classes(&self) -> &[&'static ClassSpec] {
        &self.classes
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

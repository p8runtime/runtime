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
    /// Whether a user error handler takes errors of `level` now (it runs
    /// after the builtin returns).
    fn user_handles(&self, level: i64) -> bool;
    /// A fatal error at the current line: the request ends with it.
    fn fatal(&self, message: String) -> Throw;
    /// ini_get(): the setting's current value as a string (null when PHP
    /// has no value for it), or None for an unknown directive.
    fn ini_get(&self, name: &[u8]) -> Option<Value>;
    /// ini_set(): set a known directive for this request; its old value,
    /// or None for an unknown directive.
    fn ini_set(&mut self, name: &[u8], value: Value) -> Option<Value>;
    /// A directive's master value (phpinfo()'s "Master Value"): the host's
    /// setting for it, else its built-in default; None for an unknown
    /// directive.
    fn ini_master(&self, name: &[u8]) -> Option<Value> {
        self.ini_get(name)
    }
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
    /// The compile cache the host keeps, as Zend OPcache sees it (None:
    /// the host keeps none).
    fn code_cache(&self) -> Option<std::sync::Arc<dyn crate::opcache::CodeCache>> {
        None
    }
    /// opcache_compile_file(): compile `path` (resolved as include does)
    /// into the cache without running it. Ok(false): it could not be
    /// opened (warned as include warns) or did not compile.
    fn compile_file(&mut self, path: &[u8]) -> Result<bool, Throw> {
        let _ = path;
        Ok(false)
    }
    /// What fetching builtin constant `name` raises if it is deprecated
    /// (Registry::deprecate_constant).
    fn constant_deprecation(&self, name: &[u8]) -> Option<&'static str> {
        let _ = name;
        None
    }
    /// define(): false when `name` is already defined.
    fn define(&mut self, name: &[u8], value: Value) -> bool;
    /// Set the constant `name`, whether or not it is defined: for the one
    /// PHP changes while it runs (SID).
    fn set_constant(&mut self, name: &[u8], value: Value);
    /// Whether a function `name` exists (any case, optional leading `\`).
    fn function_exists(&self, name: &[u8]) -> bool;
    /// get_defined_functions(): the builtins' names, and the script's
    /// functions (lowercase) in the order declared.
    fn defined_functions(&self) -> (Vec<&'static str>, Vec<Vec<u8>>);
    /// get_defined_constants(): each constant with its module (PHP's
    /// extension names; "user" for the script's own), builtins first.
    fn defined_constants(&self) -> Vec<(String, PhpStr, Value)>;
    /// Where `include name` would find its file (include_path, then the
    /// running file's directory), as a canonical path.
    fn resolve_include(&self, name: &str) -> Option<std::path::PathBuf>;
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
    /// Keep `t` for the engine to raise once the running builtin returns:
    /// how a builtin on the borrowed ABI ([`Fast`]) ends with an error.
    /// Use [`Ctx::fault`]; this is the storage under it.
    fn set_fault(&mut self, t: Throw);
    /// The error side of a [`Fast`] builtin's result: `t` waits in the
    /// context, the builtin returns `Err(ctx.fault(t))`. (A [`Fault`] is
    /// made only here, so an `Err` always has its Throw.)
    fn fault(&mut self, t: Throw) -> Fault {
        self.set_fault(t);
        Fault(())
    }
    /// [`Ctx::throw`] as a [`Fault`]: the error `class` raised at the
    /// current line.
    fn fault_with(&mut self, class: &'static str, message: String) -> Fault {
        let t = self.throw(class, message);
        self.fault(t)
    }
    /// The class `name` (any case, optional leading `\`), if declared.
    fn class(&mut self, name: &[u8]) -> Option<Rc<Class>>;
    /// The class the running code is in (None outside any class).
    fn scope(&self) -> Option<Rc<Class>>;
    /// Call a PHP callable (only from a builtin that takes a callable:
    /// the engine lends it what running PHP code needs).
    fn call(&mut self, callable: &Value, args: &mut [Value]) -> Result<Value, Throw>;
    /// is_callable(): whether `v` names something that can be called.
    fn is_callable(&mut self, v: &Value) -> bool;
    /// Why `v` cannot be called from the running code, as PHP words it
    /// (zend_is_callable's error: `function "f" not found or invalid
    /// function name`, …); None when it can.
    fn why_not_callable(&mut self, v: &Value) -> Option<String>;
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
    /// The calls that led here, as debug_backtrace() gives them (with each
    /// method call's `object` when `objects`): the innermost `limit` of
    /// them, or all when `limit` is 0. The walk stops at the limit (it may
    /// give one entry more: a builtin's entry comes with its caller's).
    fn backtrace(&self, objects: bool, limit: usize) -> crate::array::PhpArray;
    /// get_included_files(): every file run, in order.
    fn included_files(&self) -> Vec<String>;
    /// Per-request state an extension keeps (json_last_error, ...).
    fn state(&mut self, key: &'static str) -> &mut Value;
    /// A new resource (its number is the next one).
    fn new_resource(&mut self, resource: crate::stream::Resource) -> Value;
    /// Write to the host directly, past output buffers (PHP's STDOUT).
    fn write_stdout(&mut self, bytes: &[u8]);
    /// The class `name`, after the autoloaders had their go: they run
    /// right here, as zend_lookup_class runs them, so this builtin's frame
    /// is below theirs (from a builtin marked as reentering, or a method:
    /// the registers are lent; anything else is an engine error).
    fn class_load(&mut self, name: &[u8]) -> Result<Option<Rc<Class>>, Throw>;
    /// The value of `o`'s property at `slot` as `$o->name` reads it here:
    /// through its get hook (from a builtin marked as reentering); None
    /// when it has none (unset, or virtual without a get hook).
    fn prop_value(&mut self, o: &Rc<crate::object::Object>, slot: usize) -> Result<Option<Value>, Throw>;
    /// A class constant's value, its code run first if it has not run yet
    /// (as `C::NAME` does); None when the class has no such constant.
    fn class_constant(&mut self, class: &Rc<Class>, name: &[u8]) -> Result<Option<Value>, Throw>;
    /// Reflection: a function's or method's declaration (None: none such).
    fn function_info(&mut self, callee: &crate::reflect::Callee) -> Option<crate::reflect::FnInfo>;
    /// Reflection: the declaration of method `m` of `class` (found
    /// already: `function_info` of a method callee without the lookup).
    fn method_info(&mut self, class: &Rc<Class>, m: &crate::object::Method) -> crate::reflect::FnInfo;
    /// Reflection: a function's static variables (a closure's `use` values
    /// first), by name: their values now, null for one not yet initialized.
    fn static_variables(&mut self, callee: &crate::reflect::Callee) -> crate::array::PhpArray;
    /// Reflection: a closure's `use` variables, by name (empty for anything
    /// else): getClosureUsedVariables().
    fn closure_used_variables(&mut self, _callee: &crate::reflect::Callee) -> crate::array::PhpArray {
        crate::array::PhpArray::new()
    }
    /// Reflection: call it (whatever its visibility), with `this` for a method.
    fn invoke(&mut self, callee: &crate::reflect::Callee, this: Option<Rc<crate::object::Object>>, args: &mut [Value]) -> Result<Value, Throw>;
    /// `new C(...args)` as the engine makes it (the instantiation checks,
    /// computed defaults), its constructor run when `construct`.
    fn new_instance(&mut self, class: &Rc<Class>, args: &mut [Value], construct: bool) -> Result<Rc<crate::object::Object>, Throw>;
    /// `new C(...args)` with the arguments placed by name (string keys)
    /// and position, as a call with named arguments does.
    fn new_instance_named(&mut self, class: &Rc<Class>, args: &crate::array::PhpArray) -> Result<Rc<crate::object::Object>, Throw>;
    /// A declaration's doc comment, attribute names and lines (reflection).
    fn reflect_meta(&mut self, subject: &crate::reflect::Subject) -> crate::reflect::Meta;
    /// The arguments of attribute `index` of a declaration, evaluated:
    /// named ones keyed by name.
    fn reflect_attr_args(&mut self, subject: &crate::reflect::Subject, index: usize) -> Result<crate::array::PhpArray, Throw>;
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
    /// get_error_handler (PHP 8.5): the handler in effect, or null.
    fn current_error_handler(&self) -> Value;
    /// get_exception_handler (PHP 8.5): the handler in effect, or null.
    fn current_exception_handler(&self) -> Value;
    /// spl_autoload_register (`Some(prepend)`) or unregister (None):
    /// whether it was done.
    fn autoloader(&mut self, callable: Value, register: Option<bool>) -> bool;
    /// spl_autoload_functions()
    fn autoloaders(&self) -> Vec<Value>;
    /// register_shutdown_function()
    fn on_shutdown(&mut self, callable: Value, args: Vec<Value>);
    /// A shutdown function kept under `name`: set (replacing the one of
    /// that name where it stands in the list, else appended) with
    /// `Some`, removed with `None` (PHP's register_user_shutdown_function
    /// and remove_user_shutdown_function, which ext/session uses).
    fn shutdown_named(&mut self, name: &'static str, entry: Option<(Value, Vec<Value>)>);
    /// Typed state an extension keeps for the request (dropped with it):
    /// the slot for `key`, empty until the extension fills it.
    fn ext_data(&mut self, key: &'static str) -> &mut Option<Box<dyn std::any::Any>>;
    /// The cell of global variable `name`, made (null) when the global is
    /// not set: what a reference to the global shares (for a builtin
    /// marked as reentering: the registers are lent to the context then).
    fn global_cell(&mut self, name: &[u8]) -> Rc<std::cell::RefCell<Value>>;
    /// The file and line the running code is at (an empty file when no
    /// code is running).
    fn location(&self) -> (Rc<str>, u32);
    /// Every class, interface, trait and enum, as get_declared_classes()
    /// lists them, each with the name it is listed by: the built-in ones
    /// (by name), then the script's in the order it declared them, an
    /// alias (class_alias) by its lowercase name.
    fn declared_classes(&self) -> Vec<(PhpStr, Rc<Class>)>;
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

    /// Closure::getCurrent(): the Closure object the calling code runs as.
    fn current_closure(&mut self) -> Result<Value, Throw> {
        Err(self.throw("Error", "Current function is not a closure".into()))
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

    /// clone($object, $withProperties) (ZEND_FUNCTION(clone)): a clone of
    /// `object`, its __clone run, then each of `with`'s properties
    /// assigned to it as the calling code would assign it.
    fn clone_with(&mut self, object: &Rc<crate::object::Object>, with: &crate::array::PhpArray) -> Result<Value, Throw> {
        let _ = (object, with);
        Err(self.throw("Error", "clone() is not supported here".into()))
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

/// An ini directive's update handler (PHP's OnUpdate): called before
/// ini_set() stores `new` for the directive `name`; false refuses it (the
/// handler has said why) and ini_set() returns false.
pub type IniCheck = fn(&mut dyn Ctx, &[u8], &PhpStr) -> bool;

/// Work at the start or the end of a request (PHP's RINIT and RSHUTDOWN).
/// The end hooks run after the shutdown functions, the destructors and the
/// output buffers; a diagnostic they raise is located "Unknown" on line 0,
/// as PHP words one from RSHUTDOWN.
pub type RequestHook = fn(&mut dyn Ctx) -> Result<(), Throw>;

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

/// The error of a [`Fast`] builtin: zero-sized, so `Result<Value, Fault>`
/// is a `Value`'s 16 bytes (the error in the tag's niche) and comes back
/// in two registers. What was thrown waits in the context: only
/// [`Ctx::fault`] makes one.
#[derive(Debug)]
pub struct Fault(());

/// A builtin on the borrowed ABI: its arguments are the caller's argument
/// registers, lent (no copy, nothing moved; a builtin that needs one owned
/// clones it), and it takes no parameter by reference. When the arguments
/// are plainly of their declared types (nothing to convert, nothing to
/// warn about) the call is frameless: the call site's arm calls the
/// function pointer directly, with no frame of either kind. Anything else
/// goes through the checked path first, which converts the arguments in
/// place and then lends them the same way.
pub type Fast = fn(&mut dyn Ctx, &[Value]) -> Result<Value, Fault>;

/// A builtin of the borrowed ABI, as an extension declares it.
pub struct FastBuiltin {
    /// Lowercase.
    pub name: &'static str,
    pub run: Fast,
}

/// A builtin's name and by-reference parameters, whichever ABI it has.
#[derive(Clone, Copy)]
pub struct Entry {
    /// Lowercase.
    pub name: &'static str,
    /// Parameters passed by reference.
    pub by_ref: &'static [usize],
}

/// How a builtin is called.
#[derive(Clone, Copy)]
pub enum Imp {
    /// The arguments moved out of the caller's registers ([`Run`]).
    Moved(Run),
    /// The arguments lent ([`Fast`]).
    Lent(Fast),
}

/// [`Ctx::fault`]'s Throw, for a [`Fast`] builtin calling code that
/// returns `Result<_, Throw>`: `helper(..).or_fault(ctx)?`.
pub trait OrFault<T> {
    fn or_fault(self, ctx: &mut dyn Ctx) -> Result<T, Fault>;
}

impl<T> OrFault<T> for Result<T, Throw> {
    #[inline]
    fn or_fault(self, ctx: &mut dyn Ctx) -> Result<T, Fault> {
        self.map_err(|t| ctx.fault(t))
    }
}

/// What calling a builtin needs, read from one place on every call: the
/// function, its stub signature (None: its arguments are not checked) with
/// the argument counts it takes ([`crate::args::counts`], worked out once),
/// whether PHP deprecates it (the message: [`Registry::deprecation`]) and
/// whether it may run PHP code (the registers are lent to it).
#[derive(Clone, Copy)]
pub struct Call {
    pub run: Imp,
    /// The function, when a call may be frameless (see [`frameless`]);
    /// the call site still checks that the arguments need no conversion.
    pub frameless: Option<Imp>,
    /// A frameless call may have object arguments: the builtin runs no PHP
    /// code on them ([`Registry::objects_inert`]).
    pub objects_ok: bool,
    /// The kinds each of the first [`PLAIN_ARGS`] parameters takes as they
    /// are ([`crate::args::kinds_taken`]): a frameless call's test is a
    /// bit test per argument.
    pub accepts: [u16; PLAIN_ARGS],
    pub sig: Option<&'static p8_stubs::Sig>,
    pub counts: (usize, usize),
    pub deprecated: bool,
    pub reenters: bool,
    /// Whether the builtin checks its callables itself, after work PHP
    /// does first (session_set_save_handler's deprecation): the engine
    /// does not check them before it runs.
    pub own_callbacks: bool,
    /// Its by-reference parameters take a value too (the stub's
    /// `@prefer-ref`: ZEND_SEND_PREFER_REF).
    pub prefer_ref: bool,
}

/// How many arguments a frameless call's test reads from [`Call::accepts`]
/// (a call passing more is tested from the signature).
pub const PLAIN_ARGS: usize = 4;

impl Call {
    /// Whether a frameless call may skip the checks for `args`: as many as
    /// the builtin takes, each plainly of its parameter's type (no
    /// conversion, warning or refusal), no reference, nothing undefined, an
    /// object only where it is inert ([`crate::args::plain_call`], read
    /// from the bits worked out at startup).
    #[inline(always)]
    pub fn plain(&self, args: &[Value]) -> bool {
        let (required, max) = self.counts;
        if args.len() < required || args.len() > max {
            return false;
        }
        if args.len() > PLAIN_ARGS {
            return self.sig.is_some_and(|sig| crate::args::plain_call(sig, self.counts, self.objects_ok, args));
        }
        args.iter().zip(self.accepts).all(|(a, bits)| (bits >> (a.kind() as u16)) & 1 != 0)
    }

    fn set_accepts(&mut self) {
        let Some(sig) = self.sig else { return };
        for (i, bits) in self.accepts.iter_mut().enumerate() {
            let param = sig.params.get(i).or_else(|| sig.params.last().filter(|p| p.variadic));
            *bits = param.map_or(0, |p| crate::args::kinds_taken(p, self.objects_ok));
        }
    }
}

/// Whether a call of `call` may be frameless: the builtin has a signature
/// (the call site checks the arguments against it), is not deprecated
/// (the notice is the checked path's) and cannot run PHP code. Either
/// ABI: a builtin of the moved one ([`Run`]) gets the registers mutably,
/// as the checked path's common case already gave them; one of the
/// borrowed ABI ([`Fast`]) reads them in place and answers in two
/// registers. (A by-reference argument is a reference cell, which a
/// frameless call never has: `str_replace`'s `$count` left out, the call
/// may be frameless; passed, it is the checked path's.)
fn frameless(call: &Call) -> Option<Imp> {
    (call.sig.is_some() && !call.deprecated && !call.reenters).then_some(call.run)
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
            Constant::Null => Value::NULL,
            Constant::Bool(b) => Value::bool(b),
            Constant::Int(i) => Value::int(i),
            Constant::Float(f) => Value::float(f),
            Constant::Str(s) => Value::str(PhpStr::from(s)),
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
    /// Where log_errors writes an error (php_log_err: the error_log ini's
    /// file, dated, or the SAPI's log), as the standard extension does it.
    error_logger: Option<fn(&mut dyn Ctx, &[u8])>,
    functions: Vec<Entry>,
    /// What a call of each function reads, by index (see [`Call`]).
    calls: Vec<Call>,
    /// "Function f() is deprecated…" for the functions PHP deprecates.
    deprecations: Vec<Option<String>>,
    /// The functions by name (lowercase).
    index: crate::fast::HashMap<crate::symbol::Sym, usize>,
    /// The constants by name (namespace lowercased, see `const_key`).
    constants: crate::fast::HashMap<crate::symbol::Sym, Constant>,
    /// The constants in the order they were added, each with its module
    /// (get_defined_constants(true) groups by it: "Core", "pcre", ...).
    constant_order: Vec<(&'static str, &'static [u8], crate::symbol::Sym)>,
    /// What fetching a deprecated constant says (its #[\Deprecated]: "Constant
    /// E_STRICT is deprecated since 8.4, the error level was removed"), by
    /// its key's symbol.
    constant_deprecations: crate::fast::HashMap<crate::symbol::Sym, &'static str>,
    /// The extension registering now (its constants' module, by default).
    module: &'static str,
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
    /// ini directives with an update handler, by name.
    ini_checks: HashMap<&'static [u8], IniCheck>,
    request_start: Vec<RequestHook>,
    request_end: Vec<RequestHook>,
    startup_checks: Vec<fn(&mut dyn Ctx)>,
    /// Every name compiled code and the builtins use (see [`crate::symbol`]).
    symbols: std::sync::Arc<crate::symbol::Symbols>,
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
    pub cast_int: Option<fn(&crate::object::Object) -> i64>,
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
            registry.module = extension.name;
            (extension.register)(&mut registry);
        }
        registry
    }

    pub fn add_classes(&mut self, classes: &'static [ClassSpec]) {
        self.classes.extend(classes.iter());
    }

    /// Declare a PHP module (as extension_loaded names it) provided.
    pub fn set_error_logger(&mut self, log: fn(&mut dyn Ctx, &[u8])) {
        self.error_logger = Some(log);
    }

    pub fn error_logger(&self) -> Option<fn(&mut dyn Ctx, &[u8])> {
        self.error_logger
    }

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
            self.add_function(Entry { name: f.name, by_ref: f.by_ref }, Imp::Moved(f.run));
        }
    }

    /// Builtins of the borrowed ABI ([`Fast`]).
    pub fn add_fast(&mut self, functions: &'static [FastBuiltin]) {
        for f in functions {
            self.add_function(Entry { name: f.name, by_ref: &[] }, Imp::Lent(f.run));
        }
    }

    fn add_function(&mut self, entry: Entry, run: Imp) {
        let i = self.functions.len();
        self.functions.push(entry);
        let sig = p8_stubs::lookup(entry.name.as_bytes());
        let deprecation = sig.and_then(p8_stubs::deprecation);
        // A callable, or a Traversable (gone through by running its code).
        let reenters = sig.is_some_and(|s| {
            s.params.iter().any(|p| p.mask & p8_stubs::T_CALLABLE != 0 || p.classes.iter().any(|c| c.eq_ignore_ascii_case("Traversable")))
        });
        let mut call = Call {
            run,
            frameless: None,
            objects_ok: false,
            accepts: [0; PLAIN_ARGS],
            sig,
            counts: sig.map_or((0, 0), crate::args::counts),
            deprecated: deprecation.is_some(),
            reenters,
            own_callbacks: false,
            prefer_ref: false,
        };
        call.frameless = frameless(&call);
        call.set_accepts();
        self.calls.push(call);
        self.deprecations.push(deprecation);
        let sym = self.symbols.intern(entry.name.as_bytes());
        self.index.insert(sym, i);
    }

    /// The names' symbol table, shared by the compiler and every isolate.
    pub fn symbols(&self) -> &std::sync::Arc<crate::symbol::Symbols> {
        &self.symbols
    }

    /// An ini directive and its default (None: PHP has no value for it).
    pub fn add_ini(&mut self, name: &'static str, default: Option<&'static [u8]>) {
        self.ini.insert(name.as_bytes(), default);
    }

    /// Run `check` before ini_set() stores a value for the directive `name`.
    pub fn add_ini_check(&mut self, name: &'static str, check: IniCheck) {
        self.ini_checks.insert(name.as_bytes(), check);
    }

    /// The update handler of the directive `name`, if it has one.
    pub fn ini_check(&self, name: &[u8]) -> Option<IniCheck> {
        self.ini_checks.get(name).copied()
    }

    /// Run `hook` when a request starts (after the host's settings and the
    /// request's variables are set, before the script runs).
    pub fn add_request_start(&mut self, hook: RequestHook) {
        self.request_start.push(hook);
    }

    /// Run `hook` when a request ends (see [`RequestHook`]).
    pub fn add_request_end(&mut self, hook: RequestHook) {
        self.request_end.push(hook);
    }

    /// Say what PHP says at startup about the host's settings once they
    /// are all set (a deprecated one), as the standard extension does for
    /// assert.*; the checks run in the order extensions registered them.
    pub fn add_startup_check(&mut self, check: fn(&mut dyn Ctx)) {
        self.startup_checks.push(check);
    }

    pub fn startup_checks(&self) -> &[fn(&mut dyn Ctx)] {
        &self.startup_checks
    }

    pub fn request_start_hooks(&self) -> &[RequestHook] {
        &self.request_start
    }

    pub fn request_end_hooks(&self) -> &[RequestHook] {
        &self.request_end
    }

    /// An ini directive's default: None if PHP has no such directive.
    pub fn ini_default(&self, name: &[u8]) -> Option<Option<&'static [u8]>> {
        self.ini.get(name).copied()
    }

    pub fn add_constant(&mut self, name: &'static [u8], value: Constant) {
        self.add_constant_in(self.module, name, value);
    }

    /// Mark constant `name` deprecated, with the message fetching it raises.
    pub fn deprecate_constant(&mut self, name: &[u8], message: &'static str) {
        let sym = self.symbols.intern(&const_key(name));
        self.constant_deprecations.insert(sym, message);
    }

    /// What fetching the constant whose key is `sym` raises, if it is deprecated.
    pub fn constant_deprecation(&self, sym: crate::symbol::Sym) -> Option<&'static str> {
        self.constant_deprecations.get(&sym).copied()
    }

    /// A constant that belongs to `module` (PHP's name for its extension).
    pub fn add_constant_in(&mut self, module: &'static str, name: &'static [u8], value: Constant) {
        let sym = self.symbols.intern(&const_key(name));
        if self.constants.insert(sym, value).is_none() {
            self.constant_order.push((module, name, sym));
        }
    }

    /// Every constant with its module, in the order they were added.
    pub fn constant_list(&self) -> impl Iterator<Item = (&'static str, &'static [u8], Constant)> + '_ {
        self.constant_order.iter().filter_map(|&(module, name, sym)| self.constants.get(&sym).map(|c| (module, name, *c)))
    }

    /// The builtin named `name` (lowercase).
    pub fn lookup(&self, name: &[u8]) -> Option<usize> {
        self.symbols.find(name).and_then(|s| self.builtin(s))
    }

    /// The builtin whose (lowercase) name is `sym`.
    pub fn builtin(&self, sym: crate::symbol::Sym) -> Option<usize> {
        self.index.get(&sym).copied()
    }

    /// Every builtin's name, in the order registered.
    pub fn function_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.functions.iter().map(|f| f.name)
    }

    pub fn function(&self, i: usize) -> Entry {
        self.functions[i]
    }

    /// Builtin `i`'s signature from PHP's stubs, which its arguments are
    /// checked against before it runs.
    pub fn sig(&self, i: usize) -> Option<&'static p8_stubs::Sig> {
        self.calls[i].sig
    }

    /// What a call of builtin `i` reads (one entry, for the call path).
    #[inline]
    pub fn call(&self, i: usize) -> &Call {
        &self.calls[i]
    }

    /// The deprecation PHP reports on every call of builtin `i`, if any.
    pub fn deprecation(&self, i: usize) -> Option<&str> {
        self.deprecations[i].as_deref()
    }

    /// Whether builtin `i` may call back into PHP code (it takes a
    /// callable): the engine lends it the registers.
    pub fn reenters(&self, i: usize) -> bool {
        self.calls[i].reenters
    }

    /// Builtins whose stub signature is not checked (assert: the compiler
    /// passes it the assertion's source as an extra argument).
    pub fn unchecked(&mut self, names: &[&str]) {
        for n in names {
            if let Some(i) = self.lookup(n.as_bytes()) {
                self.calls[i].sig = None;
                self.calls[i].frameless = None;
            }
        }
    }

    /// Builtins that run PHP code or reach the caller's variables though
    /// they take no callable (serialize, compact): the registers are lent.
    pub fn mark_reenters(&mut self, names: &[&str]) {
        for n in names {
            if let Some(i) = self.lookup(n.as_bytes()) {
                self.calls[i].reenters = true;
                self.calls[i].frameless = None;
            }
        }
    }

    /// Builtins that run PHP code only on an object argument (count: a
    /// Countable's count()): the checked path lends them the registers,
    /// and a frameless call stays possible, since a frameless call has no
    /// object argument ([`crate::args::plain_call`]).
    pub fn mark_reenters_on_objects(&mut self, names: &[&str]) {
        for n in names {
            if let Some(i) = self.lookup(n.as_bytes()) {
                self.calls[i].reenters = true;
            }
        }
    }

    /// Builtins that only look at an object argument (its class, its id,
    /// whether it is one): no __toString(), __debugInfo() or other PHP
    /// code can run, so a frameless call may pass them objects.
    pub fn objects_inert(&mut self, names: &[&str]) {
        for n in names {
            if let Some(i) = self.lookup(n.as_bytes()) {
                self.calls[i].objects_ok = true;
                self.calls[i].set_accepts();
            }
        }
    }

    /// Builtins that check their callables themselves (PHP's parse their
    /// arguments by hand, after a deprecation or a choice of signature):
    /// the engine leaves their `callable` arguments to them.
    pub fn own_callbacks(&mut self, names: &[&str]) {
        for n in names {
            if let Some(i) = self.lookup(n.as_bytes()) {
                self.calls[i].own_callbacks = true;
            }
        }
    }

    /// Builtins whose by-reference parameters also take a value (the
    /// stubs' `@prefer-ref`: array_multisort's).
    pub fn mark_prefer_ref(&mut self, names: &[&str]) {
        for n in names {
            if let Some(i) = self.lookup(n.as_bytes()) {
                self.calls[i].prefer_ref = true;
                self.calls[i].frameless = None;
            }
        }
    }

    /// Whether builtin `i`'s by-reference parameter `n` must get a
    /// reference (not `@prefer-ref`).
    pub fn must_be_ref(&self, i: usize, n: usize) -> bool {
        self.by_ref(i, n) && !self.calls[i].prefer_ref
    }

    pub fn by_ref(&self, i: usize, n: usize) -> bool {
        self.functions[i].by_ref.contains(&n)
    }

    /// The constant `name` (as written; a namespace in any case).
    pub fn constant(&self, name: &[u8]) -> Option<Constant> {
        self.symbols.find_const(name).and_then(|s| self.constant_sym(s))
    }

    /// The constant whose key (see [`const_key`]) is `sym`.
    pub fn constant_sym(&self, sym: crate::symbol::Sym) -> Option<Constant> {
        self.constants.get(&sym).copied()
    }
}

/// A constant's key in the name tables: its namespace is case-insensitive,
/// its own name is not (`A\B\C` and `a\b\C` are one constant).
pub fn const_key(name: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    match name.iter().rposition(|&c| c == b'\\') {
        Some(at) if name[..at].iter().any(u8::is_ascii_uppercase) => std::borrow::Cow::Owned([name[..at].to_ascii_lowercase().as_slice(), &name[at..]].concat()),
        _ => std::borrow::Cow::Borrowed(name),
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

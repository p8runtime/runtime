//! Isolates and contexts: what a host creates, and where a request's
//! output goes.

use super::*;

/// Where a context's output goes: the embedder's side of a request (a
/// terminal, an HTTP response).
pub trait Host {
    fn write(&mut self, bytes: &[u8]);
    fn flush(&mut self) {}
    /// The response head, once, before the first output reaches the host
    /// (or at the end of the request when there is none): for a context a
    /// host made a web request ([`Context::serve`]).
    fn head(&mut self, response: &p8_runtime::ext::Response) {
        let _ = response;
    }
}

/// A function's `static` variables, by index. The function is held: its
/// address is the key, and an eval'd function dropped while its statics
/// were kept would hand them to the next function allocated there. PHP
/// keeps eval'd code until the request ends, too.
pub(crate) struct FunctionStatics {
    _func: Arc<p8_bytecode::Function>,
    pub(crate) cells: HashMap<u16, Rc<RefCell<Value>>>,
}

impl FunctionStatics {
    pub(crate) fn new(func: &Arc<p8_bytecode::Function>) -> FunctionStatics {
        FunctionStatics { _func: Arc::clone(func), cells: HashMap::default() }
    }
}

/// An output buffer (ob_start): its contents, and the handler (PHP code)
/// that turns them into what is passed on.
pub(crate) struct Buffer {
    pub(crate) data: Vec<u8>,
    pub(crate) handler: Option<Value>,
    pub(crate) chunk: usize,
    /// The handler was called (later calls lack the START phase).
    pub(crate) started: bool,
}

/// Output, with output buffers (ob_start) stacked on top of the host.
pub(crate) struct Output<'i> {
    pub(crate) buffers: Vec<Buffer>,
    pub(crate) host: &'i mut dyn Host,
    /// Some output has reached the host (the headers are sent).
    pub(crate) started: bool,
    /// The response head the script builds, and whether the host has it.
    pub(crate) response: p8_runtime::ext::Response,
    pub(crate) head_sent: bool,
}

impl Output<'_> {
    pub(crate) fn write(&mut self, bytes: &[u8]) {
        match self.buffers.last_mut() {
            Some(buffer) => buffer.data.extend_from_slice(bytes),
            None => self.write_host(bytes),
        }
    }

    pub(crate) fn write_host(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.send_head();
        }
        self.started |= !bytes.is_empty();
        self.host.write(bytes);
    }

    /// Hand the host the response head, once (a web request only): the
    /// default Content-Type joins the headers when the script set none.
    pub(crate) fn send_head(&mut self) {
        if !self.response.server || self.head_sent {
            return;
        }
        self.head_sent = true;
        if let Some(default) = self.response.default_content_type.take() {
            let mut line = b"Content-type: ".to_vec();
            line.extend_from_slice(&default);
            self.response.headers.push(line);
        }
        self.host.head(&self.response);
    }

    /// Write to the level under the innermost buffer.
    pub(crate) fn write_below(&mut self, bytes: &[u8]) {
        let n = self.buffers.len();
        match n.checked_sub(2).and_then(|i| self.buffers.get_mut(i)) {
            Some(below) => below.data.extend_from_slice(bytes),
            None => self.write_host(bytes),
        }
    }

    /// Everything buffered goes out as it is (a fatal error's report).
    pub(crate) fn flush_all(&mut self) {
        while let Some(buffer) = self.buffers.pop() {
            self.write(&buffer.data);
        }
        self.host.flush();
    }
}

/// How ob_list_handlers() names a buffer's handler.
pub(crate) fn handler_name(handler: &Option<Value>) -> String {
    match handler {
        None => "default output handler".into(),
        Some(Value::Str(s)) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
        Some(Value::Object(o)) if o.class.lower == b"closure" => "Closure::__invoke".into(),
        Some(Value::Array(a)) => {
            let parts: Vec<Value> = a.values().map(Value::deref).collect();
            let class = match parts.first() {
                Some(Value::Object(o)) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                Some(v) => String::from_utf8_lossy(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned(),
                None => String::new(),
            };
            let method = parts.get(1).map_or_else(String::new, |m| String::from_utf8_lossy(m.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned());
            format!("{class}::{method}")
        }
        Some(Value::Object(o)) => format!("{}::__invoke", String::from_utf8_lossy(o.class.name.as_bytes())),
        Some(_) => "???".into(),
    }
}

/// PHP_OUTPUT_HANDLER_* phases a handler is told.
pub(crate) const OB_START: i64 = 1;
pub(crate) const OB_CLEAN: i64 = 2;
pub(crate) const OB_FLUSH: i64 = 4;
pub(crate) const OB_FINAL: i64 = 8;

/// No interrupt pending.
pub(crate) const NO_INTERRUPT: u64 = 0;
/// InterruptHandle::terminate (other values: a timeout of that many seconds).
pub(crate) const TERMINATE: u64 = u64::MAX;

/// Stops the request an isolate is running, from any thread: what a host's
/// watchdog holds. The request ends with a fatal error at the next loop
/// iteration or call; the isolate stays usable for the next request.
#[derive(Clone)]
pub struct InterruptHandle(Arc<AtomicU64>);

impl InterruptHandle {
    /// End the request with PHP's max_execution_time error.
    pub fn timeout(&self, seconds: u64) {
        self.0.store(seconds.clamp(1, TERMINATE - 1), AtomicOrdering::Relaxed);
    }

    /// End the request (the host is shutting it down).
    pub fn terminate(&self) {
        self.0.store(TERMINATE, AtomicOrdering::Relaxed);
    }
}

/// One engine on one thread. It runs one context (request) at a time: a
/// context borrows it mutably for as long as it lives. What it keeps
/// between requests (the register file, buffers) stays allocated, so a warm
/// isolate starts the next request without allocating them again.
///
/// `!Send`: values are reference-counted without atomics, so an isolate
/// and everything in it stays on the thread that made it.
pub struct Isolate {
    /// The builtins and their constants (shared by every isolate).
    pub(crate) registry: Arc<Registry>,
    /// The running task's registers (task.rs; the request's first task's
    /// stay allocated between requests). While code runs, the interpreter
    /// holds the register file itself (taken out of here) so the hot loop
    /// works on a plain local slice.
    pub(crate) regs: Vec<Value>,
    /// Set by an InterruptHandle; cleared when a request starts.
    pub(crate) interrupt: Arc<AtomicU64>,
    /// The classes every request starts with.
    pub(crate) builtin_classes: builtin_classes::Table,
    /// The builtin classes declared when first wanted (reflection's, the
    /// extensions'), by name.
    pub(crate) on_demand: HashMap<Sym, &'static p8_runtime::ext::ClassSpec>,
    /// The symbols of classes the engine makes objects of itself.
    pub(crate) known: Known,
    /// The names this isolate looked up, by the bytes given (found with no
    /// lock, warm across requests); in front of the registry's symbols.
    pub(crate) names: Rc<p8_runtime::symbol::SymbolCache>,
    /// The ini defaults as strings, made once (ini_get() of a setting
    /// no request changed hands out the same string each time).
    pub(crate) ini_texts: RefCell<HashMap<(usize, usize), PhpStr>>,
    /// Compiled code's string and array constants as values, made once
    /// for the isolate (every request's closures and classes share them,
    /// as PHP's requests share opcache's interned literals): by the
    /// address of the constant's shared bytes or entries, the constant
    /// kept beside so the address stays its own. A value is never written
    /// in place (one holder more than a register's: a write copies).
    pub(crate) thawed: RefCell<HashMap<usize, (p8_runtime::frozen::Frozen, Value)>>,
    /// What a declared function's closure slot holds until the function
    /// is first resolved (sites.rs `Closure::unbuilt`).
    pub(crate) unbuilt: Rc<Closure>,
}

/// The names of classes the engine makes objects of itself (a closure, a
/// generator), looked up without going through the symbol table.
pub(crate) struct Known {
    pub(crate) closure: Sym,
    pub(crate) generator: Sym,
    pub(crate) stdclass: Sym,
    pub(crate) error: Sym,
    pub(crate) fiber: Sym,
}

impl Isolate {
    pub fn new(registry: Arc<Registry>) -> Isolate {
        let names = Rc::new(p8_runtime::symbol::SymbolCache::new(registry.symbols()));
        Isolate {
            regs: Vec::with_capacity(1024),
            interrupt: Arc::new(AtomicU64::new(NO_INTERRUPT)),
            builtin_classes: builtin_classes::build(&names),
            on_demand: builtin_classes::on_demand(&registry),
            known: {
                let s = registry.symbols();
                Known { closure: s.intern(b"closure"), generator: s.intern(b"generator"), stdclass: s.intern(b"stdclass"), error: s.intern(b"error"), fiber: s.intern(b"fiber") }
            },
            names,
            ini_texts: RefCell::new(HashMap::default()),
            thawed: RefCell::new(HashMap::default()),
            unbuilt: Closure::unbuilt(&registry),
            registry,
        }
    }

    /// A constant of compiled code as a value: a string or an array made
    /// once for the isolate (see `thawed`), anything else as it is.
    pub(crate) fn thaw(&self, f: &p8_runtime::frozen::Frozen) -> Value {
        use p8_runtime::frozen::Frozen;
        let at = match f {
            Frozen::Str(t) => t.as_bytes().as_ptr() as usize,
            Frozen::Array(entries) => entries.as_ptr() as usize,
            _ => return f.to_value(),
        };
        let mut thawed = self.thawed.borrow_mut();
        if let Some((_, v)) = thawed.get(&at) {
            return v.clone();
        }
        // (Code an isolate only evals, again and again, would grow this
        // without end: past a bound it starts over.)
        if thawed.len() >= 1 << 18 {
            thawed.clear();
        }
        let v = f.to_value();
        thawed.insert(at, (f.clone(), v.clone()));
        v
    }

    /// An ini default (a registry's static text) as a string, shared.
    pub(crate) fn ini_text(&self, text: &'static [u8]) -> PhpStr {
        self.ini_texts.borrow_mut().entry((text.as_ptr() as usize, text.len())).or_insert_with(|| PhpStr::from(text)).clone()
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// A handle that stops this isolate's running request from another thread.
    pub fn interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle(Arc::clone(&self.interrupt))
    }

    /// Start a request whose output goes to `host`. An interrupt left from
    /// an earlier request is cleared.
    pub fn new_context<'i>(&'i mut self, host: &'i mut dyn Host) -> Context<'i> {
        self.interrupt.store(NO_INTERRUPT, AtomicOrdering::Relaxed);
        let stop = Arc::clone(&self.interrupt);
        let classes = self.builtin_classes.clone();
        let ids = Rc::new(Ids::default());
        let resource_ids = Rc::new(Ids::monotonic_beside(&ids));
        Context {
            stop,
            isolate: self,
            frames: Frames::default(),
            functions: HashMap::default(),
            closures: Vec::new(),
            codes: Vec::new(),
            constants: p8_runtime::fast::IndexMap::default(),
            globals: p8_runtime::fast::IndexMap::default(),
            main_cvs: None,
            out: Output { buffers: Vec::new(), host, started: false, response: Default::default(), head_sent: false },
            request: None,
            main_return: None,
            carry: None,
            output_start: None,
            diags: Vec::new(),
            ini: HashMap::default(),
            error_level: E_DEFAULT,
            display_errors: true,
            pc: 0,
            classes,
            failed_call: None,
            class_defs: Vec::new(),
            declared: HashMap::default(),
            class_origins: HashMap::default(),
            defs_by_name: HashMap::default(),
            declared_order: Vec::new(),
            binding_file: None,
            loader: None,
            included: std::collections::HashSet::new(),
            included_order: Vec::new(),
            ids,
            native_class: None,
            ext_state: HashMap::default(),
            header_callback_set: false,
            ended_globals: Vec::new(),
            resource_classes: HashMap::default(),
            resource_ids,
            enum_objects: HashMap::default(),
            resolved: std::collections::HashSet::new(),
            stack_base: 0,
            ref_wanted: Vec::new(),
            named_extra: None,
            class_defaults: HashMap::default(),
            new_infos: HashMap::default(),
            const_cache: HashMap::default(),
            const_evaluating: Vec::new(),
            autoloaders: Vec::new(),
            autoload_tried: std::collections::HashSet::new(),
            autoload_retry: None,
            error_handlers: Vec::new(),
            exception_handlers: Vec::new(),
            shutdown: Vec::new(),
            shutdown_names: Vec::new(),
            ext_data: HashMap::default(),
            request_end: false,
            deprecated_made: Default::default(),
            pending_errors: Vec::new(),
            last_error: None,
            in_handler: false,
            arg_buf: Vec::new(),
            key_path: Vec::new(),
            dim_keys: Vec::new(),
            lambdas: HashMap::default(),
            natives: Vec::new(),
            native_args: Vec::new(),
            running_builtin: None,
            statics: HashMap::default(),
            magic_guard: Vec::new(),
            result_reg: 0,
            header_throw: None,
            deferred_write: None,
            fibers: Vec::new(),
            fiber_suspended: None,
            fiber_throw: None,
            fiber_exit: None,
            gc_runs: 0,
            gc_collected: 0,
            cfg: Vec::new(),
            ini_master: HashMap::default(),
            disabled: std::collections::HashSet::new(),
            argv: None,
            started: std::time::Instant::now(),
        }
    }
}

/// One request: its declared functions, constants and globals, its output.
/// Dropping it ends the request.
pub struct Context<'i> {
    pub(crate) isolate: &'i mut Isolate,
    /// The isolate's interrupt flag (kept here: read on every step).
    pub(crate) stop: Arc<AtomicU64>,
    pub(crate) frames: Frames,
    /// Declared functions by (lowercase) name, as indices in `closures`
    /// (only ever grows, so call sites
    /// can cache what they resolve).
    pub(crate) functions: HashMap<Sym, u32>,
    /// The closure of every declared function and method, by index (call
    /// sites refer to them so). A slot is the isolate's `unbuilt` stand-in
    /// until the function is first resolved: `closure_at` makes it then.
    pub(crate) closures: Vec<Rc<Closure>>,
    /// The compiled code at each index of `closures`, made or not.
    pub(crate) codes: Vec<Arc<Function>>,
    /// Constants the script defined (define(), const).
    /// The script's own constants, in the order defined, by key
    /// (p8_runtime::ext::const_key): each with its name as defined.
    pub(crate) constants: p8_runtime::fast::IndexMap<Sym, (Sym, Value)>,
    /// Globals not held in the main frame's registers.
    /// In the order made, as PHP's symbol table ($GLOBALS, get_defined_vars).
    pub(crate) globals: p8_runtime::fast::IndexMap<Vec<u8>, Rc<RefCell<Value>>>,
    /// The main script's variables by name (its code, identified by its
    /// address): superglobal uses look a global up each time, as PHP does
    /// in its symbol table.
    pub(crate) main_cvs: Option<(usize, HashMap<Vec<u8>, usize>)>,
    pub(crate) out: Output<'i>,
    pub(crate) diags: Vec<Diag>,
    /// ini settings this request changed (ini_set, error_reporting()).
    pub(crate) ini: HashMap<Vec<u8>, Value>,
    /// error_reporting's level: which diagnostics are shown.
    pub(crate) error_level: i64,
    /// display_errors: whether diagnostics are shown at all.
    pub(crate) display_errors: bool,
    /// The running frame's pc (Frame.pc is written when it calls out).
    pub(crate) pc: usize,
    /// Linked classes by (lowercase) name.
    pub(crate) classes: HashMap<Sym, Rc<Class>>,
    /// The frame of a call that failed before it ran, for the trace of
    /// the error it raised (see `failed_call`).
    pub(crate) failed_call: Option<PhpArray>,
    /// The script's class declarations (DeclareClass indexes them), and
    /// which are linked.
    pub(crate) class_defs: Vec<Arc<ClassDef>>,
    /// Keyed by the declaration's address, holding it so that address
    /// is not reused by another file's class while it is remembered.
    pub(crate) declared: HashMap<usize, Arc<ClassDef>>,
    /// Where each class this request declared was declared (file, line),
    /// for "previously declared in"; and the file whose classes are being
    /// bound before it runs.
    pub(crate) class_origins: HashMap<Sym, (Rc<str>, u32)>,
    /// Each declared class's compiled declaration (reflection reads its metadata).
    pub(crate) defs_by_name: HashMap<Sym, Arc<ClassDef>>,
    /// The classes the script declared, in order, each by the name it is
    /// listed by (an alias by its lowercase name): get_declared_classes().
    pub(crate) declared_order: Vec<(PhpStr, Rc<Class>)>,
    pub(crate) binding_file: Option<Rc<str>>,
    /// Compiles the files `include` runs (the embedder's compiler).
    pub(crate) loader: Option<Rc<dyn Loader>>,
    /// Files included so far, for `_once` (resolved paths).
    pub(crate) included: std::collections::HashSet<std::path::PathBuf>,
    /// The files run so far, in order (get_included_files).
    pub(crate) included_order: Vec<String>,
    /// Object ids (handles), reused as PHP reuses them.
    pub(crate) ids: Rc<Ids>,
    /// A builtin's arguments while it runs (kept for its capacity).
    pub(crate) arg_buf: Vec<Value>,
    /// A dimension write's keys while it runs (kept for its capacity).
    pub(crate) key_path: Vec<Key>,
    /// A property dimension write's keys as written (AssignPropDim; kept
    /// for its capacity).
    pub(crate) dim_keys: Vec<Value>,
    /// Extensions' per-request state (Ctx::state).
    pub(crate) ext_state: HashMap<&'static str, Value>,
    /// Whether header_register_callback() was ever called here: until
    /// then a builtin call skips looking its callback up in `ext_state`.
    pub(crate) header_callback_set: bool,
    /// The main frame's globals as the script left them, kept until the
    /// request's end hooks have run.
    pub(crate) ended_globals: Vec<Value>,
    /// The class of each kind of resource (and whether it is a user-space
    /// wrapper's stream, which has a destructor), and their numbers.
    pub(crate) resource_classes: HashMap<(&'static str, bool), Rc<Class>>,
    pub(crate) resource_ids: Rc<Ids>,
    /// The class a static builtin method runs for (an enum's cases()).
    pub(crate) native_class: Option<Rc<Class>>,
    /// Enum case objects, one per (enum, case) in a request.
    pub(crate) enum_objects: HashMap<(Vec<u8>, Vec<u8>), Rc<Object>>,
    /// Result registers (absolute) of calls in a reference context: a
    /// function returning by reference returns the reference there.
    pub(crate) ref_wanted: Vec<usize>,
    /// Named arguments for the variadic parameter of the function being
    /// called (at this frame depth), by name.
    pub(crate) named_extra: Option<(usize, PhpArray)>,
    /// The native stack's address where the script started (0: unknown),
    /// to stop runaway nesting (PHP's "Maximum call stack size").
    pub(crate) stack_base: usize,
    /// Lazy classes (CLASS_LAZY) whose default code has run, by address.
    pub(crate) resolved: std::collections::HashSet<usize>,
    /// A lazy class's computed instance defaults: (slot, value).
    pub(crate) class_defaults: HashMap<usize, Rc<Vec<(usize, Value)>>>,
    /// How to make each class's objects, by the class's address
    /// (instances.rs): worked out at its first `new`.
    pub(crate) new_infos: HashMap<usize, Rc<instances::NewInfo>>,
    /// Class constants computed by code: (declaring class, name) → value.
    pub(crate) const_cache: HashMap<(Vec<u8>, Vec<u8>), Value>,
    /// The ones being computed (a constant that needs itself is an error).
    pub(crate) const_evaluating: Vec<(Vec<u8>, Vec<u8>)>,
    /// spl_autoload_register's callables, and the classes they are being
    /// asked for (a lookup of one of those meanwhile fails: no recursion).
    pub(crate) autoloaders: Vec<Value>,
    pub(crate) autoload_tried: std::collections::HashSet<Vec<u8>>,
    /// The class whose autoloaders just ran: the instruction that asked
    /// runs again, and its lookup of the class (still missing) fails once.
    pub(crate) autoload_retry: Option<Vec<u8>>,
    /// set_error_handler's (callable, levels), innermost last.
    pub(crate) error_handlers: Vec<(Value, i64)>,
    pub(crate) exception_handlers: Vec<Value>,
    /// register_shutdown_function's (callable, arguments).
    pub(crate) shutdown: Vec<(Value, Vec<Value>)>,
    /// The name each of `shutdown` was registered under, if any
    /// (Ctx::shutdown_named).
    pub(crate) shutdown_names: Vec<Option<&'static str>>,
    /// Typed per-request state of the extensions (Ctx::ext_data).
    pub(crate) ext_data: HashMap<&'static str, Option<Box<dyn std::any::Any>>>,
    /// The constants an extension made while the request ran that are
    /// deprecated (SID), by constant key.
    pub(crate) deprecated_made: p8_runtime::fast::HashSet<Sym>,
    /// The request-end hooks of the extensions are running: what they
    /// report is located "Unknown" on line 0, as PHP words RSHUTDOWN's.
    pub(crate) request_end: bool,
    /// Diagnostics for the error handler, raised by the running instruction.
    pub(crate) pending_errors: Vec<(i64, String, Rc<str>, u32)>,
    /// error_get_last(): the last error that reached the default handler.
    pub(crate) last_error: Option<(i64, String, String, u32)>,
    /// Where output first reached the host (headers_sent()).
    pub(crate) output_start: Option<(Rc<str>, u32)>,
    pub(crate) in_handler: bool,
    /// The closure of each function a Closure object runs (by its code).
    pub(crate) lambdas: HashMap<usize, Rc<Closure>>,
    /// Builtins running that may call back into PHP, innermost last.
    pub(crate) natives: Vec<NativeCall>,
    /// Argument lists of finished `natives`, kept for the next ones (a
    /// call then allocates nothing).
    pub(crate) native_args: Vec<Vec<Value>>,
    /// The builtin running now, innermost (its name for its messages).
    pub(crate) running_builtin: Option<usize>,
    /// `static` variables, by function (its code's address) and index.
    pub(crate) statics: HashMap<usize, FunctionStatics>,
    /// The magic property methods running: (object, property, which), so
    /// a property access inside `__get('x')` is a plain one (zend's guards).
    pub(crate) magic_guard: Vec<(usize, PhpStr, crate::classes::Magic)>,
    /// The register a running nested call's result lands in: destructors
    /// that run as its frame goes (after it returned) start above it.
    pub(crate) result_reg: usize,
    /// What header_register_callback's callback threw while a builtin's
    /// output sent the headers: the builtin's call ends with it.
    pub(crate) header_throw: Option<Throw>,
    /// A write held back until the error handler has run (`$undef op= v`).
    pub(crate) deferred_write: Option<(usize, Value)>,
    /// The fibers running, innermost last.
    pub(crate) fibers: Vec<fiber::FiberRun>,
    /// Fiber::suspend()'s value, while the running fiber is being left.
    pub(crate) fiber_suspended: Option<Value>,
    /// Fiber::throw()'s exception, raised where the fiber goes on.
    pub(crate) fiber_throw: Option<Rc<Object>>,
    /// The exit unwinding a fiber being destroyed (no catch matches it).
    pub(crate) fiber_exit: Option<Rc<Object>>,
    /// gc_status(): collections run, values they freed.
    pub(crate) gc_runs: i64,
    pub(crate) gc_collected: i64,
    /// The host's settings, as given (get_cfg_var).
    pub(crate) cfg: Vec<(Vec<u8>, String)>,
    /// The value each host setting gave a directive (its master value).
    pub(crate) ini_master: HashMap<Vec<u8>, Value>,
    /// The builtins disable_functions took away: they are undefined, and
    /// the script may declare functions of their names.
    pub(crate) disabled: std::collections::HashSet<Vec<u8>>,
    /// The command line, when the host gives one: $argv, $argc, $_SERVER.
    pub(crate) argv: Option<Vec<String>>,
    /// When the request started.
    pub(crate) started: std::time::Instant,
    /// The request arrays a web host gives ([`Context::serve`]).
    pub(crate) request: Option<RequestGlobals>,
    /// What the main script returned (a router script's answer).
    pub(crate) main_return: Option<Value>,
    /// The request arrays as a main script that returned false left them.
    pub(crate) carry: Option<RequestGlobals>,
}

/// The request arrays of a web request: what a host parsed out of it.
pub struct RequestGlobals {
    pub server: PhpArray,
    pub get: PhpArray,
    pub post: PhpArray,
    pub cookie: PhpArray,
    pub files: PhpArray,
    pub request: PhpArray,
}

#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8: an embeddable PHP 8.4 engine (APS 3 in p8runtime/aps: engine and
//! hosts). Hosts use this crate only; the
//! crates behind it (syntax, compiler, bytecode, runtime, interpreter,
//! extensions) are not part of the API.
//!
//! ```no_run
//! struct Stdout;
//! impl p8::Host for Stdout {
//!     fn write(&mut self, bytes: &[u8]) {
//!         use std::io::Write;
//!         let _ = std::io::stdout().write_all(bytes);
//!     }
//! }
//!
//! let platform = p8::Platform::new(&[p8::extensions::STANDARD]); // once per process
//! let mut isolate = platform.new_isolate();                      // once per thread
//! let script = platform.compile(b"<?php echo 1 + 2;", "a.php");
//! let mut host = Stdout;
//! let mut context = isolate.new_context(&mut host);              // once per request
//! let status = match &script {
//!     Ok(script) => context.run(script),
//!     Err(error) => context.fail(error),
//! };
//! ```

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Ctx, Registry};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

mod file_cache;
mod opcache;
pub use opcache::OpcacheSettings;
mod request;

pub use p8_interp::{Host, InterruptHandle};
pub use p8_ext_standard::{P8_VERSION, PHP_VERSION, powered_by};
pub use p8_runtime::ext::Response;
pub use p8_runtime::ext::Extension;
/// realpath(3) as p8 resolves a script's path (WASI has none; p8 walks the
/// path as PHP's tsrm_realpath() does).
pub use p8_runtime::os::canonicalize;

/// The extensions p8 ships.
pub mod extensions {
    pub use p8_ext_standard::EXTENSION as STANDARD;
    pub use p8_ext_xml::EXTENSION as XML;
    pub use p8_ext_tokenizer::EXTENSION as TOKENIZER;
    pub use p8_ext_mbstring::EXTENSION as MBSTRING;
    pub use p8_ext_pcre::EXTENSION as PCRE;
    // Not in a WASI build (wasm32-wasip1 has no sockets): mysqli, curl,
    // sockets, pdo_pgsql and pdo_mysql, as a PHP built without them.
    #[cfg(not(target_os = "wasi"))]
    pub use p8_ext_mysqli::EXTENSION as MYSQLI;
    #[cfg(not(target_os = "wasi"))]
    pub use p8_ext_curl::EXTENSION as CURL;
    pub use p8_ext_gd::EXTENSION as GD;
    pub use p8_ext_zip::EXTENSION as ZIP;
    pub use p8_ext_fileinfo::EXTENSION as FILEINFO;
    pub use p8_ext_session::EXTENSION as SESSION;
    /// PDO's core; register it before the drivers.
    pub use p8_ext_pdo::EXTENSION as PDO;
    pub use p8_ext_pdo_sqlite::EXTENSION as PDO_SQLITE;
    #[cfg(not(target_os = "wasi"))]
    pub use p8_ext_sockets::EXTENSION as SOCKETS;
    pub use p8_ext_openssl::EXTENSION as OPENSSL;
    pub use p8_ext_intl::EXTENSION as INTL;
    #[cfg(not(target_os = "wasi"))]
    pub use p8_ext_pdo_pgsql::EXTENSION as PDO_PGSQL;
    #[cfg(not(target_os = "wasi"))]
    pub use p8_ext_pdo_mysql::EXTENSION as PDO_MYSQL;
    /// What the built-in web server adds (register it after STANDARD).
    pub use p8_ext_standard::server::EXTENSION as SERVER;
}

/// Process-wide state shared by every isolate: the registered extensions,
/// and the compiled files when the host keeps them.
pub struct Platform {
    compiler: Arc<Compiler>,
}

impl Platform {
    pub fn new(extensions: &[Extension]) -> Platform {
        Platform { compiler: Arc::new(Compiler { registry: Arc::new(Registry::new(extensions)), cache: None, files: None, compiled: AtomicUsize::new(0), opcache: None, partials: AtomicUsize::new(0) }) }
    }

    /// Report the cache this Platform keeps (keep_compiled, file_cache) as
    /// Zend OPcache, started with `settings`: the `opcache_*` functions act
    /// on it, and while OPcache runs it caches by OPcache's rules (see the
    /// `opcache` module).
    pub fn opcache(self, settings: OpcacheSettings) -> Platform {
        let compiler = Arc::try_unwrap(self.compiler).unwrap_or_else(|_| panic!("opcache() is set before the Platform is shared"));
        Platform { compiler: Arc::new(Compiler { opcache: Some(opcache::Opcache::new(settings)), ..compiler }) }
    }

    /// An ini directive's built-in default (for a host reading its own
    /// settings over the defaults).
    pub fn ini_default(&self, name: &str) -> Option<String> {
        self.compiler.registry.ini_default(name.as_bytes()).flatten().map(|v| String::from_utf8_lossy(v).into_owned())
    }

    /// Keep every file compiled (the script and what it includes), for
    /// every isolate, and compile a file again only when its source has
    /// changed: what opcache is to PHP. Nothing a script can see changes;
    /// the cost is memory: each kept file's source and code, never let go.
    pub fn keep_compiled(self) -> Platform {
        let files = self.compiler.files.clone();
        self.with(Some(Mutex::new(HashMap::new())), files)
    }

    /// Keep every compiled file on disk in `dir` (made if missing), and
    /// read it back instead of compiling it again, in this process or any
    /// later one: what `opcache.file_cache` is to PHP. A file is found by
    /// its source and path with this build of p8, so a changed file or a
    /// rebuilt engine compiles again; an entry that is damaged or not
    /// p8's is compiled over. Without an executable to tell builds apart
    /// by (WASI), nothing is kept.
    pub fn file_cache(self, dir: impl Into<PathBuf>) -> Platform {
        self.file_cache_with(dir.into(), false)
    }

    /// A file cache in `dir` that is only read (`opcache.file_cache_read_only`):
    /// entries made ahead of time are used, nothing is written or removed,
    /// a file without a sound entry is compiled each time.
    pub fn read_only_file_cache(self, dir: impl Into<PathBuf>) -> Platform {
        self.file_cache_with(dir.into(), true)
    }

    fn file_cache_with(self, dir: PathBuf, read_only: bool) -> Platform {
        let files = file_cache::FileCache::new(dir, self.compiler.registry.modules(), read_only).map(Arc::new);
        let cache = self.compiler.cache.as_ref().map(|_| Mutex::new(HashMap::new()));
        self.with(cache, files)
    }

    fn with(self, cache: Option<Mutex<HashMap<String, Kept>>>, files: Option<Arc<file_cache::FileCache>>) -> Platform {
        let compiler = Arc::try_unwrap(self.compiler).unwrap_or_else(|_| panic!("a Platform is configured before it is shared"));
        Platform { compiler: Arc::new(Compiler { cache, files, compiled: AtomicUsize::new(0), partials: AtomicUsize::new(0), ..compiler }) }
    }

    /// How many files this Platform has compiled from their source (not
    /// taken from what it keeps in memory or on disk).
    pub fn compiled_files(&self) -> usize {
        self.compiler.compiled.load(Ordering::Relaxed)
    }

    /// How many partial applications' closures this Platform has compiled
    /// (one per site and function applied, for each isolate: a call of
    /// the site again only binds its values).
    pub fn compiled_partials(&self) -> usize {
        self.compiler.partials.load(Ordering::Relaxed)
    }

    /// An engine for the calling thread.
    pub fn new_isolate(&self) -> Isolate {
        Isolate { inner: p8_interp::Isolate::new(Arc::clone(&self.compiler.registry)), compiler: Arc::clone(&self.compiler) }
    }

    /// Compile a file's source.
    pub fn compile(&self, source: &[u8], file: &str) -> Result<Script, Error> {
        self.compiler.file(source, file).map(|program| Script { program, registry: Arc::clone(&self.compiler.registry) })
    }

    /// A script's bytecode, as text (for `--dump`).
    pub fn disassemble(&self, script: &Script) -> String {
        disassemble(&script.program)
    }
}

/// What compiles a script and the files it includes.
struct Compiler {
    registry: Arc<Registry>,
    cache: Option<Mutex<HashMap<String, Kept>>>,
    /// Where compiled files are kept on disk, if anywhere.
    files: Option<Arc<file_cache::FileCache>>,
    /// Files compiled from source.
    compiled: AtomicUsize,
    /// Zend OPcache over the cache, when the host reports one.
    opcache: Option<opcache::Opcache>,
    /// Partial applications' closures compiled.
    partials: AtomicUsize,
}

/// A compiled file kept by its path: the source it was compiled from and
/// what OPcache knows of it.
struct Kept {
    source: Vec<u8>,
    program: Arc<p8_bytecode::Program>,
    stats: opcache::ScriptStats,
}

impl Compiler {
    fn file(&self, source: &[u8], file: &str) -> Result<Arc<p8_bytecode::Program>, Error> {
        let Some(cache) = &self.cache else { return self.load(source, file) };
        // While OPcache runs, a file it caches is checked by its rules (so,
        // as in PHP, a changed file can run as it was until revalidated);
        // anything else p8 keeps is used again only while its source is
        // the same, which no script can tell from compiling it again.
        let opcache = self.opcache.as_ref().filter(|o| o.active());
        {
            let mut kept = cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(entry) = kept.get_mut(file) {
                let valid = match opcache {
                    Some(o) if entry.stats.opcache => o.still_valid(file, &mut entry.stats),
                    // One OPcache would not cache: a miss each time it is
                    // loaded, while it still would not (else it is
                    // compiled again, to be cached now).
                    Some(o) => entry.source.as_slice() == source
                        && match o.refusal(file, source.len()) {
                            Some(refusal) => {
                                o.miss(refusal == opcache::Refusal::Blacklisted);
                                true
                            }
                            None => false,
                        },
                    None => entry.source.as_slice() == source,
                };
                if valid {
                    if let Some(o) = opcache.filter(|_| entry.stats.opcache) {
                        o.hit(&mut entry.stats);
                    }
                    return Ok(Arc::clone(&entry.program));
                }
                kept.remove(file);
            }
        }
        let program = self.load(source, file)?;
        let stats = match opcache {
            Some(o) => match o.refusal(file, source.len()) {
                Some(refusal) => {
                    o.miss(refusal == opcache::Refusal::Blacklisted);
                    opcache::ScriptStats::default()
                }
                None => {
                    o.miss(false);
                    o.new_stats(file)
                }
            },
            None => opcache::ScriptStats::default(),
        };
        cache.lock().unwrap_or_else(|e| e.into_inner()).insert(file.to_string(), Kept { source: source.to_vec(), program: Arc::clone(&program), stats });
        Ok(program)
    }

    /// A request starts: OPcache's pending restart (opcache_reset())
    /// empties the cache now.
    fn begin_request(&self) {
        if let Some(o) = &self.opcache
            && o.begin_request()
            && let Some(cache) = &self.cache
        {
            cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
    }

    /// A file's program: from the file cache, else compiled (and kept there).
    fn load(&self, source: &[u8], file: &str) -> Result<Arc<p8_bytecode::Program>, Error> {
        let symbols = self.registry.symbols();
        if let Some(files) = &self.files
            && let Some(program) = files.load(source, file, symbols)
        {
            return Ok(Arc::new(program));
        }
        self.compiled.fetch_add(1, Ordering::Relaxed);
        let program = compile(&self.registry, source, file, false)?;
        if let Some(files) = &self.files {
            files.store(source, file, &program, symbols);
        }
        Ok(program)
    }
}

/// What compiles the files a context's script includes.
struct Load(Arc<Compiler>);

impl p8_interp::Loader for Load {
    fn load(&self, source: &[u8], file: &str) -> Result<Arc<p8_bytecode::Program>, p8_interp::LoadError> {
        match self.0.file(source, file) {
            Ok(program) => Ok(program),
            Err(e) => Err(p8_interp::LoadError { parse: e.kind == ErrorKind::Parse, message: e.message, line: e.line }),
        }
    }

    fn code_cache(&self) -> Option<Arc<dyn p8_runtime::opcache::CodeCache>> {
        self.0.opcache.as_ref()?;
        Some(Arc::clone(&self.0) as Arc<dyn p8_runtime::opcache::CodeCache>)
    }

    fn load_eval(&self, code: &[u8], file: &str) -> Result<Arc<p8_bytecode::Program>, p8_interp::LoadError> {
        let source = [b"<?php ".as_slice(), code].concat();
        match compile(&self.0.registry, &source, file, true) {
            Ok(program) => Ok(program),
            Err(e) => Err(p8_interp::LoadError { parse: e.kind == ErrorKind::Parse, message: e.message, line: e.line }),
        }
    }

    fn load_partial(&self, source: &[u8], file: &str, name: &[u8], class: Option<&[u8]>, is_static: bool) -> Result<Arc<p8_bytecode::Function>, p8_interp::LoadError> {
        self.0.partials.fetch_add(1, Ordering::Relaxed);
        let program = compile(&self.0.registry, source, file, false).map_err(|e| p8_interp::LoadError { parse: e.kind == ErrorKind::Parse, message: e.message, line: e.line })?;
        // A program just compiled is held only here: its one closure is
        // taken out of it and given the partial application's name.
        let unshared = || p8_interp::LoadError { parse: false, message: "partial application: the closure's code is shared".into(), line: 0 };
        let program = Arc::try_unwrap(program).map_err(|_| unshared())?;
        let main = Arc::try_unwrap(program.main).map_err(|_| unshared())?;
        let closure = main.lambdas.into_iter().next().ok_or_else(unshared)?;
        let mut closure = Arc::try_unwrap(closure).map_err(|_| unshared())?;
        closure.name = p8_runtime::frozen::Text::from(name);
        closure.class = class.map(p8_runtime::frozen::Text::from);
        closure.is_static = is_static;
        Ok(Arc::new(closure))
    }
}

fn compile(registry: &Registry, source: &[u8], file: &str, eval: bool) -> Result<Arc<p8_bytecode::Program>, Error> {
    let arena = bumpalo::Bump::new();
    let lexer = p8_syntax::lexer::Lexer::new(source);
    let mut parser = p8_syntax::parser::Parser::new(lexer, &arena);
    let program = parser.parse_program();
    if let Some(error) = program.errors.first() {
        let line = error.span.line_info(source).map_or(1, |info| info.line as u32);
        return Err(Error { kind: ErrorKind::Parse, message: error.message.to_string(), file: file.into(), line });
    }
    let mut compiler = p8_compiler::Compiler::new(source, file, registry);
    compiler.eval = eval;
    match compiler.compile(program.statements) {
        Ok(program) => Ok(Arc::new(program)),
        Err(e) => Err(Error { kind: ErrorKind::Fatal, message: e.message, file: file.into(), line: e.line }),
    }
}

fn disassemble(program: &p8_bytecode::Program) -> String {
    let mut out = String::new();
    for f in std::iter::once(&program.main).chain(&program.functions) {
        out.push_str(&format!(
            "function {} (regs {}, consts at {}: {:?})\n",
            String::from_utf8_lossy(f.name.as_bytes()),
            f.nregs,
            f.consts_at,
            f.consts
        ));
        for (pc, instr) in f.code.iter().enumerate() {
            out.push_str(&format!("  {pc:4} L{:<4} {instr:?}\n", f.lines[pc]));
        }
    }
    out
}

/// A compiled file: immutable, and `Send + Sync`, so one compile serves
/// every isolate on every thread of the Platform that compiled it (its
/// names are that Platform's symbols).
pub struct Script {
    program: Arc<p8_bytecode::Program>,
    registry: Arc<Registry>,
}

/// A file that does not compile.
#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// PHP's "Parse error".
    Parse,
    /// PHP's "Fatal error" raised while compiling.
    Fatal,
}

/// One engine on one thread; it runs one context (request) at a time.
pub struct Isolate {
    inner: p8_interp::Isolate,
    compiler: Arc<Compiler>,
}

impl Isolate {
    /// A handle that stops this isolate's running request from any thread
    /// (a host's watchdog holds one).
    pub fn interrupt_handle(&self) -> InterruptHandle {
        self.inner.interrupt_handle()
    }

    /// Start a request whose output goes to `host`. The context borrows the
    /// isolate until it is dropped, which ends the request.
    pub fn new_context<'i>(&'i mut self, host: &'i mut dyn Host) -> Context<'i> {
        self.compiler.begin_request();
        let mut inner = self.inner.new_context(host);
        inner.set_loader(Rc::new(Load(Arc::clone(&self.compiler))));
        Context { inner, warnings: Vec::new(), uploads: Vec::new() }
    }
}

/// One request.
pub struct Context<'i> {
    inner: p8_interp::Context<'i>,
    /// What parsing a web request's input warned about, shown when it runs.
    warnings: Vec<String>,
    /// The temporary files a web request's uploads were saved in: removed
    /// when the request ends, whatever the script did not move.
    uploads: Vec<PathBuf>,
}

impl Drop for Context<'_> {
    fn drop(&mut self) {
        for path in &self.uploads {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Context<'_> {
    /// Set an ini setting for this request (`php -d name=value`); false
    /// when p8 has no such setting.
    pub fn set_ini(&mut self, name: &str, value: &str) -> bool {
        self.inner.set_ini(name.as_bytes(), value)
    }

    /// The command line the script sees: `$argv` (the script's path
    /// first), `$argc`, and `$_SERVER` with them, as the CLI sets them.
    pub fn set_args(&mut self, argv: Vec<String>) {
        self.inner.set_args(argv);
    }

    /// Make this request a web request: the script sees `request`'s
    /// variables ($_SERVER, $_GET, $_POST, $_COOKIE, $_FILES, $_REQUEST,
    /// php://input), and the response head it builds (header(),
    /// http_response_code()) goes to [`Host::head`] with the first output.
    /// Call it after the settings are made. Needs the [`extensions::SERVER`]
    /// extension for `PHP_SAPI` and `getallheaders()`.
    pub fn serve(&mut self, request: &Request, carried: Option<Carried>) {
        use request::{Input, Limits};
        let ini = |inner: &p8_interp::Context, name: &str| match Ctx::ini_get(inner, name.as_bytes()) {
            Some(Value::Str(s)) => Some(String::from_utf8_lossy(s.as_bytes()).into_owned()),
            Some(Value::Null) | None => None,
            Some(v) => v.to_php_string().map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned()),
        };
        let number = |inner: &p8_interp::Context, name: &str, default: i64| ini(inner, name).map_or(default, |v| ini_number(&v));
        let limits = Limits {
            post_max_size: number(&self.inner, "post_max_size", 8 << 20),
            upload_max_filesize: number(&self.inner, "upload_max_filesize", 2 << 20),
            max_file_uploads: number(&self.inner, "max_file_uploads", 20),
            max_input_vars: number(&self.inner, "max_input_vars", 1000),
            max_input_nesting_level: number(&self.inner, "max_input_nesting_level", 64),
            max_multipart_body_parts: number(&self.inner, "max_multipart_body_parts", -1),
            file_uploads: ini(&self.inner, "file_uploads").is_none_or(|v| ini_bool(&v)),
            upload_tmp_dir: ini(&self.inner, "upload_tmp_dir").filter(|d| !d.is_empty()).map(PathBuf::from),
            variables_order: ini(&self.inner, "variables_order").unwrap_or_else(|| "EGPCS".into()).into_bytes(),
            request_order: ini(&self.inner, "request_order").filter(|o| !o.is_empty()).map(String::into_bytes),
        };
        let header = |name: &str| request.headers.iter().find(|(k, _)| k == name.as_bytes()).map(|(_, v)| v.as_slice());
        let content_type = header("content-type");
        let parsed = request::parse(&Input { method: &request.method, query: request.query.as_deref(), cookie: header("cookie"), content_type, body: &request.body }, &limits);
        self.warnings = parsed.warnings;
        self.uploads = parsed.uploads;

        // $_SERVER: the host's variables, the authentication ones, the time, argv.
        let mut warnings = Vec::new();
        let mut server = PhpArray::new();
        for (name, value, known) in &request.server {
            let value = Value::Str(PhpStr::from(value.as_slice()));
            if *known {
                server.insert(Key::Str(PhpStr::from(name.as_slice())), value);
            } else {
                request::register_variable(&mut server, name, value, false, limits.max_input_nesting_level, &mut warnings);
            }
        }
        if let Some(auth) = header("authorization") {
            if auth.len() >= 6 && auth[..6].eq_ignore_ascii_case(b"Basic ") {
                if let Some(decoded) = request::base64_decode(&auth[6..])
                    && let Some(colon) = decoded.iter().position(|&c| c == b':')
                {
                    server.insert(Key::Str(PhpStr::from("PHP_AUTH_USER")), Value::Str(PhpStr::from(&decoded[..colon])));
                    if colon + 1 < decoded.len() {
                        server.insert(Key::Str(PhpStr::from("PHP_AUTH_PW")), Value::Str(PhpStr::from(&decoded[colon + 1..])));
                    }
                }
            } else if auth.len() > 7 && auth[..7].eq_ignore_ascii_case(b"Digest ") {
                server.insert(Key::Str(PhpStr::from("PHP_AUTH_DIGEST")), Value::Str(PhpStr::from(&auth[7..])));
            }
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        server.insert(Key::Str(PhpStr::from("REQUEST_TIME_FLOAT")), Value::Float(now.as_secs_f64()));
        server.insert(Key::Str(PhpStr::from("REQUEST_TIME")), Value::Int(now.as_secs() as i64));
        let register_argv = !matches!(ini(&self.inner, "register_argc_argv").as_deref(), Some("" | "0" | "off" | "no" | "false"));
        if register_argv {
            // php_build_argv(): the query string split on '+'.
            let argv: Vec<Value> = match request.query.as_deref() {
                Some(q) if !q.is_empty() => q.split(|&c| c == b'+').map(|a| Value::Str(PhpStr::from(a))).collect(),
                _ => Vec::new(),
            };
            let argc = argv.len() as i64;
            server.insert(Key::Str(PhpStr::from("argv")), Value::Array(Rc::new(PhpArray::from_list(argv))));
            server.insert(Key::Str(PhpStr::from("argc")), Value::Int(argc));
        }
        let mut globals = p8_interp::RequestGlobals { server, get: parsed.get, post: parsed.post, cookie: parsed.cookie, files: parsed.files, request: parsed.request };
        // A router that declined the request left its arrays as they were.
        if let Some(Carried(Some(old))) = carried {
            globals = old;
        }

        // php://input: the body, except what the multipart parser took.
        let multipart = content_type.is_some_and(|ct| ct.len() >= 19 && ct[..19].eq_ignore_ascii_case(b"multipart/form-data"));
        let input = if request.method == "POST" && multipart { Vec::new() } else { request.body.clone() };
        self.inner.set_state("php_input", Value::Str(PhpStr::new(input)));
        let mut headers = PhpArray::new();
        for (name, value) in &request.headers_original {
            headers.insert(Key::from_string(PhpStr::from(name.as_slice())), Value::Str(PhpStr::from(value.as_slice())));
        }
        self.inner.set_state(p8_ext_standard::server::REQUEST_HEADERS, Value::Array(Rc::new(headers)));
        let uploaded: Vec<Value> = self.uploads.iter().map(|p| Value::Str(PhpStr::from(p.to_string_lossy().as_ref()))).collect();
        self.inner.set_state("rfc1867_uploaded_files", Value::Array(Rc::new(PhpArray::from_list(uploaded))));

        let mut headers = Vec::new();
        if ini(&self.inner, "expose_php").is_none_or(|v| ini_bool(&v)) {
            headers.push(format!("X-Powered-By: {}", powered_by()).into_bytes());
        }
        // The CLI server's proto_num is 101 for HTTP/1.1, never > 1000, so
        // a redirect is never 303.
        self.inner.serve(globals, 200, headers, false);
    }

    /// Whether the main script returned `false` (a router script declining
    /// the request): the request arrays it left, for the script that serves
    /// the request instead ([`Context::serve`]).
    pub fn declined(&mut self) -> Option<Carried> {
        let declined = matches!(self.inner.take_main_return(), Some(Value::Bool(false)));
        let carried = self.inner.take_carry();
        declined.then_some(Carried(carried))
    }

    /// The last error that reached PHP's default handler: type, message,
    /// file and line (what the built-in server's log adds for a fatal one).
    pub fn last_error(&self) -> Option<(i64, String, String, u32)> {
        self.inner.last_error().cloned()
    }

    /// The response status so far (200 for a web request until the script
    /// changes it).
    pub fn status(&self) -> i64 {
        self.inner.response().status
    }

    /// Run a script's top-level code; returns the exit status. The script
    /// must come from this isolate's Platform.
    pub fn run(&mut self, script: &Script) -> i32 {
        assert!(Arc::ptr_eq(&script.registry, self.inner.registry()), "a Script runs on an isolate of the Platform that compiled it");
        self.inner.startup_diagnostics();
        for warning in std::mem::take(&mut self.warnings) {
            self.inner.startup_warning(&warning);
        }
        for f in &script.program.functions {
            if let Err(message) = self.inner.declare(Arc::clone(f)) {
                self.inner.echo_and_flush(format!("\nFatal error: {message} in {}\n", f.file).as_bytes());
                self.inner.finish_response();
                return 255;
            }
        }
        self.inner.declare_classes(&script.program.classes);
        self.inner.run_main(Arc::clone(&script.program.main))
    }

    /// Report a file that did not compile, as PHP does; returns the exit status.
    pub fn fail(&mut self, error: &Error) -> i32 {
        let label = match error.kind {
            ErrorKind::Parse => "Parse error",
            ErrorKind::Fatal => "Fatal error",
        };
        let text = self.inner.error_text(label, &error.message, &error.file, error.line, true);
        self.inner.echo_and_flush(text.as_bytes());
        self.inner.finish_response();
        255
    }
}

/// A web request, for [`Context::serve`]: what the host read off the
/// connection, and the `$_SERVER` entries its server software defines.
pub struct Request {
    pub method: String,
    /// The query string, if the request had one.
    pub query: Option<Vec<u8>>,
    /// The headers by lowercase name (a repeated one joined with ", "), for
    /// the cookies, the content type and authentication.
    pub headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// The headers as the client wrote their names (getallheaders()).
    pub headers_original: Vec<(Vec<u8>, Vec<u8>)>,
    /// The `$_SERVER` entries, in order: name, value, and whether the name
    /// is one the server defines (not mangled like a variable from the
    /// client, which a header name may be).
    pub server: Vec<(Vec<u8>, Vec<u8>, bool)>,
    pub body: Vec<u8>,
}

/// An ini size: `8M`, `128K`, `1G`, or a plain number.
fn ini_number(text: &str) -> i64 {
    let t = text.trim();
    let digits: String = t.chars().enumerate().take_while(|(i, c)| c.is_ascii_digit() || *i == 0 && (*c == '-' || *c == '+')).map(|(_, c)| c).collect();
    let n: i64 = digits.parse().unwrap_or(0);
    match t.chars().last().map(|c| c.to_ascii_lowercase()) {
        Some('k') => n.saturating_mul(1 << 10),
        Some('m') => n.saturating_mul(1 << 20),
        Some('g') => n.saturating_mul(1 << 30),
        _ => n,
    }
}

/// An ini flag: `1`, `On`, `true`, `yes`.
fn ini_bool(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    matches!(t.as_str(), "on" | "true" | "yes") || t.parse::<i64>().is_ok_and(|n| n != 0)
}

/// What a router script that declined a request left of the request's
/// variables: the script that serves the request then sees them as they
/// were left ([`Context::serve`]).
pub struct Carried(Option<p8_interp::RequestGlobals>);

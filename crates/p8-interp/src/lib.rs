#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The p8 interpreter: one loop over register instructions, a task's
//! frames over its one register file (task.rs), calls resolved once per
//! call site.

mod arrayaccess;
mod builtin_classes;
mod callable;
mod classes;
mod handlers;
mod fiber;
mod generators;
mod lazy;
mod include;
mod instances;
mod meta;
mod partial;
mod reflect_info;
mod reflection;
mod refs;
mod throw;
mod fast;
mod vmops;

/// How one instruction is run out of the loop (`step`, `cold_run`): on the
/// context `C`, its registers, the frame's closure, base, argument count
/// and the instruction's pc.
type Run<C> = fn(&mut C, &mut Vec<Value>, &Closure, usize, usize, usize) -> Result<Next, Throw>;
mod frames;
mod isolate;
mod sites;
mod task;
use fast::*;
use frames::*;
pub use isolate::*;
use sites::*;

use p8_bytecode::{CastKind, ClassDef, Cmp, Function, Instr, Reg, Rare, const_index};
pub use include::{LoadError, Loader};
use p8_runtime::array::{Key, KeyIssue, PhpArray};
use p8_runtime::ext::{Ctx, Imp, Registry, const_key};
pub use p8_runtime::ext::Throw;
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::object::{Class, HOOK_GET, HOOK_SET, Ids, Object, Slot};
use p8_runtime::ops::{self, BinOp, Diag, OpError};
use p8_runtime::string::PhpStr;
use p8_runtime::symbol::Sym;
use p8_runtime::value::{Kind, Value};
use std::cell::{Cell, OnceCell, RefCell};
use std::cmp::Ordering;
use p8_runtime::fast::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// The globals a request starts with, in the order PHP makes them.
const REQUEST_GLOBALS: &[&str] = &["_GET", "_POST", "_COOKIE", "_FILES", "argv", "argc", "_SERVER", "_ENV", "_REQUEST"];

/// isset($s[k]) on a string (zend_isset_dim_slow): the character at an
/// integer offset (from the end when negative), with k an int, a bool,
/// null, a float, or a string that is wholly an integer (" 1", "01");
/// None for anything else or an offset outside the string.
fn isset_string_offset(s: &PhpStr, k: &Value) -> Option<Value> {
    let i = match k.kind() {
        Kind::Int => k.as_int().expect("an int"),
        Kind::Bool => i64::from(k.as_bool().expect("a bool")),
        Kind::Null => 0,
        Kind::Float => k.as_float().expect("a float") as i64,
        Kind::Str => match p8_runtime::numeric::numeric_string(k.as_str().expect("a string").as_bytes())? {
            p8_runtime::numeric::Number::Int(i) => i,
            p8_runtime::numeric::Number::Float(_) => return None,
        },
        _ => return None,
    };
    let len = s.len() as i64;
    let at = if i < 0 { len + i } else { i };
    (0..len).contains(&at).then(|| Value::str(PhpStr::from(&s.as_bytes()[at as usize..at as usize + 1])))
}

/// Whether two callables are one autoloader (spl_autoload_register's
/// check): the same object, function names equal but for case, or the same
/// object (or class) and method, however written ("C::m" or ['C', 'm']).
/// Two closures are never one unless they are the same Closure.
fn same_autoloader(a: &Value, b: &Value) -> bool {
    enum Target {
        Object(Rc<p8_runtime::object::Object>),
        Name(Vec<u8>),
    }
    let name = |s: &[u8]| s.strip_prefix(b"\\").unwrap_or(s).to_ascii_lowercase();
    let target = |v: &Value| {
        let v = v.deref();
        if let Some(o) = v.cloned_object() {
            Some(Target::Object(o))
        } else {
            v.as_str().map(|s| Target::Name(name(s.as_bytes())))
        }
    };
    // (what, method): a function or object has no method.
    let key = |v: &Value| -> Option<(Target, Option<Vec<u8>>)> {
        let v = v.deref();
        if let Some(o) = v.cloned_object() {
            Some((Target::Object(o), None))
        } else if let Some(s) = v.as_str() {
            match s.as_bytes().windows(2).position(|w| w == b"::") {
                Some(i) => Some((Target::Name(name(&s.as_bytes()[..i])), Some(s.as_bytes()[i + 2..].to_ascii_lowercase()))),
                None => Some((Target::Name(name(s.as_bytes())), None)),
            }
        } else if let Some(arr) = v.as_array()
            && arr.len() == 2
        {
            let method = arr.get_int(1)?.deref().to_php_string()?.as_bytes().to_ascii_lowercase();
            Some((target(arr.get_int(0)?)?, Some(method)))
        } else {
            None
        }
    };
    let (Some((x, xm)), Some((y, ym))) = (key(a), key(b)) else { return false };
    let same = match (x, y) {
        (Target::Object(p), Target::Object(q)) => Rc::ptr_eq(&p, &q),
        (Target::Name(p), Target::Name(q)) => p == q,
        _ => false,
    };
    same && xm == ym
}

/// A builtin that is running PHP code (array_map's callback): where it
/// was called (the frame count then), its name and arguments.
struct NativeCall {
    depth: usize,
    name: &'static str,
    /// A builtin method's name, class and object (None: called
    /// statically), shown in place of `name`.
    method: Option<(PhpStr, PhpStr, Option<Rc<Object>>)>,
    args: Vec<Value>,
    /// Shown in a trace even when the error is raised inside it (no PHP
    /// frame above): the Fiber methods.
    top: bool,
}

/// What a dimension write did: the value written (an assignment's
/// result), or the object (ArrayAccess) its path reached at that level,
/// with the value still to write.
pub(crate) enum Written {
    Done(Value),
    Object(Rc<Object>, usize, Value),
}

/// What `step` leaves the frame loop to do.
enum Next {
    /// Continue this frame at the pc.
    Go(usize),
    /// A call or return changed the current frame.
    Switch,
}


impl Context<'_> {
    pub fn declare(&mut self, func: Arc<Function>) -> Result<(), String> {
        let key = func.sym;
        if let Some(&i) = self.functions.get(&key) {
            let name = String::from_utf8_lossy(func.name.as_bytes());
            let first = &self.codes[i as usize];
            return Err(format!("Cannot redeclare function {name}() (previously declared in {}:{})", first.file, first.line));
        }
        if self.builtin(key).is_some() {
            let name = String::from_utf8_lossy(func.name.as_bytes());
            return Err(format!("Cannot redeclare function {name}()"));
        }
        // Its closure is made when it is first called (sites.rs).
        let i = self.add_code(func);
        self.functions.insert(key, i);
        Ok(())
    }

    /// The script's class declarations: those whose parent and
    /// interfaces are known are linked when the run starts, the others
    /// when their declaration runs.
    pub fn declare_classes(&mut self, classes: &[Arc<ClassDef>]) {
        self.class_defs.extend(classes.iter().cloned());
    }

    /// The builtins and the symbol table this request's code is compiled against.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.isolate.registry
    }

    /// What compiles the files the script includes.
    pub fn set_loader(&mut self, loader: Rc<dyn Loader>) {
        self.loader = Some(loader);
    }

    pub fn function_exists(&self, name: &[u8]) -> bool {
        self.sym_of(name).is_some_and(|key| self.functions.contains_key(&key) || self.builtin(key).is_some())
    }

    /// The symbol of a function or class name written in any case, with
    /// an optional leading `\`: None when no code ever named it (so no
    /// function or class has that name). Allocates nothing.
    pub(crate) fn sym_of(&self, name: &[u8]) -> Option<Sym> {
        self.isolate.names.find_lower(name.strip_prefix(b"\\").unwrap_or(name))
    }

    /// The symbol of a method name (any case): None when no code ever
    /// named it (so no class has such a method). Allocates nothing.
    pub(crate) fn method_key(&self, name: &[u8]) -> Option<Sym> {
        self.isolate.names.find_lower(name)
    }

    /// The symbol of a function or class name being declared (any case,
    /// optional leading `\`).
    pub(crate) fn intern(&self, name: &[u8]) -> Sym {
        self.isolate.names.intern_lower(name.strip_prefix(b"\\").unwrap_or(name))
    }

    /// The declared class named `name` (any case, optional leading `\`),
    /// without declaring a builtin one or autoloading.
    pub(crate) fn class_named(&self, name: &[u8]) -> Option<&Rc<Class>> {
        self.classes.get(&self.sym_of(name)?)
    }

    /// The compiled declaration of the class named `name` (any case).
    pub(crate) fn def_named(&self, name: &[u8]) -> Option<&Arc<ClassDef>> {
        self.defs_by_name.get(&self.sym_of(name)?)
    }

    /// The function `name` (any case, optional leading `\`): a declared
    /// one, else a builtin.
    pub(crate) fn function_named(&self, name: &[u8]) -> Option<Callee> {
        let key = self.sym_of(name)?;
        match self.functions.get(&key) {
            Some(&f) => Some(Callee::User(f)),
            None => self.builtin(key).map(Callee::Builtin),
        }
    }

    /// The builtin named `sym`, unless disable_functions took it away.
    pub(crate) fn builtin(&self, sym: Sym) -> Option<usize> {
        self.isolate.registry.builtin(sym).filter(|&i| self.disabled.is_empty() || !self.disabled.contains(self.isolate.registry.function(i).name.as_bytes()))
    }

    /// An ini value that may be an expression (`E_ALL & ~E_NOTICE`):
    /// integers and constants with | & ^ ~ (left to right, ~ binding tightest).
    fn ini_bits(&self, text: &str) -> i64 {
        let mut acc: Option<i64> = None;
        let mut pending: Option<char> = None;
        let mut not = false;
        let mut chars = text.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
                continue;
            }
            if matches!(c, '|' | '&' | '^') {
                pending = Some(c);
                chars.next();
                continue;
            }
            if c == '~' || c == '!' {
                not = !not;
                chars.next();
                continue;
            }
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' && word.is_empty() {
                    word.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            if word.is_empty() {
                chars.next();
                continue;
            }
            let mut v = word.parse::<i64>().ok().or_else(|| self.isolate.registry.constant(word.as_bytes()).map(|c| c.to_value().to_int())).unwrap_or(0);
            if std::mem::take(&mut not) {
                v = !v;
            }
            acc = Some(match (acc, pending.take()) {
                (Some(a), Some('|')) => a | v,
                (Some(a), Some('&')) => a & v,
                (Some(a), Some('^')) => a ^ v,
                _ => v,
            });
        }
        acc.unwrap_or(0)
    }

    /// An ini setting from the host (`-d`); false when unknown.
    pub fn set_ini(&mut self, name: &[u8], value: &str) -> bool {
        self.cfg.push((name.to_vec(), value.to_string()));
        // The ini scanner reads an unquoted true, on and yes as "1", and
        // false, off, no and none as "".
        let value = match value.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" => "1",
            "false" | "off" | "no" | "none" => "",
            _ => value,
        };
        // (what PHP says about a setting is said at startup: it does not
        // send the headers)
        let (started, start_at) = (self.out.started, self.output_start.clone());
        let set = Ctx::ini_set(self, name, Value::str(value)).is_some();
        self.out.started = started;
        self.output_start = start_at;
        if let Some(v) = set.then(|| self.ini.get(name).cloned()).flatten() {
            if name == b"disable_functions" {
                // zend_disable_functions: names split at spaces and commas,
                // matched as written (exit and die are not functions).
                let list = v.to_php_string().unwrap_or_else(PhpStr::empty);
                self.disabled = list.as_bytes().split(|&b| b == b' ' || b == b',').filter(|n| !n.is_empty()).map(<[u8]>::to_vec).collect();
            }
            self.ini_master.insert(name.to_vec(), v);
        }
        set
    }

    pub fn set_args(&mut self, argv: Vec<String>) {
        self.argv = Some(argv);
    }

    /// Make this a web request: the response head is built for the host
    /// ([`Host::head`]), starting at `status` with `headers`, and the
    /// script sees the request arrays `globals` instead of a command line.
    /// `default_content_type` goes out when the script sets none;
    /// `redirect_see_other` is whether a `Location` header answers 303.
    pub fn serve(&mut self, globals: RequestGlobals, status: i64, headers: Vec<Vec<u8>>, redirect_see_other: bool) {
        self.request = Some(globals);
        let mimetype = match Ctx::ini_get(self, b"default_mimetype").as_ref().and_then(Value::as_str) {
            Some(s) => s.as_bytes().to_vec(),
            None => b"text/html".to_vec(),
        };
        let charset = match Ctx::ini_get(self, b"default_charset").as_ref().and_then(Value::as_str) {
            Some(s) => s.as_bytes().to_vec(),
            None => b"UTF-8".to_vec(),
        };
        // get_default_content_type(): the charset joins a text/* type.
        let default = if mimetype.is_empty() {
            None
        } else if mimetype.starts_with(b"text/") && !charset.is_empty() {
            let mut line = mimetype;
            line.extend_from_slice(b"; charset=");
            line.extend_from_slice(&charset);
            Some(line)
        } else {
            Some(mimetype)
        };
        let response = &mut self.out.response;
        response.server = true;
        response.status = status;
        response.headers = headers;
        response.default_content_type = default;
        response.default_charset = charset;
        response.redirect_see_other = redirect_see_other;
    }

    /// A warning PHP raises while a request starts (an input limit hit):
    /// shown now, as `Warning: PHP Request Startup: ... in Unknown on line 0`.
    pub fn startup_warning(&mut self, message: &str) {
        self.print_diag(E_WARNING, message, "Unknown", 0);
    }

    /// The end of the request: the host gets the response head if no
    /// output carried it. Idempotent.
    pub fn finish_response(&mut self) {
        self.out.send_head();
    }

    /// What the main script returned (once): `return false;` in a router.
    pub fn take_main_return(&mut self) -> Option<Value> {
        self.main_return.take()
    }

    /// The request arrays a main script that returned false left (once).
    pub fn take_carry(&mut self) -> Option<RequestGlobals> {
        self.carry.take()
    }

    /// The request arrays as the main frame (registers `w`, from its base)
    /// holds them: a variable of the main script, or a global.
    fn capture_request_globals(&self, w: &[Value], main: &Function) -> RequestGlobals {
        let get = |name: &str| -> PhpArray {
            let value = match main.cv_names.iter().position(|n| n.as_bytes() == name.as_bytes()) {
                Some(r) => w.get(r).cloned(),
                None => self.globals.get(name.as_bytes()).map(|cell| cell.borrow().clone()),
            };
            match value.map(|v| v.deref()).as_ref().and_then(Value::as_array) {
                Some(a) => a.clone(),
                None => PhpArray::new(),
            }
        };
        RequestGlobals { server: get("_SERVER"), get: get("_GET"), post: get("_POST"), cookie: get("_COOKIE"), files: get("_FILES"), request: get("_REQUEST") }
    }

    /// Per-request state for an extension (see [`Ctx::state`]).
    pub fn set_state(&mut self, key: &'static str, value: Value) {
        if key == "header_callback" {
            self.header_callback_set = true;
        }
        self.ext_state.insert(key, value);
    }

    /// The response being built so far.
    pub fn response(&self) -> &p8_runtime::ext::Response {
        &self.out.response
    }

    /// The last error that reached the default handler (error_get_last()):
    /// type, message, file, line.
    pub fn last_error(&self) -> Option<&(i64, String, String, u32)> {
        self.last_error.as_ref()
    }

    /// The globals the CLI starts a script with (php_cli.c and
    /// php_variables.c): $argv and $argc, $_SERVER (the environment, the
    /// script's names, the request time, argv/argc), $_ENV when
    /// variables_order has E, and the empty request arrays.
    fn request_globals(&mut self) -> Vec<(&'static str, Value)> {
        if let Some(r) = self.request.take() {
            // A web request (php_cli_server.c): the host built the arrays;
            // $_ENV is the environment when variables_order has E.
            let order = match Ctx::ini_get(self, b"variables_order").as_ref().and_then(Value::as_str) {
                Some(s) => s.as_bytes().to_ascii_uppercase(),
                None => b"EGPCS".to_vec(),
            };
            let mut env = PhpArray::new();
            if order.contains(&b'E') {
                for (k, v) in std::env::vars_os() {
                    env.insert(Key::Str(PhpStr::from(k.as_encoded_bytes())), Value::str(v.as_encoded_bytes()));
                }
            }
            let array = |a: PhpArray| Value::array(a);
            return vec![
                ("_SERVER", array(r.server)),
                ("_ENV", array(env)),
                ("_GET", array(r.get)),
                ("_POST", array(r.post)),
                ("_COOKIE", array(r.cookie)),
                ("_FILES", array(r.files)),
                ("_REQUEST", array(r.request)),
            ];
        }
        let Some(argv) = self.argv.clone() else { return Vec::new() };
        let text = |s: &str| Value::str(s);
        let key = |s: &str| Key::Str(PhpStr::from(s));
        let args = Value::array(PhpArray::from_list(argv.iter().map(|a| text(a)).collect()));
        let argc = Value::int(argv.len() as i64);
        let mut env = PhpArray::new();
        for (k, v) in std::env::vars_os() {
            env.insert(Key::Str(PhpStr::from(k.as_encoded_bytes())), Value::str(v.as_encoded_bytes()));
        }
        let mut server = env.clone();
        let script = argv.first().cloned().unwrap_or_default();
        for name in ["PHP_SELF", "SCRIPT_NAME", "SCRIPT_FILENAME", "PATH_TRANSLATED"] {
            server.insert(key(name), text(&script));
        }
        server.insert(key("DOCUMENT_ROOT"), text(""));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        server.insert(key("REQUEST_TIME_FLOAT"), Value::float(now.as_secs_f64()));
        server.insert(key("REQUEST_TIME"), Value::int(now.as_secs() as i64));
        // The command line's arguments are always there (the CLI hands PHP
        // its argc, php_auto_globals_create_server): register_argc_argv,
        // off by default since 8.5, only decides whether argv is made from a
        // query string, which a script run from the command line has not.
        let register = true;
        if register {
            server.insert(key("argv"), args.clone());
            server.insert(key("argc"), argc.clone());
        }
        let order = match Ctx::ini_get(self, b"variables_order").as_ref().and_then(Value::as_str) {
            Some(s) => s.as_bytes().to_ascii_uppercase(),
            None => b"EGPCS".to_vec(),
        };
        let empty = || Value::array(PhpArray::new());
        let mut out = vec![("_SERVER", Value::array(server))];
        if register {
            out.extend([("argv", args), ("argc", argc)]);
        }
        out.push(("_ENV", if order.contains(&b'E') { Value::array(env) } else { empty() }));
        for name in ["_GET", "_POST", "_COOKIE", "_FILES", "_REQUEST"] {
            out.push((name, empty()));
        }
        out
    }

    /// What PHP says about the host's settings once they are all set: the
    /// assert.* ones are deprecated away from their defaults (in the order
    /// ext/standard registers them).
    pub fn startup_diagnostics(&mut self) {
        // (what PHP says at startup is not output of the script: the
        // headers are not sent by it)
        let (started, start_at) = (self.out.started, self.output_start.clone());
        self.startup_diagnostics_inner();
        let registry = Arc::clone(&self.isolate.registry);
        for check in registry.startup_checks() {
            check(self);
        }
        self.out.started = started;
        self.output_start = start_at;
    }

    fn startup_diagnostics_inner(&mut self) {
        let on = |v: Option<Value>| match v {
            Some(v) => match v.as_str() {
                Some(s) => {
                    let t = s.as_bytes().to_ascii_lowercase();
                    matches!(t.as_slice(), b"true" | b"yes" | b"on") || v.to_int() != 0
                }
                None => v.to_bool(),
            },
            None => false,
        };
        for (name, default) in [("assert.active", true), ("assert.bail", false), ("assert.warning", true), ("assert.callback", false), ("assert.exception", true)] {
            let v = Ctx::ini_get(self, name.as_bytes());
            let deprecated = if name == "assert.callback" {
                v.as_ref().and_then(Value::as_str).is_some_and(|s| !s.as_bytes().is_empty())
            } else {
                on(v) != default
            };
            if deprecated {
                self.print_diag(E_DEPRECATED, &format!("PHP Startup: {name} INI setting is deprecated"), "Unknown", 0);
            }
        }
    }

    /// Run the innermost buffer's handler on its contents (`phase`); what it
    /// gives back (its contents, without a handler or when it returns false).
    /// What the user error handler throws while the deprecation about a
    /// handler's output is raised is thrown once the ob_* operation is done
    /// (its output written), as PHP's comes after php_output_handler_op:
    /// `late`.
    fn ob_handle(&mut self, regs: &mut Vec<Value>, phase: i64, late: &mut Option<Throw>) -> Result<Vec<u8>, Throw> {
        let Some(buffer) = self.out.buffers.last_mut() else { return Ok(Vec::new()) };
        let data = std::mem::take(&mut buffer.data);
        let Some(handler) = buffer.handler.clone() else { return Ok(data) };
        let phase = if buffer.started { phase } else { phase | OB_START };
        buffer.started = true;
        // The handler's own output is not buffered output (see
        // Output::write_while_handling).
        let at = self.out.buffers.len() - 1;
        let outer = self.out.running.replace(at);
        let r = self.call_with_regs(regs, &handler, &mut [Value::str(PhpStr::new(data.clone())), Value::int(phase)]);
        // Output from the handler: deprecated (8.5), raised with its buffer
        // disabled, so the message goes past it.
        if r.is_ok() && self.out.buffers.get(at).is_some_and(|b| b.produced) {
            self.out.buffers[at].produced = false;
            self.out.buffers[at].disabled = true;
            let message = format!("Producing output from user output handler {} is deprecated", handler_name(&Some(handler.clone())));
            match self.running_builtin {
                Some(i) => {
                    let name = self.isolate.registry.function(i).name;
                    self.show(E_DEPRECATED, &format!("{name}(): {message}"));
                    // A user error handler runs now, inside the ob_* call.
                    if let Err(t) = self.dispatch_errors(regs) {
                        *late = Some(t);
                    }
                }
                None => self.print_diag(E_DEPRECATED, &format!("PHP Request Shutdown: {message}"), "Unknown", 0),
            }
            if let Some(b) = self.out.buffers.get_mut(at) {
                b.disabled = false;
            }
        }
        self.out.running = outer;
        let r = r?;
        Ok(if r.as_bool() == Some(false) { data } else { r.to_php_string().map_or_else(Vec::new, |s| s.as_bytes().to_vec()) })
    }

    pub(crate) fn ob_with(&mut self, regs: &mut Vec<Value>, op: p8_runtime::ext::Ob) -> Result<Value, Throw> {
        use p8_runtime::ext::Ob;
        let none = self.out.buffers.is_empty();
        Ok(match op {
            Ob::Start { handler, chunk } => {
                let handler = (!handler.is_null()).then_some(handler);
                self.out.buffers.push(Buffer { data: Vec::new(), handler, chunk: chunk.max(0) as usize, started: false, produced: false, disabled: false });
                Value::TRUE
            }
            _ if none && !matches!(op, Ob::Level | Ob::Handlers | Ob::Status { .. }) => Value::FALSE,
            Ob::Contents => Value::str(PhpStr::new(self.out.buffers.last().map_or_else(Vec::new, |b| b.data.clone()))),
            Ob::Length => Value::int(self.out.buffers.last().map_or(0, |b| b.data.len()) as i64),
            Ob::Level => Value::int(self.out.buffers.len() as i64),
            Ob::Clean => {
                let mut late = None;
                self.ob_handle(regs, OB_CLEAN, &mut late)?;
                late.map_or(Ok(()), Err)?;
                Value::TRUE
            }
            Ob::Flush => {
                let mut late = None;
                let out = self.ob_handle(regs, OB_FLUSH, &mut late)?;
                self.out.write_below(&out);
                self.note_output_start();
                late.map_or(Ok(()), Err)?;
                Value::TRUE
            }
            Ob::End { flush } => {
                let phase = if flush { OB_FINAL } else { OB_CLEAN | OB_FINAL };
                let mut late = None;
                let out = self.ob_handle(regs, phase, &mut late)?;
                if flush {
                    self.out.write_below(&out);
                    self.note_output_start();
                }
                self.out.buffers.pop();
                late.map_or(Ok(()), Err)?;
                Value::TRUE
            }
            Ob::Handlers => {
                let names: Vec<Value> = self.out.buffers.iter().map(|b| Value::str(handler_name(&b.handler))).collect();
                Value::array(PhpArray::from_list(names))
            }
            Ob::Status { full } => {
                let status = |level: usize, b: &Buffer| {
                    let mut a = PhpArray::new();
                    let user = b.handler.is_some();
                    a.insert(Key::from_string(PhpStr::from("name")), Value::str(handler_name(&b.handler)));
                    a.insert(Key::from_string(PhpStr::from("type")), Value::int(i64::from(user)));
                    // PHP_OUTPUT_HANDLER_STDFLAGS (| USER); STARTED | PROCESSED once called.
                    a.insert(Key::from_string(PhpStr::from("flags")), Value::int(if user { 0x71 } else { 0x70 } | if b.started { 0x5000 } else { 0 }));
                    a.insert(Key::from_string(PhpStr::from("level")), Value::int(level as i64));
                    a.insert(Key::from_string(PhpStr::from("chunk_size")), Value::int(b.chunk as i64));
                    a.insert(Key::from_string(PhpStr::from("buffer_size")), Value::int(16384));
                    a.insert(Key::from_string(PhpStr::from("buffer_used")), Value::int(b.data.len() as i64));
                    Value::array(a)
                };
                if full {
                    let all: Vec<Value> = self.out.buffers.iter().enumerate().map(|(i, b)| status(i, b)).collect();
                    Value::array(PhpArray::from_list(all))
                } else {
                    match self.out.buffers.last() {
                        Some(b) => status(self.out.buffers.len() - 1, b),
                        None => Value::array(PhpArray::new()),
                    }
                }
            }
        })
    }

    /// The request's end: each buffer left is flushed through its handler.
    fn end_buffers(&mut self, regs: &mut Vec<Value>) {
        while !self.out.buffers.is_empty() {
            if let Err(t) = self.ob_with(regs, p8_runtime::ext::Ob::End { flush: true }) {
                let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
                self.report_end(Err(t));
                self.out.buffers.pop();
            }
        }
    }

    /// Write to the output and flush it (messages the host shows as they come).
    pub fn echo_and_flush(&mut self, bytes: &[u8]) {
        self.write_out(bytes);
        self.out.flush_all();
        self.note_output_start();
    }

    /// header_register_callback(): its callback runs once, just before the
    /// headers go out: when output first reaches the host (not while it is
    /// buffered), or at the end of the script (`at_end`).
    fn run_header_callback(&mut self, regs: &mut Vec<Value>, at_end: bool) -> Result<(), Throw> {
        if self.out.started || self.out.head_sent || (!at_end && !self.out.buffers.is_empty()) {
            return Ok(());
        }
        match self.ext_state.remove("header_callback") {
            Some(callback) if !callback.is_null() => self.call_with_regs(regs, &callback, &mut []).map(|_| ()),
            _ => Ok(()),
        }
    }

    /// Write to the output (the innermost buffer, or the host).
    fn write_out(&mut self, bytes: &[u8]) {
        self.out.write(bytes);
        self.note_output_start();
    }

    /// The first output to reach the host sends the headers: remember
    /// where the code was (headers_sent(), "headers already sent").
    fn note_output_start(&mut self) {
        if self.out.started && self.output_start.is_none() {
            self.output_start = Some(self.location());
        }
    }

    /// Run a file's top-level code; returns the exit status.
    pub fn run_main(&mut self, main: Arc<Function>) -> i32 {
        if let Ok(path) = p8_runtime::os::canonicalize(&*main.file) {
            self.included_order.push(path.to_string_lossy().into_owned());
            self.included.insert(path);
        }
        self.standard_streams();
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let ncv = main.ncv as usize;
        let here = 0u8;
        self.stack_base = std::ptr::addr_of!(here) as usize;
        self.frames.push(&Closure::new(Arc::clone(&main), self.isolate), 0, 0, NO_RET);
        self.frames.last_mut().expect("the main frame").global = true;
        enter(&mut self.frames, &mut regs);
        for (name, value) in self.request_globals() {
            match main.cv_names.iter().position(|n| n.as_bytes() == name.as_bytes()) {
                Some(r) => regs[r] = value,
                None => {
                    self.globals.insert(name.as_bytes().to_vec(), Rc::new(RefCell::new(value)));
                }
            }
        }
        let defs = std::mem::take(&mut self.class_defs);
        self.raise_compile_diags(&main);
        let started = self.run_request_hooks(&mut regs, false);
        let mut result = match started.and_then(|()| self.early_bind(&defs, Some(Rc::from(&*main.file)))) {
            Ok(()) => self.execute(&mut regs),
            Err(e) => Err(self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), e)),
        };
        if let Err(t) = result {
            result = Err(self.handle_uncaught(&mut regs, t));
        }
        // exit() and an uncaught exception end the script cleanly enough
        // for destructors (a fatal error does not).
        let clean_end = matches!(result, Err(Throw::Exit(_) | Throw::Object(_) | Throw::Error { .. } | Throw::Autoload(_)));
        let mut status = self.report_end(result);
        // Shutdown functions (when the main frame returned, they ran then).
        if !self.shutdown.is_empty() {
            self.frames.floor = 0;
            let r = self.run_shutdown(&mut regs);
            if let Err(t) = r {
                let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
                let t = self.handle_uncaught(&mut regs, t);
                status = self.report_end(Err(t));
            }
        }
        if clean_end {
            self.frames.floor = 0;
            if let Err(t) = self.run_destructors(&mut regs).and_then(|()| self.end_destructors(&mut regs, ncv)) {
                let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
                let t = self.handle_uncaught(&mut regs, t);
                status = self.report_end(Err(t));
            }
        }
        // The headers go out now at the latest: header_register_callback's
        // callback runs first.
        self.frames.floor = 0;
        if let Err(t) = self.run_header_callback(&mut regs, true) {
            let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
            let t = self.handle_uncaught(&mut regs, t);
            status = self.report_end(Err(t));
        }
        self.end_buffers(&mut regs);
        // The extensions' request-end work (a session is written here).
        let ended = self.run_request_hooks(&mut regs, true);
        let ended = ended.and_then(|()| if self.pending_errors.is_empty() { Ok(()) } else { self.dispatch_errors(&mut regs) });
        if let Err(t) = ended {
            let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
            let t = self.handle_uncaught(&mut regs, t);
            status = self.report_end(Err(t));
            self.end_buffers(&mut regs);
        }
        // The request's variables go now (the isolate keeps the capacity).
        self.ended_globals.clear();
        regs.clear();
        self.frames.clear();
        self.isolate.regs = regs;
        self.out.flush_all();
        self.out.send_head();
        status
    }

    /// The extensions' hooks for the start (`end` false) or the end of the
    /// request, the registers lent to them so that they may run PHP code.
    fn run_request_hooks(&mut self, regs: &mut Vec<Value>, end: bool) -> Result<(), Throw> {
        let registry = Arc::clone(&self.isolate.registry);
        let hooks = if end { registry.request_end_hooks() } else { registry.request_start_hooks() };
        if hooks.is_empty() {
            return Ok(());
        }
        self.isolate.regs = std::mem::take(regs);
        self.request_end = true;
        let mut result = Ok(());
        for hook in hooks {
            // A hook that throws at the end does not stop the others.
            if let Err(t) = hook(self) {
                if result.is_ok() {
                    result = Err(t);
                }
                if !end {
                    break;
                }
            }
        }
        self.request_end = false;
        *regs = std::mem::take(&mut self.isolate.regs);
        result
    }

    /// The exit status for how the code ended, with PHP's message for an
    /// error nothing caught.
    fn report_end(&mut self, result: Result<(), Throw>) -> i32 {
        match result {
            Ok(()) => 0,
            Err(Throw::Exit(status)) => status,
            Err(Throw::Fatal { message, file, line }) => {
                // With the stack trace it was raised with (8.5+).
                let trace = self.fatal_trace.take();
                let text = self.error_text_traced("Fatal error", &message, &file, line, true, trace.as_deref());
                self.write_out(text.as_bytes());
                self.last_error = Some((1, message, file.to_string(), line));
                255
            }
            Err(Throw::Parse { message, file, line }) => {
                let text = self.error_text("Parse error", &message, &file, line, true);
                self.write_out(text.as_bytes());
                self.last_error = Some((4, message, file.to_string(), line));
                255
            }
            // An uncaught ParseError is reported as the parse error it is
            // (zend_exception_error), not as "Uncaught".
            Err(Throw::Object(e)) if e.class.name.as_bytes().eq_ignore_ascii_case(b"ParseError") => {
                let message = e.class.read(&e, b"message", b"error").to_php_string().unwrap_or_else(PhpStr::empty);
                let file = e.class.read(&e, b"file", b"error").to_php_string().unwrap_or_else(PhpStr::empty);
                let line = e.class.read(&e, b"line", b"error").to_int() as u32;
                self.report_end(Err(Throw::Parse { message: String::from_utf8_lossy(message.as_bytes()).into_owned(), file: Rc::from(&*String::from_utf8_lossy(file.as_bytes())), line }))
            }
            Err(Throw::Error { class: "ParseError", message, file, line, .. }) => self.report_end(Err(Throw::Parse { message, file, line })),
            Err(Throw::Object(e)) => {
                let shown = builtin_classes::describe(&e);
                let root: &[u8] = if e.class.is_a(b"exception") { b"exception" } else { b"error" };
                let file = e.class.read(&e, b"file", root).to_php_string().unwrap_or_else(PhpStr::empty);
                let line = e.class.read(&e, b"line", root).to_int();
                let file = String::from_utf8_lossy(file.as_bytes());
                let text = self.error_text("Fatal error", &format!("Uncaught {shown}\n  thrown"), &file, line as u32, true);
                self.write_out(text.as_bytes());
                self.last_error = Some((1, format!("Uncaught {shown}\n  thrown"), file.to_string(), line as u32));
                255
            }
            Err(Throw::Error { class, message, file, line, .. }) => {
                let text = self.error_text("Fatal error", &format!("Uncaught {class}: {message} in {file}:{line}\nStack trace:\n#0 {{main}}\n  thrown"), &file, line, true);
                self.write_out(text.as_bytes());
                self.last_error = Some((1, format!("Uncaught {class}: {message} in {file}:{line}\nStack trace:\n#0 {{main}}\n  thrown"), file.to_string(), line));
                255
            }
            Err(Throw::Autoload(name)) => {
                let (file, line) = self.location();
                let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
                let text = self.error_text("Fatal error", &format!("Uncaught Error: Class \"{name}\" not found in {file}:{line}\nStack trace:\n#0 {{main}}\n  thrown"), &file, line, true);
                self.write_out(text.as_bytes());
                self.last_error = Some((1, format!("Uncaught Error: Class \"{name}\" not found in {file}:{line}\nStack trace:\n#0 {{main}}\n  thrown"), file.to_string(), line));
                255
            }
        }
    }

    /// The fatal error an interrupt ends the request with.
    #[cold]
    #[inline(never)]
    fn interrupted(&self) -> Throw {
        let (file, line) = self.location();
        let message = match self.stop.load(AtomicOrdering::Relaxed) {
            TERMINATE => "Execution was terminated by the host".to_string(),
            seconds => format!("Maximum execution time of {seconds} second{} exceeded", if seconds == 1 { "" } else { "s" }),
        };
        Throw::Fatal { message, file, line }
    }

    /// `(int)` / `(float)` of an object that has no such cast (not a
    /// resource, no cast handler): PHP warns and makes it 1
    /// (convert_object_to_type). (Only an object, or a reference, is
    /// looked at further: a scalar's cast goes on at once.)
    #[inline(always)]
    fn object_cast_warning(&mut self, v: &Value, to: &str) {
        if v.is_object() || v.is_reference() {
            self.object_cast_warning_slow(v, to);
        }
    }

    #[cold]
    #[inline(never)]
    fn object_cast_warning_slow(&mut self, v: &Value, to: &str) {
        if let Some(o) = v.deref().as_object()
            && o.class.flags & p8_runtime::object::CLASS_RESOURCE == 0
            && o.class.cast_int.is_none()
            && o.class.cast_string.is_none()
        {
            self.diags.push(Diag::Warning(format!("Object of class {} could not be converted to {to}", String::from_utf8_lossy(o.class.name.as_bytes()))));
            self.report_diags();
        }
    }

    fn throw(&self, class: &'static str, message: impl Into<String>) -> Throw {
        let (file, line) = self.location();
        Throw::Error { class, message: message.into(), file, line, props: Vec::new() }
    }

    fn location(&self) -> (Rc<str>, u32) {
        if self.request_end {
            return (Rc::from("Unknown"), 0);
        }
        match self.frames.last() {
            // A constant expression's code: where it was needed; a class's
            // (its constants, defaults, attributes), where it is written
            // (zend_ast_evaluate_ex's filename and lineno override).
            Some(f) if f.func.code.name.as_bytes() == p8_bytecode::CONST_EXPR.as_bytes() && !(f.func.code.own_location && f.scope.is_some()) => {
                let n = self.frames.len();
                match self.frames.slots[..n].iter().rev().find(|f| f.func.code.name.as_bytes() != p8_bytecode::CONST_EXPR.as_bytes()) {
                    Some(f) => (Rc::from(&*f.func.code.file), f.func.code.lines.get((f.pc as usize).saturating_sub(1)).copied().unwrap_or(0)),
                    None => (Rc::from(""), 0),
                }
            }
            Some(f) => (Rc::from(&*f.func.code.file), f.func.code.lines.get(self.pc.saturating_sub(1)).copied().unwrap_or(0)),
            // A fiber's task before its first frame: where the task that
            // started it is.
            None => match self.waiting_tasks().filter_map(|t| Some((t.frames.last()?, t.pc))).last() {
                Some((f, pc)) => (Rc::from(&*f.func.code.file), f.func.code.lines.get(pc.saturating_sub(1)).copied().unwrap_or(0)),
                None => (Rc::from(""), 0),
            },
        }
    }

    fn op_error(&self, e: OpError) -> Throw {
        match e {
            OpError::Type(m) => self.throw("TypeError", m),
            OpError::DivisionByZero(m) => self.throw("DivisionByZeroError", m),
            OpError::Arithmetic(m) => self.throw("ArithmeticError", m),
            OpError::Error(m) => self.throw("Error", m),
        }
    }

    /// Show the diagnostics the last operation raised, at the current line.
    #[inline]
    fn report_diags(&mut self) {
        if !self.diags.is_empty() {
            self.print_diags();
        }
    }

    #[cold]
    #[inline(never)]
    fn print_diags(&mut self) {
        for d in std::mem::take(&mut self.diags) {
            match d {
                Diag::Warning(m) => self.show(E_WARNING, &m),
                Diag::Deprecated(m) => self.show(E_DEPRECATED, &m),
            }
        }
    }

    /// Print a diagnostic of PHP error `level` at the current line, as the
    /// CLI does, if error_reporting and display_errors let it through.
    /// Whether the user error handler takes errors of `level` now.
    fn user_handles_level(&self, level: i64) -> bool {
        !self.in_handler && matches!(self.error_handlers.last(), Some((h, mask)) if mask & level != 0 && !h.is_null())
    }

    fn show(&mut self, level: i64, message: &str) {
        // A user error handler gets it (after the instruction), whatever
        // error_reporting says.
        if self.user_handles_level(level) {
            let (file, line) = self.location();
            self.pending_errors.push((level, message.to_string(), file, line));
            return;
        }
        let (file, line) = self.location();
        // (an error raised where no file runs is located "Unknown")
        let file = if file.is_empty() { Rc::from("Unknown") } else { file };
        self.print_diag(level, message, &file, line);
    }

    /// Print a diagnostic as the CLI does, if error_reporting and
    /// display_errors let it through.
    /// How an error is worded for display: PHP's text, or HTML when
    /// html_errors is on (a web server's default). The message of a fatal
    /// error is escaped (`escape`), the others are not, as in php_error_cb().
    pub fn error_text(&self, label: &str, message: &str, file: &str, line: u32, escape: bool) -> String {
        self.error_text_traced(label, message, file, line, escape, None)
    }

    /// error_text, with the stack trace a fatal error was raised with
    /// (php_error_cb: after "on line N" in text; the HTML of an E_ERROR,
    /// which is escaped, shows none).
    pub fn error_text_traced(&self, label: &str, message: &str, file: &str, line: u32, escape: bool, trace: Option<&str>) -> String {
        let html = match Ctx::ini_get(self, b"html_errors") {
            Some(v) => match v.as_str() {
                Some(s) => matches!(s.as_bytes().to_ascii_lowercase().as_slice(), b"1" | b"on" | b"true" | b"yes"),
                None => v.to_bool(),
            },
            None => false,
        };
        if !html {
            return match trace {
                Some(trace) => format!("\n{label}: {message} in {file} on line {line}\nStack trace:\n{trace}\n"),
                None => format!("\n{label}: {message} in {file} on line {line}\n"),
            };
        }
        let message = if escape { message.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") } else { message.to_string() };
        format!("<br />\n<b>{label}</b>:  {message} in <b>{file}</b> on line <b>{line}</b><br />\n")
    }

    fn print_diag(&mut self, level: i64, message: &str, file: &str, line: u32) {
        // Recorded even when silenced (`@`, error_reporting), as PHP does.
        self.last_error = Some((level, message.to_string(), file.to_string(), line));
        if self.error_level & level == 0 {
            return;
        }
        let label = match level {
            E_WARNING | E_USER_WARNING => "Warning",
            E_NOTICE | E_USER_NOTICE => "Notice",
            E_DEPRECATED | E_USER_DEPRECATED => "Deprecated",
            _ => "Fatal error",
        };
        // log_errors: to error_log too (plain text whatever html_errors says).
        if let Some(log) = self.isolate.registry.error_logger()
            && Ctx::ini_get(self, b"log_errors").is_some_and(|v| ini_on(&v))
        {
            log(self, format!("PHP {label}:  {message} in {file} on line {line}").as_bytes());
        }
        if !self.display_errors {
            return;
        }
        let text = self.error_text(label, message, file, line, false);
        self.write_out(text.as_bytes());
    }

    fn warn(&mut self, message: impl Into<String>) {
        self.diags.push(Diag::Warning(message.into()));
        self.report_diags();
    }

    fn notice(&mut self, message: &str) {
        self.show(E_NOTICE, message);
    }

    /// A property instruction's object: THIS is the running method's own
    /// `$this` (from its frame), else the register's value.
    #[inline]
    fn prop_target(&mut self, w: &[Value], clo: &Closure, obj: Reg) -> Result<Value, Throw> {
        if obj == p8_bytecode::THIS {
            return match self.frames.last().and_then(|f| f.this.clone()) {
                Some(this) => Ok(Value::object(this)),
                None => Err(self.throw("Error", "Using $this when not in object context")),
            };
        }
        Ok(self.operand(w, clo, obj))
    }

    /// The value in register `r` of the frame window `w`, as an operand:
    /// looks through references; an undefined variable warns and reads as null.
    #[inline(always)]
    fn operand(&mut self, w: &[Value], clo: &Closure, r: Reg) -> Value {
        // A constant operand: the closure's constant, read in place.
        if let Some(k) = const_index(r) {
            return clo.consts[k].clone();
        }
        let func = &clo.code;
        let v = &w[r as usize];
        if v.is_undef() {
            self.undefined(func, r)
        } else if let Some(cell) = v.as_reference() {
            let inner = cell.borrow();
            // A global slot another file unset.
            if inner.is_undef() {
                drop(inner);
                self.undefined(func, r)
            } else {
                inner.clone()
            }
        } else {
            v.clone()
        }
    }

    /// `operand`, moved out of the register when `take` (the compiler marks
    /// a temporary's only read: the value is consumed, as PHP consumes a
    /// TMP operand, with no copy and no later drop of the register's).
    fn operand_take(&mut self, w: &mut [Value], clo: &Closure, r: Reg, take: bool) -> Value {
        if const_index(r).is_some() {
            return self.operand(w, clo, r);
        }
        let slot = &w[r as usize];
        if slot.is_undef() {
            self.operand(w, clo, r)
        } else if slot.is_reference() {
            // A reference's value; a taken register lets go of it too.
            let v = self.operand(w, clo, r);
            if take {
                clear(&mut w[r as usize]);
            }
            v
        } else if take {
            std::mem::take(&mut w[r as usize])
        } else {
            slot.clone()
        }
    }

    /// `operand` for an instruction that only looks at the value: the
    /// register's own value, borrowed (no copy, so no drop of one after);
    /// a copy only for an undefined register (it warns) or a reference.
    #[inline]
    fn operand_ref<'a>(&mut self, w: &'a [Value], clo: &'a Closure, r: Reg) -> std::borrow::Cow<'a, Value> {
        if let Some(k) = const_index(r) {
            return std::borrow::Cow::Borrowed(&clo.consts[k]);
        }
        let v = &w[r as usize];
        if v.is_undef() || v.is_reference() { std::borrow::Cow::Owned(self.operand(w, clo, r)) } else { std::borrow::Cow::Borrowed(v) }
    }

    /// A register's or a constant operand's value, in place (what `w[r]`
    /// is for a register: no undefined check, no reference read).
    #[inline(always)]
    fn val<'a>(w: &'a [Value], clo: &'a Closure, r: Reg) -> &'a Value {
        match const_index(r) {
            Some(k) => &clo.consts[k],
            None => &w[r as usize],
        }
    }

    /// What compiling a file raised (its main code's `compile_diags`),
    /// raised now that it is compiled and before it runs, as PHP 8.5 does
    /// with errors delayed during compilation: a user error handler gets
    /// them (after the include or eval), else they are shown at their line.
    pub(crate) fn raise_compile_diags(&mut self, main: &Function) {
        if main.compile_diags.is_empty() {
            return;
        }
        let file: Rc<str> = Rc::from(&*main.file);
        for d in &main.compile_diags {
            let message = String::from_utf8_lossy(d.message.as_bytes()).into_owned();
            if self.user_handles_level(d.level) {
                self.pending_errors.push((d.level, message, Rc::clone(&file), d.line));
            } else {
                self.print_diag(d.level, &message, &file, d.line);
            }
        }
    }

    /// A register as a condition (i_zend_is_true): NAN is true, with
    /// PHP 8.5's "unexpected NAN value was coerced to bool" warning.
    #[inline]
    fn truthy(&mut self, w: &[Value], clo: &Closure, r: Reg) -> bool {
        let v = self.operand_ref(w, clo, r);
        // One look at the value: bools (most conditions) answer at once, and
        // only a float or a reference can be NAN (#312).
        if let Some(b) = v.as_bool() {
            b
        } else if (v.is_float() || v.is_reference()) && v.is_nan() {
            self.nan_coerced("bool");
            true
        } else {
            v.to_bool()
        }
    }

    #[cold]
    #[inline(never)]
    fn nan_coerced(&mut self, type_name: &str) {
        self.warn(format!("unexpected NAN value was coerced to {type_name}"));
    }

    /// Reading an undefined register: a warning for a variable, then null.
    #[cold]
    #[inline(never)]
    fn undefined(&mut self, func: &Function, r: Reg) -> Value {
        if let Some(name) = func.cv_names.get(r as usize) {
            let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
            self.warn(format!("Undefined variable ${name}"));
        }
        Value::NULL
    }

    /// Move the value out of variable register `r` (leaving null), so it
    /// can be changed in place without a copy; undefined warns.
    fn take_var(&mut self, w: &mut [Value], func: &Function, r: Reg) -> Value {
        let slot = &mut w[r as usize];
        if slot.is_undef() {
            self.undefined(func, r)
        } else if let Some(cell) = slot.as_reference() {
            std::mem::take(&mut *cell.borrow_mut())
        } else {
            std::mem::take(slot)
        }
    }

    /// What call site `site` names (cached after the first time).
    #[inline(always)]
    fn resolve(&mut self, clo: &Closure, site: u16) -> Result<Callee, Throw> {
        match clo.sites[site as usize].get() {
            Some(c) => Ok(c.clone()),
            None => self.resolve_slow(clo, site),
        }
    }

    #[cold]
    #[inline(never)]
    fn resolve_slow(&mut self, clo: &Closure, site: u16) -> Result<Callee, Throw> {
        let s = &clo.code.sites[site as usize];
        let key = s.sym;
        let global = s.fallback.as_ref().map(|g| g.1);
        let callee = if let Some(f) = self.functions.get(&key) {
            Callee::User(*f)
        } else if let Some(i) = self.builtin(key) {
            Callee::Builtin(i)
        } else if let Some(f) = global.and_then(|g| self.functions.get(&g)) {
            Callee::User(*f)
        } else if let Some(i) = global.and_then(|g| self.builtin(g)) {
            Callee::Builtin(i)
        } else {
            let name = String::from_utf8_lossy(s.display.as_bytes()).into_owned();
            return Err(self.throw("Error", format!("Call to undefined function {name}()")));
        };
        // A user function's closure is made with the first site that calls
        // it: `fast` then finds it at the index this site caches.
        if let Callee::User(f) = callee {
            self.closure_at(f);
        }
        let _ = clo.sites[site as usize].set(callee.clone());
        Ok(callee)
    }

    /// Call builtin `i` from the loop (`Instr::Call` at `pc` of the frame
    /// at `base`, its call site resolved to the builtin; `call` is the
    /// instruction's arguments' start, count and result register): the
    /// call, then what it raised (an exception unwound to its handler,
    /// errors to the error handler, the destructors due). Returns the pc
    /// to go on at.
    #[cold]
    #[inline(never)]
    fn builtin_call(&mut self, regs: &mut Vec<Value>, pc: usize, base: usize, i: usize, call: (Reg, u8, Reg)) -> Result<usize, Throw> {
        let (args, argc, dst) = call;
        self.pc = pc + 1;
        let next = match self.call_builtin(regs, base, i, args, argc, dst) {
            Ok(()) => pc + 1,
            Err(t) => self.unwind(regs, t, pc)?,
        };
        if !self.pending_errors.is_empty() || self.ids.has_pending() {
            return self.after_errors(regs, next, pc);
        }
        Ok(next)
    }

    /// A frameless call (PHP's ZEND_FRAMELESS_ICALL): builtin `i`, by
    /// `run` (from its registry entry), with arguments `args` that the
    /// call site found plainly of their declared types. No frame of either
    /// kind is pushed and no argument is checked, moved or copied: the
    /// builtin reads the registers in place. The result, or None when it
    /// raised (the Throw waits in `fault`). The loop does the rest in
    /// line (`execute`'s `frameless!`), or `frameless_tail` when there is
    /// more to do. (A builtin of the moved ABI gets the registers mutably,
    /// as the checked path's common case gives them; its error waits in
    /// `fault` like a borrowed builtin's.)
    #[inline(always)]
    pub(crate) fn frameless_run(&mut self, i: usize, run: Imp, args: &mut [Value]) -> Option<Value> {
        let outer = self.running_builtin.replace(i);
        let r = match run {
            Imp::Lent(run) => run(self, args).ok(),
            Imp::Moved(run) => match run(self, args) {
                Ok(v) => Some(v),
                Err(t) => {
                    self.fault = Some(t);
                    None
                }
            },
        };
        self.running_builtin = outer;
        r
    }

    /// The rest of the frameless call at `at` that raised (`r` None) or
    /// left diagnostics: the arguments `argc` at `start` (absolute) are
    /// cleared when they are the call's own (`temps`), the result goes to
    /// `dst`, and what was raised is handled as the checked path would.
    /// Returns the pc to go on at.
    #[cold]
    #[inline(never)]
    pub(crate) fn frameless_tail(&mut self, regs: &mut Vec<Value>, at: usize, i: usize, r: Option<Value>, (start, argc, dst, temps): (usize, usize, usize, bool)) -> Result<usize, Throw> {
        let Some(v) = r else { return self.frameless_fault(regs, at, i, (start, argc, temps)) };
        if !self.diags.is_empty() || !self.pending_errors.is_empty() {
            self.frameless_diags(&regs[start..start + argc], i);
        }
        if temps {
            for r in &mut regs[start..start + argc] {
                let _ = clear(r);
            }
        }
        let _ = put(&mut regs[dst], v);
        if !self.pending_errors.is_empty() || self.ids.has_pending() {
            return self.after_errors(regs, at + 1, at);
        }
        Ok(at + 1)
    }

    /// What a frameless call reported, as the checked path reports it: an
    /// E_USER_ERROR waiting for the user handler (trigger_error()) starts
    /// its trace with this call, and the warnings are shown.
    #[cold]
    #[inline(never)]
    fn frameless_diags(&mut self, args: &[Value], i: usize) {
        if self.pending_errors.iter().any(|e| e.0 == 256) {
            self.failed_call(self.isolate.registry.function(i).name.as_bytes(), None, args);
        }
        self.report_diags();
    }

    /// A frameless call that raised: the builtin is the trace's first
    /// frame (as the checked path makes it), then the exception unwinds
    /// from the call.
    #[cold]
    #[inline(never)]
    fn frameless_fault(&mut self, regs: &mut Vec<Value>, pc: usize, i: usize, (start, argc, temps): (usize, usize, bool)) -> Result<usize, Throw> {
        let mut t = self.take_fault();
        match t {
            Throw::Error { .. } | Throw::Fatal { .. } => self.failed_call(self.isolate.registry.function(i).name.as_bytes(), None, &regs[start..start + argc]),
            Throw::Autoload(n) => t = self.unlent_autoload(n),
            _ => {}
        }
        self.report_diags();
        if temps {
            for r in &mut regs[start..start + argc] {
                clear(r);
            }
        }
        let next = self.unwind(regs, t, pc)?;
        if !self.pending_errors.is_empty() || self.ids.has_pending() {
            return self.after_errors(regs, next, pc);
        }
        Ok(next)
    }

    /// Run the instruction at `pc` of the top frame the way `execute`'s
    /// loop does not: through `step`, then what it raised (an exception
    /// unwound to its handler, errors to the error handler, the
    /// destructors due). Called from inside the loop, which goes on with
    /// the pc this gives back.
    #[cold]
    #[inline(never)]
    fn slow(&mut self, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
        self.detour(regs, pc, Self::step)
    }

    /// A generated instruction's case its handler does not take (`Ctl::Cold`):
    /// its cold method `run` (the instruction whole, as `step` runs it, with
    /// none of `step`'s dispatch), then what `slow` does after. An
    /// interrupt due (a backward jump's) is raised first.
    #[cold]
    #[inline(never)]
    fn cold(&mut self, regs: &mut Vec<Value>, pc: usize, run: Run<Self>) -> Result<Flow, Throw> {
        self.detour(regs, pc, run)
    }

    /// Run the instruction at `pc` by `run` out of the loop, and what it
    /// raised (`slow`'s doc).
    #[inline(always)]
    fn detour(&mut self, regs: &mut Vec<Value>, mut pc: usize, run: Run<Self>) -> Result<Flow, Throw> {
        let frame = self.frames.last().expect("a frame");
        let func = Rc::clone(&frame.func);
        let (base, argc) = (frame.base as usize, frame.argc as usize);
        self.pc = pc + 1;
        let at = pc;
        let ran = if self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT { Err(self.interrupted()) } else { run(self, regs, &func, base, argc, pc) };
        match ran {
            Ok(Next::Go(next)) => {
                pc = next;
                // Fiber::suspend(): leave the fiber's frames where they are.
                if self.fiber_suspended.is_some() {
                    if let Some(top) = self.frames.last_mut() {
                        top.pc = pc as u32;
                    }
                    return Ok(Flow::Done);
                }
            }
            Ok(Next::Switch) => {
                if self.frames.len() == self.frames.floor {
                    if !self.pending_errors.is_empty() {
                        self.dispatch_errors(regs)?;
                    }
                    if self.ids.has_pending() {
                        self.run_destructors(regs)?;
                    }
                    return Ok(Flow::Done);
                }
                pc = self.frames.last().expect("a frame").pc as usize;
            }
            // A class an autoloader may declare: run them, then this
            // instruction again.
            Err(Throw::Autoload(name)) => pc = self.autoload_then(regs, name, at)?,
            Err(t) => pc = self.unwind(regs, t, pc)?,
        }
        if !self.pending_errors.is_empty() || self.ids.has_pending() {
            pc = self.after_errors(regs, pc, at)?;
        }
        Ok(Flow::Go(pc))
    }

    /// Run the autoloaders for `name`, then go on at `at` (the instruction
    /// that wanted the class, again).
    #[cold]
    fn autoload_then(&mut self, regs: &mut Vec<Value>, name: PhpStr, at: usize) -> Result<usize, Throw> {
        // Their frames show the line of the instruction.
        if let Some(top) = self.frames.last_mut() {
            top.pc = at as u32 + 1;
        }
        match self.run_autoload(regs, name) {
            Ok(()) => Ok(at),
            Err(t) => self.unwind(regs, t, at),
        }
    }

    /// Call the error handler for the diagnostics the last instruction
    /// raised, then the destructors of the objects it freed; an exception
    /// either throws is raised there.
    #[cold]
    fn after_errors(&mut self, regs: &mut Vec<Value>, pc: usize, at: usize) -> Result<usize, Throw> {
        let r = if self.pending_errors.is_empty() { Ok(()) } else { self.dispatch_errors(regs) };
        if let Some((slot, v)) = self.deferred_write.take()
            && let Some(cell) = regs.get_mut(slot)
        {
            match cell.as_reference() {
                Some(r) => *r.borrow_mut() = v,
                None => *cell = v,
            }
        }
        match r.and_then(|()| self.run_destructors_at(regs, Some(at + 1))) {
            Ok(()) => Ok(pc),
            Err(t) => self.unwind(regs, t, at),
        }
    }

    /// Runs a magic property method found by prop_magic, its guard held.
    fn run_magic(&mut self, regs: &mut Vec<Value>, found: (crate::classes::MethodFrame, (usize, PhpStr, crate::classes::Magic)), args: &mut [Value]) -> Result<Value, Throw> {
        let (m, key) = found;
        self.magic_guard.push(key.clone());
        let r = self.run_nested(regs, m, args);
        if let Some(i) = self.magic_guard.iter().rposition(|k| *k == key) {
            self.magic_guard.remove(i);
        }
        r
    }

    /// `$o->p op= …`, `$o->p++` on an overloaded property: read through
    /// __get (or the property), compute, write through __set (or the
    /// property). None: no magic method takes part.
    fn magic_update(
        &mut self,
        regs: &mut Vec<Value>,
        target: &Value,
        name: &Value,
        compute: impl FnOnce(&mut Self, &Value) -> Result<(Value, Value), Throw>,
    ) -> Result<Option<Value>, Throw> {
        use crate::classes::Magic;
        let get = self.prop_magic(target, name, Magic::Get);
        let has_set = self.prop_magic(target, name, Magic::Set).is_some();
        if get.is_none() && !has_set {
            return Ok(None);
        }
        let pname = Value::str(name.to_php_string().unwrap_or_else(PhpStr::empty));
        let x = match get {
            Some(found) => self.run_magic(regs, found, &mut [pname.clone()])?.deref(),
            None => self.read_for_update(target, name)?,
        };
        let (stored, result) = compute(self, &x)?;
        match self.prop_magic(target, name, Magic::Set) {
            Some(found) => {
                self.run_magic(regs, found, &mut [pname, stored])?;
            }
            None => self.write_prop(target, name, stored)?,
        }
        Ok(Some(result))
    }

    /// `$o->p op= …`, `$o->p++` on a hooked property: read through its get
    /// hook (or the stored value), compute (the stored value, the
    /// expression's), write through its set hook. None: not hooked.
    fn hooked_update(
        &mut self,
        regs: &mut Vec<Value>,
        target: &Value,
        name: &Value,
        compute: impl FnOnce(&mut Self, &Value) -> Result<(Value, Value), Throw>,
    ) -> Result<Option<Value>, Throw> {
        let get = self.prop_hook(target, name, HOOK_GET);
        let set = self.prop_hook(target, name, HOOK_SET);
        if get.is_none() && set.is_none() {
            return Ok(None);
        }
        let x = match get {
            Some(hook) => self.run_nested(regs, hook?, &mut [])?,
            None => self.read_for_update(target, name)?,
        };
        let (stored, result) = compute(self, &x)?;
        match self.prop_hook(target, name, HOOK_SET) {
            Some(hook) => {
                self.run_nested(regs, hook?, &mut [stored])?;
            }
            None => self.write_prop(target, name, stored)?,
        }
        Ok(Some(result))
    }

    /// Execute the instruction at `pc` of `func`'s frame (at `base`).
    #[inline(never)]
    fn step(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, argc: usize, at: usize) -> Result<Next, Throw> {
        // This instruction's index (a lazy object's initialization runs it again).
        let insn_at = at;
        let func = &*clo.code;
        if self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
            return Err(self.interrupted());
        }
        // The one-operator arithmetic opcodes run here as `Binary`.
        let instr = match func.code[at].as_binary() {
            Some((op, dst, a, b)) => Instr::Binary { op, dst, a, b },
            None => func.code[at],
        };
        let mut pc = at + 1;
        // `w` is this frame's window (registers are frame-relative); `below`
        // holds the frames under it, the main frame's globals first.
        let in_main = base == 0;
        let (below, w) = regs.split_at_mut(base);
        let w = &mut w[..func.nregs as usize];
        macro_rules! reg {
            ($r:expr) => {
                w[$r as usize]
            };
        }
        // A name operand (a property's, a class constant's, a static
        // property's): its constant, read in place, else the register a
        // run-time name is in. (Any operand that may be a constant:
        // `consts::place_consts` says which.)
        macro_rules! name {
            ($r:expr) => {
                Self::val(w, clo, $r)
            };
        }
        match instr {
                // The generated instructions' (`cold_run`, their cold path).
                Instr::Move { .. }
                | Instr::Assign { .. }
                | Instr::AssignTake { .. }
                | Instr::Free { .. }
                | Instr::LoadConst { .. }
                | Instr::This { .. }
                | Instr::JumpIf { .. }
                | Instr::JumpIfNot { .. }
                | Instr::JumpLt { .. }
                | Instr::JumpNotLt { .. }
                | Instr::JumpLe { .. }
                | Instr::JumpNotLe { .. }
                | Instr::JumpEq { .. }
                | Instr::JumpNotEq { .. }
                | Instr::JumpIdentical { .. }
                | Instr::JumpNotIdentical { .. }
                | Instr::FetchDim { .. }
                | Instr::AssignDim { .. }
                | Instr::IssetDim { .. }
                | Instr::FetchProp { .. }
                | Instr::AssignProp { .. } => return self.cold_run(regs, clo, base, argc, at),
                Instr::LoadNull { dst } => reg!(dst) = Value::NULL,
                Instr::Add { .. } | Instr::Sub { .. } | Instr::Mul { .. } => unreachable!("normalized to Binary above"),
                Instr::Binary { op, dst, a, b } => {
                    // Fast paths for the common int cases.
                    let (x, y) = (Self::val(w, clo, a), Self::val(w, clo, b));
                    let fast = match (x.as_int(), y.as_int(), x.as_float(), y.as_float(), op) {
                        (Some(x), Some(y), _, _, BinOp::Add) => x.checked_add(y).map(Value::int),
                        (Some(x), Some(y), _, _, BinOp::Sub) => x.checked_sub(y).map(Value::int),
                        (Some(x), Some(y), _, _, BinOp::Mul) => x.checked_mul(y).map(Value::int),
                        (_, _, Some(x), Some(y), _) => float_op(op, x, y),
                        (Some(x), _, _, Some(y), _) => float_op(op, x as f64, y),
                        (_, Some(y), Some(x), _, _) => float_op(op, x, y as f64),
                        _ => None,
                    };
                    let v = match fast {
                        Some(v) => v,
                        None => {
                            let mut x = self.operand(w, clo, a);
                            let mut y = self.operand(w, clo, b);
                            if op == BinOp::Concat && (x.is_object() || y.is_object()) {
                                x = self.string_of(regs, x)?;
                                y = self.string_of(regs, y)?;
                                let r = ops::binary(op, &x, &y, &mut self.diags);
                                self.report_diags();
                                let v = r.map_err(|e| self.op_error(e))?;
                                assign(&mut regs[base..], dst, v);
                                return Ok(Next::Go(pc));
                            }
                            let r = ops::binary(op, &x, &y, &mut self.diags);
                            self.report_diags();
                            r.map_err(|e| self.op_error(e))?
                        }
                    };
                    // The result may go straight to a variable (`$x = $a + $b`).
                    assign(w, dst, v);
                }
                Instr::AssignOp { op, var, src } => {
                    let y = self.operand(w, clo, src);
                    if op == BinOp::Concat {
                        let y = self.string_of(regs, y)?;
                        // An object on the left: its __toString() first.
                        if regs[base + var as usize].deref().is_object() {
                            let x = regs[base + var as usize].deref();
                            let x = self.string_of(regs, x)?;
                            assign(&mut regs[base..], var, x);
                        }
                        let w = &mut regs[base..base + func.nregs as usize];
                        // `.=` appends in place when the string is not shared
                        let x = self.take_var(w, func, var);
                        if let Some(e) = ops::object_string_error(&x).or_else(|| ops::object_string_error(&y)) {
                            assign(w, var, x);
                            return Err(self.op_error(e));
                        }
                        let mut s = match x.into_str() {
                            Ok(s) => s,
                            Err(other) => ops::to_string_lossy(&other, &mut self.diags),
                        };
                        let tail = ops::to_string_lossy(&y, &mut self.diags);
                        self.report_diags();
                        s.push_bytes(tail.as_bytes());
                        assign(w, var, Value::str(s));
                    } else {
                        let undefined = w[var as usize].is_undef();
                        let x = self.operand(w, clo, var);
                        let r = ops::binary(op, &x, &y, &mut self.diags);
                        self.report_diags();
                        let v = r.map_err(|e| self.op_error(e))?;
                        // An undefined variable's warning goes to the error
                        // handler before the write, as in PHP (it may unset
                        // the variable): the write waits for it.
                        if undefined && !self.pending_errors.is_empty() {
                            self.deferred_write = Some((base + var as usize, v));
                        } else {
                            assign(w, var, v);
                        }
                    }
                }
                Instr::Compare { cmp, dst, a, b } => {
                    let v = if cmp == Cmp::Spaceship {
                        let x = self.operand_ref(w, clo, a);
                        let y = self.operand_ref(w, clo, b);
                        compare(cmp, &x, &y)
                    } else {
                        Value::bool(self.test_regs(w, clo, cmp, a, b))
                    };
                    assign(w, dst, v);
                }
                // A fused step and test runs here as its step alone; the
                // test is the next instruction.
                Instr::IncVar { var }
                | Instr::DecVar { var }
                | Instr::IncJumpLt { var, .. }
                | Instr::IncJumpLe { var, .. }
                | Instr::DecJumpGt { var, .. }
                | Instr::DecJumpGe { var, .. } => {
                    let inc = matches!(instr, Instr::IncVar { .. } | Instr::IncJumpLt { .. } | Instr::IncJumpLe { .. });
                    if let Some(i) = reg!(var).as_int()
                        && let Some(n) = if inc { i.checked_add(1) } else { i.checked_sub(1) }
                    {
                        reg!(var) = Value::int(n);
                        return Ok(Next::Go(pc));
                    }
                    let old = self.operand(w, clo, var);
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    assign(w, var, new);
                }
                Instr::Not { dst, src } => {
                    let t = self.truthy(w, clo, src);
                    reg!(dst) = Value::bool(!t);
                }
                Instr::ToBool { dst, src } => {
                    let t = self.truthy(w, clo, src);
                    reg!(dst) = Value::bool(t);
                }
                Instr::Neg { dst, src } => {
                    let v = self.operand(w, clo, src);
                    let r = ops::negate(&v, &mut self.diags);
                    self.report_diags();
                    reg!(dst) = r.map_err(|e| self.op_error(e))?;
                }
                Instr::BitNot { dst, src } => {
                    let v = self.operand(w, clo, src);
                    let r = ops::bit_not(&v, &mut self.diags);
                    self.report_diags();
                    reg!(dst) = r.map_err(|e| self.op_error(e))?;
                }
                Instr::Cast { kind: CastKind::String, dst, src } => {
                    let v = self.operand(w, clo, src);
                    let v = self.string_of(regs, v)?;
                    if let Some(e) = ops::object_string_error(&v) {
                        return Err(self.op_error(e));
                    }
                    let s = ops::to_string_lossy(&v, &mut self.diags);
                    self.report_diags();
                    regs[base + dst as usize] = Value::str(s);
                }
                Instr::Cast { kind, dst, src } => {
                    let v = self.operand(w, clo, src);
                    reg!(dst) = match kind {
                        CastKind::Int => {
                            // zval_get_long: a float through zend_dval_to_lval,
                            // which warns when it isn't representable (8.5+).
                            if let Some(f) = v.deref().as_float() {
                                self.diags.extend(ops::float_to_int_cast_diag(f));
                                self.report_diags();
                            }
                            self.object_cast_warning(&v, "int");
                            Value::int(v.to_int())
                        }
                        CastKind::Float => {
                            self.object_cast_warning(&v, "float");
                            Value::float(v.to_float())
                        }
                        CastKind::Bool => {
                            if v.is_nan() {
                                self.nan_coerced("bool");
                            }
                            Value::bool(v.to_bool())
                        }
                        CastKind::String => {
                            let v = self.object_to_string(v)?;
                            if let Some(e) = ops::object_string_error(&v) {
                                return Err(self.op_error(e));
                            }
                            let s = ops::to_string_lossy(&v, &mut self.diags);
                            self.report_diags();
                            Value::str(s)
                        }
                        CastKind::Array => {
                            if v.is_array() {
                                v
                            } else if let Some(o) = v.cloned_object() {
                                // A lazy object is initialized; a proxy is its instance.
                                if o.is_uninit_lazy() {
                                    self.lazy_init(regs, &o)?;
                                    return Ok(Next::Go(insn_at));
                                }
                                // A Closure is put in an array, as a scalar is (ZEND_CAST).
                                if callable::closure_data(&o).is_some() {
                                    Value::array(PhpArray::from_list(vec![Value::object(o)]))
                                } else {
                                    Value::array(classes::object_to_array(&o.proxy_instance().unwrap_or(o)))
                                }
                            } else if v.is_null() {
                                Value::array(PhpArray::new())
                            } else {
                                if v.is_nan() {
                                    self.nan_coerced("array");
                                }
                                Value::array(PhpArray::from_list(vec![v]))
                            }
                        }
                        CastKind::Unset => Value::NULL,
                    };
                }
                Instr::IncDec { var, dst, inc, post } => {
                    if let Some(i) = reg!(var).as_int()
                        && let Some(n) = if inc { i.checked_add(1) } else { i.checked_sub(1) }
                    {
                        reg!(var) = Value::int(n);
                        reg!(dst) = Value::int(if post { i } else { n });
                        return Ok(Next::Go(pc));
                    }
                    let old = self.operand(w, clo, var);
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    assign(w, var, new.clone());
                    reg!(dst) = if post { old } else { new };
                }
                Instr::Jump { target } => pc = target as usize,
                Instr::JumpIfSet { src, dst, target } => {
                    let s = &reg!(src);
                    let v = if s.is_null_or_undef() {
                        None
                    } else if let Some(cell) = s.as_reference() {
                        let v = cell.borrow().clone();
                        (!v.is_null_or_undef()).then_some(v)
                    } else {
                        Some(s.clone())
                    };
                    if let Some(v) = v {
                        reg!(dst) = v;
                        pc = target as usize;
                    }
                }
                Instr::NewArray { dst, n } => reg!(dst) = Value::array(PhpArray::list_with_capacity(n as usize)),
                Instr::ArrayPush { arr, val, take } => {
                    let v = self.operand_take(w, clo, val, take);
                    if let Some(a) = reg!(arr).array_mut()
                        && a.push(v).is_err()
                    {
                        return Err(self.throw("Error", "Cannot add element to the array as the next element is already occupied"));
                    }
                }
                Instr::ArraySet { arr, key, val, take } => {
                    let k = self.key(w, clo, key)?;
                    let v = self.operand_take(w, clo, val, take);
                    if let Some(a) = reg!(arr).array_mut() {
                        a.insert(k, v);
                    }
                }
                Instr::AssignDimOp { op, var, keys, n, val, dst } => {
                    let v = self.operand(w, clo, val);
                    reg!(dst) = match self.assign_dim(w, clo, var, keys, n, false, v, Some(op))? {
                        Written::Done(r) => r,
                        Written::Object(o, at, v) => {
                            let rest: Vec<Value> = w[keys as usize + at..keys as usize + n as usize].iter().map(Value::deref).collect();
                            let r = self.aa_write(regs, &Value::object(o), &rest, false, arrayaccess::DimOp::Op(op, v))?;
                            regs[base + dst as usize] = r;
                            return Ok(Next::Go(pc));
                        }
                    };
                }
                Instr::IssetVar { dst, var } => {
                    let v = &reg!(var);
                    let set = if v.is_null_or_undef() {
                        false
                    } else if let Some(cell) = v.as_reference() {
                        !cell.borrow().is_null_or_undef()
                    } else {
                        true
                    };
                    reg!(dst) = Value::bool(set);
                }
                Instr::UnsetVar { var } => {
                    // In a file included at global scope the variable is the
                    // global itself: unset removes it there too.
                    if !in_main
                        && self.frames.last().is_some_and(|f| f.global)
                        && let Some(name) = func.cv_names.get(var as usize)
                    {
                        let name = name.as_bytes().to_vec();
                        self.with_main(below, |cx, main| cx.global_clear(main, &name));
                    }
                    reg!(var) = Value::UNDEF;
                }
                Instr::UnsetDim { var, keys, n } => {
                    let key_range = keys as usize..keys as usize + n as usize;
                    if arrayaccess::path_has_object(&reg!(var), &w[key_range.clone()], false) {
                        let derefed: Vec<Value> = w[key_range].iter().map(Value::deref).collect();
                        let root = reg!(var).deref();
                        self.aa_write(regs, &root, &derefed, false, arrayaccess::DimOp::Unset)?;
                        return Ok(Next::Go(pc));
                    }
                    // An undefined variable is warned (ZEND_UNSET_DIM's op1
                    // is fetched for unset), then there is nothing to do.
                    if reg!(var).is_undef() {
                        if let Some(name) = func.cv_names.get(var as usize) {
                            self.warn(format!("Undefined variable ${}", String::from_utf8_lossy(name.as_bytes())));
                        }
                        return Ok(Next::Go(pc));
                    }
                    // In place: a copy of the variable's array would make
                    // every unset copy the whole array (copy-on-write). The
                    // array is taken out of the variable while the element
                    // goes, so the keys are read where they are (unless one
                    // is a reference, which might be to the variable itself:
                    // then their values are read first).
                    if !key_range.contains(&(var as usize)) && !w[key_range.clone()].iter().any(Value::is_reference) {
                        let mut held = match reg!(var).as_reference() {
                            Some(cell) => std::mem::take(&mut *cell.borrow_mut()),
                            None => std::mem::take(&mut reg!(var)),
                        };
                        let r = self.unset_path(&mut held, &w[key_range]);
                        match reg!(var).as_reference() {
                            Some(cell) => *cell.borrow_mut() = held,
                            None => reg!(var) = held,
                        }
                        r?;
                        return Ok(Next::Go(pc));
                    }
                    let derefed: Vec<Value> = w[key_range].iter().map(Value::deref).collect();
                    self.unset_path(&mut reg!(var), &derefed)?;
                }
                Instr::IterInit { src, iter, end } => {
                    let v = self.operand(w, clo, src);
                    if let Some(a) = v.as_array() {
                        if !a.is_empty() {
                            reg!(iter) = v;
                            reg!(iter + 1) = Value::int(0);
                        } else {
                            pc = end as usize;
                        }
                    } else {
                        match v.into_object() {
                            Ok(o) => {
                                let it = self.iter_object(regs, o)?;
                                let w = &mut regs[base..base + func.nregs as usize];
                                match it {
                                    Some(v) => {
                                        w[iter as usize] = v;
                                        w[iter as usize + 1] = Value::int(0);
                                    }
                                    None => pc = end as usize,
                                }
                            }
                            Err(other) => {
                                self.warn(format!(
                                    "foreach() argument must be of type array|object, {} given",
                                    other.type_name()
                                ));
                                pc = end as usize;
                            }
                        }
                    }
                }
                Instr::IterNext { iter, val, key, has_key, end } if reg!(iter).is_object() => {
                    let object = reg!(iter).cloned_object().expect("an object");
                    let first = reg!(iter + 1).as_int() == Some(0);
                    let next = self.iter_object_next(regs, &object, first)?;
                    let w = &mut regs[base..base + func.nregs as usize];
                    w[iter as usize + 1] = Value::int(1);
                    match next {
                        Some((v, k)) => {
                            assign(w, val, v);
                            if has_key {
                                assign(w, key, k);
                            }
                        }
                        None => pc = end as usize,
                    }
                }
                Instr::IterNext { iter, val, key, has_key, end } => {
                    if !iter_next(w, iter, val, key, has_key) {
                        pc = end as usize;
                    }
                }
                Instr::SendVar { site, n, arg, var } => {
                    let by_ref = match self.resolve(clo, site)? {
                        Callee::User(f) => self.codes[f as usize].param_by_ref(n as usize),
                        Callee::Builtin(i) => self.isolate.registry.by_ref(i, n as usize),
                        Callee::Constant(_) => false,
                    };
                    reg!(arg) = if by_ref {
                        Value::reference(make_ref(w, var))
                    } else {
                        self.operand(w, clo, var)
                    };
                }
                Instr::SendVarMethod { obj, method, n, arg, var } => {
                    let site = &func.sites[method as usize];
                    let by_ref = match reg!(obj).deref().as_object() {
                        Some(o) => match self.cached_takes_ref(&clo.methods[method as usize], &o.class, n as usize) {
                            Some(by_ref) => by_ref,
                            None => self.method_takes_ref(&o.class, Some(site.sym), n as usize),
                        },
                        None => false,
                    };
                    reg!(arg) = if by_ref { Value::reference(make_ref(w, var)) } else { self.operand(w, clo, var) };
                }
                Instr::SendVarStatic { class, method, n, arg, var } => {
                    let site = &func.sites[method as usize];
                    let target = self.class_at(clo, class, w)?;
                    let by_ref = match self.cached_takes_ref(&clo.methods[method as usize], &target, n as usize) {
                        Some(by_ref) => by_ref,
                        None => self.method_takes_ref(&target, Some(site.sym), n as usize),
                    };
                    reg!(arg) = if by_ref { Value::reference(make_ref(w, var)) } else { self.operand(w, clo, var) };
                }
                Instr::MethodArgByRef { dst, obj, method, n } => {
                    let site = &func.sites[method as usize];
                    reg!(dst) = Value::bool(match reg!(obj).deref().as_object() {
                        Some(o) => match self.cached_takes_ref(&clo.methods[method as usize], &o.class, n as usize) {
                            Some(by_ref) => by_ref,
                            None => self.method_takes_ref(&o.class, Some(site.sym), n as usize),
                        },
                        None => false,
                    });
                }
                Instr::FuncArgByRef { dst, site, n } => {
                    reg!(dst) = Value::bool(match self.resolve(clo, site)? {
                        Callee::User(f) => self.codes[f as usize].param_by_ref(n as usize),
                        Callee::Builtin(i) => self.isolate.registry.by_ref(i, n as usize),
                        Callee::Constant(_) => false,
                    });
                }
                Instr::StaticArgByRef { dst, class, method, n } => {
                    let site = &func.sites[method as usize];
                    let target = self.class_at(clo, class, w)?;
                    reg!(dst) = Value::bool(match self.cached_takes_ref(&clo.methods[method as usize], &target, n as usize) {
                        Some(by_ref) => by_ref,
                        None => self.method_takes_ref(&target, Some(site.sym), n as usize),
                    });
                }
                // Passed (and not a hole a named argument skipped): no default.
                Instr::ArgDefault { n, target } => {
                    if (n as usize) < argc && !reg!(n as Reg).is_undef() {
                        pc = target as usize;
                    }
                }
                Instr::Echo { src } => {
                    let v = self.operand(w, clo, src);
                    let v = self.string_of(regs, v)?;
                    if let Some(e) = ops::object_string_error(&v) {
                        return Err(self.op_error(e));
                    }
                    if self.ext_state.contains_key("header_callback") && !v.as_str().is_some_and(|s| s.is_empty()) {
                        self.run_header_callback(regs, false)?;
                    }
                    match v.as_str() {
                        Some(s) => self.write_out(s.as_bytes()),
                        None => {
                            let s = ops::to_string_lossy(&v, &mut self.diags);
                            self.report_diags();
                            self.write_out(s.as_bytes());
                        }
                    }
                }
                Instr::FetchConst { site, dst } => {
                    let s = &func.sites[site as usize];
                    let v = match clo.sites[site as usize].get() {
                        Some(Callee::Constant(v)) => v.clone(),
                        _ => match self.constant_by_key(s.sym).map(|v| (v, s.sym)).or_else(|| s.fallback.as_ref().and_then(|g| self.constant_by_key(g.1).map(|v| (v, g.1)))) {
                            // Deprecated: said on every fetch, so never cached.
                            // A builtin one says what its #[\Deprecated] does.
                            Some((v, key))
                                if let Some(message) = self.isolate.registry.constant_deprecation(key).map(str::to_string).or_else(|| {
                                    self.runtime_deprecated(s.name.as_bytes(), s.fallback.as_ref().map(|g| g.0.as_bytes())).map(|name| format!("Constant {name} is deprecated"))
                                }) =>
                            {
                                self.diags.push(Diag::Deprecated(message));
                                self.report_diags();
                                v
                            }
                            Some((v, _)) => {
                                let _ = clo.sites[site as usize].set(Callee::Constant(v.clone()));
                                v
                            }
                            None => {
                                let name = String::from_utf8_lossy(s.display.as_bytes()).into_owned();
                                return Err(self.throw("Error", format!("Undefined constant \"{name}\"")));
                            }
                        },
                    };
                    reg!(dst) = v;
                }
                Instr::ConcatN { dst, start, n } if w[start as usize..start as usize + n as usize].iter().any(|v| v.deref().is_object()) => {
                    // An object part: its __toString() may run PHP code.
                    let values: Vec<Value> = (0..n).map(|i| self.operand(w, clo, start + i as Reg)).collect();
                    let mut bytes = Vec::new();
                    for v in values {
                        let v = self.string_of(regs, v)?;
                        if let Some(e) = ops::object_string_error(&v) {
                            return Err(self.op_error(e));
                        }
                        ops::push_string(&mut bytes, &v, &mut self.diags);
                    }
                    self.report_diags();
                    regs[base + dst as usize] = Value::str(PhpStr::new(bytes));
                    free_parts(&mut regs[base..], start, n);
                }
                Instr::ConcatN { dst, start, n } => {
                    let parts = start as usize..start as usize + n as usize;
                    let mut bytes = Vec::with_capacity(w[parts].iter().map(ops::string_len_hint).sum());
                    for i in 0..n {
                        let v = self.operand(w, clo, start + i as Reg);
                        let v = self.object_to_string(v)?;
                        if let Some(e) = ops::object_string_error(&v) {
                            return Err(self.op_error(e));
                        }
                        ops::push_string(&mut bytes, &v, &mut self.diags);
                    }
                    self.report_diags();
                    reg!(dst) = Value::str(PhpStr::new(bytes));
                    free_parts(w, start, n);
                }
                Instr::BindGlobal { var, name } => {
                    // The name's constant, shared (not copied).
                    let name = clo.consts[name as usize].as_str().expect("BindGlobal names a constant string").as_bytes();
                    // The main frame's window starts at 0: it is `w` when
                    // the main frame is running, else the start of `below`
                    // (or of the first task's registers, in a fiber).
                    let cell = if in_main { self.global_cell(w, name) } else { self.with_main(below, |cx, main| cx.global_cell(main, name)) };
                    reg!(var) = Value::reference(cell);
                }
                Instr::MatchError { src } => {
                    let v = self.operand(w, clo, src);
                    return Err(self.throw("UnhandledMatchError", format!("Unhandled match case {}", match_case(&v))));
                }
                Instr::Exit { src, has } => {
                    let v = self.operand(w, clo, src);
                    let status = match (has, v.as_int()) {
                        (true, Some(i)) => i as i32,
                        (true, None) => {
                            let s = ops::to_string_lossy(&v, &mut self.diags);
                            self.write_out(s.as_bytes());
                            0
                        }
                        (false, _) => 0,
                    };
                    return Err(Throw::Exit(status));
                }
                Instr::DeclareClass { class } => {
                    let def = Arc::clone(&func.decls[class as usize]);
                    self.declare_class(&def)?;
                }
                Instr::DeclareFunction { func: i } => {
                    let declared = Arc::clone(&func.lambdas[i as usize]);
                    if let Err(message) = self.declare(Arc::clone(&declared)) {
                        return Err(Throw::Fatal { message, file: Rc::from(&*declared.file), line: declared.line });
                    }
                }
                Instr::MakeClosure { dst, func: i, is_static } => {
                    let object = self.make_closure(&func.lambdas[i as usize], is_static);
                    reg!(dst) = Value::object(object);
                }
                Instr::Capture { closure, var, by_ref, quiet } => {
                    let v = if by_ref {
                        Value::reference(make_ref(w, var))
                    } else if quiet {
                        reg!(var).deref()
                    } else {
                        self.operand(w, clo, var)
                    };
                    if let Some(o) = reg!(closure).as_object()
                        && let Some(data) = callable::closure_data(o)
                    {
                        data.captured.borrow_mut().push(v);
                    }
                }
                Instr::Rare { op, a, b, c, class } => match op {
                    Rare::PropRef => {
                            let (dst, obj, name) = (a, b, c);
                        let target = self.operand(w, clo, obj);
                        let name = name!(name).clone();
                        if let Some(lo) = self.lazy_pending(&target, &name) {
                            self.lazy_init(regs, &lo)?;
                            return Ok(Next::Go(insn_at));
                        }
                        let target = self.lazy_forward(&target, &name).unwrap_or(target);
                        let cell = self.prop_ref(&target, &name, class != 0)?;
                        reg!(dst) = Value::reference(cell);
                    }
                    Rare::AssignPropRef => {
                            let (obj, name, src) = (a, b, c);
                        let target = self.operand(w, clo, obj);
                        let name = name!(name).clone();
                        let cell = make_ref(w, src);
                        if let Some(lo) = self.lazy_pending(&target, &name) {
                            self.lazy_init(regs, &lo)?;
                            return Ok(Next::Go(insn_at));
                        }
                        let target = self.lazy_forward(&target, &name).unwrap_or(target);
                        self.prop_bind(&target, &name, cell)?;
                    }
                    Rare::StaticRef => {
                            let (dst, name) = (a, b);
                        if let Some(c) = self.lazy_class(func, class, w)? {
                            self.resolve_class(regs, &c)?;
                            return Ok(Next::Go(at));
                        }
                        let name = name!(name).clone();
                        let cell = self.static_cell(func, class, &name, w)?;
                        let inner = cell.borrow().cloned_reference();
                        let r = match inner {
                            Some(r) => r,
                            None => {
                                let v = std::mem::take(&mut *cell.borrow_mut());
                                let v = if v.is_undef() { Value::NULL } else { v };
                                let r = Rc::new(RefCell::new(v));
                                *cell.borrow_mut() = Value::reference(Rc::clone(&r));
                                r
                            }
                        };
                        reg!(dst) = Value::reference(r);
                    }
                    Rare::AssignStaticRef => {
                            let (name, src) = (a, b);
                        if let Some(c) = self.lazy_class(func, class, w)? {
                            self.resolve_class(regs, &c)?;
                            return Ok(Next::Go(at));
                        }
                        let name = name!(name).clone();
                        let cell = self.static_cell(func, class, &name, w)?;
                        let r = make_ref(w, src);
                        *cell.borrow_mut() = Value::reference(r);
                    }
                    Rare::WantRef => self.ref_wanted.push(base + a as usize),
                    Rare::BindRefResult => {
                            let (var, src) = (a, b);
                        let at_reg = base + src as usize;
                        self.ref_wanted.retain(|&r| r != at_reg);
                        match reg!(src).clone().into_reference() {
                            Ok(cell) => reg!(var) = Value::reference(cell),
                            Err(v) => {
                                self.notice("Only variables should be assigned by reference");
                                assign(w, var, v);
                            }
                        }
                    }
                    Rare::ArrayPushRef => {
                        let cell = make_ref(w, b);
                        if let Some(arr) = reg!(a).array_mut() {
                            let _ = arr.push(Value::reference(cell));
                        }
                    }
                    Rare::ArraySetRef => {
                        let key = self.key(w, clo, b)?;
                        let cell = make_ref(w, c);
                        if let Some(arr) = reg!(a).array_mut() {
                            arr.insert(key, Value::reference(cell));
                        }
                    }
                    Rare::ToObject => {
                        let v = self.operand(w, clo, b);
                        reg!(a) = self.object_cast(v);
                    }
                    // An intrinsic the loop did not run (a conversion, an
                    // object, a warning): the builtin's call, as a Call's.
                    Rare::Strlen | Rare::Count | Rare::IsNull | Rare::IsBool | Rare::IsInt | Rare::IsFloat | Rare::IsString | Rare::IsArray | Rare::IsObject | Rare::ArrayKeyExists => {
                        return self.call_site(regs, clo, base, class, (b, c as u8, a), pc);
                    }
                    Rare::VerifyArgs => self.verify_args(regs, base, argc)?,
                    Rare::VerifyReturn => {
                        if let Some(ty) = func.ret_type.as_ref().filter(|_| !func.is_generator) {
                            let v = self.operand(w, clo, a);
                            match self.check_type(ty, &v, func.strict)? {
                                Ok(Some(converted)) => reg!(a) = converted,
                                Ok(None) => {}
                                Err(()) => {
                                    if let Some(s) = self.stringable(regs, ty, &v, func.strict)? {
                                        regs[base + a as usize] = s;
                                        return Ok(Next::Go(pc));
                                    }
                                    // b: 0 a value returned, else none (`return;`, the end).
                                    let given = if b != 0 { "none".into() } else { p8_runtime::args::value_name(&v) };
                                    let name = self.function_display();
                                    let shown = self.type_shown(ty);
                                    return Err(self.throw("TypeError", format!("{name}(): Return value must be of type {shown}, {given} returned")));
                                }
                            }
                        }
                    }
                    Rare::VarRef => {
                        let name = self.var_name(w, clo, b);
                        let global = c == 1 || self.frames.last().is_some_and(|f| f.global);
                        let cell = if global && !in_main { self.with_main(below, |cx, main| cx.global_cell(main, &name)) } else { self.var_cell(w, in_main, &name) };
                        reg!(a) = Value::reference(cell);
                    }
                    Rare::VarGet => {
                        let name = self.var_name(w, clo, b);
                        let global = c & 2 != 0;
                        let scope = global || self.frames.last().is_some_and(|f| f.global);
                        let v = if scope && !in_main { self.with_main(below, |cx, main| cx.global_value(main, &name)) } else { self.var_value(w, in_main, &name) };
                        match v {
                            Some(v) => reg!(a) = v,
                            None => {
                                if c & 1 == 0 {
                                    let what = if global { "global variable" } else { "variable" };
                                    self.warn(format!("Undefined {what} ${}", String::from_utf8_lossy(&name)));
                                }
                                reg!(a) = Value::NULL;
                            }
                        }
                    }
                    Rare::VarUnset => {
                        let name = self.var_name(w, clo, a);
                        let at_global = self.frames.last().is_some_and(|f| f.global);
                        if at_global && !in_main {
                            // A file included at global scope: the global is
                            // emptied, and this file's binding to it goes.
                            self.with_main(below, |cx, main| cx.global_clear(main, &name));
                            if let Some(r) = func.cv_names.iter().position(|n| n.as_bytes() == name.as_slice()) {
                                w[r] = Value::UNDEF;
                            }
                        } else if b == 1 && !in_main {
                            self.with_main(below, |cx, main| cx.global_unset(main, &name));
                        } else {
                            self.var_unset(w, in_main, &name);
                        }
                    }
                    Rare::PipeArgByRef => {
                        // The callee as the call finds it (an error finding
                        // it is the call's own, reported there).
                        let found: Option<(bool, String, String)> = match c {
                            0 => match self.resolve(clo, b)? {
                                Callee::User(fi) => {
                                    let code = &self.codes[fi as usize];
                                    let param = code.params.first().map(|p| String::from_utf8_lossy(p.name.as_bytes()).into_owned()).unwrap_or_default();
                                    Some((code.param_by_ref(0), String::from_utf8_lossy(code.name.as_bytes()).into_owned(), param))
                                }
                                Callee::Builtin(i) => {
                                    let registry = &self.isolate.registry;
                                    let param = registry.sig(i).and_then(|s| s.params.first()).map(|p| p.name.to_string()).unwrap_or_default();
                                    Some((registry.must_be_ref(i, 0), registry.function(i).name.to_string(), param))
                                }
                                _ => None,
                            },
                            _ => {
                                let callable = match c {
                                    1 => {
                                        let name = Value::str(func.sites[b as usize].display.as_bytes());
                                        Value::array(PhpArray::from_list(vec![reg!(a).deref(), name]))
                                    }
                                    2 => {
                                        let target = self.class_at(clo, class, w)?;
                                        let name = Value::str(func.sites[b as usize].display.as_bytes());
                                        Value::array(PhpArray::from_list(vec![Value::str(target.name.clone()), name]))
                                    }
                                    _ => reg!(a).clone(),
                                };
                                let retry = self.autoload_retry.clone();
                                match self.resolve_callable(&callable) {
                                    Ok(t) => {
                                        let (name, params) = self.callable_params(&t);
                                        let by_ref = match &t {
                                            callable::Target::Builtin(i) => self.isolate.registry.must_be_ref(*i, 0),
                                            t => self.takes_ref(t, 0),
                                        };
                                        Some((by_ref, name, params.into_iter().next().unwrap_or_default()))
                                    }
                                    Err(t @ Throw::Autoload(_)) => return Err(t),
                                    Err(_) => {
                                        self.autoload_retry = retry;
                                        None
                                    }
                                }
                            }
                        };
                        if let Some((true, name, param)) = found {
                            let param = if param.is_empty() { String::new() } else { format!(" (${param})") };
                            return Err(self.throw("Error", format!("{name}(): Argument #1{param} could not be passed by reference")));
                        }
                    }
                    Rare::AutoGlobal => {
                        // The name is a constant string: read, not converted.
                        let name = self.operand(w, clo, b).into_str().expect("AutoGlobal names a constant string");
                        let main: &mut [Value] = if in_main { w } else { below };
                        reg!(a) = Value::reference(self.with_main(main, |cx, main| cx.global_slot(main, name.as_bytes())));
                    }
                    Rare::GlobalBind => {
                        let name = self.var_name(w, clo, a);
                        let cell = make_ref(w, b);
                        let main: &mut [Value] = if in_main { w } else { below };
                        self.with_main(main, |cx, main| cx.global_bind(main, &name, cell));
                    }
                    Rare::VarBind => {
                        let name = self.var_name(w, clo, a);
                        let at_global = in_main || self.frames.last().is_some_and(|f| f.global);
                        if c == 1 {
                            // `global $$name` at global scope changes nothing.
                            if !at_global {
                                let cell = self.with_main(below, |cx, main| cx.global_cell(main, &name));
                                self.bind_local(w, &name, cell);
                            }
                        } else {
                            let cell = make_ref(w, b);
                            if at_global {
                                let main: &mut [Value] = if in_main { w } else { below };
                                self.with_main(main, |cx, main| cx.global_bind(main, &name, cell));
                            } else {
                                self.bind_local(w, &name, cell);
                            }
                        }
                    }
                    Rare::Globals => {
                        let main: &mut [Value] = if in_main { w } else { below };
                        let globals = self.with_main(main, |cx, main| cx.globals_array(main));
                        reg!(a) = Value::array(globals);
                    }
                    Rare::IssetStatic => {
                        let set = self.isset_static(func, class, &reg!(b).clone(), w)?;
                        reg!(a) = Value::bool(set);
                    }
                    Rare::ThrowError => {
                        let message = reg!(a).to_php_string().unwrap_or_else(PhpStr::empty);
                        return Err(self.throw("Error", String::from_utf8_lossy(message.as_bytes()).into_owned()));
                    }
                    Rare::ConstCallable => {
                        let v = self.operand(w, clo, b);
                        reg!(a) = self.static_callable(&v, true)?;
                    }
                    Rare::Partial => {
                        let callee = self.operand(w, clo, b);
                        let n = func.partials[class as usize].args.len();
                        let values: Vec<Value> = (0..n).map(|i| w[c as usize + i].clone()).collect();
                        reg!(a) = self.partial(&clo.code, class, callee, values, func.strict)?;
                    }
                    Rare::CallableArgByRef => {
                        let callee = reg!(b).clone();
                        // A closure answers for itself (no frame made for it).
                        if class != u16::MAX
                            && let Some(by_ref) = Self::closure_takes_ref(&callee, class as usize)
                        {
                            reg!(a) = Value::bool(by_ref);
                            return Ok(Next::Go(pc));
                        }
                        let retry = self.autoload_retry.clone();
                        let by_ref = match self.resolve_callable(&callee) {
                            Ok(t) if class == u16::MAX => {
                                let name = reg!(c).to_php_string().unwrap_or_else(PhpStr::empty);
                                self.named_takes_ref(&t, name.as_bytes())
                            }
                            Ok(t) => self.takes_ref(&t, class as usize),
                            // A class not loaded yet: the autoloaders run and the
                            // question is asked again (not answered "no", which
                            // left the class unloaded for the call itself).
                            Err(t @ Throw::Autoload(_)) => return Err(t),
                            // Still not there after they ran: the call itself is
                            // told so too, not running them again.
                            Err(_) => {
                                self.autoload_retry = retry;
                                false
                            }
                        };
                        reg!(a) = Value::bool(by_ref);
                    }
                    Rare::ReturnRef => {
                            let src = a;
                        let ret = self.frames.last().map_or(NO_RET, |f| f.ret);
                        let wanted = ret != NO_RET && self.ref_wanted.contains(&(ret as usize));
                        if wanted {
                            self.ref_wanted.retain(|&r| r != ret as usize);
                        }
                        let v = if wanted { Value::reference(make_ref(w, src)) } else { self.operand(w, clo, src) };
                        // A temporary's handle goes (none is left at exit).
                        if is_temp(func, src) {
                            clear(&mut reg!(src));
                        }
                        if let Some(g) = self.frames.last_mut().and_then(|f| f.take_generator()) {
                            self.finish(&g, v);
                            leave(&mut self.frames, regs, Value::NULL);
                            return Ok(Next::Switch);
                        }
                        leave(&mut self.frames, regs, v);
                        return Ok(Next::Switch);
                    }
                },
                Instr::BindRef { var, src } => {
                    if var != src {
                        let cell = make_ref(w, src);
                        reg!(var) = Value::reference(cell);
                    }
                }
                // No keys and no `[]`: the root itself (`&$GLOBALS['x']` is the
                // global's reference).
                Instr::RefDim { var, root, n: 0, append: false, .. } => {
                    let cell = make_ref(w, root);
                    reg!(var) = Value::reference(cell);
                }
                Instr::RefDim { var, root, keys, n, append } => {
                    let path = self.key_path(w, clo, keys, n)?;
                    let cell = make_ref(w, root);
                    let mut value = std::mem::take(&mut *cell.borrow_mut());
                    let r = self.dim_ref(&mut value, &path, append);
                    self.key_path_done(path);
                    *cell.borrow_mut() = value;
                    reg!(var) = Value::reference(r?);
                }
                Instr::DimSetRef { root, keys, n, append, src } => {
                    let path = self.key_path(w, clo, keys, n)?;
                    let target = make_ref(w, src);
                    let cell = make_ref(w, root);
                    let mut value = std::mem::take(&mut *cell.borrow_mut());
                    let r = self.dim_set_ref(&mut value, &path, append, target);
                    self.key_path_done(path);
                    *cell.borrow_mut() = value;
                    r?;
                }
                Instr::IterInitRef { src, iter, end } => {
                    let cell = make_ref(w, src);
                    let object = cell.borrow().cloned_object();
                    // A plain object: its properties the code can see, each
                    // made a reference as the loop reaches it.
                    if let Some(mut o) = object {
                        // An IteratorAggregate: the iterator it gives (the call
                        // back into PHP needs the registers, so they are
                        // taken afresh after it).
                        while o.class.is_a(b"iteratoraggregate") && !o.class.is_a(b"arrayobject") {
                            match self.call_named(regs, &o, b"getIterator")?.into_object() {
                                Ok(it) if it.class.is_a(b"traversable") => o = it,
                                _ => return Err(self.throw("Error", "An iterator cannot be used with foreach by reference")),
                            }
                        }
                        let w = &mut regs[base..base + func.nregs as usize];
                        // ArrayObject / ArrayIterator: their stored array, by reference.
                        let storage = ["arrayiterator", "arrayobject"]
                            .iter()
                            .find(|c| o.class.is_a(c.as_bytes()))
                            .and_then(|c| o.class.slot(b"storage", Some(c.as_bytes())));
                        if let Some(i) = storage
                            && o.slots()[i].peek(|v| v.deref().is_array())
                        {
                            let cell = o.slots()[i].modify(refs::make_ref);
                            let empty = cell.borrow().as_array().is_some_and(PhpArray::is_empty);
                            if empty {
                                pc = end as usize;
                            } else {
                                w[iter as usize] = Value::int(0);
                                w[iter as usize + 1] = Value::reference(cell);
                            }
                            return Ok(Next::Go(pc));
                        }
                        if o.class.is_a(b"traversable") {
                            return Err(self.throw("Error", "An iterator cannot be used with foreach by reference"));
                        }
                        // A plain object: its properties the code can see, each
                        // made a reference as the loop reaches it.
                        let names: Vec<Value> = self.visible_props(&o).iter().map(|(k, _)| k.to_value()).collect();
                        if names.is_empty() {
                            pc = end as usize;
                        } else {
                            w[iter as usize] = Value::int(0);
                            w[iter as usize + 1] = Value::array(PhpArray::from_list(vec![Value::object(o), Value::array(PhpArray::from_list(names))]));
                        }
                        return Ok(Next::Go(pc));
                    }
                    let state = match cell.borrow().as_array() {
                        Some(a) => Ok(!a.is_empty()),
                        None => Err(cell.borrow().type_name()),
                    };
                    match state {
                        Ok(true) => {
                            reg!(iter) = Value::int(0);
                            reg!(iter + 1) = Value::reference(cell);
                        }
                        Ok(false) => pc = end as usize,
                        Err(ty) => {
                            self.warn(format!("foreach() argument must be of type array|object, {ty} given"));
                            pc = end as usize;
                        }
                    }
                }
                Instr::IterNextRef { iter, val, key, has_key, end } => {
                    let pos = reg!(iter).as_int().map_or(0, |p| p as usize);
                    let state = &reg!(iter + 1);
                    let next = if let Some(cell) = state.as_reference() {
                        refs::next_ref(cell, pos)
                    } else if let Some(state) = state.as_array() {
                        self.next_prop_ref(state, pos)
                    } else {
                        None
                    };
                    match next {
                        Some((p, k, cell)) => {
                            reg!(iter) = Value::int(p as i64 + 1);
                            reg!(val) = Value::reference(cell);
                            if has_key {
                                assign(w, key, k.into_value());
                            }
                        }
                        None => pc = end as usize,
                    }
                }
                Instr::CollectVariadic { var } => {
                    // Its own argument (if passed), then those past it:
                    // the list made once, at its size.
                    let extra = self.frames.last().map_or(0, |f| f.args().len());
                    let mut list = p8_runtime::array::values_buffer(usize::from(argc > var as usize) + extra);
                    if argc > var as usize {
                        list.push(std::mem::replace(&mut reg!(var), Value::UNDEF));
                    }
                    if let Some(frame) = self.frames.last() {
                        list.extend(frame.args().iter().cloned());
                    }
                    let mut arr = PhpArray::from_list(list);
                    // Named arguments no parameter takes: by their names.
                    if let Some((depth, named)) = self.named_extra.take() {
                        if depth == self.frames.len() {
                            for (k, v) in named.iter() {
                                arr.insert(k, v.clone());
                            }
                        } else {
                            self.named_extra = Some((depth, named));
                        }
                    }
                    reg!(var) = Value::array(arr);
                }
                Instr::FetchList { dst, arr, key } => {
                    let container = match reg!(arr).as_reference() {
                        Some(cell) => cell.borrow().clone(),
                        None => reg!(arr).clone(),
                    };
                    reg!(dst) = if container.is_array() {
                        self.fetch_dim(&container, w, clo,key, false)?
                    } else if let Some(o) = container.as_object() {
                        // An ArrayAccess object: its offsetGet.
                        let k = self.operand(w, clo, key);
                        let v = self.aa_get(regs, o, k, false)?;
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    } else {
                        Value::NULL
                    };
                }
                Instr::StaticVar { var, index, skip } => {
                    let (cell, new) = self.static_var(&clo.code, index);
                    reg!(var) = Value::reference(cell);
                    if !new {
                        pc = skip as usize;
                    }
                }
                Instr::FromCallable { dst, src } => {
                    let v = self.operand(w, clo, src);
                    reg!(dst) = self.static_callable(&v, false)?;
                }
                Instr::CallValue { dst, callee, base: args, argc, vars } => {
                    let target = self.operand(w, clo, callee);
                    // An undefined callee: its warning reaches the error
                    // handler, then the call fails before any argument is
                    // read (INIT_DYNAMIC_CALL).
                    if !self.pending_errors.is_empty() && target.is_null() {
                        self.dispatch_errors(regs)?;
                        return self.call_value(regs, &target, base + args as usize, argc, (base + dst as usize) as u32, pc);
                    }
                    if vars != p8_bytecode::NO_VARS {
                        // By reference where the callable takes one: a
                        // closure answers itself; anything else resolved (a
                        // callable that is not one fails at the call).
                        let resolved = match Self::closure_takes_ref(&target, 0) {
                            Some(_) => None,
                            None => {
                                let retry = self.autoload_retry.clone();
                                match self.resolve_callable(&target) {
                                    Ok(t) => Some(t),
                                    // A class not loaded yet: the autoloaders run
                                    // and this instruction again.
                                    Err(t @ Throw::Autoload(_)) => return Err(t),
                                    // Not callable: the call says so before any
                                    // argument is read (INIT_DYNAMIC_CALL fails first).
                                    Err(_) => {
                                        self.autoload_retry = retry;
                                        return self.call_value(regs, &target, base + args as usize, argc, (base + dst as usize) as u32, pc);
                                    }
                                }
                            }
                        };
                        self.send_vars(w, clo, vars, args, |cx, n| match &resolved {
                            Some(t) => cx.takes_ref(t, n),
                            None => Self::closure_takes_ref(&target, n).unwrap_or(false),
                        });
                    }
                    return self.call_value(regs, &target, base + args as usize, argc, (base + dst as usize) as u32, pc);
                }
                Instr::Silence { saved } => {
                    reg!(saved) = Value::int(self.error_level);
                    // E_ERROR | E_CORE_ERROR | E_COMPILE_ERROR | E_USER_ERROR | E_RECOVERABLE_ERROR | E_PARSE
                    self.error_level &= 4437;
                }
                Instr::Unsilence { saved } => {
                    // A level the silenced code set stays (as in PHP),
                    // unless it only narrowed it to the silenced one.
                    if let Some(level) = reg!(saved).as_int()
                        && self.error_level == level & 4437
                    {
                        self.error_level = level;
                    }
                }
                Instr::Include { dst, src, kind } => {
                    let v = self.operand(w, clo, src);
                    // The registers are lent: a user-space wrapper's methods
                    // may serve the file.
                    self.isolate.regs = std::mem::take(regs);
                    let loaded = self.load_include(&v, kind, &func.file);
                    *regs = std::mem::take(&mut self.isolate.regs);
                    let Some(program) = loaded? else {
                        regs[base + dst as usize] = Value::bool(kind.once());
                        return Ok(Next::Go(pc));
                    };
                    let program = match program {
                        Ok(p) => p,
                        Err(()) => {
                            regs[base + dst as usize] = Value::FALSE;
                            return Ok(Next::Go(pc));
                        }
                    };
                    return Ok(self.run_in_scope(regs, base, func, &program.main, dst, pc));
                }
                Instr::Eval { dst, src } => {
                    let code = self.operand(w, clo, src);
                    let program = self.load_eval(&code)?;
                    return Ok(self.run_in_scope(regs, base, func, &program.main, dst, pc));
                }
                Instr::DimIncDec { var, keys, n, dst, inc, post } => {
                    let key_regs = &w[keys as usize..keys as usize + n as usize];
                    if arrayaccess::path_has_object(&reg!(var), key_regs, false) {
                        let key_values: Vec<Value> = key_regs.iter().map(Value::deref).collect();
                        let root = reg!(var).deref();
                        let r = self.aa_write(regs, &root, &key_values, false, arrayaccess::DimOp::IncDec { inc, post })?;
                        regs[base + dst as usize] = r;
                        return Ok(Next::Go(pc));
                    }
                    let path = self.key_path(w, clo, keys, n)?;
                    let mut root = match reg!(var).as_reference() {
                        Some(cell) => std::mem::take(&mut *cell.borrow_mut()),
                        None => std::mem::take(&mut reg!(var)),
                    };
                    let r = self.incdec_path(&mut root, &path, inc);
                    self.key_path_done(path);
                    match reg!(var).as_reference() {
                        Some(cell) => *cell.borrow_mut() = root,
                        None => reg!(var) = root,
                    }
                    let (old, new) = r?;
                    reg!(dst) = if post { old } else { new };
                }
                Instr::CallSpread { dst, callee, args } => {
                    let target = self.operand(w, clo, callee);
                    // (What the callable is, once it was resolved for the list.)
                    let mut resolved = None;
                    let list: Vec<Value> = match self.operand(w, clo, args).into_array() {
                        // Named arguments: placed by the target's parameters.
                        Ok(a) if a.iter().any(|(k, _)| matches!(k, p8_runtime::array::Key::Str(_))) => {
                            match self.resolve_callable(&target)? {
                                // __call / __callStatic take the named ones
                                // in their array, by name.
                                callable::Target::Frame(mut m) if m.trampoline.is_some() => {
                                    let name = m.trampoline.take().unwrap_or_else(PhpStr::empty);
                                    let mut packed = PhpArray::new();
                                    for (k, v) in a.iter() {
                                        packed.insert(k.clone(), v.deref());
                                    }
                                    let at = base + func.nregs as usize;
                                    if regs.len() < at + 3 {
                                        regs.resize(at + 3, Value::UNDEF);
                                    }
                                    regs[at] = Value::str(name);
                                    regs[at + 1] = Value::array(packed);
                                    return self.call_method(regs, m, at, 2, (base + dst as usize) as u32, pc);
                                }
                                t => {
                                    let list = self.place_args(&t, &a, true)?;
                                    resolved = Some(t);
                                    list
                                }
                            }
                        }
                        // References (the arguments compiled as ones) kept
                        // for the parameters that take them.
                        Ok(a) if a.values().any(Value::is_reference) => {
                            let t = self.resolve_callable(&target)?;
                            let list = a.values().enumerate().map(|(n, v)| if v.is_reference() && self.takes_ref(&t, n) { v.clone() } else { v.deref() }).collect();
                            resolved = Some(t);
                            list
                        }
                        Ok(a) => a.values().map(Value::deref).collect(),
                        Err(_) => Vec::new(),
                    };
                    let at = base + func.nregs as usize;
                    if regs.len() < at + list.len() + 1 {
                        regs.resize(at + list.len() + 1, Value::UNDEF);
                    }
                    let argc = list.len() as u8;
                    for (i, v) in list.into_iter().enumerate() {
                        regs[at + i] = v;
                    }
                    let ret = (base + dst as usize) as u32;
                    return match resolved {
                        Some(t) => self.call_target(regs, t, at, argc, ret, pc),
                        None => self.call_value(regs, &target, at, argc, ret, pc),
                    };
                }
                Instr::ArrayUnpack { arr, src } => {
                    let v = self.operand(w, clo, src);
                    let items = if let Some(a) = v.as_array() {
                        // A reference held elsewhere too stays one (as PHP's
                        // unpacking does): a call takes it by reference.
                        a.iter()
                            .map(|(k, v)| {
                                let v = match v.as_reference() {
                                    Some(cell) if Rc::strong_count(cell) > 1 => v.clone(),
                                    _ => v.deref(),
                                };
                                (k.into_value(), v)
                            })
                            .collect()
                    } else if v.as_object().is_some_and(|o| o.class.is_a(b"traversable")) {
                        let items = self.iterate_all(regs, &v)?;
                        let w = &mut regs[base..base + func.nregs as usize];
                        return self.unpack_into(w, arr, items).map(|()| Next::Go(pc));
                    } else {
                        return Err(self.throw("Error", "Only arrays and Traversables can be unpacked"));
                    };
                    self.unpack_into(w, arr, items)?;
                }
                Instr::New { dst, class: i } => {
                    // A named class: its site's cache has the class and how
                    // to make its objects (instances.rs).
                    let info = match Self::cached_new(clo, i) {
                        Some(info) => info,
                        None => {
                            let class = self.class_at(clo, i, w)?;
                            if self.unresolved(&class) {
                                self.resolve_class(regs, &class)?;
                                return Ok(Next::Go(at));
                            }
                            let (info, kept) = self.new_info(&class);
                            if kept && matches!(func.classes[i as usize], p8_bytecode::ClassRef::Named(..) | p8_bytecode::ClassRef::Forward(..)) {
                                Self::remember_new(clo, i, &info);
                            }
                            info
                        }
                    };
                    let object = {
                        let read = |i: usize| if i < base { below[i].clone() } else { w[i - base].clone() };
                        self.make_from(&read, &info, true, false)?.0
                    };
                    // Not destructed unless constructed (an argument may throw first).
                    object.destructed.set(true);
                    reg!(dst) = Value::object(object);
                    // For the Construct after the arguments.
                    self.last_new = Some(info);
                }
                Instr::ConstructSpread { obj, args } => {
                    let object = reg!(obj).cloned_object().expect("ConstructSpread follows New");
                    object.destructed.set(false);
                    let arr = self.operand(w, clo, args);
                    let Some(m) = self.constructor(&object) else { return Ok(Next::Go(pc)) };
                    let target = callable::Target::Frame(m);
                    let list = match arr.as_array() {
                        Some(a) => self.place_args(&target, a, true)?,
                        None => Vec::new(),
                    };
                    let callable::Target::Frame(m) = target else { unreachable!("a frame") };
                    let at = base + func.nregs as usize;
                    if regs.len() < at + list.len() + 1 {
                        regs.resize(at + list.len() + 1, Value::UNDEF);
                    }
                    let argc = list.len() as u8;
                    for (i, v) in list.into_iter().enumerate() {
                        regs[at + i] = v;
                    }
                    return self.call_method(regs, m, at, argc, NO_RET, pc);
                }
                Instr::Construct { obj, base: args, argc } => {
                    let object = reg!(obj).cloned_object().expect("Construct follows New");
                    object.destructed.set(false);
                    // The class's NewInfo: the one its `new` left, else looked up.
                    let info = match self.last_new.take() {
                        Some(info) if Rc::ptr_eq(&info.class, &object.class) => info,
                        _ => self.new_info(&object.class).0,
                    };
                    match &info.ctor {
                        // A PHP constructor: its frame pushed here, on the
                        // arguments where they are (as ZEND_NEW's call
                        // frame), with no MethodFrame made and taken apart.
                        Some(instances::Ctor { func: classes::Code::User(f), declaring, .. }) if !f.code.is_generator && argc as usize >= f.code.required => {
                            if let Some(caller) = self.frames.last_mut() {
                                caller.pc = pc as u32;
                            }
                            self.frames.push(f, base + args as usize, argc as usize, NO_RET);
                            let frame = self.frames.last_mut().expect("the frame just pushed");
                            frame.this = Some(object);
                            frame.scope = Some(Rc::clone(declaring));
                            frame.called = Some(Rc::clone(&info.class));
                            enter(&mut self.frames, regs);
                            return Ok(Next::Switch);
                        }
                        Some(ctor) => {
                            let m = ctor.frame(object, &info.class);
                            return self.call_method(regs, m, base + args as usize, argc, NO_RET, pc);
                        }
                        None => {
                            for r in &mut w[args as usize..args as usize + argc as usize] {
                                clear(r);
                            }
                        }
                    }
                }
                Instr::AssignPropOp { op, obj, name, val, dst } => {
                    let target = self.prop_target(w, clo, obj)?;
                    let y = self.operand(w, clo, val);
                    // The name's constant: the cache entry reads and writes use.
                    let site = const_index(name).and_then(|i| clo.props.get(i));
                    if let (Some(site), Some(o), true) = (site, target.as_object(), name!(name).is_str())
                        && let Some(x) = cached_prop(site.get(), o, self.scope_addr())
                    {
                        let (o, pname, slot) = (Rc::clone(o), name!(name).clone(), site.get().slot as usize);
                        let r = ops::binary(op, &x, &y, &mut self.diags);
                        self.report_diags();
                        let v = r.map_err(|e| self.op_error(e))?;
                        self.write_cached(&o, slot, &pname, v.clone())?;
                        reg!(dst) = v;
                        return Ok(Next::Go(pc));
                    }
                    let name = name!(name).clone();
                    if self.prop_hook(&target, &name, HOOK_GET).is_some() || self.prop_hook(&target, &name, HOOK_SET).is_some() {
                        let v = self.hooked_update(regs, &target, &name, |c, x| {
                        let r = ops::binary(op, x, &y, &mut c.diags);
                        c.report_diags();
                        r.map(|v| (v.clone(), v)).map_err(|e| c.op_error(e))
                    })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::NULL);
                        return Ok(Next::Go(pc));
                    }
                    if self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || self.prop_magic(&target, &name, crate::classes::Magic::Set).is_some() {
                        let v = self.magic_update(regs, &target, &name, |c, x| {
                            let r = ops::binary(op, x, &y, &mut c.diags);
                            c.report_diags();
                            r.map(|v| (v.clone(), v)).map_err(|e| c.op_error(e))
                        })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::NULL);
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    let x = self.read_for_update(&target, &name)?;
                    let r = ops::binary(op, &x, &y, &mut self.diags);
                    self.report_diags();
                    let v = r.map_err(|e| self.op_error(e))?;
                    self.write_prop(&target, &name, v.clone())?;
                    if let (Some(site), Some(o)) = (site, target.as_object())
                        && site.get().class != Rc::as_ptr(&o.class) as usize
                    {
                        self.remember_prop(site, o, &name);
                    }
                    reg!(dst) = v;
                }
                Instr::PropIncDec { obj, name, dst, inc, post } => {
                    let target = self.prop_target(w, clo, obj)?;
                    let site = const_index(name).and_then(|i| clo.props.get(i));
                    if let (Some(site), Some(o), true) = (site, target.as_object(), name!(name).is_str())
                        && let Some(old) = cached_prop(site.get(), o, self.scope_addr())
                    {
                        let (o, pname, slot) = (Rc::clone(o), name!(name).clone(), site.get().slot as usize);
                        let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                        self.report_diags();
                        let new = r.map_err(|e| self.op_error(e))?;
                        self.write_cached(&o, slot, &pname, new.clone())?;
                        reg!(dst) = if post { old } else { new };
                        return Ok(Next::Go(pc));
                    }
                    let name = name!(name).clone();
                    if self.prop_hook(&target, &name, HOOK_GET).is_some() || self.prop_hook(&target, &name, HOOK_SET).is_some() {
                        let v = self.hooked_update(regs, &target, &name, |c, x| {
                        let r = if inc { ops::increment(x, &mut c.diags) } else { ops::decrement(x, &mut c.diags) };
                        c.report_diags();
                        r.map(|new| (new.clone(), if post { x.clone() } else { new })).map_err(|e| c.op_error(e))
                    })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::NULL);
                        return Ok(Next::Go(pc));
                    }
                    if self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || self.prop_magic(&target, &name, crate::classes::Magic::Set).is_some() {
                        let v = self.magic_update(regs, &target, &name, |c, x| {
                            let r = if inc { ops::increment(x, &mut c.diags) } else { ops::decrement(x, &mut c.diags) };
                            c.report_diags();
                            r.map(|new| (new.clone(), if post { x.clone() } else { new })).map_err(|e| c.op_error(e))
                        })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::NULL);
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    let old = self.read_for_update(&target, &name)?;
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    self.write_prop(&target, &name, new.clone())?;
                    if let (Some(site), Some(o)) = (site, target.as_object())
                        && site.get().class != Rc::as_ptr(&o.class) as usize
                    {
                        self.remember_prop(site, o, &name);
                    }
                    reg!(dst) = if post { old } else { new };
                }
                Instr::AssignPropDim { op, base: at, name: name_reg, n, append, val, dst } => {
                    // DIM_THIS: on the method's own `$this`, from the frame.
                    let (this, n) = (n & p8_bytecode::DIM_THIS != 0, n & !p8_bytecode::DIM_THIS);
                    let target = if this { self.prop_target(w, clo, p8_bytecode::THIS)? } else { self.operand(w, clo, at) };
                    let name = name!(name_reg).clone();
                    // A name constant's cache entry: the one property reads
                    // and writes of that name use.
                    let site = const_index(name_reg).and_then(|i| clo.props.get(i));
                    // A hooked property's value is not modified in place.
                    if let (Some(o), Some(pn)) = (target.as_object(), name.to_php_string())
                        && let Some(p) = self.hooked_property(o, pn.as_bytes())
                    {
                        let class = String::from_utf8_lossy(p.declaring_name.as_bytes()).into_owned();
                        return Err(self.throw("Error", format!("Indirect modification of {class}::${} is not allowed", String::from_utf8_lossy(pn.as_bytes()))));
                    }
                    // The keys as written: an ArrayAccess object takes any
                    // (SplObjectStorage's are objects); an array's are converted.
                    let mut keys = std::mem::take(&mut self.dim_keys);
                    keys.clear();
                    for i in 0..n {
                        let k = self.operand(w, clo, at + 2 + i as Reg);
                        keys.push(k);
                    }
                    let v = self.operand(w, clo, val);
                    // A declared property the cache knows for this class and
                    // scope (visible, no hooks, the object not lazy; one that
                    // is unset only in a class without magic): its slot,
                    // without the lookup by name. A readonly or asymmetric
                    // one takes the full path (its checks).
                    if let (Some(site), Some(o)) = (site, target.as_object())
                        && let Some(slot) = cached_slot(site.get(), o, self.scope_addr())
                        && o.class.properties[slot].set_visibility.is_none()
                        && !o.class.properties[slot].readonly
                        && let Some(pname) = name.cloned_str()
                    {
                        let o = Rc::clone(o);
                        let written = self.update_slot_dim(&o, Some(slot), &pname, &keys, append, v, op);
                        let r = self.finish_prop_dim(regs, written, &keys, append, op);
                        keys.clear();
                        self.dim_keys = keys;
                        regs[base + dst as usize] = r?;
                        return Ok(Next::Go(pc));
                    }
                    // An overloaded property: __get's value is written into
                    // only when it is an object; otherwise nothing changes.
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Get) {
                        let pname = name.to_php_string().unwrap_or_else(PhpStr::empty);
                        let got = self.run_magic(regs, found, &mut [Value::str(pname.clone())])?;
                        if got.is_object() {
                            // Into the object as `$obj[k]... = v` writes (ArrayAccess, SimpleXML).
                            let dim_op = match op {
                                Some(op) => arrayaccess::DimOp::Op(op, v),
                                None => arrayaccess::DimOp::Set(v),
                            };
                            regs[base + dst as usize] = self.aa_write(regs, &got, &keys, append, dim_op)?;
                        } else {
                            let class = match target.as_object() {
                                Some(o) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                                None => String::new(),
                            };
                            self.notice(&format!("Indirect modification of overloaded property {class}::${} has no effect", String::from_utf8_lossy(pname.as_bytes())));
                            regs[base + dst as usize] = v;
                        }
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    // The path walked once from the property; an object on it
                    // (ArrayAccess: `$obj->p[k] = v` is its offsetSet) takes
                    // the keys from there.
                    let written = self.update_prop_dim(&target, &name, &keys, append, v, op);
                    let r = self.finish_prop_dim(regs, written, &keys, append, op);
                    // (Its room kept for the next; what it holds goes now.)
                    keys.clear();
                    self.dim_keys = keys;
                    regs[base + dst as usize] = r?;
                    // The next write of this name here finds the slot cached.
                    if let (Some(site), Some(o)) = (site, target.as_object()) {
                        self.remember_prop(site, o, &name);
                    }
                }
                Instr::IssetProp { dst, obj, name } => {
                    let target = reg!(obj).deref();
                    let name = name!(name).clone();
                    if let Some(hook) = self.prop_hook(&target, &name, HOOK_GET) {
                        let v = self.run_nested(regs, hook?, &mut [])?;
                        regs[base + dst as usize] = Value::bool(!v.deref().is_null_or_undef());
                        return Ok(Next::Go(pc));
                    }
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Isset) {
                        let pname = Value::str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        let v = self.run_magic(regs, found, &mut [pname])?;
                        regs[base + dst as usize] = Value::bool(v.to_bool());
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    reg!(dst) = Value::bool(self.isset_prop(&target, &name)?);
                }
                Instr::UnsetProp { obj, name } => {
                    let target = reg!(obj).deref();
                    let name = name!(name).clone();
                    if let (Some(o), Some(n)) = (target.as_object(), name.to_php_string())
                        && let Some(p) = self.hooked_property(o, n.as_bytes())
                    {
                        let class = String::from_utf8_lossy(p.declaring_name.as_bytes()).into_owned();
                        return Err(self.throw("Error", format!("Cannot unset hooked property {class}::${}", String::from_utf8_lossy(n.as_bytes()))));
                    }
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Unset) {
                        let pname = Value::str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        self.run_magic(regs, found, &mut [pname])?;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    self.unset_prop(&target, &name)?;
                }
                Instr::MethodCall { dst, obj, method, base: args, argc } => {
                    let target = self.operand(w, clo, obj);
                    // A temporary (`(new A)->f()`) is released as it is
                    // used: the call's `$this` holds the object.
                    if is_temp(func, obj) {
                        clear(&mut reg!(obj));
                    }
                    if let Some(o) = target.as_object()
                        && let Some(m) = self.cached_method(&clo.methods[method as usize], o)
                    {
                        return self.call_method(regs, m, base + args as usize, argc, (base + dst as usize) as u32, pc);
                    }
                    let site = &func.sites[method as usize];
                    let m = self.method_frame(&target, Some(site.sym), site.display.as_bytes())?;
                    if let Some(o) = target.as_object() {
                        self.remember_method(&clo.methods[method as usize], o, &m);
                    }
                    return self.call_method(regs, m, base + args as usize, argc, (base + dst as usize) as u32, pc);
                }
                Instr::StaticCall { dst, class, method, base: args, argc, vars } => {
                    let site = &func.sites[method as usize];
                    // `parent::$prop::get()` on a parent whose property has
                    // no such hook: the stored value of `$this->prop`.
                    if let Some(rest) = site.display.as_bytes().strip_prefix(b"$")
                        && let Some(at_sep) = rest.windows(2).position(|x| x == b"::")
                    {
                        let target = self.class_of(func, class, w)?;
                        if target.method_sym(site.sym).is_none() {
                            let this = self.frames.last().and_then(|f| f.this.clone());
                            let Some(this) = this else { return Err(self.throw("Error", "Using $this when not in object context")) };
                            let prop = Value::str(&rest[..at_sep]);
                            let v = if rest.ends_with(b"::set") {
                                let v = reg!(args).deref();
                                self.write_prop(&Value::object(this), &prop, v.clone())?;
                                v
                            } else {
                                self.fetch_prop(&Value::object(this), &prop, false)?
                            };
                            for r in &mut w[args as usize..args as usize + argc as usize] {
                                clear(r);
                            }
                            reg!(dst) = v;
                            return Ok(Next::Go(pc));
                        }
                    }
                    // The site's cache entry for this class and scope; else
                    // the full resolution, remembered for the next call.
                    let target = self.class_at(clo, class, w)?;
                    let cache = &clo.methods[method as usize];
                    let m = match self.cached_static(cache, &target, func, class) {
                        Some(m) => m,
                        None => {
                            let m = self.static_frame(func, class, Some(site.sym), site.display.as_bytes(), w)?;
                            self.remember_static(cache, &target, &m);
                            m
                        }
                    };
                    // The variables passed as they are: sent now the method
                    // is known (by reference where it takes one; __callStatic
                    // takes them by value).
                    if vars != p8_bytecode::NO_VARS {
                        let user = match (&m.func, &m.trampoline) {
                            (classes::Code::User(f), None) => Some(Rc::clone(f)),
                            _ => None,
                        };
                        let native = matches!(m.func, classes::Code::Native(..)) && m.trampoline.is_none();
                        let sym = site.sym;
                        self.send_vars(w, clo, vars, args, |cx, n| match &user {
                            Some(f) => f.code.param_by_ref(n),
                            None => native && cx.method_takes_ref(&target, Some(sym), n),
                        });
                    }
                    return self.call_method(regs, m, base + args as usize, argc, (base + dst as usize) as u32, pc);
                }
                Instr::ClassConst { dst, class, name } => {
                    let name_reg = name;
                    let name = name!(name).clone();
                    // `self::class` with no class to name (zend_compile's
                    // ZEND_FETCH_CLASS_NAME): its own error.
                    let is_class = name.as_str().is_some_and(|n| n.as_bytes().eq_ignore_ascii_case(b"class"));
                    if is_class
                        && matches!(func.classes[class as usize], p8_bytecode::ClassRef::ParentScope)
                        && self.scope().0.is_some_and(|c| c.parent.is_none())
                    {
                        return Err(self.throw("Error", "Cannot use \"parent\" when current class scope has no parent"));
                    }
                    if is_class && self.scope().0.is_none() {
                        let word = match &func.classes[class as usize] {
                            p8_bytecode::ClassRef::Scope => Some("self"),
                            p8_bytecode::ClassRef::ParentScope => Some("parent"),
                            p8_bytecode::ClassRef::Static => Some("static"),
                            _ => None,
                        };
                        if let Some(word) = word {
                            return Err(self.throw("Error", format!("Cannot use \"{word}\" in the global scope")));
                        }
                    }
                    if let Some((thunk, scope, key)) = self.pending_constant(func, class, &name, w)? {
                        self.eval_constant(regs, &thunk, scope, key)?;
                        return Ok(Next::Go(at));
                    }
                    let v = self.class_constant(func, class, &name, w)?;
                    if let Some(site) = const_index(name_reg).and_then(|i| clo.class_consts.get(i))
                        && site.get().is_none()
                        && self.class_const_cacheable(clo, class, &name, w)
                    {
                        let _ = site.set((class, v.clone()));
                    }
                    reg!(dst) = v;
                }
                Instr::FetchStaticProp { dst, class, name } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = name!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let v = cell.borrow().deref();
                    reg!(dst) = if v.is_undef() { Value::NULL } else { v };
                }
                Instr::AssignStaticProp { class, name, val, dst } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = name!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let v = self.operand(w, clo, val);
                    store(&cell, v.clone());
                    reg!(dst) = v;
                }
                Instr::AssignStaticPropOp { op, class, name, val, dst } => {
                    if let Some(c) = self.lazy_class(func, class, w)? {
                        self.resolve_class(regs, &c)?;
                        return Ok(Next::Go(at));
                    }
                    let name = name!(name).clone();
                    let cell = self.static_cell(func, class, &name, w)?;
                    let y = self.operand(w, clo, val);
                    let x = cell.borrow().deref();
                    let r = ops::binary(op, &x, &y, &mut self.diags);
                    self.report_diags();
                    let v = r.map_err(|e| self.op_error(e))?;
                    store(&cell, v.clone());
                    reg!(dst) = v;
                }
                Instr::InstanceOf { dst, src, class } => {
                    let v = self.operand(w, clo, src);
                    reg!(dst) = Value::bool(self.instance_of(func, &v, class, w));
                }
                Instr::Clone { dst, src } => {
                    let v = self.operand(w, clo, src);
                    // A lazy object is initialized before it is cloned.
                    if let Some(o) = v.as_object()
                        && o.is_uninit_lazy()
                    {
                        let o = Rc::clone(o);
                        self.lazy_init(regs, &o)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let (copy, hook) = self.clone_object(&v)?;
                    if let Some(m) = hook {
                        // Its readonly properties writable once more meanwhile.
                        self.run_clone_hook(regs, &copy, m)?;
                        regs[base + dst as usize] = Value::object(copy);
                        return Ok(Next::Go(pc));
                    }
                    reg!(dst) = Value::object(copy);
                }
                Instr::Throw { src } => {
                    let v = self.operand(w, clo, src);
                    return Err(match v.into_object() {
                        Ok(o) if o.class.is_a(b"throwable") => Throw::Object(o),
                        // A force-closed fiber's exit, rethrown past a catch or finally.
                        Ok(o) if self.fiber_exit.as_ref().is_some_and(|m| Rc::ptr_eq(m, &o)) => Throw::Object(o),
                        _ => self.throw("Error", "Can only throw objects"),
                    });
                }
                Instr::CatchIf { src, class, target } => {
                    if self.instance_of(func, &reg!(src), class, w) {
                        pc = target as usize;
                    }
                }
                Instr::Call { site, base: args, argc, dst } => return self.call_site(regs, clo, base, site, (args, argc, dst), pc),
                Instr::Return { src } => {
                    // The main frame's variables are the globals, which the
                    // script's end still sees: copied. Elsewhere moved out
                    // when nothing reads the register again (`returned`).
                    let main = self.frames.len() == 1 && self.frames.floor == 0;
                    let moved = if main { None } else { returned(w, self.frames.last().expect("the returning frame"), 0, src) };
                    let v = match moved {
                        Some(v) => v,
                        None => self.operand(w, clo, src),
                    };
                    if self.frames.len() == 1 && self.frames.floor == 0 && self.fibers.is_empty() {
                        // The script ends (the main frame returns, not a
                        // fiber's first): shutdown functions and destructors,
                        // with its globals still there.
                        self.script_end(regs)?;
                        if v.as_bool() == Some(false) {
                            // A router declining the request: the script that
                            // serves it sees the request arrays as it left them.
                            self.carry = Some(self.capture_request_globals(&regs[base..], func));
                        }
                        self.main_return = Some(v.clone());
                    }
                    if let Some(g) = self.frames.last_mut().and_then(|f| f.take_generator()) {
                        self.finish(&g, v);
                        leave(&mut self.frames, regs, Value::NULL);
                        return Ok(Next::Switch);
                    }
                    leave(&mut self.frames, regs, v);
                    return Ok(Next::Switch);
                }

                Instr::ReturnNull => {
                    if self.frames.len() == 1 && self.frames.floor == 0 && self.fibers.is_empty() {
                        self.script_end(regs)?;
                    }
                    if let Some(g) = self.frames.last_mut().and_then(|f| f.take_generator()) {
                        self.finish(&g, Value::NULL);
                    }
                    leave(&mut self.frames, regs, Value::NULL);
                    return Ok(Next::Switch);
                }
                Instr::Yield { dst, value, key, has_key } => {
                    let v = self.operand(w, clo, value);
                    let k = if has_key { Some(self.operand(w, clo, key)) } else { None };
                    return Ok(self.suspend(regs, v, k, pc, Some(dst)));
                }
                Instr::YieldFrom { dst, src } => {
                    match self.yield_from(regs, base, src, at)? {
                        Ok(next) => return Ok(next),
                        Err(result) => regs[base + dst as usize] = result,
                    }
                }
        }
        Ok(Next::Go(pc))
    }

    /// The generated instructions' cold path (`cold=` in ops.def): their
    /// `Instr` run whole, every case (warnings, references, objects, magic
    /// and hooked properties), with none of `step`'s dispatch; `step` runs
    /// them here too (a function with registers past the window).
    #[cold]
    #[inline(never)]
    fn cold_run(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, _argc: usize, at: usize) -> Result<Next, Throw> {
        // This instruction's index (a lazy object's initialization runs it again).
        let insn_at = at;
        let func = &*clo.code;
        let mut pc = at + 1;
        let w = &mut regs[base..];
        let w = &mut w[..func.nregs as usize];
        macro_rules! reg {
            ($r:expr) => {
                w[$r as usize]
            };
        }
        macro_rules! name {
            ($r:expr) => {
                Self::val(w, clo, $r)
            };
        }
        let instr = func.code[at];
        match instr {
            Instr::Move { dst, src } => {
                let v = self.operand(w, clo, src);
                reg!(dst) = v;
            }
            Instr::LoadConst { dst, k } => reg!(dst) = clo.consts[k as usize].clone(),
            Instr::Assign { var, src } => {
                let v = self.operand(w, clo, src);
                assign(w, var, v);
            }
            Instr::AssignTake { var, src } => {
                let v = self.operand(w, clo, src);
                clear(&mut reg!(src));
                assign(w, var, v);
            }
            Instr::Free { reg } => {
                clear(&mut reg!(reg));
            }
            Instr::This { dst } => match self.frames.last().and_then(|f| f.this.clone()) {
                Some(this) => reg!(dst) = Value::object(this),
                None => return Err(self.throw("Error", "Using $this when not in object context")),
            },
            Instr::JumpIf { cond, target } | Instr::JumpIfNot { cond, target } => {
                let c = match reg!(cond).as_bool() {
                    Some(b) => b,
                    None => self.truthy(w, clo, cond),
                };
                if c == matches!(instr, Instr::JumpIf { .. }) {
                    pc = target as usize;
                }
            }
            Instr::JumpLt { .. }
            | Instr::JumpNotLt { .. }
            | Instr::JumpLe { .. }
            | Instr::JumpNotLe { .. }
            | Instr::JumpEq { .. }
            | Instr::JumpNotEq { .. }
            | Instr::JumpIdentical { .. }
            | Instr::JumpNotIdentical { .. } => {
                let Some((cmp, a, b, when, target)) = instr.as_jump_cmp() else { unreachable!("a compare-and-branch") };
                if self.test_regs(w, clo, cmp, a, b) == when {
                    pc = target as usize;
                }
            }
            Instr::FetchDim { dst, arr, key, quiet, empty } => {
                let container = if quiet {
                    match reg!(arr).as_reference() {
                        Some(cell) => cell.borrow().clone(),
                        None => reg!(arr).clone(),
                    }
                } else {
                    self.operand(w, clo, arr)
                };
                // An ArrayAccess object: its offsetGet.
                if let Some(o) = container.as_object() {
                    let k = self.operand(w, clo, key);
                    let v = self.aa_get(regs, o, k, quiet)?;
                    regs[base + dst as usize] = v;
                    return Ok(Next::Go(pc));
                }
                // empty()'s last dimension: its key checked as isset
                // checks it (`fast` took an int or string key already).
                if empty {
                    let k = self.operand(w, clo, key);
                    let v = if let Some(a) = container.as_array() {
                        let k = self.isset_key(&k, true)?;
                        a.get(&k).map_or(Value::NULL, Value::deref)
                    } else if let Some(s) = container.cloned_str() {
                        self.isset_str_offset(&s, &k, true)?.unwrap_or(Value::NULL)
                    } else {
                        Value::NULL
                    };
                    reg!(dst) = v;
                    return Ok(Next::Go(pc));
                }
                let v = self.fetch_dim(&container, w, clo, key, quiet)?;
                reg!(dst) = v;
            }
            // The path is walked once: each level's key converted for its
            // container; an ArrayAccess object on the way takes the rest.
            Instr::AssignDim { var, keys, n, append, val, dst } => {
                let v = self.operand(w, clo, val);
                // `$a[] = v` on an array variable (the commonest write):
                // appended in place, nothing else to look at.
                if n == 0
                    && append
                    && let Some(a) = reg!(var).array_mut()
                {
                    reg!(dst) = self.append(a, v)?;
                    return Ok(Next::Go(pc));
                }
                // The result is what was stored: a string offset's one byte.
                reg!(dst) = match self.assign_dim(w, clo, var, keys, n, append, v, None)? {
                    Written::Done(r) => r,
                    Written::Object(o, at, v) => {
                        // (The one key may be a constant operand.)
                        let rest: Vec<Value> = match const_index(keys) {
                            Some(k) if at == 0 => vec![clo.consts[k].clone()],
                            Some(_) => Vec::new(),
                            None => w[keys as usize + at..keys as usize + n as usize].iter().map(Value::deref).collect(),
                        };
                        let r = self.aa_write(regs, &Value::object(o), &rest, append, arrayaccess::DimOp::Set(v))?;
                        regs[base + dst as usize] = r;
                        return Ok(Next::Go(pc));
                    }
                };
            }
            Instr::IssetDim { dst, var, keys, n } => {
                // (The one key may be a constant operand, read in place.)
                let key_regs: &[Value] = match const_index(keys) {
                    Some(k) => std::slice::from_ref(&clo.consts[k]),
                    None => &w[keys as usize..keys as usize + n as usize],
                };
                if arrayaccess::path_has_object(&reg!(var), key_regs, false) {
                    let key_values: Vec<Value> = key_regs.iter().map(Value::deref).collect();
                    let root = reg!(var).deref();
                    let set = self.aa_isset(regs, &root, &key_values)?;
                    regs[base + dst as usize] = Value::bool(set);
                    return Ok(Next::Go(pc));
                }
                let mut cur = match reg!(var).as_reference() {
                    Some(cell) => cell.borrow().clone(),
                    None => reg!(var).clone(),
                };
                let mut set = true;
                for i in 0..n {
                    let last = i + 1 == n;
                    let next = if let Some(s) = cur.cloned_str() {
                        // A string's offset: the character there.
                        self.isset_str_offset(&s, &key_regs[i as usize], last)?
                    } else if let Some(a) = cur.as_array() {
                        // An int or string key as it is; anything
                        // else by isset's rules (isset_key).
                        // (A string key looked up as it is: no handle made.)
                        let kv = &key_regs[i as usize];
                        let found = if let Some(i) = kv.as_int() {
                            a.get_int(i)
                        } else if let Some(s) = kv.as_str() {
                            a.get_str(s)
                        } else {
                            let other = kv.clone();
                            a.get(&self.isset_key(&other, last)?)
                        };
                        found.map(|v| v.deref())
                    } else {
                        None
                    };
                    match next {
                        Some(v) => cur = v,
                        None => {
                            set = false;
                            break;
                        }
                    }
                }
                reg!(dst) = Value::bool(set && !cur.is_null_or_undef());
            }
            Instr::FetchProp { dst, obj, name, quiet } => {
                let target = if quiet { reg!(obj).deref() } else { self.prop_target(w, clo, obj)? };
                // The name's constant: its cache entry.
                let site = const_index(name).and_then(|i| clo.props.get(i));
                if let (Some(site), Some(o)) = (site, target.as_object())
                    && let Some(v) = cached_prop(site.get(), o, self.scope_addr())
                {
                    reg!(dst) = v;
                    return Ok(Next::Go(pc));
                }
                let name = name!(name).clone();
                if let Some(hook) = self.prop_hook(&target, &name, HOOK_GET) {
                    let v = self.run_nested(regs, hook?, &mut [])?;
                    regs[base + dst as usize] = v;
                    return Ok(Next::Go(pc));
                }
                if let Some(o) = target.as_object()
                    && o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS != 0
                    && (self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || (quiet && self.prop_magic(&target, &name, crate::classes::Magic::Isset).is_some()))
                {
                    use crate::classes::Magic;
                    let pname = Value::str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                    // `??`: __isset decides when there is one, then __get reads.
                    let set = if quiet {
                        match self.prop_magic(&target, &name, Magic::Isset) {
                            Some(found) => self.run_magic(regs, found, &mut [pname.clone()])?.to_bool(),
                            None => true,
                        }
                    } else {
                        true
                    };
                    let v = match self.prop_magic(&target, &name, Magic::Get) {
                        Some(found) if set => self.run_magic(regs, found, &mut [pname])?.deref(),
                        Some(_) => Value::NULL,
                        None => self.fetch_prop(&target, &name, quiet)?,
                    };
                    regs[base + dst as usize] = v;
                    return Ok(Next::Go(pc));
                }
                if let Some(lo) = self.lazy_pending(&target, &name) {
                    self.lazy_init(regs, &lo)?;
                    return Ok(Next::Go(insn_at));
                }
                let target = self.lazy_forward(&target, &name).unwrap_or(target);
                reg!(dst) = self.fetch_prop(&target, &name, quiet)?;
                if let (Some(site), Some(o)) = (site, target.as_object()) {
                    self.remember_prop(site, o, &name);
                }
            }
            Instr::AssignProp { obj, name, val, dst, take } => {
                let target = self.prop_target(w, clo, obj)?;
                // The name's constant: the same cache entry reads use.
                let site = const_index(name).and_then(|i| clo.props.get(i));
                if let (Some(site), Some(o), true) = (site, target.as_object(), name!(name).is_str())
                    && let Some(slot) = cached_slot(site.get(), o, self.scope_addr())
                {
                    let pname = name!(name).clone();
                    let v = self.operand_take(w, clo, val, take);
                    self.write_cached(o, slot, &pname, v.clone())?;
                    reg!(dst) = v;
                    return Ok(Next::Go(pc));
                }
                // (A lazy object's initialization below runs this
                // instruction again: the value is only copied here.)
                let v = self.operand(w, clo, val);
                let name = name!(name).clone();
                if let Some(hook) = self.prop_hook(&target, &name, HOOK_SET) {
                    self.run_nested(regs, hook?, &mut [v.clone()])?;
                    if take {
                        clear(&mut regs[base + val as usize]);
                    }
                    regs[base + dst as usize] = v;
                    return Ok(Next::Go(pc));
                }
                if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Set) {
                    let pname = Value::str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                    self.run_magic(regs, found, &mut [pname, v.clone()])?;
                    if take {
                        clear(&mut regs[base + val as usize]);
                    }
                    regs[base + dst as usize] = v;
                    return Ok(Next::Go(pc));
                }
                if let Some(lo) = self.lazy_pending(&target, &name) {
                    self.lazy_init(regs, &lo)?;
                    return Ok(Next::Go(insn_at));
                }
                let target = self.lazy_forward(&target, &name).unwrap_or(target);
                self.write_prop(&target, &name, v.clone())?;
                // A site already holding this class missed for another
                // reason (an unset slot with __set): looking again finds the same.
                if let (Some(site), Some(o)) = (site, target.as_object())
                    && site.get().class != Rc::as_ptr(&o.class) as usize
                {
                    self.remember_prop(site, o, &name);
                }
                if take {
                    clear(&mut reg!(val));
                }
                reg!(dst) = v;
            }
            _ => unreachable!("cold_run runs the generated instructions"),
        }
        Ok(Next::Go(pc))
    }

    /// A call of what call site `site` names: a user function's frame
    /// pushed on its arguments (`call`: their start, count and the result
    /// register, frame-relative), a builtin called.
    fn call_site(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, site: u16, call: (Reg, u8, Reg), pc: usize) -> Result<Next, Throw> {
        let (args, argc, dst) = call;
        match self.resolve(clo, site)? {
            Callee::User(target) => {
                let target = self.closure_at(target);
                if (argc as usize) < target.code.required {
                    let passed = &regs[base + args as usize..base + args as usize + argc as usize];
                    return Err(self.missing_args(&target.code, passed, None));
                }
                if target.code.is_generator {
                    let m = classes::MethodFrame { func: classes::Code::User(Rc::clone(&target)), this: None, scope: None, called: None, trampoline: None, closure: None };
                    let g = self.make_generator(regs, m, target, base + args as usize, argc as usize);
                    regs[base + dst as usize] = Value::object(g);
                    return Ok(Next::Go(pc));
                }
                if let Some(caller) = self.frames.last_mut() {
                    caller.pc = pc as u32;
                }
                // Missing optional arguments are filled by the callee's defaults.
                self.frames.push(&target, base + args as usize, argc as usize, (base + dst as usize) as u32);
                enter(&mut self.frames, regs);
                Ok(Next::Switch)
            }
            Callee::Builtin(i) => {
                self.call_builtin(regs, base, i, args, argc, dst)?;
                Ok(Next::Go(pc))
            }
            Callee::Constant(_) => unreachable!("a call site resolves to a function"),
        }
    }

    /// Push the frame of a method call: its arguments at `args`
    /// (absolute), its result to `ret` (NO_RET: discarded, as a
    /// constructor's is).
    fn call_method(&mut self, regs: &mut Vec<Value>, mut m: classes::MethodFrame, args: usize, argc: u8, ret: u32, pc: usize) -> Result<Next, Throw> {
        // __call / __callStatic: the arguments become (name, [arguments]).
        let argc = match m.trampoline.take() {
            Some(name) => {
                // (Moved out: no argument register keeps a value.)
                let packed: Vec<Value> = regs[args..args + argc as usize].iter_mut().map(|r| std::mem::take(r).deref()).collect();
                if regs.len() < args + 2 {
                    regs.resize(args + 2, Value::UNDEF);
                }
                regs[args] = Value::str(name);
                regs[args + 1] = Value::array(PhpArray::from_list(packed));
                2
            }
            None => argc,
        };
        let func = match m.func {
            classes::Code::User(func) => func,
            classes::Code::Native(native, name) => {
                // (The arguments' buffer is the context's, kept for its capacity.)
                let mut buf = std::mem::take(&mut self.arg_buf);
                buf.extend(regs[args..args + argc as usize].iter_mut().map(|r| std::mem::replace(r, Value::UNDEF)));
                let saved = std::mem::replace(&mut self.native_class, m.called.take());
                // Lent, as to a builtin: Closure::call runs PHP code, and
                // a class it looks up is autoloaded with it running.
                self.isolate.regs = std::mem::take(regs);
                if let Some(top) = self.frames.last_mut() {
                    top.pc = pc as u32;
                }
                let r = self.run_native_method(native, &name, m.scope.as_ref(), m.this.as_ref(), &mut buf);
                *regs = std::mem::take(&mut self.isolate.regs);
                self.native_class = saved;
                // An error the method raised: the trace shows its call on
                // top (not a hook's, `$name::get`: PHP has no frame there).
                if let (Err(Throw::Error { .. }), Some(scope)) = (&r, &m.scope)
                    && name.as_bytes().first() != Some(&b'$')
                {
                    self.failed_call(name.as_bytes(), Some((&scope.name, m.this.is_some())), &buf);
                }
                buf.clear();
                self.arg_buf = buf;
                let v = r?;
                if ret != NO_RET {
                    regs[ret as usize] = v;
                }
                return Ok(Next::Go(pc));
            }
        };
        if (argc as usize) < func.code.required {
            let class = m.scope.as_ref().map(|c| (c.name.clone(), m.this.is_some()));
            let class = class.as_ref().map(|(n, this)| (n, *this));
            return Err(self.missing_args(&func.code, &regs[args..args + argc as usize], class));
        }
        if func.code.is_generator {
            let frame = classes::MethodFrame { func: classes::Code::User(Rc::clone(&func)), this: m.this, scope: m.scope, called: m.called, trampoline: None, closure: m.closure };
            let g = self.make_generator(regs, frame, Rc::clone(&func), args, argc as usize);
            if ret != NO_RET {
                regs[ret as usize] = Value::object(g);
            }
            return Ok(Next::Go(pc));
        }
        if let Some(caller) = self.frames.last_mut() {
            caller.pc = pc as u32;
        }
        self.frames.push(&func, args, argc as usize, ret);
        let frame = self.frames.last_mut().expect("the frame just pushed");
        frame.this = m.this;
        frame.scope = m.scope;
        frame.called = m.called;
        frame.owner = m.closure;
        enter(&mut self.frames, regs);
        // A closure's `use` values go into their variables, from the
        // Closure called (only a closure's code has `bound` registers).
        if !func.code.bound.is_empty()
            && let Some(data) = self.frames.last().and_then(|f| f.owner.as_deref()).and_then(callable::closure_data)
        {
            for (r, v) in func.code.bound.iter().zip(data.captured.borrow().iter()) {
                regs[args + *r as usize] = v.clone();
            }
        }
        Ok(Next::Switch)
    }

    /// Run a file's code (include, eval) as a frame above the running one,
    /// with its variables, `$this` and class scope; its result into `dst`.
    /// The variable `name` of the scope the running code shares, for a file
    /// it includes: the running frame's run-time variable, else, where the
    /// running frame is itself an included file's (one symbol table with
    /// the code that included it), the including frames' variables, down to
    /// the function or the globals the chain runs in; made in the running
    /// frame when none has it. (`global $x` in a file included from a
    /// function, then `$x = …` in a file that one includes, is that global.)
    fn shared_scope_cell(&mut self, regs: &mut [Value], name: &[u8]) -> Rc<RefCell<Value>> {
        let top = self.frames.len - 1;
        let mut i = top;
        loop {
            let frame = &self.frames.slots[i];
            if let Some(c) = frame.vars().and_then(|v| v.get(name)) {
                return Rc::clone(c);
            }
            if i < top
                && let Some(r) = frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name)
            {
                let base = frame.base as usize;
                return make_ref(&mut regs[base..], r as Reg);
            }
            if frame.global {
                return self.global_cell(regs, name);
            }
            let shared = frame.extra.as_ref().is_some_and(|e| e.shared.is_some());
            if !shared || i == 0 {
                break;
            }
            i -= 1;
        }
        self.frames.slots[top].vars_mut().cell(name)
    }

    fn run_in_scope(&mut self, regs: &mut Vec<Value>, base: usize, func: &Function, main: &Arc<Function>, dst: Reg, pc: usize) -> Next {
        // The file runs in the including code's scope: the globals when
        // that is global scope (however deep in includes), else its locals.
        let global = self.frames.last().is_some_and(|f| f.global);
        let mut cells: Vec<Rc<RefCell<Value>>> = Vec::with_capacity(main.cv_names.len());
        let mut origins: Vec<Origin> = Vec::new();
        for name in &main.cv_names {
            cells.push(match func.cv_names.iter().position(|n| n == name) {
                Some(r) => {
                    if !global {
                        origins.push(Origin::Slot((base + r) as u32));
                    }
                    make_ref(&mut regs[base..base + func.nregs as usize], r as Reg)
                }
                // (The main frame's window starts at register 0.)
                None if global => self.global_cell(regs, name.as_bytes()),
                None => {
                    origins.push(Origin::Var(name.as_bytes().to_vec()));
                    self.shared_scope_cell(regs, name.as_bytes())
                }
            });
        }
        let (this, scope, called) = match self.frames.last() {
            Some(f) => (f.this.clone(), f.scope.clone(), f.called.clone()),
            None => (None, None, None),
        };
        if let Some(caller) = self.frames.last_mut() {
            caller.pc = pc as u32;
        }
        let at = base + func.nregs as usize;
        // A file's code has new `static` variables each time it runs (its
        // compiled code may be the last run's, kept by the loader).
        self.statics.remove(&(Arc::as_ptr(main) as usize));
        self.frames.push(&Closure::new(Arc::clone(main), self.isolate), at, 0, (base + dst as usize) as u32);
        enter(&mut self.frames, regs);
        for (i, cell) in cells.into_iter().enumerate() {
            regs[at + i] = Value::reference(cell);
        }
        let frame = self.frames.last_mut().expect("the file's frame");
        (frame.this, frame.scope, frame.called, frame.global) = (this, scope, called, global);
        if !global {
            frame.extra.get_or_insert_with(Default::default).shared = Some(origins.into_boxed_slice());
        }
        Next::Switch
    }

    /// `[...$x]`: int keys renumbered, string keys kept (PHP 8.1).
    fn unpack_into(&mut self, w: &mut [Value], arr: Reg, items: Vec<(Value, Value)>) -> Result<(), Throw> {
        let Some(a) = w[arr as usize].array_mut() else { return Ok(()) };
        for (k, v) in items {
            match k.into_str() {
                Ok(s) => a.insert(Key::from_string(s), v),
                Err(_) => {
                    if a.push(v).is_err() {
                        return Err(self.throw("Error", "Cannot add element to the array as the next element is already occupied"));
                    }
                }
            }
        }
        Ok(())
    }

    /// `++`/`--` on `target[k1]..[kn]`: (old, new). A missing element
    /// warns and counts as null; arrays are made along the way.
    fn incdec_path(&mut self, target: &mut Value, path: &[Key], inc: bool) -> Result<(Value, Value), Throw> {
        if target.is_null_or_undef() {
            *target = Value::array(PhpArray::new());
        }
        if let Some(cell) = target.as_reference() {
            let mut inner = std::mem::take(&mut *cell.borrow_mut());
            let r = self.incdec_path(&mut inner, path, inc);
            *cell.borrow_mut() = inner;
            return r;
        }
        if target.is_str() {
            return Err(self.throw("Error", "Cannot increment/decrement string offsets"));
        }
        let Some(arr) = target.array_mut() else { return Err(self.throw("Error", "Cannot use a scalar value as an array")) };
        let Some((key, rest)) = path.split_first() else { return Ok((Value::NULL, Value::NULL)) };
        if !rest.is_empty() {
            if !arr.contains_key(key) {
                arr.insert(key.clone(), Value::NULL);
            }
            let mut child = std::mem::take(arr.get_mut(key).expect("present"));
            let r = self.incdec_path(&mut child, rest, inc);
            *arr.get_mut(key).expect("present") = child;
            return r;
        }
        let old = match arr.get(key) {
            Some(v) => v.deref(),
            None => {
                let shown = match key {
                    Key::Int(i) => i.to_string(),
                    Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
                };
                self.warn(format!("Undefined array key {shown}"));
                Value::NULL
            }
        };
        let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
        self.report_diags();
        let new = r.map_err(|e| self.op_error(e))?;
        match arr.get(key).and_then(Value::as_reference) {
            Some(cell) => *cell.borrow_mut() = new.clone(),
            None => arr.insert(key.clone(), new.clone()),
        }
        Ok((old, new))
    }

    /// A resource object (numbered by the resources' own counter).
    fn make_resource(&mut self, resource: p8_runtime::stream::Resource) -> Rc<Object> {
        // A user-space wrapper's stream is closed (its stream_close run)
        // when the resource goes: its class has that as a destructor.
        let user = resource.stream.borrow().user().is_some();
        let dtor: Option<p8_runtime::object::NativeMethod> = if user { Some(p8_runtime::user_stream::free) } else { None };
        let class = self
            .resource_classes
            .entry((resource.kind, user))
            .or_insert_with(|| {
                Rc::new(Class {
                    name: PhpStr::from(resource.kind),
                    lower: resource.kind.as_bytes().to_vec(),
                    flags: p8_runtime::object::CLASS_RESOURCE | p8_runtime::object::CLASS_FINAL | if user { p8_runtime::object::CLASS_DESTRUCTOR } else { 0 },
                    parent: None,
                    interfaces: Vec::new(),
                    properties: Vec::new(),
                    slots: p8_runtime::object::Slots::default(),
                    methods: p8_runtime::object::Methods::new(&self.isolate.names),
                    constants: p8_runtime::object::Constants::default(),
                    statics: Default::default(),
                    property_order: Vec::new(),
                    debug_info: None,
                    get_properties: None,
                    cast_bool: None,
                    cast_string: None,
                    cast_int: None,
                    clone_data: None,
                    forward_calls: None,
                    make_data: None,
                    native_dtor: dtor,
                    enum_cases: Vec::new(),
                    enum_backing: None,
                    static_inits: Vec::new(),
                })
            })
            .clone();
        Object::with_data(class, &self.resource_ids, Some(Rc::new(resource)))
    }

    /// STDIN, STDOUT, STDERR (resources 1 to 3; the next one is 5, as in the CLI).
    fn standard_streams(&mut self) {
        use p8_runtime::stream::{Resource, Stream, Target};
        // STDIN, STDOUT and STDERR are the CLI's; a web request has none.
        if self.out.response.server {
            return;
        }
        for (name, target, path, mode) in [
            ("STDIN", Target::Stdin, "php://stdin", "r"),
            ("STDOUT", Target::Stdout, "php://stdout", "w"),
            ("STDERR", Target::Stderr, "php://stderr", "w"),
        ] {
            let r = Resource { kind: "stream", closed: std::cell::Cell::new(false), stream: RefCell::new(Stream::new(target, path, mode)) };
            let v = Value::object(self.make_resource(r));
            let key = self.isolate.names.intern(name.as_bytes());
            self.constants.insert(key, (key, v));
        }
        self.resource_ids.take();
    }

    /// A builtin starts running at the current frame count (see NativeCall).
    pub(crate) fn push_native(&mut self, name: &'static str, method: Option<(PhpStr, PhpStr, Option<Rc<Object>>)>, args: &[Value], top: bool) {
        let mut list = self.native_args.pop().unwrap_or_default();
        list.extend(args.iter().map(Value::deref));
        self.natives.push(NativeCall { depth: self.frames.len(), name, method, args: list, top });
    }

    /// The innermost builtin of `natives` is done.
    pub(crate) fn pop_native(&mut self) {
        if let Some(mut n) = self.natives.pop() {
            n.args.clear();
            self.native_args.push(n.args);
        }
    }

    /// Run a builtin class's method `name` of class `scope` as a call: the
    /// frames of PHP code it runs (a callback, an autoloader) show it
    /// below them, as PHP shows an internal method's frame. A property
    /// hook's (`$name::get`) has no frame in PHP.
    pub(crate) fn run_native_method(&mut self, native: p8_runtime::object::Native, name: &PhpStr, scope: Option<&Rc<Class>>, this: Option<&Rc<Object>>, args: &mut [Value]) -> Result<Value, Throw> {
        let Some(scope) = scope.filter(|_| name.as_bytes().first() != Some(&b'$')) else {
            return self.run_native(native, this, args);
        };
        let method = Some((name.clone(), scope.name.clone(), this.cloned()));
        self.push_native("", method, args, false);
        let r = self.run_native(native, this, args);
        self.pop_native();
        r
    }

    /// Run a builtin class's method.
    pub(crate) fn run_native(&mut self, native: p8_runtime::object::Native, this: Option<&Rc<Object>>, args: &mut [Value]) -> Result<Value, Throw> {
        let r = match (native, this) {
            (p8_runtime::object::Native::Method(f), Some(this)) => f(self, this, args),
            (p8_runtime::object::Native::Static(f), _) => f(self, args),
            (p8_runtime::object::Native::Method(_), None) => Err(self.throw("Error", "Non-static method cannot be called statically")),
        };
        let r = match r {
            Err(Throw::Autoload(n)) => Err(self.unlent_autoload(n)),
            r => r,
        };
        self.report_diags();
        r
    }

    /// Objects passed to `string` parameters become their __toString(), as
    /// zend_parse_arg_str does (when the registers are lent, so PHP code
    /// can run).
    /// (Only an object argument can need it: the common call looks no
    /// further than the arguments' kinds.)
    #[inline]
    fn stringify_args(&mut self, sig: &p8_runtime::stubs::Sig, args: &mut [Value]) -> Result<(), Throw> {
        if self.isolate.regs.is_empty() || !args.iter().any(Value::is_object) {
            return Ok(());
        }
        self.stringify_objects(sig, args)
    }

    #[inline(never)]
    fn stringify_objects(&mut self, sig: &p8_runtime::stubs::Sig, args: &mut [Value]) -> Result<(), Throw> {
        use p8_runtime::stubs::{T_CALLABLE, T_MIXED, T_OBJECT, T_STRING};
        for (i, arg) in args.iter_mut().enumerate() {
            let Some(param) = sig.params.get(i).or_else(|| sig.params.last().filter(|p| p.variadic)) else { break };
            if param.by_ref || param.mask & T_STRING == 0 || param.mask & (T_OBJECT | T_MIXED | T_CALLABLE) != 0 {
                continue;
            }
            let Some(o) = arg.as_object() else { continue };
            if param.classes.iter().any(|c| o.class.is_a(c.to_ascii_lowercase().as_bytes())) || o.class.method(b"__tostring").is_none() {
                continue;
            }
            let method = Value::array(PhpArray::from_list(vec![Value::object(Rc::clone(o)), Value::str("__toString")]));
            *arg = self.call_from_builtin(&method, &mut [])?;
        }
        Ok(())
    }

    /// An object as a string, when its class's __toString is native (a
    /// Throwable's); anything else as it is.
    /// A value in a string context (echo, `.`, interpolation, a string
    /// cast): an object becomes what its __toString() returns, run here
    /// as PHP code when it is; one without __toString stays for the caller
    /// to report.
    pub(crate) fn string_of(&mut self, regs: &mut Vec<Value>, v: Value) -> Result<Value, Throw> {
        let v = match v.as_reference() {
            Some(cell) => cell.borrow().clone(),
            None => v,
        };
        let Some(o) = v.as_object() else { return Ok(v) };
        if o.class.flags & p8_runtime::object::CLASS_RESOURCE != 0 || o.class.cast_string.is_some() {
            return Ok(v);
        }
        match o.class.method(b"__tostring") {
            Some(m) if m.native.is_some() => self.object_to_string(v),
            Some(_) => {
                let callable = Value::array(PhpArray::from_list(vec![v.clone(), Value::str("__toString")]));
                self.call_with_regs(regs, &callable, &mut [])
            }
            None => Ok(v),
        }
    }

    fn object_to_string(&mut self, v: Value) -> Result<Value, Throw> {
        let Some(o) = v.as_object() else { return Ok(v) };
        let Some(p8_runtime::object::Native::Method(native)) = o.class.method(b"__tostring").and_then(|m| m.native) else {
            return Ok(v);
        };
        let o = Rc::clone(o);
        native(self, &o, &mut [])
    }

    /// Call builtin `i` on the `argc` arguments at `args` of the frame at
    /// `base`, its result into `dst`. The arguments move into a buffer
    /// and the register file is lent to the context while the builtin
    /// runs, so it can call back into PHP (array_map, usort) and run the
    /// autoloaders where it looks a class up (class_exists, is_a).
    #[inline(never)]
    fn call_builtin(&mut self, regs: &mut Vec<Value>, base: usize, i: usize, args: Reg, argc: u8, dst: Reg) -> Result<(), Throw> {
        let start = base + args as usize;
        // An object argument may need its __toString() (PHP code) to become
        // a string parameter: that takes the path that can call back.
        let objects = regs[start..start + argc as usize].iter().any(Value::is_object);
        // Before the headers go out, output may run header_register_callback's callback.
        let header_due = !self.out.started && self.header_callback_set && self.ext_state.contains_key("header_callback");
        if !self.isolate.registry.reenters(i) && !objects && !header_due {
            // The common case: the builtin works on the registers directly.
            let args = &mut regs[start..start + argc as usize];
            let r = self.run_builtin(i, args);
            for v in args {
                clear(v);
            }
            regs[base + dst as usize] = r?;
            return Ok(());
        }
        let mut buf = std::mem::take(&mut self.arg_buf);
        buf.extend(regs[start..start + argc as usize].iter_mut().map(|r| std::mem::replace(r, Value::UNDEF)));
        self.isolate.regs = std::mem::take(regs);
        // Running: frames it calls show it in their traces.
        // Its frames show this call's line.
        if let Some(top) = self.frames.last_mut() {
            top.pc = self.pc as u32;
        }
        let name = self.isolate.registry.function(i).name;
        self.push_native(name, None, &buf, false);
        let r = self.run_builtin(i, &mut buf);
        self.pop_native();
        *regs = std::mem::take(&mut self.isolate.regs);
        buf.clear();
        self.arg_buf = buf;
        regs[base + dst as usize] = r?;
        Ok(())
    }

    /// Run builtin `i` on `args`: PHP's argument checks and conversions
    /// (from the stub signature), then the function.
    #[inline(always)]
    /// A builtin's `callable` arguments, checked as zend_parse_arg_func
    /// does (zend_fcall_info_init): Err the TypeError for the first that
    /// cannot be called from here.
    fn check_callbacks(&mut self, sig: &p8_runtime::stubs::Sig, args: &[Value]) -> Result<Result<(), p8_runtime::args::Refused>, Throw> {
        for (n, arg) in args.iter().enumerate() {
            let Some(param) = sig.params.get(n).or_else(|| sig.params.last().filter(|p| p.variadic)) else { break };
            if !p8_runtime::args::wants_callable_check(param, arg) {
                continue;
            }
            if let Some(why) = self.why_not_callable(arg, true)? {
                return Ok(Err(p8_runtime::args::callback_error(sig, n, &why)));
            }
        }
        Ok(Ok(()))
    }

    /// What the borrowed builtin that returned `Err(Fault)` raised.
    #[cold]
    pub(crate) fn take_fault(&mut self) -> Throw {
        self.fault.take().expect("a Fault is made only by Ctx::fault, which keeps its Throw")
    }

    fn run_builtin(&mut self, i: usize, args: &mut [Value]) -> Result<Value, Throw> {
        let call = *self.isolate.registry.call(i);
        if call.deprecated
            && let Some(message) = self.isolate.registry.deprecation(i)
        {
            self.diags.push(Diag::Deprecated(message.to_string()));
        }
        if let Some(sig) = call.sig {
            self.stringify_args(sig, args)?;
            let mut checked = p8_runtime::args::check_counted(sig, call.counts, args, &mut self.diags);
            self.report_diags();
            // Callbacks: whether each can be called from here (a builtin
            // taking one is a builtin that may run PHP code).
            if checked.is_ok() && call.reenters && !call.own_callbacks {
                checked = self.check_callbacks(sig, args)?;
            }
            if let Err((class, message)) = checked {
                let name = self.isolate.registry.function(i).name;
                if !opcode_function(name, args.len()) {
                    self.failed_call(name.as_bytes(), None, args);
                }
                return Err(self.throw(class, message));
            }
        }
        let outer = self.running_builtin.replace(i);
        let mut r = match call.run {
            Imp::Moved(run) => run(self, args),
            // The adapter: a borrowed builtin, its arguments checked and
            // converted in place above, lent as they are now.
            Imp::Lent(run) => run(self, args).map_err(|_| self.take_fault()),
        };
        self.running_builtin = outer;
        // (Checked before it is taken: taking moves the whole slot.)
        if self.header_throw.is_some()
            && let Some(t) = self.header_throw.take()
        {
            r = Err(t);
        }
        match r {
            // The builtin the error was raised in is the trace's first
            // frame (an exception's, and a fatal error's since 8.5).
            Err(Throw::Error { .. } | Throw::Fatal { .. }) => self.failed_call(self.isolate.registry.function(i).name.as_bytes(), None, args),
            // An E_USER_ERROR waiting for the user handler (trigger_error()):
            // if the handler declines it, the fatal error's trace starts
            // with this call (see dispatch_errors).
            Ok(_) if self.pending_errors.iter().any(|e| e.0 == 256) => self.failed_call(self.isolate.registry.function(i).name.as_bytes(), None, args),
            Err(Throw::Autoload(n)) => r = Err(self.unlent_autoload(n)),
            _ => {}
        }
        self.report_diags();
        r
    }

    /// Whether `a cmp b` holds for two registers (`<=>`: whether it is
    /// nonzero), with fast paths for numbers.
    #[inline(always)]
    fn test_regs(&mut self, w: &[Value], clo: &Closure, cmp: Cmp, a: Reg, b: Reg) -> bool {
        let (x, y) = (Self::val(w, clo, a), Self::val(w, clo, b));
        let fast = match (x.as_int(), y.as_int(), x.as_float(), y.as_float()) {
            (Some(x), Some(y), _, _) => Some(x.cmp(&y)),
            (_, _, Some(x), Some(y)) => x.partial_cmp(&y),
            // === between an int and a float is false: no fast path there
            (Some(x), _, _, Some(y)) if !matches!(cmp, Cmp::Identical | Cmp::NotIdentical) => (x as f64).partial_cmp(&y),
            (_, Some(y), Some(x), _) if !matches!(cmp, Cmp::Identical | Cmp::NotIdentical) => x.partial_cmp(&(y as f64)),
            _ => None,
        };
        match fast {
            Some(o) => match cmp {
                Cmp::Lt => o.is_lt(),
                Cmp::Le => o.is_le(),
                Cmp::Gt => o.is_gt(),
                Cmp::Ge => o.is_ge(),
                Cmp::Eq | Cmp::Identical => o.is_eq(),
                Cmp::NotEq | Cmp::NotIdentical | Cmp::Spaceship => o.is_ne(),
            },
            None => self.test_slow(w, clo, cmp, a, b),
        }
    }

    /// A comparison that is not of two numbers (strings, null, arrays…):
    /// out of line, but not rare (`$s === ''`, `$x !== null`).
    #[inline(never)]
    fn test_slow(&mut self, w: &[Value], clo: &Closure, cmp: Cmp, a: Reg, b: Reg) -> bool {
        let (x, y) = (Self::val(w, clo, a), Self::val(w, clo, b));
        // Two defined, non-reference registers: looked at in place.
        if !(x.is_undef() || x.is_reference()) && !(y.is_undef() || y.is_reference()) {
            return test_values(cmp, x, y);
        }
        let x = self.operand_ref(w, clo, a);
        let y = self.operand_ref(w, clo, b);
        test_values(cmp, &x, &y)
    }

    /// A call's variables passed as they are (`Function::arg_vars[vars]`)
    /// into its argument registers from `args`: by reference where
    /// `by_ref(n)` says the parameter takes one (an undefined variable
    /// becomes null, no warning), else the value (an undefined one warns,
    /// as reading it does): SEND_VAR_EX.
    fn send_vars(&mut self, w: &mut [Value], clo: &Closure, vars: u16, args: Reg, by_ref: impl Fn(&Self, usize) -> bool) {
        for &(n, var) in &clo.code.arg_vars[vars as usize] {
            let v = if by_ref(self, n as usize) { Value::reference(make_ref(w, var)) } else { self.operand(w, clo, var) };
            w[args as usize + n as usize] = v;
        }
    }

    /// ArgumentCountError for a call passing fewer than `func.required`
    /// arguments: reported at the declaration, naming the call's line.
    #[cold]
    fn missing_args(&mut self, func: &Function, args: &[Value], class: Option<(&PhpStr, bool)>) -> Throw {
        let passed = args.len();
        self.failed_call(func.name.as_bytes(), class, args);
        let (file, line) = self.location();
        let declared = func.params.iter().filter(|p| !p.variadic).count();
        let exactly = if func.required == declared { "exactly" } else { "at least" };
        let name = match &func.class {
            Some(class) => format!("{}::{}", String::from_utf8_lossy(class.as_bytes()), String::from_utf8_lossy(func.name.as_bytes())),
            None => String::from_utf8_lossy(func.name.as_bytes()).into_owned(),
        };
        Throw::Error {
            class: "ArgumentCountError",
            message: format!(
                "Too few arguments to function {name}(), {passed} passed in {file} on line {line} and {exactly} {} expected",
                func.required
            ),
            file: Rc::from(&*func.file),
            line: func.line,
            props: Vec::new(),
        }
    }

    /// The cell of global `$name`: the main frame's variable if it has one.
    /// `main` is the main frame's window (it starts at register 0).
    /// The name in register `r`, for `$$name`.
    fn var_name(&mut self, w: &[Value], clo: &Closure, r: Reg) -> Vec<u8> {
        let v = self.operand(w, clo, r);
        ops::to_string_lossy(&v, &mut self.diags).as_bytes().to_vec()
    }

    /// The running frame's variable `name`, made a reference: a compiled
    /// variable, a main-script global, or one only named at run time.
    fn var_cell(&mut self, w: &mut [Value], in_main: bool, name: &[u8]) -> Rc<RefCell<Value>> {
        if in_main {
            return self.global_cell(w, name);
        }
        let frame = self.frames.last_mut().expect("a running frame");
        if let Some(r) = frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            return make_ref(w, r as Reg);
        }
        frame.vars_mut().cell(name)
    }

    /// The running frame's variable `name` (a compiled one, or one only
    /// named at run time) made the reference `cell`.
    fn bind_local(&mut self, w: &mut [Value], name: &[u8], cell: Rc<RefCell<Value>>) {
        let frame = self.frames.last_mut().expect("a running frame");
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => w[r] = Value::reference(cell),
            None => frame.vars_mut().bind(name, cell),
        }
    }

    /// The running frame's variable `name`, if set.
    fn var_value(&self, w: &[Value], in_main: bool, name: &[u8]) -> Option<Value> {
        let frame = self.frames.last()?;
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => Some(w[r].deref()).filter(|v| !v.is_undef()),
            None if in_main => self.globals.get(name).map(|c| c.borrow().clone()).filter(|v| !v.is_undef()),
            None => frame.vars()?.get(name).map(|c| c.borrow().clone()),
        }
    }

    fn var_unset(&mut self, w: &mut [Value], in_main: bool, name: &[u8]) {
        let Some(frame) = self.frames.last_mut() else { return };
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => w[r] = Value::UNDEF,
            None if in_main => {
                self.globals.shift_remove(name);
            }
            None => {
                if let Some(e) = &mut frame.extra {
                    e.vars.remove(name);
                }
            }
        }
    }

    /// The global variable `name`'s value, if set (`main`: the main frame's window).
    fn global_value(&self, main: &[Value], name: &[u8]) -> Option<Value> {
        let code = &self.main_frame()?.func.code;
        match code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => Some(main[r].deref()).filter(|v| !v.is_undef()),
            None => self.globals.get(name).map(|c| c.borrow().deref()).filter(|v| !v.is_undef()),
        }
    }

    /// The global variable `name` becomes `cell` (a reference binding).
    fn global_bind(&mut self, main: &mut [Value], name: &[u8], cell: Rc<RefCell<Value>>) {
        let Some(frame) = self.main_frame() else { return };
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => main[r] = Value::reference(cell),
            None => {
                self.globals.insert(name.to_vec(), cell);
            }
        }
    }

    fn global_unset(&mut self, main: &mut [Value], name: &[u8]) {
        let Some(frame) = self.main_frame() else { return };
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => main[r] = Value::UNDEF,
            None => {
                self.globals.shift_remove(name);
            }
        }
    }

    /// `unset($x)` in a file included at global scope: the global's slot is
    /// emptied, not removed, so every file bound to it (and a later
    /// assignment through any of them) still shares it, as PHP's files share
    /// symbol-table slots.
    fn global_clear(&mut self, main: &mut [Value], name: &[u8]) {
        let Some(frame) = self.main_frame() else { return };
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => match main[r].as_reference() {
                Some(cell) => *cell.borrow_mut() = Value::UNDEF,
                None => main[r] = Value::UNDEF,
            },
            None => {
                if let Some(cell) = self.globals.get(name) {
                    *cell.borrow_mut() = Value::UNDEF;
                }
            }
        }
    }

    /// `$GLOBALS` read whole: the set global variables, by name, in the
    /// order get_defined_vars() gives them: the ones the request starts
    /// with, the main script's, then the others as they were made.
    fn globals_array(&self, main: &[Value]) -> p8_runtime::array::PhpArray {
        let mut out = p8_runtime::array::PhpArray::new();
        for n in REQUEST_GLOBALS {
            if let Some(c) = self.globals.get(n.as_bytes()) {
                let v = c.borrow().deref();
                if !v.is_undef() {
                    out.insert(Key::from_string(PhpStr::from(*n)), v);
                }
            }
        }
        if let Some(frame) = self.main_frame() {
            for (r, n) in frame.func.code.cv_names.iter().enumerate() {
                if n.as_bytes() == b"GLOBALS" {
                    continue;
                }
                if let Some(v) = main.get(r).map(Value::deref)
                    && !v.is_undef()
                {
                    out.insert(Key::from_string(PhpStr::new(n.as_bytes().to_vec())), v);
                }
            }
        }
        for (n, c) in &self.globals {
            let v = c.borrow().deref();
            if !v.is_undef() {
                out.insert(Key::from_string(PhpStr::new(n.clone())), v);
            }
        }
        out
    }

    /// The main script's variable `name`, by its register.
    fn main_cv(&mut self, name: &[u8]) -> Option<usize> {
        let code = &self.main_frame().expect("the main frame").func.code;
        let key = Arc::as_ptr(code) as usize;
        if self.main_cvs.as_ref().is_none_or(|(k, _)| *k != key) {
            let index = code.cv_names.iter().enumerate().map(|(i, n)| (n.as_bytes().to_vec(), i)).collect();
            self.main_cvs = Some((key, index));
        }
        self.main_cvs.as_ref().and_then(|(_, m)| m.get(name).copied())
    }

    /// The global variable `name`'s cell as it is now; one not set is a
    /// new unset one (not null, as `global $x` makes it).
    fn global_slot(&mut self, main: &mut [Value], name: &[u8]) -> Rc<RefCell<Value>> {
        if let Some(r) = self.main_cv(name) {
            return make_ref(main, r as Reg);
        }
        self.global_entry(name, Value::UNDEF)
    }

    /// The global `name`'s cell in `globals`, made (holding `made`) if
    /// there is none: the name is copied only then.
    fn global_entry(&mut self, name: &[u8], made: Value) -> Rc<RefCell<Value>> {
        if let Some(cell) = self.globals.get(name) {
            return Rc::clone(cell);
        }
        let cell = Rc::new(RefCell::new(made));
        self.globals.insert(name.to_vec(), Rc::clone(&cell));
        cell
    }

    fn global_cell(&mut self, main: &mut [Value], name: &[u8]) -> Rc<RefCell<Value>> {
        if let Some(r) = self.main_cv(name) {
            // `global $x` makes a global that is not set a null one.
            let v = &mut main[r];
            if let Some(cell) = v.as_reference() {
                if cell.borrow().is_undef() {
                    *cell.borrow_mut() = Value::NULL;
                }
            } else if v.is_undef() {
                *v = Value::NULL;
            }
            return make_ref(main, r as Reg);
        }
        let cell = self.global_entry(name, Value::NULL);
        if cell.borrow().is_undef() {
            *cell.borrow_mut() = Value::NULL;
        }
        cell
    }

    /// The keys of a dimension path (`$a[k1]..[kn]`) from the `n`
    /// registers at `keys`, in the context's reusable buffer: a write
    /// into an array allocates no list of its keys. Hand it back with
    /// key_path_done (a nested one meanwhile gets a buffer of its own).
    fn key_path(&mut self, w: &[Value], clo: &Closure, keys: Reg, n: u8) -> Result<Vec<Key>, Throw> {
        let mut path = std::mem::take(&mut self.key_path);
        path.clear();
        for i in 0..n {
            path.push(self.key(w, clo, keys + i as Reg)?);
        }
        Ok(path)
    }

    fn key_path_done(&mut self, mut path: Vec<Key>) {
        path.clear();
        self.key_path = path;
    }

    /// An array key from register `r`, reporting what PHP reports.
    fn key(&mut self, w: &[Value], clo: &Closure, r: Reg) -> Result<Key, Throw> {
        let v = Self::val(w, clo, r);
        if v.is_undef() || v.is_reference() {
            // Undefined (it warns) or a reference: its value.
            let v = self.operand(w, clo, r);
            self.key_of(&v, "access")
        } else {
            // Else the register itself, without a copy of it first.
            self.key_of(v, "access")
        }
    }

    /// A resource used as an array key: its number, warned
    /// (zend_use_resource_as_offset). None for anything else.
    fn resource_key(&mut self, v: &Value) -> Option<Key> {
        let resource_id = |v: &Value| v.as_object().filter(|o| o.class.flags & p8_runtime::object::CLASS_RESOURCE != 0).map(|o| o.id);
        let id = match v.as_reference() {
            Some(cell) => resource_id(&cell.borrow())?,
            None => resource_id(v)?,
        };
        self.warn(format!("Resource ID#{id} used as offset, casting to integer ({id})"));
        Some(Key::Int(i64::from(id)))
    }

    /// The array key `v` names, reporting what PHP reports: a float that
    /// is not an int loses precision (deprecated), a resource is its
    /// number (warned), an array or object is a TypeError ("Cannot `what`
    /// offset of type …": access, unset).
    #[inline]
    fn key_of(&mut self, v: &Value, what: &str) -> Result<Key, Throw> {
        match Key::from_value(v) {
            Ok((k, None)) => Ok(k),
            Ok((k, Some(KeyIssue::LosesPrecision(f)))) => {
                ops::float_to_int_diags(f, &mut self.diags);
                self.report_diags();
                Ok(k)
            }
            // Null as an offset: deprecated (8.5), but not when unsetting.
            Ok((k, Some(KeyIssue::NullOffset))) => {
                if what != "unset" {
                    self.diags.push(Diag::Deprecated(p8_runtime::array::NULL_OFFSET_DEPRECATION.into()));
                    self.report_diags();
                }
                Ok(k)
            }
            // from_value reports illegal keys as errors only
            Ok((k, Some(KeyIssue::Illegal(_)))) => Ok(k),
            Err(KeyIssue::NullOffset) => Ok(Key::Str(PhpStr::empty())),
            Err(KeyIssue::Illegal(t)) => {
                // (Only here, off the common path: a resource is a key.)
                if let Some(k) = self.resource_key(v) {
                    return Ok(k);
                }
                // An object is named by its class (zend_illegal_container_offset).
                let t = match v.deref().as_object() {
                    Some(o) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                    None => t.to_string(),
                };
                Err(self.throw("TypeError", format!("Cannot {what} offset of type {t} on array")))
            }
            Err(KeyIssue::LosesPrecision(f)) => Ok(Key::Int(f as i64)),
        }
    }

    /// An array key as isset and empty take it (zend_execute.c
    /// zend_isset_dim_slow / zend_fetch_dimension_address_inner with
    /// BP_VAR_IS): a float that is not an int deprecated, a resource its
    /// number (warned), an array or object a TypeError, "in isset or empty"
    /// at the last dimension (`last`), "on array" on the way to it.
    #[cold]
    fn isset_key(&mut self, v: &Value, last: bool) -> Result<Key, Throw> {
        match Key::from_value(v) {
            Ok((k, Some(KeyIssue::LosesPrecision(f)))) => {
                ops::float_to_int_diags(f, &mut self.diags);
                self.report_diags();
                Ok(k)
            }
            Ok((k, Some(KeyIssue::NullOffset))) => {
                self.diags.push(Diag::Deprecated(p8_runtime::array::NULL_OFFSET_DEPRECATION.into()));
                self.report_diags();
                Ok(k)
            }
            Ok((k, _)) => Ok(k),
            Err(KeyIssue::LosesPrecision(f)) => Ok(Key::Int(f as i64)),
            Err(KeyIssue::NullOffset) => Ok(Key::Str(PhpStr::empty())),
            Err(KeyIssue::Illegal(t)) => {
                if let Some(k) = self.resource_key(v) {
                    return Ok(k);
                }
                let shown = match v.deref().as_object() {
                    Some(o) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                    None => t.to_string(),
                };
                let place = if last { "in isset or empty" } else { "on array" };
                Err(self.throw("TypeError", format!("Cannot access offset of type {shown} {place}")))
            }
        }
    }

    /// A string's offset as isset and empty read it: at the last
    /// dimension an int, a scalar or an integer string (a float that is
    /// not an int deprecated), anything else not set; on the way to it,
    /// as `??` reads it (an array, object or resource key is a TypeError).
    fn isset_str_offset(&mut self, s: &PhpStr, k: &Value, last: bool) -> Result<Option<Value>, Throw> {
        let k = k.deref();
        if k.is_array() || k.is_object() {
            if last {
                return Ok(None);
            }
            let t = match k.as_object() {
                Some(o) if o.class.flags & p8_runtime::object::CLASS_RESOURCE != 0 => "resource".to_string(),
                Some(o) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                None => k.type_name().to_string(),
            };
            return Err(self.throw("TypeError", format!("Cannot access offset of type {t} on string")));
        }
        if let Some(f) = k.as_float()
            && last
            && !ops::float_is_int(f)
        {
            ops::float_to_int_diags(f, &mut self.diags);
            self.report_diags();
        }
        Ok(isset_string_offset(s, &k))
    }

    /// `unset($c[k1]..[kn])` on `target`, in place (ZEND_UNSET_DIM, the
    /// containers on the way fetched as FETCH_DIM_UNSET fetches them).
    /// An element that is a reference is followed (`$r = &$a['k'];
    /// unset($a['k'][0])` changes what both see). A missing element or a
    /// null on the way ends it quietly, false with a deprecation; a key
    /// is converted only when its level is reached. (An ArrayAccess object
    /// on the path is the caller's: offsetUnset.)
    fn unset_path(&mut self, target: &mut Value, keys: &[Value]) -> Result<(), Throw> {
        let Some((first, rest)) = keys.split_first() else { return Ok(()) };
        let last = rest.is_empty();
        if let Some(cell) = target.as_reference() {
            // Out of the reference while it changes (no borrow held
            // across the warnings a key may raise).
            let mut held = std::mem::take(&mut *cell.borrow_mut());
            let r = self.unset_path(&mut held, keys);
            *cell.borrow_mut() = held;
            return r;
        }
        match target.kind() {
            Kind::Array => {
                // An int or string key is the common case: taken as it is,
                // nothing to report; anything else goes through key_of.
                let k = if let Some(i) = first.as_int() {
                    Key::Int(i)
                } else if let Some(s) = first.cloned_str() {
                    Key::from_string(s)
                } else {
                    self.key_of(first, if last { "unset" } else { "access" })?
                };
                // A shared array is copied only when the element is there
                // (`array_mut` separates it).
                let present = target.as_array().expect("an array").contains_key(&k);
                if last {
                    if present {
                        target.array_mut().expect("an array").remove(&k);
                    }
                    return Ok(());
                }
                if !present {
                    return Ok(());
                }
                match target.array_mut().expect("an array").get_mut(&k) {
                    Some(child) => self.unset_path(child, rest),
                    None => Ok(()),
                }
            }
            Kind::Undef | Kind::Null => Ok(()),
            Kind::Bool if target.as_bool() == Some(false) => {
                self.diags.push(Diag::Deprecated("Automatic conversion of false to array is deprecated".into()));
                self.report_diags();
                Ok(())
            }
            Kind::Str if last => Err(self.throw("Error", "Cannot unset string offsets")),
            Kind::Str => Err(self.throw("Error", "Cannot use string offset as an array")),
            // Reached through path_has_object's ArrayAccess path instead.
            Kind::Object => Ok(()),
            Kind::Bool | Kind::Int | Kind::Float | Kind::Ref => Err(self.throw("Error", "Cannot unset offset in a non-array variable")),
        }
    }

    fn fetch_dim(&mut self, container: &Value, w: &[Value], clo: &Closure, key: Reg, quiet: bool) -> Result<Value, Throw> {
        if let Some(a) = container.as_array() {
            let k = self.key(w, clo, key)?;
            return match a.get(&k) {
                Some(v) => Ok(v.deref()),
                None => {
                    if !quiet {
                        let shown = match &k {
                            Key::Int(i) => i.to_string(),
                            Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
                        };
                        self.warn(format!("Undefined array key {shown}"));
                    }
                    Ok(Value::NULL)
                }
            };
        }
        if let Some(s) = container.as_str() {
            // zend_fetch_string_offset: an integer, a string that starts
            // with one (with junk after it, a warning), or a scalar cast.
            let k = self.operand(w, clo, key).deref();
            let i = match k.kind() {
                Kind::Int => k.as_int().expect("an int"),
                Kind::Str => {
                    let t = k.as_str().expect("a string");
                    match p8_runtime::numeric::leading_numeric(t.as_bytes()) {
                        Some((p8_runtime::numeric::Number::Int(i), end)) => {
                            if t.as_bytes()[end..].iter().any(|c| !b" \t\n\r\x0b\x0c".contains(c)) {
                                self.warn(format!("Illegal string offset \"{}\"", String::from_utf8_lossy(t.as_bytes())));
                            }
                            i
                        }
                        _ if quiet => return Ok(Value::NULL),
                        _ => return Err(self.throw("TypeError", "Cannot access offset of type string on string")),
                    }
                }
                Kind::Float | Kind::Bool | Kind::Null | Kind::Undef => {
                    if !quiet {
                        self.warn("String offset cast occurred");
                    }
                    k.to_int()
                }
                Kind::Array | Kind::Object | Kind::Ref => {
                    let t = match k.as_object() {
                        Some(o) if o.class.flags & p8_runtime::object::CLASS_RESOURCE != 0 => "resource".to_string(),
                        Some(o) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                        None => k.type_name().to_string(),
                    };
                    return Err(self.throw("TypeError", format!("Cannot access offset of type {t} on string")));
                }
            };
            let len = s.len() as i64;
            let at = if i < 0 { len + i } else { i };
            if at < 0 || at >= len {
                if !quiet {
                    self.warn(format!("Uninitialized string offset {i}"));
                }
                return Ok(if quiet { Value::NULL } else { Value::str(PhpStr::empty()) });
            }
            return Ok(Value::str(PhpStr::byte(s.as_bytes()[at as usize])));
        }
        if container.is_null_or_undef() {
            if !quiet {
                self.warn("Trying to access array offset on null");
            }
            return Ok(Value::NULL);
        }
        if !quiet {
            self.warn(format!("Trying to access array offset on {}", container.type_name()));
        }
        Ok(Value::NULL)
    }

    /// var[k1]..[kn] = val (or op= val), copy-on-write down the path.
    // The operands of AssignDim/AssignDimOp, decoded once by the caller.
    #[allow(clippy::too_many_arguments)]
    fn assign_dim(
        &mut self,
        w: &mut [Value],
        clo: &Closure,
        var: Reg,
        keys: Reg,
        n: u8,
        append: bool,
        val: Value,
        op: Option<BinOp>,
    ) -> Result<Written, Throw> {
        let slot = var as usize;
        // `$s[k] = v` on a string: the offset as a string reads it (casts
        // warn as such), before any array key conversion.
        let is_str = match w[slot].as_reference() {
            Some(cell) => cell.borrow().is_str(),
            None => w[slot].is_str(),
        };
        if is_str {
            if append {
                return Err(self.throw("Error", "[] operator not supported for strings"));
            }
            let raw = self.operand(w, clo, keys).deref();
            let offset = self.string_offset(&raw)?;
            if n > 1 {
                return Err(self.throw("Error", "Cannot use string offset as an array"));
            }
            if op.is_some() {
                return Err(self.throw("Error", "Cannot use assign-op operators with string offsets"));
            }
            let s = match w[slot].as_reference() {
                Some(cell) => std::mem::take(&mut *cell.borrow_mut()),
                None => std::mem::take(&mut w[slot]),
            };
            let mut text = s.into_str().unwrap_or_else(|_| unreachable!("checked a string"));
            let r = self.assign_string_offset(&mut text, offset, val);
            let s = Value::str(text);
            match w[slot].as_reference() {
                Some(cell) => *cell.borrow_mut() = s,
                None => w[slot] = s,
            }
            return r.map(Written::Done);
        }
        self.write_dim(w, clo, slot, keys, n, append, val, op)
    }

    /// A string offset written (zend_check_dimension_interned's rules, as
    /// for a read): an int, a string that is one (junk after it warns),
    /// a cast scalar (warns); anything else is a TypeError.
    fn string_offset(&mut self, k: &Value) -> Result<i64, Throw> {
        match k.kind() {
            Kind::Int => Ok(k.as_int().expect("an int")),
            Kind::Str => {
                let t = k.as_str().expect("a string");
                match p8_runtime::numeric::leading_numeric(t.as_bytes()) {
                    Some((p8_runtime::numeric::Number::Int(i), end)) => {
                        if t.as_bytes()[end..].iter().any(|c| !b" \t\n\r\x0b\x0c".contains(c)) {
                            self.warn(format!("Illegal string offset \"{}\"", String::from_utf8_lossy(t.as_bytes())));
                        }
                        Ok(i)
                    }
                    _ => Err(self.throw("TypeError", "Cannot access offset of type string on string")),
                }
            }
            Kind::Float | Kind::Bool | Kind::Null | Kind::Undef => {
                self.warn("String offset cast occurred");
                Ok(k.to_int())
            }
            Kind::Object => {
                let o = k.as_object().expect("an object");
                Err(self.throw("TypeError", format!("Cannot access offset of type {} on string", String::from_utf8_lossy(o.class.name.as_bytes()))))
            }
            Kind::Array | Kind::Ref => Err(self.throw("TypeError", format!("Cannot access offset of type {} on string", k.type_name()))),
        }
    }

    /// `$s[offset] = val` (zend_assign_to_string_offset): the value's first
    /// byte at the offset (from the end when negative), the string padded
    /// with spaces to reach it; the byte assigned is the result.
    fn assign_string_offset(&mut self, s: &mut PhpStr, offset: i64, val: Value) -> Result<Value, Throw> {
        let len = s.len() as i64;
        let at = if offset < 0 { offset + len } else { offset };
        if at < 0 {
            self.warn(format!("Illegal string offset {offset}"));
            return Ok(Value::NULL);
        }
        let v = val.deref().to_php_string().unwrap_or_else(PhpStr::empty);
        let Some(&byte) = v.as_bytes().first() else {
            return Err(self.throw("Error", "Cannot assign an empty string to a string offset"));
        };
        if v.len() > 1 {
            self.warn("Only the first byte will be assigned to the string offset");
        }
        let mut bytes = s.as_bytes().to_vec();
        if at as usize >= bytes.len() {
            bytes.resize(at as usize + 1, b' ');
        }
        bytes[at as usize] = byte;
        *s = PhpStr::new(bytes);
        Ok(Value::str(PhpStr::byte(byte)))
    }

    /// A dimension write's root, through `n` key registers at `keys`:
    /// the value written (a string offset's one byte), or the object
    /// (ArrayAccess) the path reached, at that level, with the value.
    /// An undefined key register warns first, as reading it does.
    #[allow(clippy::too_many_arguments)] // AssignDim/AssignDimOp's operands, decoded once by the caller
    fn write_dim(&mut self, w: &mut [Value], clo: &Closure, slot: usize, keys: Reg, n: u8, append: bool, val: Value, op: Option<BinOp>) -> Result<Written, Throw> {
        // The one key may be a constant operand (`$a['k'] = v`), read in
        // place; a path of keys is a block of registers.
        let range = match const_index(keys) {
            Some(_) => 0..0,
            None => keys as usize..keys as usize + n as usize,
        };
        for r in range.clone() {
            if w[r].is_undef() {
                self.undefined(&clo.code, r as Reg);
            }
        }
        // Taken out, written, put back (nothing runs PHP code meanwhile).
        let mut root = match w[slot].as_reference() {
            Some(cell) => std::mem::take(&mut *cell.borrow_mut()),
            None => std::mem::take(&mut w[slot]),
        };
        let path: &[Value] = match const_index(keys) {
            Some(k) => std::slice::from_ref(&clo.consts[k]),
            None => &w[range],
        };
        let result = self.update_path(&mut root, path, 0, append, val, op);
        match w[slot].as_reference() {
            Some(cell) => *cell.borrow_mut() = root,
            None => w[slot] = root,
        }
        result
    }

    /// `$a[] = val` into `arr`: the value (the assignment's result), or
    /// PHP's error when the next key is taken.
    #[inline]
    fn append(&mut self, arr: &mut PhpArray, val: Value) -> Result<Value, Throw> {
        if arr.push(val.clone()).is_err() {
            return Err(self.throw("Error", "Cannot add element to the array as the next element is already occupied"));
        }
        Ok(val)
    }

    /// A property dimension write's end: its result, or (the path reached
    /// an object) the ArrayAccess write of the keys from there.
    fn finish_prop_dim(&mut self, regs: &mut Vec<Value>, written: Result<Written, Throw>, keys: &[Value], append: bool, op: Option<BinOp>) -> Result<Value, Throw> {
        match written? {
            Written::Done(r) => Ok(r),
            Written::Object(o, at, v) => {
                let dim_op = match op {
                    Some(op) => arrayaccess::DimOp::Op(op, v),
                    None => arrayaccess::DimOp::Set(v),
                };
                self.aa_write(regs, &Value::object(o), &keys[at..], append, dim_op)
            }
        }
    }

    /// "Undefined array key …" for reading `key` that is not there.
    #[cold]
    fn undefined_key(&mut self, key: &Key) {
        let shown = match key {
            Key::Int(i) => i.to_string(),
            Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
        };
        self.warn(format!("Undefined array key {shown}"));
    }

    /// `target[k1]..[kn] = val` (or `op=`), one level at a time as PHP's
    /// zend_fetch_dimension_address does: each container separated
    /// (copy-on-write) and written in place, its key converted only now,
    /// for the kind of container it is. `raw` are the keys as written (a
    /// reference or an undefined register is read through). A container
    /// that is an object stops the walk: Written::Object hands it back,
    /// with the keys still to go and the value, for its ArrayAccess path
    /// (which runs PHP code, so the caller puts the variable back first).
    fn update_path(&mut self, target: &mut Value, raw: &[Value], depth: usize, append: bool, val: Value, op: Option<BinOp>) -> Result<Written, Throw> {
        if target.is_null_or_undef() {
            *target = Value::array(PhpArray::new());
        }
        if let Some(o) = target.as_object() {
            return Ok(Written::Object(Rc::clone(o), depth, val));
        }
        if let Some(cell) = target.as_reference() {
            if let Some(o) = cell.borrow().as_object() {
                return Ok(Written::Object(Rc::clone(o), depth, val));
            }
            let mut inner = std::mem::take(&mut *cell.borrow_mut());
            let r = self.update_path(&mut inner, raw, depth, append, val, op);
            *cell.borrow_mut() = inner;
            return r;
        }
        if target.as_bool() == Some(false) {
            self.diags.push(Diag::Deprecated("Automatic conversion of false to array is deprecated".into()));
            self.report_diags();
            *target = Value::array(PhpArray::new());
            return self.update_path(target, raw, depth, append, val, op);
        }
        if target.is_str() {
            // A string inside an array or a property: its offset from
            // the key as written (casts warn as such).
            if append {
                return Err(self.throw("Error", "[] operator not supported for strings"));
            }
            let Some((key, rest)) = raw.split_first() else { return Err(self.throw("Error", "[] operator not supported for strings")) };
            let offset = self.string_offset(&key.deref())?;
            if !rest.is_empty() {
                return Err(self.throw("Error", "Cannot use string offset as an array"));
            }
            if op.is_some() {
                return Err(self.throw("Error", "Cannot use assign-op operators with string offsets"));
            }
            // The string out of its slot while it is written, then back.
            let mut text = std::mem::take(target).into_str().unwrap_or_else(|_| unreachable!("a string"));
            let r = self.assign_string_offset(&mut text, offset, val);
            *target = Value::str(text);
            return r.map(Written::Done);
        }
        let Some(arr) = target.array_mut() else { return Err(self.throw("Error", "Cannot use a scalar value as an array")) };
        let Some((raw_key, rest)) = raw.split_first() else {
            return self.append(arr, val).map(Written::Done);
        };
        // This level is an array: its key, converted as an array's.
        let key = &self.key_of(raw_key, "access")?;
        Ok(Written::Done(match (rest.is_empty() && !append, op) {
            (true, None) => {
                // One lookup: the element written in place, else added.
                match arr.get_mut(key) {
                    Some(slot) => match slot.as_reference() {
                        Some(cell) => *cell.borrow_mut() = val.clone(),
                        None => *slot = val.clone(),
                    },
                    None => arr.insert(key.clone(), val.clone()),
                }
                val
            }
            (true, Some(op)) => {
                let old = match arr.get(key) {
                    Some(v) => v.deref(),
                    None => {
                        self.undefined_key(key);
                        Value::NULL
                    }
                };
                let r = ops::binary(op, &old, &val, &mut self.diags);
                self.report_diags();
                let new = r.map_err(|e| self.op_error(e))?;
                arr.insert(key.clone(), new.clone());
                new
            }
            // A level on the way: its element (made null if missing),
            // written into where it is (ZEND_FETCH_DIM_W), one lookup. A
            // compound assignment reads it first (ZEND_FETCH_DIM_RW): a
            // missing one warns.
            (false, op) => {
                if op.is_some() && !arr.contains_key(key) {
                    self.undefined_key(key);
                }
                return self.update_path(arr.slot_mut(key), rest, depth + 1, append, val, op);
            }
        }))
    }

    /// Arguments past the parameters, for func_get_args().
    fn extra_args(&self) -> &[Value] {
        self.frames.last().map_or(&[], |f| f.args())
    }
}


impl Context<'_> {
    /// The constant `name`: the script's own, else an extension's.
    /// The name of a constant an extension made deprecated while running.
    fn runtime_deprecated(&self, name: &[u8], fallback: Option<&[u8]>) -> Option<String> {
        if self.deprecated_made.is_empty() {
            return None;
        }
        [Some(name), fallback]
            .into_iter()
            .flatten()
            .map(|n| n.strip_prefix(b"\\").unwrap_or(n))
            .find(|n| self.isolate.names.find_const(n).is_some_and(|key| self.deprecated_made.contains(&key)))
            .map(|n| String::from_utf8_lossy(n).into_owned())
    }

    /// A constant being defined: its key's symbol, and its name's as
    /// written (they differ when its namespace has capitals).
    fn constant_syms(&self, name: &[u8]) -> (Sym, Sym) {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        let symbols = &self.isolate.names;
        let key = const_key(name);
        let spelled = symbols.intern(name);
        let key = if key.as_ref() == name { spelled } else { symbols.intern(&key) };
        (key, spelled)
    }

    fn constant_value(&self, name: &[u8]) -> Option<Value> {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        // true, false and null in any case (they are keywords).
        match name {
            n if n.eq_ignore_ascii_case(b"true") => return Some(Value::TRUE),
            n if n.eq_ignore_ascii_case(b"false") => return Some(Value::FALSE),
            n if n.eq_ignore_ascii_case(b"null") => return Some(Value::NULL),
            _ => {}
        }
        self.constant_by_key(self.isolate.names.find_const(name)?)
    }

    /// The constant whose key (const_key) is `key`.
    fn constant_by_key(&self, key: Sym) -> Option<Value> {
        match self.constants.get(&key) {
            Some((_, v)) => Some(v.clone()),
            None => self.isolate.registry.constant_sym(key).map(|c| c.to_value()),
        }
    }
}

impl Ctx for Context<'_> {
    fn echo(&mut self, bytes: &[u8]) {
        // Output from a builtin (var_dump, printf) sends the headers too:
        // the callback runs first; what it throws ends the builtin's call.
        if !bytes.is_empty() && self.ext_state.contains_key("header_callback") && !self.isolate.regs.is_empty() && self.header_throw.is_none() {
            let mut regs = std::mem::take(&mut self.isolate.regs);
            let r = self.run_header_callback(&mut regs, false);
            self.isolate.regs = regs;
            self.header_throw = r.err();
        }
        self.write_out(bytes);
    }

    fn response(&mut self) -> &mut p8_runtime::ext::Response {
        &mut self.out.response
    }

    fn flush_output(&mut self) {
        self.out.send_head();
        self.out.host.flush();
        self.note_output_start();
        // The head went out without output: headers_sent() is true, with
        // nothing to say about where output started.
        if self.out.head_sent && self.output_start.is_none() {
            self.output_start = Some((Rc::from(""), 0));
        }
    }

    fn ob(&mut self, op: p8_runtime::ext::Ob) -> Result<Value, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = self.ob_with(&mut regs, op);
        self.isolate.regs = regs;
        r
    }

    fn warn(&mut self, message: &str) {
        Context::warn(self, message);
    }

    fn deprecated(&mut self, message: &str) {
        self.diags.push(Diag::Deprecated(message.to_string()));
        self.report_diags();
    }

    fn notice(&mut self, message: &str) {
        Context::notice(self, message);
    }

    fn report(&mut self, level: i64, message: &str) {
        self.report_diags();
        self.show(level, message);
    }

    fn user_handles(&self, level: i64) -> bool {
        self.user_handles_level(level)
    }

    fn fatal(&self, message: String) -> Throw {
        let (file, line) = self.location();
        Throw::Fatal { message, file, line }
    }

    fn ini_get(&self, name: &[u8]) -> Option<Value> {
        let default = self.isolate.registry.ini_default(name)?;
        Some(match self.ini.get(name) {
            Some(v) => v.clone(),
            None => default.map_or(Value::NULL, |d| Value::str(self.isolate.ini_text(d))),
        })
    }

    fn ini_master(&self, name: &[u8]) -> Option<Value> {
        let default = self.isolate.registry.ini_default(name)?;
        Some(match self.ini_master.get(name) {
            Some(v) => v.clone(),
            None => default.map_or(Value::NULL, Value::str),
        })
    }

    fn ini_set(&mut self, name: &[u8], value: Value) -> Option<Value> {
        let old = self.ini_get(name)?;
        let text = value.to_php_string().unwrap_or_else(PhpStr::empty);
        if let Some(check) = self.isolate.registry.ini_check(name)
            && !check(self, name, &text)
        {
            return None;
        }
        match name {
            // An expression of E_* constants (`E_ALL & ~E_NOTICE`).
            b"error_reporting" => self.error_level = self.ini_bits(&String::from_utf8_lossy(text.as_bytes())),
            b"display_errors" => {
                let t = text.as_bytes().to_ascii_lowercase();
                self.display_errors = !matches!(t.as_slice(), b"" | b"0" | b"off" | b"no" | b"false" | b"stderr");
            }
            _ => {}
        }
        self.ini.insert(name.to_vec(), Value::str(text));
        Some(old)
    }

    fn error_level(&mut self, new: Option<i64>) -> i64 {
        let old = self.error_level;
        if let Some(level) = new {
            self.error_level = level;
            self.ini.insert(b"error_reporting".to_vec(), Value::str(level.to_string()));
        }
        old
    }

    fn constant(&self, name: &[u8]) -> Option<Value> {
        self.constant_value(name)
    }

    fn constant_deprecation(&self, name: &[u8]) -> Option<&'static str> {
        let registry = &self.isolate.registry;
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        registry.symbols().find_const(name).and_then(|sym| registry.constant_deprecation(sym))
    }

    fn set_constant(&mut self, name: &[u8], value: Value) {
        let (key, spelled) = self.constant_syms(name);
        // One made here is deprecated (CONST_DEPRECATED); the script's own
        // define() of the name is replaced, as it was made.
        if !self.constants.contains_key(&key) {
            self.deprecated_made.insert(key);
        }
        self.constants.insert(key, (spelled, value));
    }

    fn define(&mut self, name: &[u8], value: Value) -> bool {
        if self.constant_value(name).is_some() {
            return false;
        }
        let (key, spelled) = self.constant_syms(name);
        self.constants.insert(key, (spelled, value));
        true
    }

    fn builtin_name(&self) -> &'static str {
        self.running_builtin.map_or("", |i| self.isolate.registry.function(i).name)
    }

    fn modules(&self) -> &[&'static str] {
        self.isolate.registry.modules()
    }

    fn defined_constants(&self) -> Vec<(String, PhpStr, Value)> {
        // PHP's module names: its core is "Core", the rest as their extensions are named.
        let module = |m: &str| if m.eq_ignore_ascii_case("core") { "Core".to_string() } else { m.to_string() };
        let mut out: Vec<(String, PhpStr, Value)> =
            self.isolate.registry.constant_list().map(|(m, name, c)| (module(m), PhpStr::from(name), c.to_value())).collect();
        // The CLI's stream constants are made at startup like the script's,
        // but PHP counts them as Core's.
        let streams = |n: &[u8]| matches!(n, b"STDIN" | b"STDOUT" | b"STDERR");
        let symbols = self.isolate.registry.symbols();
        out.extend(self.constants.values().map(|&(spelled, ref v)| {
            let name = symbols.name(spelled);
            let m = if streams(&name) { "Core" } else { "user" };
            (m.to_string(), PhpStr::from(&*name), v.clone())
        }));
        // Core's first, as PHP registers it before every extension.
        out.sort_by_key(|(m, _, _)| m != "Core");
        out
    }

    fn defined_functions(&self) -> (Vec<&'static str>, Vec<Vec<u8>>) {
        let mut user: Vec<(u32, Sym)> = self.functions.iter().map(|(&k, &i)| (i, k)).collect();
        user.sort();
        let builtins = self.isolate.registry.function_names().filter(|n| !self.disabled.contains(n.as_bytes())).collect();
        let symbols = self.isolate.registry.symbols();
        (builtins, user.into_iter().map(|(_, k)| symbols.name(k).to_vec()).collect())
    }

    fn resolve_include(&self, name: &str) -> Option<std::path::PathBuf> {
        self.resolve_include_path(name)
    }

    fn function_exists(&self, name: &[u8]) -> bool {
        Context::function_exists(self, name)
    }

    fn extra_args(&self) -> &[Value] {
        Context::extra_args(self)
    }

    fn throw(&self, class: &'static str, message: String) -> Throw {
        Context::throw(self, class, message)
    }

    fn set_fault(&mut self, t: Throw) {
        self.fault = Some(t);
    }

    fn class(&mut self, name: &[u8]) -> Option<Rc<Class>> {
        let key = self.sym_of(name)?;
        match self.classes.get(&key) {
            Some(c) => Some(Rc::clone(c)),
            None => self.builtin_on_demand(key),
        }
    }

    fn scope(&self) -> Option<Rc<Class>> {
        Context::scope(self).0
    }

    fn call(&mut self, callable: &Value, args: &mut [Value]) -> Result<Value, Throw> {
        if self.isolate.regs.is_empty() {
            return Err(Ctx::fatal(self, "a builtin that takes no callable called back into PHP".into()));
        }
        self.call_from_builtin(callable, args)
    }

    fn is_callable(&mut self, v: &Value) -> bool {
        self.callable(v)
    }

    fn why_not_callable(&mut self, v: &Value) -> Option<String> {
        let saved = std::mem::take(&mut self.failed_call);
        let r = Context::why_not_callable(self, v, false);
        self.failed_call = saved;
        match r {
            Ok(why) => why,
            // A class an autoloader could not declare (not lent: asked
            // again later).
            Err(Throw::Autoload(name)) => {
                self.autoload_tried.remove(&name.as_bytes().to_ascii_lowercase());
                Some(format!("class \"{}\" not found", String::from_utf8_lossy(name.as_bytes())))
            }
            Err(_) => Some("no array or string given".into()),
        }
    }

    fn output_started(&self) -> Option<(Rc<str>, u32)> {
        self.output_start.clone()
    }

    fn last_error(&mut self, clear: bool) -> Option<(i64, String, String, u32)> {
        if clear { self.last_error.take() } else { self.last_error.clone() }
    }

    fn bind_closure(&mut self, closure: &Rc<Object>, new_this: Value, scope: Option<Value>) -> Result<Value, Throw> {
        self.bind(closure, new_this, scope)
    }

    /// Closure::getCurrent(): the Closure the calling frame was called as,
    /// one written as a closure (not one made from a named function or
    /// method: ZEND_ACC_FAKE_CLOSURE). In a generator's code, the Closure
    /// the generator was made by (PHP: the generator frame's function).
    fn current_closure(&mut self) -> Result<Value, Throw> {
        let closure = self.frames.last().and_then(|f| match &f.owner {
            Some(g) if f.func.code.is_generator => generators::closure_of(g),
            owner => owner.clone(),
        });
        match closure {
            Some(o) if callable::closure_data(&o).is_some_and(|d| !d.fake) => Ok(Value::object(o)),
            _ => Err(self.throw("Error", "Current function is not a closure")),
        }
    }

    /// Closure::fromCallable(): checked as zend_create_closure_from_callable
    /// does (zend_is_callable_ex) first. (`f(...)` and reflection make
    /// their closures without that check.)
    fn closure_from(&mut self, callable: &Value) -> Result<Value, Throw> {
        if !callable.deref().as_object().is_some_and(|o| callable::closure_data(o).is_some())
            && let Some(why) = self.why_not_callable(callable, true)?
        {
            return Err(self.throw("TypeError", format!("Failed to create closure from callable: {why}")));
        }
        self.closure_from_callable(callable)
    }

    fn prop_value(&mut self, o: &Rc<Object>, slot: usize) -> Result<Option<Value>, Throw> {
        let p = &o.class.properties[slot];
        let raw = || Some(o.slots()[slot].peek(Value::deref)).filter(|v| !v.is_undef());
        if p.hooks == 0 || self.isolate.regs.is_empty() {
            return Ok(raw());
        }
        let name = Value::str(p.name.clone());
        match self.prop_hook_seen(&Value::object(Rc::clone(o)), &name, HOOK_GET, false) {
            Some(Ok(m)) => {
                let mut regs = std::mem::take(&mut self.isolate.regs);
                let r = self.run_nested(&mut regs, m, &mut []);
                self.isolate.regs = regs;
                r.map(Some)
            }
            // Virtual and write-only: nothing to read.
            Some(Err(_)) => Ok(None),
            None => Ok(raw()),
        }
    }

    fn class_constant(&mut self, class: &Rc<Class>, name: &[u8]) -> Result<Option<Value>, Throw> {
        let Some(c) = class.constants.get(name) else { return Ok(None) };
        // An enum case is its object (constant(), reflection), not its value.
        if c.is_case {
            return Ok(Some(Value::object(self.enum_case(class, name))));
        }
        let Some(init) = c.init.clone() else { return Ok(Some(c.value.clone())) };
        let key = (c.declaring_name.as_bytes().to_ascii_lowercase(), name.to_vec());
        if !self.const_cache.contains_key(&key) {
            let scope = self.class_named(&key.0).cloned().unwrap_or_else(|| Rc::clone(class));
            let mut regs = std::mem::take(&mut self.isolate.regs);
            if regs.is_empty() {
                self.isolate.regs = regs;
                return Err(Ctx::fatal(self, "a builtin that takes no callable computed a class constant".into()));
            }
            let r = self.eval_constant(&mut regs, &init, scope, key.clone());
            self.isolate.regs = regs;
            r?;
        }
        Ok(self.const_cache.get(&key).cloned())
    }

    fn function_info(&mut self, callee: &p8_runtime::reflect::Callee) -> Option<p8_runtime::reflect::FnInfo> {
        self.info_of(callee)
    }

    fn static_variables(&mut self, callee: &p8_runtime::reflect::Callee) -> PhpArray {
        self.static_variables_of(callee)
    }

    fn closure_used_variables(&mut self, callee: &p8_runtime::reflect::Callee) -> PhpArray {
        self.used_variables_of(callee)
    }

    fn invoke(&mut self, callee: &p8_runtime::reflect::Callee, this: Option<Rc<Object>>, args: &mut [Value]) -> Result<Value, Throw> {
        self.invoke_callee(callee, this, args)
    }

    fn new_instance(&mut self, class: &Rc<Class>, args: &mut [Value], construct: bool) -> Result<Rc<Object>, Throw> {
        self.new_instance_of(class, args, construct)
    }

    fn new_instance_named(&mut self, class: &Rc<Class>, args: &PhpArray) -> Result<Rc<Object>, Throw> {
        self.new_instance_placed(class, args)
    }

    fn reflect_meta(&mut self, subject: &p8_runtime::reflect::Subject) -> p8_runtime::reflect::Meta {
        self.meta_facts(subject)
    }

    fn reflect_attr_args(&mut self, subject: &p8_runtime::reflect::Subject, index: usize) -> Result<PhpArray, Throw> {
        self.attr_arguments(subject, index)
    }

    fn class_defaults(&mut self, class: &Rc<Class>) -> Result<Vec<Value>, Throw> {
        self.class_defaults_of(class)
    }

    fn closure_of(&mut self, callee: &p8_runtime::reflect::Callee, this: Option<Rc<Object>>) -> Result<Value, Throw> {
        self.closure_of_callee(callee, this)
    }

    fn class_load(&mut self, name: &[u8]) -> Result<Option<Rc<Class>>, Throw> {
        self.class_if_loaded(name)
    }

    fn called_class(&self) -> Option<Rc<Class>> {
        Context::scope(self).1
    }

    fn new_object(&mut self, class: &Rc<Class>) -> Rc<Object> {
        Object::new(Rc::clone(class), &self.ids)
    }

    fn call_args(&self) -> Vec<Value> {
        let Some(frame) = self.frames.last() else { return Vec::new() };
        let regs = &self.isolate.regs;
        let base = frame.base as usize;
        let params = &frame.func.code.params;
        // A variadic parameter holds the arguments past the others: they
        // count one by one (its named ones are not func_get_args()'s).
        let variadic = params.last().is_some_and(|p| p.variadic);
        let fixed = params.len() - usize::from(variadic);
        let n = (frame.argc as usize).min(fixed);
        let mut out: Vec<Value> = (0..n).map(|i| regs.get(base + i).map(Value::deref).unwrap_or(Value::NULL)).collect();
        if variadic {
            if let Some(rest) = regs.get(base + fixed).map(Value::deref).as_ref().and_then(Value::as_array) {
                out.extend(rest.iter().filter(|(k, _)| matches!(k, Key::Int(_))).map(|(_, v)| v.deref()));
            }
        } else {
            out.extend(frame.args().iter().map(Value::deref));
        }
        out
    }

    fn local(&self, name: &[u8]) -> Option<Value> {
        let frame = self.frames.last()?;
        let regs = &self.isolate.regs;
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => Some(regs.get(frame.base as usize + r)?.deref()).filter(|v| !v.is_undef()),
            None if frame.global => self.global_value(regs, name),
            None => frame.vars()?.get(name).map(|c| c.borrow().clone()),
        }
    }

    fn set_local(&mut self, name: &[u8], value: Value) -> bool {
        let Some(frame) = self.frames.last() else { return false };
        let base = frame.base as usize;
        match frame.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => {
                if let Some(slot) = self.isolate.regs.get_mut(base + r) {
                    assign(std::slice::from_mut(slot), 0, value);
                }
                true
            }
            None if frame.global => {
                // (The main frame's window starts at register 0.)
                let mut regs = std::mem::take(&mut self.isolate.regs);
                self.global_cell(&mut regs, name).replace(value);
                self.isolate.regs = regs;
                true
            }
            None => {
                let frame = self.frames.last_mut().expect("the frame");
                frame.vars_mut().cell(name).replace(value);
                true
            }
        }
    }

    fn locals(&self) -> Vec<(PhpStr, Value)> {
        let Some(frame) = self.frames.last() else { return Vec::new() };
        // At global scope (a file included there too): the main frame's
        // variables and the other globals.
        let frame = if frame.global { self.frames.first().expect("the main frame") } else { frame };
        let regs = &self.isolate.regs;
        let base = frame.base as usize;
        let mut out: Vec<(PhpStr, Value)> = frame
            .func
            .code
            .cv_names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| match regs.get(base + i).map(Value::deref) {
                Some(v) if !v.is_undef() => Some((PhpStr::from(n.as_bytes()), v)),
                _ => None,
            })
            .collect();
        if base == 0 {
            // The ones the request starts with come first, in the order
            // PHP makes them; then the others.
            let first: Vec<(PhpStr, Value)> = REQUEST_GLOBALS
                .iter()
                .filter_map(|n| self.globals.get(n.as_bytes()).map(|c| (PhpStr::from(*n), c.borrow().clone())))
                .filter(|(_, v)| !v.is_undef())
                .collect();
            out.splice(0..0, first);
            for (n, c) in &self.globals {
                if !REQUEST_GLOBALS.iter().any(|r| r.as_bytes() == n.as_slice()) && !c.borrow().is_undef() {
                    out.push((PhpStr::from(n.as_slice()), c.borrow().clone()));
                }
            }
        } else if let Some(vars) = frame.vars() {
            for (n, c) in &vars.0 {
                out.push((PhpStr::from(n.as_slice()), c.borrow().clone()));
            }
        }
        out
    }

    fn backtrace(&self, objects: bool) -> PhpArray {
        let regs = &self.isolate.regs;
        self.trace_with(&|i| regs.get(i).cloned().unwrap_or_default(), objects)
    }

    fn included_files(&self) -> Vec<String> {
        self.included_order.clone()
    }

    fn code_cache(&self) -> Option<std::sync::Arc<dyn p8_runtime::opcache::CodeCache>> {
        self.loader.as_ref().and_then(|l| l.code_cache())
    }

    fn compile_file(&mut self, path: &[u8]) -> Result<bool, Throw> {
        self.opcache_compile_file(path)
    }

    fn state(&mut self, key: &'static str) -> &mut Value {
        // (A builtin call checks the flag before looking the callback up.)
        if key == "header_callback" {
            self.header_callback_set = true;
        }
        self.ext_state.entry(key).or_insert(Value::NULL)
    }

    fn new_resource(&mut self, resource: p8_runtime::stream::Resource) -> Value {
        Value::object(self.make_resource(resource))
    }

    fn write_stdout(&mut self, bytes: &[u8]) {
        if self.out.response.server {
            // A web request has no STDOUT of its own: this is the server's.
            use std::io::Write;
            let _ = std::io::stdout().write_all(bytes);
            return;
        }
        self.out.host.write(bytes);
    }

    fn native_class(&self) -> Option<Rc<Class>> {
        self.native_class.clone()
    }

    fn enum_case(&mut self, class: &Rc<Class>, case: &[u8]) -> Value {
        Value::object(Context::enum_case(self, class, case))
    }

    fn error_handler(&mut self, handler: Option<(Value, i64)>) -> Value {
        let old = self.error_handlers.last().map(|(h, _)| h.clone()).unwrap_or(Value::NULL);
        match handler {
            Some(h) => self.error_handlers.push(h),
            None => {
                self.error_handlers.pop();
            }
        }
        old
    }

    fn exception_handler(&mut self, handler: Option<Value>) -> Value {
        let old = self.exception_handlers.last().cloned().unwrap_or(Value::NULL);
        match handler {
            Some(h) => self.exception_handlers.push(h),
            None => {
                self.exception_handlers.pop();
            }
        }
        old
    }

    fn current_error_handler(&self) -> Value {
        self.error_handlers.last().map(|(h, _)| h.clone()).unwrap_or(Value::NULL)
    }

    fn current_exception_handler(&self) -> Value {
        self.exception_handlers.last().cloned().unwrap_or(Value::NULL)
    }

    fn autoloader(&mut self, callable: Value, register: Option<bool>) -> bool {
        let at = self.autoloaders.iter().position(|a| same_autoloader(a, &callable));
        match (register, at) {
            (Some(_), Some(_)) => true,
            (Some(true), None) => {
                self.autoloaders.insert(0, callable);
                true
            }
            (Some(false), None) => {
                self.autoloaders.push(callable);
                true
            }
            (None, Some(i)) => {
                self.autoloaders.remove(i);
                true
            }
            (None, None) => false,
        }
    }

    fn autoloaders(&self) -> Vec<Value> {
        self.autoloaders.clone()
    }

    fn on_shutdown(&mut self, callable: Value, args: Vec<Value>) {
        self.shutdown.push((callable, args));
        self.shutdown_names.push(None);
    }

    fn shutdown_named(&mut self, name: &'static str, entry: Option<(Value, Vec<Value>)>) {
        let at = self.shutdown_names.iter().position(|n| *n == Some(name));
        match (at, entry) {
            (Some(i), Some(e)) => self.shutdown[i] = e,
            (Some(i), None) => {
                self.shutdown.remove(i);
                self.shutdown_names.remove(i);
            }
            (None, Some(e)) => {
                self.shutdown.push(e);
                self.shutdown_names.push(Some(name));
            }
            (None, None) => {}
        }
    }

    fn ext_data(&mut self, key: &'static str) -> &mut Option<Box<dyn std::any::Any>> {
        self.ext_data.entry(key).or_default()
    }

    fn global_cell(&mut self, name: &[u8]) -> Rc<RefCell<Value>> {
        if self.main_frame().is_none() {
            return self.globals.entry(name.to_vec()).or_insert_with(|| Rc::new(RefCell::new(Value::NULL))).clone();
        }
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let cell = self.with_main(&mut regs, |cx, main| Context::global_cell(cx, main, name));
        self.isolate.regs = regs;
        cell
    }

    fn location(&self) -> (Rc<str>, u32) {
        Context::location(self)
    }

    fn declared_classes(&self) -> Vec<(PhpStr, Rc<Class>)> {
        let mut internal: Vec<(PhpStr, Rc<Class>)> = self
            .classes
            .iter()
            .filter(|&(&k, c)| c.flags & p8_runtime::object::CLASS_INTERNAL != 0 && self.sym_of(c.name.as_bytes()) == Some(k))
            .map(|(_, c)| (c.name.clone(), Rc::clone(c)))
            .collect();
        internal.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        internal.into_iter().chain(self.declared_order.iter().cloned()).collect()
    }

    fn class_alias(&mut self, class: Rc<Class>, alias: &[u8]) -> bool {
        let key = self.intern(alias);
        if self.classes.contains_key(&key) {
            return false;
        }
        let lower = alias.strip_prefix(b"\\").unwrap_or(alias).to_ascii_lowercase();
        self.declared_order.push((PhpStr::from(lower.as_slice()), Rc::clone(&class)));
        self.classes.insert(key, class);
        true
    }

    fn iterate(&mut self, v: &Value) -> Result<Vec<(Value, Value)>, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        if regs.is_empty() {
            return Err(Ctx::fatal(self, "an iteration with no registers lent".into()));
        }
        let r = self.iterate_all(&mut regs, v);
        self.isolate.regs = regs;
        r
    }

    fn fiber_op(&mut self, fiber: Option<&Rc<Object>>, op: u8, args: &mut [Value]) -> Result<Value, Throw> {
        Context::fiber_op(self, fiber, op, args)
    }

    fn gc_collect(&mut self) -> Result<i64, Throw> {
        let garbage = p8_runtime::gc::find_cycles(&self.ids);
        if garbage.objects.is_empty() {
            return Ok(0);
        }
        // Destructors first, oldest first; one may keep an object alive
        // (store it somewhere), so the garbage is found again after.
        let mut ran = false;
        // A destructor's exception doesn't stop the others: each new one
        // takes the ones before as its previous.
        let mut thrown: Option<Rc<Object>> = None;
        for o in &garbage.objects {
            if o.destructed.get() || o.class.method_sym(p8_runtime::symbol::known::DESTRUCT).is_none() || o.lazy.get() != 0 {
                continue;
            }
            o.destructed.set(true);
            ran = true;
            let m = self.method_frame(&Value::object(Rc::clone(o)), Some(p8_runtime::symbol::known::DESTRUCT), b"__destruct")?;
            let r = if self.fibers.is_empty() {
                let mut regs = std::mem::take(&mut self.isolate.regs);
                let r = self.run_nested(&mut regs, m, &mut []).map(|_| ());
                self.isolate.regs = regs;
                r
            } else {
                self.destruct_in_fiber(m)
            };
            match r {
                Ok(()) => {}
                Err(Throw::Object(e)) if e.class.is_a(b"throwable") => {
                    if let Some(before) = thrown.take() {
                        builtin_classes::set_previous(&e, before);
                    }
                    thrown = Some(e);
                }
                Err(t) => return Err(t),
            }
        }
        let garbage = if ran {
            drop(garbage);
            // PHP counts the pass after destructors as a run of its own.
            self.gc_runs += 1;
            p8_runtime::gc::find_cycles(&self.ids)
        } else {
            garbage
        };
        for o in &garbage.objects {
            o.destructed.set(true);
            p8_runtime::gc::clear(o);
        }
        let count = garbage.count as i64;
        drop(garbage);
        self.gc_runs += 1;
        self.gc_collected += count;
        match thrown {
            Some(e) => Err(Throw::Object(e)),
            None => Ok(count),
        }
    }

    fn gc_counts(&self) -> (i64, i64) {
        (self.gc_runs, self.gc_collected)
    }

    fn running_for(&self) -> std::time::Duration {
        self.started.elapsed()
    }

    fn cfg(&self) -> Vec<(Vec<u8>, String)> {
        self.cfg.clone()
    }

    fn parse_check(&self, source: &[u8]) -> Option<(String, u32)> {
        let loader = self.loader.as_ref()?;
        match loader.load(source, "") {
            Err(e) if e.parse => Some((e.message, e.line)),
            _ => None,
        }
    }

    /// The command line's arguments (getopt()), there whatever
    /// register_argc_argv says (see request_globals).
    fn request_argv(&self) -> Option<Vec<String>> {
        self.argv.clone()
    }

    fn global_value(&self, name: &[u8]) -> Option<Value> {
        let main = self.main_frame()?;
        // The main frame's registers: the running task's, or the first
        // task's while a fiber runs.
        let regs = self.waiting_tasks().next().map_or(&self.isolate.regs, |t| &t.regs);
        match main.func.code.cv_names.iter().position(|n| n.as_bytes() == name) {
            Some(r) => regs.get(r).map(Value::deref).filter(|v| !v.is_undef()),
            None => self.globals.get(name).map(|c| c.borrow().deref()).filter(|v| !v.is_undef()),
        }
    }

    fn lazy_make(&mut self, class: &Rc<Class>, obj: Option<Rc<Object>>, initializer: Value, options: i64, proxy: bool) -> Result<Rc<Object>, Throw> {
        self.make_lazy(class, obj, initializer, options, proxy)
    }

    fn clone_with(&mut self, object: &Rc<Object>, with: &p8_runtime::array::PhpArray) -> Result<Value, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = Context::clone_with(self, &mut regs, object, with);
        self.isolate.regs = regs;
        r
    }

    fn lazy_initialize(&mut self, o: &Rc<Object>) -> Result<Rc<Object>, Throw> {
        self.lazy_init_from_builtin(o)
    }

    fn lazy_mark_initialized(&mut self, o: &Rc<Object>) -> Result<(), Throw> {
        Context::lazy_mark_initialized(self, o)
    }

    fn lazy_slot_done(&mut self, o: &Rc<Object>, slot: usize) {
        Context::lazy_slot_done(self, o, slot)
    }

    fn generator(&mut self, generator: &Rc<Object>, op: &str, arg: Value) -> Result<Value, Throw> {
        use generators::Op;
        let op = match op {
            "current" => Op::Current,
            "key" => Op::Key,
            "next" => Op::Next,
            "send" => Op::Send,
            "valid" => Op::Valid,
            "rewind" => Op::Rewind,
            "getreturn" => Op::GetReturn,
            _ => Op::Throw,
        };
        let mut regs = std::mem::take(&mut self.isolate.regs);
        if regs.is_empty() {
            return Err(Ctx::fatal(self, "a generator resumed with no registers lent".into()));
        }
        let r = self.generator_op(&mut regs, generator, op, arg);
        self.isolate.regs = regs;
        r
    }
}

/// `x cmp y` on two ints (`<=>`: whether it is nonzero).
#[inline(always)]
fn int_test(cmp: Cmp, x: i64, y: i64) -> bool {
    match cmp {
        Cmp::Lt => x < y,
        Cmp::Le => x <= y,
        Cmp::Gt => x > y,
        Cmp::Ge => x >= y,
        Cmp::Eq | Cmp::Identical => x == y,
        Cmp::NotEq | Cmp::NotIdentical | Cmp::Spaceship => x != y,
    }
}

/// Write `value` into variable register `r` of window `w` (through a reference).
#[inline(always)]
fn assign(w: &mut [Value], r: Reg, value: Value) {
    let slot = &mut w[r as usize];
    match slot.as_reference() {
        Some(cell) => *cell.borrow_mut() = value,
        None => *slot = value,
    }
}

/// Whether PHP compiles a call of builtin `name` with `argc` arguments
/// to an opcode (zend_try_compile_special_func): its errors then come
/// with no frame of their own in the trace.
fn opcode_function(name: &str, argc: usize) -> bool {
    matches!((name, argc), ("strlen" | "count" | "sizeof", 1) | ("array_key_exists", 2))
}

/// Whether register `r` holds a temporary (not a variable or constant).
fn is_temp(func: &Function, r: Reg) -> bool {
    r >= func.ncv && const_index(r).is_none()
}

/// Write `value` into a cell (through a reference it holds).
fn store(cell: &RefCell<Value>, value: Value) {
    let inner = cell.borrow().clone();
    match inner.as_reference() {
        Some(r) => *r.borrow_mut() = value,
        None => *cell.borrow_mut() = value,
    }
}

/// Make variable register `r` a reference (if it is not one) and return its cell.
fn make_ref(w: &mut [Value], r: Reg) -> Rc<RefCell<Value>> {
    let slot = &mut w[r as usize];
    if let Some(cell) = slot.cloned_reference() {
        return cell;
    }
    let value = std::mem::take(slot);
    let value = if value.is_undef() { Value::NULL } else { value };
    let cell = Rc::new(RefCell::new(value));
    *slot = Value::reference(cell.clone());
    cell
}

/// Arguments past the parameters, moved out for func_get_args() (and a
/// variadic parameter's `CollectVariadic`).
#[cold]
#[allow(clippy::vec_box)] // see `Frames::spare`
fn take_extra(spare: &mut Vec<Box<Extra>>, regs: &mut [Value], base: usize, nparams: usize, argc: usize) -> Option<Box<Extra>> {
    // A spare one (emptied) if there is one: no allocation.
    let mut extra = spare.pop().unwrap_or_default();
    // A new one's list from the kept buffers.
    if extra.args.capacity() == 0 {
        extra.args = p8_runtime::array::values_buffer(argc - nparams);
    }
    extra.args.extend(regs[base + nparams..base + argc].iter_mut().map(std::mem::take));
    Some(extra)
}

/// `$a[k] op= v` for an int or string `k`, `$a` an array holding a number
/// there and `v` a number; false (nothing done) for anything else. Out of
/// line, so the fast loop's registers stay its own.
#[inline(never)]
fn dim_op(w: &mut [Value], op: BinOp, var: Reg, keys: Reg, val: Reg, dst: Reg) -> bool {
    let Some(a) = w[var as usize].as_array() else { return false };
    let key = &w[keys as usize];
    let r = if let Some(k) = key.as_int() {
        let Some(r) = a.get_int(k).and_then(|x| num_op(op, x, &w[val as usize])) else { return false };
        let Some(a) = w[var as usize].array_mut() else { return false };
        match a.get_int_mut(k) {
            Some(slot) => *slot = r.clone(),
            None => return false,
        }
        r
    } else if key.is_str() {
        // One lookup: the array is made writable first (a copy, if it is
        // shared, is invisible to PHP code even when nothing is written).
        let key = key.clone();
        let s = key.as_str().expect("a string");
        let v = w[val as usize].clone();
        let Some(a) = w[var as usize].array_mut() else { return false };
        let Some(slot) = a.get_str_mut(s) else { return false };
        let Some(r) = num_op(op, slot, &v) else { return false };
        *slot = r.clone();
        r
    } else {
        return false;
    };
    w[dst as usize] = r;
    true
}

/// `$str .= $s` with both strings (and `$str` not a reference): appends in
/// place unless the string is shared. False (nothing done) otherwise.
#[inline(never)]
fn append_str(w: &mut [Value], var: Reg, b: Src) -> bool {
    // A literal is read in place (program memory: never the target).
    if let Some(s) = src(w, b).as_istr() {
        return w[var as usize].str_push(s.as_bytes());
    }
    // A handle of its own: `$s .= $s` reads what it appends to.
    let Some(tail) = src(w, b).cloned_str() else { return false };
    w[var as usize].str_push(tail.as_bytes())
}

/// Whether a value becomes a string with no warning and no side effect
/// (so when it is converted cannot be observed).
#[inline(always)]
fn silent_str(v: &Value) -> bool {
    match v.kind() {
        Kind::Str | Kind::Int | Kind::Bool | Kind::Null => true,
        // NAN warns when it becomes a string (8.5+)
        Kind::Float => !v.as_float().expect("a float").is_nan(),
        Kind::Array | Kind::Ref | Kind::Object | Kind::Undef => false,
    }
}

/// `a . b` on values that convert silently, the result not into a
/// reference. A temporary string on the left that nothing else holds is
/// appended to in place (`a . b . c` builds one string). False otherwise.
#[inline(never)]
fn concat(w: &mut [Value], dst: Reg, a: Reg, b: Src) -> bool {
    let (x, y) = (&w[a as usize], src(w, b));
    if !silent_str(x) || !silent_str(y) || w[dst as usize].is_reference() {
        return false;
    }
    let mut diags = Vec::new();
    if dst == a && !matches!(b, Src::Reg(r) if r == a) && w[a as usize].is_str() {
        let mut s = std::mem::take(&mut w[a as usize]).into_str().unwrap_or_else(|_| unreachable!("checked"));
        ops::push_string(s.make_mut(), src(w, b), &mut diags);
        w[dst as usize] = Value::str(s);
        return true;
    }
    let y = src(w, b);
    if let (Some(p), Some(q)) = (x.as_str(), y.as_str()) {
        w[dst as usize] = Value::str(PhpStr::concat(p.as_bytes(), q.as_bytes()));
        return true;
    }
    // Built in the thread's buffer: a short result is one allocation.
    let mut bytes = PhpStr::scratch(ops::string_len_hint(x) + ops::string_len_hint(y));
    ops::push_string(&mut bytes, x, &mut diags);
    ops::push_string(&mut bytes, y, &mut diags);
    w[dst as usize] = Value::str(PhpStr::from_scratch(bytes));
    true
}

/// `concat` with a literal on the left (`"prefix" . $s`): nothing to
/// append to in place.
#[inline(never)]
fn concat_left(w: &mut [Value], dst: Reg, x: &Value, b: Reg) -> bool {
    let y = &w[b as usize];
    if !silent_str(x) || !silent_str(y) || w[dst as usize].is_reference() {
        return false;
    }
    if let (Some(p), Some(q)) = (x.as_str(), y.as_str()) {
        w[dst as usize] = Value::str(PhpStr::concat(p.as_bytes(), q.as_bytes()));
        return true;
    }
    let mut diags = Vec::new();
    let mut bytes = PhpStr::scratch(ops::string_len_hint(x) + ops::string_len_hint(y));
    ops::push_string(&mut bytes, x, &mut diags);
    ops::push_string(&mut bytes, y, &mut diags);
    w[dst as usize] = Value::str(PhpStr::from_scratch(bytes));
    true
}

/// Interpolation (`"a $b c"`) when every part converts silently.
#[inline(never)]
fn concat_n(w: &mut [Value], dst: Reg, start: Reg, n: u8) -> bool {
    let parts = &w[start as usize..start as usize + n as usize];
    if !parts.iter().all(silent_str) || w[dst as usize].is_reference() {
        return false;
    }
    let mut diags = Vec::new();
    // ZEND_ROPE_END: the parts measured, then written into one buffer (the
    // thread's: a short result is one allocation, a long one its buffer).
    let mut bytes = PhpStr::scratch(parts.iter().map(ops::string_len_hint).sum());
    for v in parts {
        ops::push_string(&mut bytes, v, &mut diags);
    }
    w[dst as usize] = Value::str(PhpStr::from_scratch(bytes));
    free_parts(w, start, n);
    true
}

/// A concatenation's parts (its own temporaries) are released once it is
/// made, as ZEND_ROPE_END frees them.
fn free_parts(w: &mut [Value], start: Reg, n: u8) {
    for r in &mut w[start as usize..start as usize + n as usize] {
        clear(r);
    }
}

/// How UnhandledMatchError names the value no arm matched, as PHP's
/// smart_str_append_scalar does: scalars as written (a string quoted,
/// escaped and cut at 15 characters), anything else by type.
fn match_case(v: &Value) -> String {
    match v.kind() {
        Kind::Null | Kind::Undef => "NULL".into(),
        Kind::Bool => if v.as_bool().expect("a bool") { "true" } else { "false" }.into(),
        Kind::Int => v.as_int().expect("an int").to_string(),
        Kind::Float => float_to_string(v.as_float().expect("a float"), Precision::ECHO, 'E'),
        Kind::Str => {
            let bytes = v.as_str().expect("a string").as_bytes();
            let mut out = String::from("'");
            for &c in &bytes[..bytes.len().min(15)] {
                match c {
                    b'\n' => out.push_str("\\n"),
                    b'\r' => out.push_str("\\r"),
                    b'\t' => out.push_str("\\t"),
                    0x0c => out.push_str("\\f"),
                    0x0b => out.push_str("\\v"),
                    b'\\' => out.push_str("\\\\"),
                    0x1b => out.push_str("\\e"),
                    c if !(0x20..0x7f).contains(&c) => out.push_str(&format!("\\x{c:02X}")),
                    c => out.push(c as char),
                }
            }
            if bytes.len() > 15 {
                out.push_str("...");
            }
            out.push('\'');
            out
        }
        Kind::Ref => match_case(&v.as_reference().expect("a reference").borrow()),
        Kind::Array | Kind::Object => format!("of type {}", v.type_name()),
    }
}

/// foreach's next element: its value into `val` (and key into `key`), the
/// position past it into the iterator. False when there is none.
#[inline(never)]
fn iter_next(w: &mut [Value], iter: Reg, val: Reg, key: Reg, has_key: bool) -> bool {
    let pos = w[iter as usize + 1].as_int().map_or(0, |p| p as usize);
    let Some(a) = w[iter as usize].as_array() else { return false };
    // By value: an element that is a reference (a by-reference foreach
    // made it one) gives its value, not the reference. The key is made
    // only when the loop names it.
    let (p, v, k) = if has_key {
        let Some((p, k, v)) = a.entry_at(pos) else { return false };
        (p, v.deref(), Some(k))
    } else {
        let Some((p, v)) = a.value_at(pos) else { return false };
        (p, v.deref(), None)
    };
    w[iter as usize + 1] = Value::int(p as i64 + 1);
    assign(w, val, v);
    if let Some(k) = k {
        assign(w, key, k.into_value());
    }
    true
}

/// Arithmetic on two numbers (and the bit operators on two ints), when no
/// warning, error or overflow is involved (the fast path: anything else
/// takes the slow one).
#[inline(always)]
fn num_op(op: BinOp, a: &Value, b: &Value) -> Option<Value> {
    match (a.as_int(), b.as_int(), a.as_float(), b.as_float()) {
        (Some(x), Some(y), _, _) => int_op(op, x, y).map(Value::int),
        (_, _, Some(x), Some(y)) => float_op(op, x, y),
        (Some(x), _, _, Some(y)) => float_op(op, x as f64, y),
        (_, Some(y), Some(x), _) => float_op(op, x, y as f64),
        _ => None,
    }
}

/// Arithmetic and the bit operators on two ints, when no warning, error
/// or overflow is involved (anything else takes the slow path).
#[inline(always)]
fn int_op(op: BinOp, x: i64, y: i64) -> Option<i64> {
    match op {
        BinOp::Add => x.checked_add(y),
        BinOp::Sub => x.checked_sub(y),
        BinOp::Mul => x.checked_mul(y),
        // % by zero throws and PHP_INT_MIN % -1 is 0: both take `step`.
        BinOp::Mod => x.checked_rem(y).filter(|_| y != 0),
        BinOp::BitAnd => Some(x & y),
        BinOp::BitOr => Some(x | y),
        BinOp::BitXor => Some(x ^ y),
        // A negative shift throws (`step`); 64 or more shifts every bit out.
        BinOp::Shl if (0..64).contains(&y) => Some(x << y),
        BinOp::Shl if y >= 64 => Some(0),
        BinOp::Shr if (0..64).contains(&y) => Some(x >> y),
        BinOp::Shr if y >= 64 => Some(if x < 0 { -1 } else { 0 }),
        _ => None,
    }
}

/// +, -, * and / on two floats (the fast path; / by zero takes the slow one).
#[inline]
fn float_op(op: BinOp, x: f64, y: f64) -> Option<Value> {
    Some(Value::float(match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div if y != 0.0 => x / y,
        _ => return None,
    }))
}

/// Whether `x cmp y` holds (`<=>`: whether it is nonzero), as `compare`
/// says, without making the result a value.
#[inline]
fn test_values(cmp: Cmp, x: &Value, y: &Value) -> bool {
    match cmp {
        Cmp::Eq => x.loose_eq(y),
        Cmp::NotEq => !x.loose_eq(y),
        Cmp::Identical => x.identical(y),
        Cmp::NotIdentical => !x.identical(y),
        Cmp::Lt => x.compare(y) == Some(Ordering::Less),
        Cmp::Le => matches!(x.compare(y), Some(Ordering::Less | Ordering::Equal)),
        Cmp::Gt => y.compare(x) == Some(Ordering::Less),
        Cmp::Ge => matches!(y.compare(x), Some(Ordering::Less | Ordering::Equal)),
        Cmp::Spaceship => x.compare(y) != Some(Ordering::Equal),
    }
}

fn compare(cmp: Cmp, x: &Value, y: &Value) -> Value {
    Value::bool(match cmp {
        Cmp::Eq => x.loose_eq(y),
        Cmp::NotEq => !x.loose_eq(y),
        Cmp::Identical => x.identical(y),
        Cmp::NotIdentical => !x.identical(y),
        Cmp::Lt => x.compare(y) == Some(Ordering::Less),
        Cmp::Le => matches!(x.compare(y), Some(Ordering::Less | Ordering::Equal)),
        Cmp::Gt => y.compare(x) == Some(Ordering::Less),
        Cmp::Ge => matches!(y.compare(x), Some(Ordering::Less | Ordering::Equal)),
        Cmp::Spaceship => {
            return Value::int(match x.compare(y) {
                Some(Ordering::Less) => -1,
                Some(Ordering::Equal) => 0,
                _ => 1,
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) struct Sink;
    impl Host for Sink {
        fn write(&mut self, _: &[u8]) {}
    }

    pub(crate) fn compile(source: &str, registry: &Registry) -> p8_compiler::Program {
        let arena = bumpalo::Bump::new();
        let mut parser = p8_syntax::parser::Parser::new(p8_syntax::lexer::Lexer::new(source.as_bytes()), &arena);
        let program = parser.parse_program();
        assert!(program.errors.is_empty(), "parse errors");
        p8_compiler::Compiler::new(source.as_bytes(), "t.php", registry).compile(program.statements).expect("compiles")
    }

    /// A recursive function's call site resolves to the function itself.
    /// When the request ends, every closure it made must be freed: a cycle
    /// between a closure and its call sites would leak one per request.
    #[test]
    fn a_request_frees_its_closures() {
        let registry = Arc::new(Registry::default());
        let program = compile("<?php function f($n) { return $n < 1 ? 0 : f($n - 1); } f(5); f(3);", &registry);
        let mut isolate = Isolate::new(Arc::clone(&registry));
        let mut host = Sink;
        let closures: Vec<std::rc::Weak<Closure>> = {
            let mut cx = isolate.new_context(&mut host);
            for f in &program.functions {
                cx.declare(Arc::clone(f)).expect("declares");
            }
            assert_eq!(cx.run_main(Arc::clone(&program.main)), 0);
            assert!(cx.closures.iter().any(|c| c.sites.iter().any(|s| s.get().is_some())), "a call site was resolved");
            cx.closures.iter().map(Rc::downgrade).collect()
        };
        assert!(!closures.is_empty());
        assert!(closures.iter().all(|c| c.upgrade().is_none()), "a closure outlived its request");
    }
}

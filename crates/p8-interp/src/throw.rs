//! Exceptions in the interpreter: the stack trace a Throwable records, the
//! object an engine error becomes, and unwinding to the `catch` (or
//! `finally`) that takes it.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_exceptions.c,
//! $PHP_SRC_PATH/Zend/zend_builtin_functions.c (zend_fetch_debug_backtrace),
//! $PHP_SRC_PATH/Zend/zend_execute.c (ZEND_HANDLE_EXCEPTION)

use crate::{Context, Frame, Frames, NativeCall, clear};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::object::Object;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

// A trace entry's keys (PHP's known strings: ZSTR_KNOWN(ZEND_STR_FILE),
// …), made once per thread with their hashes: an entry allocates and
// hashes none of them, and each is appended with no lookup
// (_zend_hash_append_ex), an entry holding each key at most once.
const FILE: usize = 0;
const LINE: usize = 1;
const FUNCTION: usize = 2;
const CLASS: usize = 3;
const OBJECT: usize = 4;
const TYPE: usize = 5;
const ARGS: usize = 6;

thread_local! {
    static KEYS: [PhpStr; 7] = p8_runtime::array::record_keys(["file", "line", "function", "class", "object", "type", "args"]);
}

fn key(k: usize) -> Key {
    KEYS.with(|keys| Key::Str(keys[k].clone()))
}

/// An empty trace entry: room for its keys (array_init_size(8)).
fn new_entry() -> PhpArray {
    PhpArray::hashed(8)
}

impl Context<'_> {
    /// The calls that led to the running code, innermost first, as
    /// debug_backtrace() gives them: where each was called from, the
    /// function (and class), its arguments. `regs` reads a register.
    pub(crate) fn trace(&self, regs: &dyn Fn(usize) -> Value) -> PhpArray {
        self.trace_with(regs, false)
    }

    /// trace(), with each method call's `$this` as `object` when `objects`
    /// (DEBUG_BACKTRACE_PROVIDE_OBJECT).
    pub(crate) fn trace_with(&self, regs: &dyn Fn(usize) -> Value, objects: bool) -> PhpArray {
        let mut trace = PhpArray::new();
        // The running task's frames over those of the tasks waiting under
        // it (a fiber's above whoever started or resumed it), as one
        // stack, as PHP shows it; each frame with its task's registers
        // (None: the running task's, read through `regs`), each builtin
        // at its depth in that stack.
        let mut frames: Vec<(&Frame, Option<&[Value]>)> = Vec::new();
        let mut natives: Vec<(usize, &NativeCall)> = Vec::new();
        for task in self.waiting_tasks() {
            let below = frames.len();
            natives.extend(task.natives.iter().map(|n| (below + n.depth, n)));
            frames.extend(task.frames.slots[..task.frames.len()].iter().map(|f| (f, Some(task.regs.as_slice()))));
        }
        let below = frames.len();
        natives.extend(self.natives.iter().map(|n| (below + n.depth, n)));
        frames.extend(self.frames.slots[..self.frames.len()].iter().map(|f| (f, None)));
        // An internal method the error was raised in, called from the top frame.
        if let Some(&(_, n)) = natives.last().filter(|(depth, n)| n.top && *depth == frames.len()) {
            let (file, line) = self.location();
            let mut entry = new_entry();
            entry.insert_absent(key(FILE), Value::str(PhpStr::from(&*file)));
            entry.insert_absent(key(LINE), Value::int(i64::from(line)));
            entry.insert_absent(key(FUNCTION), Value::str(n.name));
            entry.insert_absent(key(ARGS), Value::array(PhpArray::from_list(n.args.clone())));
            let _ = trace.push(Value::array(entry));
        }
        // A builtin's entry: called from `caller`, at `line`.
        let native_entry = |n: &NativeCall, caller: &Frame, line: u32| {
            let mut entry = new_entry();
            entry.insert_absent(key(FILE), Value::str(PhpStr::from(&*caller.func.code.file)));
            entry.insert_absent(key(LINE), Value::int(i64::from(line)));
            match &n.method {
                Some((name, class, this)) => {
                    entry.insert_absent(key(FUNCTION), Value::str(name.clone()));
                    entry.insert_absent(key(CLASS), Value::str(class.clone()));
                    if objects && let Some(this) = this {
                        entry.insert_absent(key(OBJECT), Value::object(Rc::clone(this)));
                    }
                    entry.insert_absent(key(TYPE), Value::str(if this.is_some() { "->" } else { "::" }));
                }
                None => {
                    entry.insert_absent(key(FUNCTION), Value::str(n.name));
                }
            }
            entry.insert_absent(key(ARGS), Value::array(PhpArray::from_list(n.args.clone())));
            entry
        };
        for k in (1..frames.len()).rev() {
            let ((frame, frame_regs), (caller, _)) = (frames[k], frames[k - 1]);
            if frame.func.code.name.as_bytes() == p8_bytecode::CONST_EXPR.as_bytes() {
                // A function's constant expression runs in place: no frame
                // of its own. A class's reports where it is written (see
                // location()), so the trace says where it was needed: the
                // builtin evaluating it (ReflectionAttribute::newInstance),
                // or "[constant expression]" (zend_fetch_debug_backtrace's
                // entry for the filename override).
                if frame.func.code.own_location && frame.scope.is_some() {
                    let line = caller.func.code.lines.get((caller.pc as usize).saturating_sub(1)).copied().unwrap_or(0);
                    let entry = match natives.iter().find(|(depth, _)| *depth == k) {
                        Some(&(_, n)) => native_entry(n, caller, line),
                        None => {
                            let mut entry = new_entry();
                            entry.insert_absent(key(FILE), Value::str(PhpStr::from(&*caller.func.code.file)));
                            entry.insert_absent(key(LINE), Value::int(i64::from(line)));
                            entry.insert_absent(key(FUNCTION), Value::str("[constant expression]"));
                            entry
                        }
                    };
                    let _ = trace.push(Value::array(entry));
                }
                continue;
            }
            let mut entry = new_entry();
            let code = &caller.func.code;
            let line = code.lines.get((caller.pc as usize).saturating_sub(1)).copied().unwrap_or(0);
            // A frame a builtin called (a callback): from no file.
            let native = natives.iter().find(|(depth, _)| *depth == k).map(|&(_, n)| n);
            if native.is_none() {
                entry.insert_absent(key(FILE), Value::str(PhpStr::from(&*code.file)));
                entry.insert_absent(key(LINE), Value::int(i64::from(line)));
            }
            let at = caller.func.code.code.get((caller.pc as usize).saturating_sub(1));
            let code = &frame.func.code;
            // A file's or eval()'s code (its main function) shows as the
            // include or eval that ran it (zend_fetch_debug_backtrace's
            // pseudo functions): `include('/path/f.php')`, its args before
            // its function; `eval()`, no args.
            if code.name.as_bytes() == b"{main}" {
                let (function, path) = match at {
                    Some(p8_bytecode::Instr::Eval { .. }) => ("eval", false),
                    Some(p8_bytecode::Instr::Include { kind, .. }) => (
                        match kind {
                            p8_bytecode::IncludeKind::Include => "include",
                            p8_bytecode::IncludeKind::IncludeOnce => "include_once",
                            p8_bytecode::IncludeKind::Require => "require",
                            p8_bytecode::IncludeKind::RequireOnce => "require_once",
                        },
                        true,
                    ),
                    _ => ("{main}", false),
                };
                if path {
                    entry.insert_absent(key(ARGS), Value::array(PhpArray::from_list(vec![Value::str(PhpStr::from(&*code.file))])));
                }
                entry.insert_absent(key(FUNCTION), Value::str(function));
                let _ = trace.push(Value::array(entry));
                continue;
            }
            entry.insert_absent(key(FUNCTION), Value::str(PhpStr::from(code.name.as_bytes())));
            // A closure made in a class shows that class, as a method does.
            if code.class.is_some() || (frame.scope.is_some() && code.name.as_bytes().starts_with(b"{closure")) {
                let class = frame.scope.as_ref().map(|c| c.name.clone()).unwrap_or_else(PhpStr::empty);
                entry.insert_absent(key(CLASS), Value::str(class));
                if objects && let Some(this) = &frame.this {
                    entry.insert_absent(key(OBJECT), Value::object(Rc::clone(this)));
                }
                entry.insert_absent(key(TYPE), Value::str(if frame.this.is_some() { "->" } else { "::" }));
            }
            let mut args = PhpArray::new();
            let base = frame.base as usize;
            for i in 0..(frame.argc as usize).min(code.params.len()) {
                let v = frame_regs.map_or_else(|| regs(base + i), |r| r.get(base + i).cloned().unwrap_or_default());
                if v.is_undef() {
                    continue;
                }
                let _ = args.push(v.deref());
            }
            for v in frame.args().iter() {
                let _ = args.push(v.deref());
            }
            entry.insert_absent(key(ARGS), Value::array(args));
            let _ = trace.push(Value::array(entry));
            if let Some(n) = native {
                let _ = trace.push(Value::array(native_entry(n, caller, line)));
            }
        }
        trace
    }

    /// The frame of a call that failed before it ran (a builtin, or a
    /// function given too few arguments): the trace shows it on top.
    pub(crate) fn failed_call(&mut self, function: &[u8], class: Option<(&PhpStr, bool)>, args: &[Value]) {
        let (file, line) = self.location();
        let mut entry = new_entry();
        entry.insert_absent(key(FILE), Value::str(PhpStr::from(&*file)));
        entry.insert_absent(key(LINE), Value::int(i64::from(line)));
        entry.insert_absent(key(FUNCTION), Value::str(PhpStr::from(function)));
        if let Some((class, this)) = class {
            entry.insert_absent(key(CLASS), Value::str(class.clone()));
            entry.insert_absent(key(TYPE), Value::str(if this { "->" } else { "::" }));
        }
        let args: Vec<Value> = args.iter().filter(|v| !v.is_undef()).map(Value::deref).collect();
        entry.insert_absent(key(ARGS), Value::array(PhpArray::from_list(args)));
        self.failed_call = Some(entry);
    }

    /// The object an engine error (`Throw::Error`) becomes.
    pub(crate) fn exception(&mut self, regs: &dyn Fn(usize) -> Value, t: Throw) -> Throw {
        let Throw::Error { class, message, file, line, props } = t else {
            // A fatal error, where it was raised: its trace, for showing with
            // it (fatal_error_backtraces; zend_error_impl fetches it then).
            if matches!(t, Throw::Fatal { .. }) && self.fatal_trace.borrow().is_none() && self.fatal_error_backtraces() {
                let trace = p8_runtime::trace::trace_string(&Value::array(self.raised_trace(regs)));
                *self.fatal_trace.borrow_mut() = Some(trace);
            }
            return t;
        };
        self.exception_object(regs, class, message, file, line, props)
    }

    /// Whether fatal errors carry their stack trace (the
    /// fatal_error_backtraces ini, on by default since 8.5).
    fn fatal_error_backtraces(&self) -> bool {
        use p8_runtime::ext::Ctx;
        Ctx::ini_get(self, b"fatal_error_backtraces").is_none_or(|v| crate::ini_on(&v))
    }

    /// The trace of where an error is being raised: the calls that led
    /// here, under the builtin it was raised in (`failed_call`), if any.
    fn raised_trace(&mut self, regs: &dyn Fn(usize) -> Value) -> PhpArray {
        let trace = self.trace(regs);
        let Some(top) = self.failed_call.take() else { return trace };
        let mut with = PhpArray::from_list(vec![Value::array(top)]);
        for (_, v) in trace.iter() {
            let _ = with.push(v.clone());
        }
        with
    }

    /// The object an engine error becomes, with the trace of where it was raised.
    fn exception_object(&mut self, regs: &dyn Fn(usize) -> Value, class: &'static str, message: String, file: Rc<str>, line: u32, props: Vec<(&'static str, Value)>) -> Throw {
        let found = self.sym_of(class.as_bytes()).and_then(|key| self.classes.get(&key).cloned().or_else(|| self.builtin_on_demand(key)));
        let class = match found {
            Some(c) => c,
            None => Rc::clone(&self.classes[&self.isolate.known.error]),
        };
        let object = Object::new(Rc::clone(&class), &self.ids);
        let root: &[u8] = if class.is_a(b"exception") { b"exception" } else { b"error" };
        class.write(&object, b"message", root, Value::str(PhpStr::from(message)));
        class.write(&object, b"file", root, Value::str(PhpStr::from(&*file)));
        class.write(&object, b"line", root, Value::int(i64::from(line)));
        let trace = self.raised_trace(regs);
        class.write(&object, b"trace", root, Value::array(trace));
        for (name, value) in props {
            class.write(&object, name.as_bytes(), &class.lower, value);
        }
        Throw::Object(object)
    }

    /// An exception thrown at `at` in the top frame: go to the handler
    /// that covers it there, or leave the frame and look in its caller.
    /// Returns where the top frame goes on; the exception if nothing
    /// catches it (or what is not an exception: exit, a fatal error).
    pub(crate) fn unwind(&mut self, regs: &mut [Value], t: Throw, mut at: usize) -> Result<usize, Throw> {
        let t = self.exception(&|i| regs.get(i).cloned().unwrap_or_default(), t);
        let Throw::Object(object) = t else { return Err(t) };
        loop {
            let frame = self.frames.last().expect("a frame");
            let base = frame.base as usize;
            // Leaving `@` expressions: their level comes back, innermost
            // first, unless the code changed it (it holds more than the
            // fatal errors then).
            let mut silences: Vec<_> = frame.func.code.silences.iter().filter(|s| s.0 as usize <= at && at < s.1 as usize).collect();
            silences.sort_by_key(|s| std::cmp::Reverse(s.0));
            for s in silences {
                if self.error_level & !4437 == 0
                    && let Some(level) = regs[base + s.2 as usize].as_int()
                {
                    self.error_level = level;
                }
            }
            let handler = frame
                .func
                .code
                .handlers
                .iter()
                .filter(|h| h.start as usize <= at && at < h.end as usize)
                .max_by_key(|h| h.depth)
                .copied();
            if let Some(h) = handler {
                regs[base + h.slot as usize] = Value::object(object);
                return Ok(h.target as usize);
            }
            // The bottom frame of this `execute`: the exception goes to
            // whoever started it (the host, a builtin calling back, or
            // the task that started or resumed a fiber). Only the main
            // frame stays (its globals, for what runs after).
            if self.frames.len() == self.frames.floor + 1 {
                if self.frames.floor > 0 || !self.fibers.is_empty() {
                    drop_frame(&mut self.frames, regs);
                }
                return Err(Throw::Object(object));
            }
            drop_frame(&mut self.frames, regs);
            at = (self.frames.last().expect("the caller").pc as usize).saturating_sub(1);
            self.pc = at + 1;
        }
    }
}

/// Pop the top frame without a result (an exception left it).
pub(crate) fn drop_frame(frames: &mut Frames, regs: &mut [Value]) {
    crate::give_back_shared(frames, regs);
    let frame = frames.pop();
    frame.extra = None;
    let base = frame.base as usize;
    let code = &frame.func.code;
    // A constructor an exception left: the object is never destructed.
    if code.name.as_bytes().eq_ignore_ascii_case(b"__construct")
        && let Some(this) = &frame.this
    {
        this.destructed.set(true);
    }
    // The variables and the temporaries (an exception may leave anything
    // in those).
    for r in &mut regs[base..base + code.nregs as usize] {
        clear(r);
    }
    frame.this = None;
    frame.scope = None;
    frame.called = None;
    frame.owner = None;
}

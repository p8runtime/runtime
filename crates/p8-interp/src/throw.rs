//! Exceptions in the interpreter: the stack trace a Throwable records, the
//! object an engine error becomes, and unwinding to the `catch` (or
//! `finally`) that takes it.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_exceptions.c,
//! $PHP_SRC_PATH/Zend/zend_builtin_functions.c (zend_fetch_debug_backtrace),
//! $PHP_SRC_PATH/Zend/zend_execute.c (ZEND_HANDLE_EXCEPTION)

use crate::{Context, Frames, clear};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::object::Object;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

fn key(k: &str) -> Key {
    Key::from_string(PhpStr::from(k))
}

impl Context<'_> {
    /// The calls that led to the running code, innermost first, as
    /// debug_backtrace() gives them: where each was called from, the
    /// function (and class), its arguments. `regs` reads a register.
    pub(crate) fn trace(&self, regs: &dyn Fn(usize) -> Value) -> PhpArray {
        let mut trace = PhpArray::new();
        let frames = &self.frames.slots[..self.frames.len()];
        // An internal method the error was raised in, called from the top frame.
        if let Some(n) = self.natives.last().filter(|n| n.top && n.depth == frames.len()) {
            let (file, line) = self.location();
            let mut entry = PhpArray::new();
            entry.insert(key("file"), Value::Str(PhpStr::from(&*file)));
            entry.insert(key("line"), Value::Int(i64::from(line)));
            entry.insert(key("function"), Value::str(n.name));
            entry.insert(key("args"), Value::Array(Rc::new(PhpArray::from_list(n.args.clone()))));
            let _ = trace.push(Value::Array(Rc::new(entry)));
        }
        for k in (1..frames.len()).rev() {
            let (frame, caller) = (&frames[k], &frames[k - 1]);
            // A constant expression's code runs in place: no frame of its own.
            if frame.func.code.name.as_bytes() == p8_bytecode::CONST_EXPR.as_bytes() {
                continue;
            }
            let mut entry = PhpArray::new();
            let code = &caller.func.code;
            let line = code.lines.get((caller.pc as usize).saturating_sub(1)).copied().unwrap_or(0);
            // A frame a builtin called (a callback): from no file.
            let native = self.natives.iter().find(|n| n.depth == k);
            if native.is_none() {
                entry.insert(key("file"), Value::Str(PhpStr::from(&*code.file)));
                entry.insert(key("line"), Value::Int(i64::from(line)));
            }
            let code = &frame.func.code;
            entry.insert(key("function"), Value::Str(PhpStr::from(code.name.as_bytes())));
            if code.class.is_some() {
                let class = frame.scope.as_ref().map(|c| c.name.clone()).unwrap_or_else(PhpStr::empty);
                entry.insert(key("class"), Value::Str(class));
                entry.insert(key("type"), Value::str(if frame.this.is_some() { "->" } else { "::" }));
            }
            let mut args = PhpArray::new();
            let base = frame.base as usize;
            for i in 0..(frame.argc as usize).min(code.params.len()) {
                let v = match regs(base + i) {
                    Value::Undef => continue,
                    v => v.deref(),
                };
                let _ = args.push(v);
            }
            for v in frame.args().iter() {
                let _ = args.push(v.deref());
            }
            entry.insert(key("args"), Value::Array(Rc::new(args)));
            let _ = trace.push(Value::Array(Rc::new(entry)));
            if let Some(n) = native {
                let mut entry = PhpArray::new();
                entry.insert(key("file"), Value::Str(PhpStr::from(&*caller.func.code.file)));
                entry.insert(key("line"), Value::Int(i64::from(line)));
                entry.insert(key("function"), Value::str(n.name));
                entry.insert(key("args"), Value::Array(Rc::new(PhpArray::from_list(n.args.clone()))));
                let _ = trace.push(Value::Array(Rc::new(entry)));
            }
        }
        trace
    }

    /// The frame of a call that failed before it ran (a builtin, or a
    /// function given too few arguments): the trace shows it on top.
    pub(crate) fn failed_call(&mut self, function: &[u8], class: Option<(&PhpStr, bool)>, args: &[Value]) {
        let (file, line) = self.location();
        let mut entry = PhpArray::new();
        entry.insert(key("file"), Value::Str(PhpStr::from(&*file)));
        entry.insert(key("line"), Value::Int(i64::from(line)));
        entry.insert(key("function"), Value::Str(PhpStr::from(function)));
        if let Some((class, this)) = class {
            entry.insert(key("class"), Value::Str(class.clone()));
            entry.insert(key("type"), Value::str(if this { "->" } else { "::" }));
        }
        let args: Vec<Value> = args.iter().filter(|v| !matches!(v, Value::Undef)).map(Value::deref).collect();
        entry.insert(key("args"), Value::Array(Rc::new(PhpArray::from_list(args))));
        self.failed_call = Some(entry);
    }

    /// The object an engine error (`Throw::Error`) becomes.
    pub(crate) fn exception(&mut self, regs: &dyn Fn(usize) -> Value, t: Throw) -> Throw {
        let Throw::Error { class, message, file, line } = t else { return t };
        let lower = class.to_ascii_lowercase();
        let class = match self.classes.get(lower.as_bytes()).cloned().or_else(|| self.builtin_on_demand(lower.as_bytes())) {
            Some(c) => c,
            None => Rc::clone(&self.classes[b"error".as_slice()]),
        };
        let object = Object::new(Rc::clone(&class), &self.ids);
        let root: &[u8] = if class.is_a(b"exception") { b"exception" } else { b"error" };
        class.write(&object, b"message", root, Value::Str(PhpStr::from(message)));
        class.write(&object, b"file", root, Value::Str(PhpStr::from(&*file)));
        class.write(&object, b"line", root, Value::Int(i64::from(line)));
        let mut trace = self.trace(regs);
        if let Some(top) = self.failed_call.take() {
            let mut with = PhpArray::from_list(vec![Value::Array(Rc::new(top))]);
            for (_, v) in trace.iter() {
                let _ = with.push(v.clone());
            }
            trace = with;
        }
        class.write(&object, b"trace", root, Value::Array(Rc::new(trace)));
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
                    && let Value::Int(level) = regs[base + s.2 as usize]
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
                regs[base + h.slot as usize] = Value::Object(object);
                return Ok(h.target as usize);
            }
            // The bottom frame of this `execute`: the exception goes to
            // whoever started it (the host, or a builtin calling back).
            if self.frames.len() == self.frames.floor + 1 {
                if self.frames.floor > 0 {
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
fn drop_frame(frames: &mut Frames, regs: &mut [Value]) {
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
    let consts = base + code.consts_at as usize..base + code.consts_at as usize + frame.func.consts.len();
    for r in &mut regs[base..consts.start] {
        clear(r);
    }
    for r in &mut regs[consts.end..base + code.nregs as usize] {
        clear(r);
    }
    frame.this = None;
    frame.scope = None;
    frame.called = None;
    frame.generator = None;
}

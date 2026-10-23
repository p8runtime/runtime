//! What a script hands the engine to call: error and exception handlers,
//! autoloaders, shutdown functions.
//!
//! Each runs where the engine holds the running frames' registers: an
//! error handler after the instruction that raised the diagnostic, an
//! autoloader before the instruction that wanted the class runs again,
//! shutdown functions while the main frame is still there (its globals).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend.c (zend_error_impl),
//! $PHP_SRC_PATH/ext/spl/php_spl.c (spl_autoload_call),
//! $PHP_SRC_PATH/main/main.c (php_call_shutdown_functions)

use crate::Context;
use p8_runtime::ext::Throw;
use p8_runtime::object::Class;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

impl Context<'_> {
    /// The class `name` (as written, any case): a declared one; else, if
    /// autoloaders may declare it, Throw::Autoload (the engine runs them
    /// and the instruction again); else "Class not found".
    pub(crate) fn find_class(&mut self, name: &[u8]) -> Result<Rc<Class>, Throw> {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        let lower = name.to_ascii_lowercase();
        if let Some(c) = self.classes.get(&lower) {
            return Ok(Rc::clone(c));
        }
        if let Some(c) = self.builtin_on_demand(&lower) {
            return Ok(c);
        }
        if !self.autoloaders.is_empty() && self.autoload_tried.insert(lower) {
            return Err(Throw::Autoload(PhpStr::from(name)));
        }
        Err(self.throw("Error", format!("Class \"{}\" not found", String::from_utf8_lossy(name))))
    }

    /// A builtin class declared when first wanted (Reflection, the
    /// extensions' classes): its parent and interfaces first. Building
    /// them all at startup cost every run ~0.8M instructions.
    pub(crate) fn builtin_on_demand(&mut self, lower: &[u8]) -> Option<Rc<Class>> {
        let spec: &'static p8_runtime::ext::ClassSpec = crate::reflection::CLASSES
            .iter()
            .chain(self.isolate.registry.classes().iter().copied())
            .find(|s| s.name.as_bytes().eq_ignore_ascii_case(lower))?;
        for dep in spec.parent.iter().chain(spec.interfaces.iter()) {
            let dep = dep.to_ascii_lowercase().into_bytes();
            if !self.classes.contains_key(&dep) {
                self.builtin_on_demand(&dep)?;
            }
        }
        crate::builtin_classes::declare_spec(&mut self.classes, spec);
        self.classes.get(lower).cloned()
    }

    /// The class `name` if declared, after the autoloaders had their go
    /// (class_exists($name, true) and the like, from a builtin).
    pub(crate) fn class_if_loaded(&mut self, name: &[u8]) -> Result<Option<Rc<Class>>, Throw> {
        match self.find_class(name) {
            Ok(c) => Ok(Some(c)),
            Err(Throw::Autoload(n)) => Err(Throw::Autoload(n)),
            Err(_) => Ok(None),
        }
    }

    /// Run the autoloaders on `name`, in order, until one declares it.
    pub(crate) fn run_autoload(&mut self, regs: &mut Vec<Value>, name: PhpStr) -> Result<(), Throw> {
        let lower = name.as_bytes().to_ascii_lowercase();
        for loader in self.autoloaders.clone() {
            self.call_with_regs(regs, &loader, &mut [Value::Str(name.clone())])?;
            if self.classes.contains_key(&lower) {
                break;
            }
        }
        Ok(())
    }

    /// Call the error handler for each diagnostic waiting: a handler that
    /// returns false lets PHP's own report show it.
    pub(crate) fn dispatch_errors(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        let pending = std::mem::take(&mut self.pending_errors);
        let Some((handler, _)) = self.error_handlers.last().cloned() else { return Ok(()) };
        for (level, message, file, line) in pending {
            self.in_handler = true;
            let mut args = [
                Value::Int(level),
                Value::Str(PhpStr::from(message.as_str())),
                Value::Str(PhpStr::from(&*file)),
                Value::Int(i64::from(line)),
            ];
            let r = self.call_with_regs(regs, &handler, &mut args);
            self.in_handler = false;
            if let Value::Bool(false) = r? {
                self.print_diag(level, &message, &file, line);
            }
        }
        Ok(())
    }

    /// The destructors due (objects whose last reference went), in order.
    pub(crate) fn run_destructors(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        self.run_destructors_at(regs, None)
    }

    /// The destructors due, called from the instruction before `pc` of
    /// the top frame (where a trace shows the call).
    pub(crate) fn run_destructors_at(&mut self, regs: &mut Vec<Value>, pc: Option<usize>) -> Result<(), Throw> {
        let saved = self.frames.last().map(|f| f.pc);
        if let (Some(pc), Some(top)) = (pc, self.frames.last_mut()) {
            top.pc = pc as u32;
        }
        let mut r = Ok(());
        while let Some(object) = self.ids.next_pending() {
            if let Some(m) = self.destructor(&object) {
                r = self.run_nested(regs, m, &mut []).map(|_| ());
                if r.is_err() {
                    break;
                }
            }
        }
        if let (Some(pc), Some(top)) = (saved, self.frames.last_mut()) {
            top.pc = pc;
        }
        r
    }

    /// The end of the script: shutdown functions, then (as PHP does) the
    /// globals that alone hold an object, last first, until none is left;
    /// then every object still alive, oldest first.
    pub(crate) fn script_end(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        if !self.shutdown.is_empty() {
            self.run_shutdown(regs)?;
        }
        let ncv = self.frames.last().map_or(0, |f| f.func.code.consts_at as usize);
        self.end_destructors(regs, ncv)
    }

    /// The destructors at the end: the globals (the main frame's first
    /// `ncv` registers) that alone hold an object, last first, until none
    /// is left; then every object still alive, oldest first.
    pub(crate) fn end_destructors(&mut self, regs: &mut Vec<Value>, ncv: usize) -> Result<(), Throw> {
        loop {
            let mut freed = false;
            for r in (0..ncv).rev() {
                if let Value::Object(o) = &regs[r]
                    && Rc::strong_count(o) == 1
                {
                    regs[r] = Value::Undef;
                    freed = true;
                    self.run_destructors(regs)?;
                }
            }
            if !freed {
                break;
            }
        }
        self.destruct_remaining(regs)
    }

    /// Destruct every object (with a destructor) still alive, oldest first.
    pub(crate) fn destruct_remaining(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        for object in self.ids.undestructed() {
            if object.destructed.replace(true) {
                continue;
            }
            if let Some(m) = self.destructor(&object) {
                self.run_nested(regs, m, &mut [])?;
            }
            self.run_destructors(regs)?;
        }
        Ok(())
    }

    /// register_shutdown_function's functions, in order (one may add more).
    pub(crate) fn run_shutdown(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        while !self.shutdown.is_empty() {
            let (callable, mut args) = self.shutdown.remove(0);
            self.call_with_regs(regs, &callable, &mut args)?;
            if !self.pending_errors.is_empty() {
                self.dispatch_errors(regs)?;
            }
        }
        Ok(())
    }

    /// An exception nothing caught: the exception handler's, if one is set.
    pub(crate) fn handle_uncaught(&mut self, regs: &mut Vec<Value>, t: Throw) -> Throw {
        let Throw::Object(e) = t else { return t };
        let Some(handler) = self.exception_handlers.last().cloned().filter(|h| !matches!(h, Value::Null)) else {
            return Throw::Object(e);
        };
        // The handler is called once; an exception it throws is uncaught.
        self.exception_handlers.clear();
        match self.call_with_regs(regs, &handler, &mut [Value::Object(e)]) {
            Ok(_) => Throw::Exit(0),
            Err(t) => t,
        }
    }
}

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
        if !self.autoloaders.is_empty() && self.autoload_tried.insert(lower) {
            return Err(Throw::Autoload(PhpStr::from(name)));
        }
        Err(self.throw("Error", format!("Class \"{}\" not found", String::from_utf8_lossy(name))))
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

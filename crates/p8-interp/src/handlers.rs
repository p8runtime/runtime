//! What a script hands the engine to call: error and exception handlers,
//! autoloaders, shutdown functions.
//!
//! Each runs where the engine holds the running frames' registers: an
//! error handler after the instruction that raised the diagnostic, an
//! autoloader where a builtin looks the class up (zend_lookup_class) or
//! before the instruction that wanted the class runs again, shutdown
//! functions while the main frame is still there (its globals).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend.c (zend_error_impl),
//! $PHP_SRC_PATH/ext/spl/php_spl.c (spl_autoload_call),
//! $PHP_SRC_PATH/main/main.c (php_call_shutdown_functions)

use crate::Context;
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::object::Class;
use p8_runtime::string::PhpStr;
use p8_runtime::symbol::Sym;
use p8_runtime::value::Value;
use std::rc::Rc;

impl Context<'_> {
    /// The class `name` (as written, any case): a declared one; else one
    /// the autoloaders declare; else "Class not found". A builtin's lookup
    /// (the registers lent) runs them right here, as zend_lookup_class
    /// does, so their frames sit above the builtin's; an instruction's
    /// gets Throw::Autoload (the engine runs them and the instruction
    /// again: PHP shows no internal frame there either).
    pub(crate) fn find_class(&mut self, name: &[u8]) -> Result<Rc<Class>, Throw> {
        match self.load_class(name)? {
            Some(c) => Ok(c),
            // Named as written: a dynamic name keeps its leading `\`.
            None => Err(self.throw("Error", format!("Class \"{}\" not found", String::from_utf8_lossy(name)))),
        }
    }

    /// find_class(), None where no class of that name is there after the
    /// autoloaders ran.
    fn load_class(&mut self, name: &[u8]) -> Result<Option<Rc<Class>>, Throw> {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        // A name no code ever named is no class's (only the autoloaders
        // may declare it).
        if let Some(key) = self.sym_of(name) {
            if let Some(c) = self.classes.get(&key) {
                return Ok(Some(Rc::clone(c)));
            }
            if let Some(c) = self.builtin_on_demand(key) {
                return Ok(Some(c));
            }
        }
        // Each lookup asks the autoloaders, as PHP's do, except while they
        // are running for this class (no recursion) and in the retry of
        // the instruction that asked them.
        // Only a name that can be a class's is autoloaded (zend_is_valid_class_name):
        // never "" or "a-b".
        let valid = !name.is_empty() && name.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80);
        if valid && !self.autoloaders.is_empty() {
            let lower = name.to_ascii_lowercase();
            if self.autoload_retry.as_ref() == Some(&lower) {
                self.autoload_retry = None;
            } else if !self.isolate.regs.is_empty() {
                return self.autoload_here(name, lower);
            } else if self.autoload_tried.insert(lower) {
                return Err(Throw::Autoload(PhpStr::from(name)));
            }
        }
        Ok(None)
    }

    /// Run the autoloaders on `name` where the lookup is, on the lent
    /// registers: the class if one declared it.
    fn autoload_here(&mut self, name: &[u8], lower: Vec<u8>) -> Result<Option<Rc<Class>>, Throw> {
        if !self.autoload_tried.insert(lower.clone()) {
            return Ok(None);
        }
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = self.run_loaders(&mut regs, PhpStr::from(name), &lower);
        self.isolate.regs = regs;
        self.autoload_tried.remove(&lower);
        r?;
        Ok(self.class_named(&lower).cloned())
    }

    /// A builtin class declared when first wanted (Reflection, the
    /// extensions' classes): its parent and interfaces first. Building
    /// them all at startup cost every run ~0.8M instructions.
    pub(crate) fn builtin_on_demand(&mut self, key: Sym) -> Option<Rc<Class>> {
        let spec: &'static p8_runtime::ext::ClassSpec = self.isolate.on_demand.get(&key)?;
        for dep in spec.parent.iter().chain(spec.interfaces.iter()) {
            let dep = self.sym_of(dep.as_bytes())?;
            if !self.classes.contains_key(&dep) {
                self.builtin_on_demand(dep)?;
            }
        }
        let make_data = self.isolate.registry.data_maker(spec.name).or_else(|| (spec.name == "Fiber").then_some(crate::fiber::make_fiber as p8_runtime::object::MakeData));
        let dtor = self.isolate.registry.native_dtor(spec.name).or_else(|| (spec.name == "Fiber").then_some(crate::fiber::destroy as p8_runtime::object::NativeMethod));
        let hooks = self.isolate.registry.class_hooks(spec.name);
        crate::builtin_classes::declare_spec(&mut crate::builtin_classes::Classes { table: &mut self.classes, symbols: self.isolate.registry.symbols() }, spec, make_data, dtor);
        if let Some(h) = hooks
            && let Some(c) = self.classes.get_mut(&key)
            && let Some(c) = Rc::get_mut(c)
        {
            c.get_properties = h.get_properties;
            c.cast_bool = h.cast_bool;
            c.cast_string = h.cast_string;
            c.cast_int = h.cast_int;
            c.clone_data = h.clone_data;
            c.forward_calls = h.forward_calls;
        }
        self.classes.get(&key).cloned()
    }

    /// The class `name` if declared, after the autoloaders had their go
    /// (class_exists($name, true) and the like, from a builtin).
    pub(crate) fn class_if_loaded(&mut self, name: &[u8]) -> Result<Option<Rc<Class>>, Throw> {
        match self.load_class(name) {
            Err(Throw::Autoload(n)) => Err(self.unlent_autoload(n)),
            r => r,
        }
    }

    /// A class a builtin wanted autoloaded without the registers lent: one
    /// not marked as reentering (Registry::mark_reenters) cannot run the
    /// autoloaders where it is, and is not run again after them.
    #[cold]
    pub(crate) fn unlent_autoload(&mut self, name: PhpStr) -> Throw {
        self.autoload_tried.remove(&name.as_bytes().to_ascii_lowercase());
        Ctx::fatal(self, format!("a builtin not marked as reentering autoloaded \"{}\"", String::from_utf8_lossy(name.as_bytes())))
    }

    /// Run the autoloaders on `name`, in order, until one declares it.
    fn run_loaders(&mut self, regs: &mut Vec<Value>, name: PhpStr, lower: &[u8]) -> Result<(), Throw> {
        for loader in self.autoloaders.clone() {
            self.call_with_regs(regs, &loader, &mut [Value::Str(name.clone())])?;
            if self.class_named(lower).is_some() {
                break;
            }
        }
        Ok(())
    }

    /// Run the autoloaders on `name` for the instruction that asked (it
    /// runs again after them).
    pub(crate) fn run_autoload(&mut self, regs: &mut Vec<Value>, name: PhpStr) -> Result<(), Throw> {
        let lower = name.as_bytes().to_ascii_lowercase();
        let result = self.run_loaders(regs, name, &lower);
        self.autoload_tried.remove(&lower);
        if result.is_ok() && self.class_named(&lower).is_none() {
            self.autoload_retry = Some(lower);
        }
        result
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
                // An E_USER_ERROR the handler declines is fatal.
                if level == 256 {
                    return Err(Throw::Fatal { message, file, line });
                }
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
        // What is due now, as one batch, in the order it went: each
        // destructor runs to its end before the next starts (the nested run
        // a destructor is sees only what it frees itself), as PHP calls
        // them one by one where each object goes.
        'batches: loop {
            let mut batch = self.ids.take_pending();
            if batch.is_empty() {
                break;
            }
            while let Some(object) = batch.pop_front() {
                self.ids.running_destructor(true);
                if let Some(dtor) = object.class.native_dtor {
                    r = self.run_native_dtor(regs, dtor, &object).map(|_| ());
                }
                if r.is_ok()
                    && let Some(m) = self.destructor(&object)
                {
                    r = self.run_nested(regs, m, &mut []).map(|_| ());
                }
                // Let go of it (its id goes back unless something kept it),
                // then the ids that waited for this destructor.
                drop(object);
                self.ids.running_destructor(false);
                self.ids.destructor_ran();
                if r.is_err() {
                    self.ids.requeue(batch);
                    break 'batches;
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
                // (Resources stay: PHP frees them after the output buffers,
                // a user-space stream's stream_close then.)
                if let Value::Object(o) = &regs[r]
                    && Rc::strong_count(o) == 1
                    && o.class.flags & p8_runtime::object::CLASS_RESOURCE == 0
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
        // What the globals still hold lives on past the output buffers, as
        // PHP's symbol table does (a resource is freed then: a user-space
        // stream's stream_close runs after the buffers are flushed).
        let kept = regs[..ncv.min(regs.len())].iter().filter(|v| !matches!(v, Value::Undef)).cloned();
        self.ended_globals.extend(kept);
        self.destruct_remaining(regs)
    }

    /// A native class's cleanup, with the registers lent so that it may call
    /// back into PHP (a ZipArchive closed on destruction runs its callbacks).
    fn run_native_dtor(&mut self, regs: &mut Vec<Value>, dtor: p8_runtime::object::NativeMethod, object: &Rc<p8_runtime::object::Object>) -> Result<Value, Throw> {
        self.isolate.regs = std::mem::take(regs);
        let r = dtor(self, object, &mut []);
        *regs = std::mem::take(&mut self.isolate.regs);
        r
    }

    /// Destruct every object (with a destructor) still alive, oldest first.
    pub(crate) fn destruct_remaining(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        for object in self.ids.undestructed() {
            if object.destructed.replace(true) {
                continue;
            }
            if let Some(dtor) = object.class.native_dtor {
                self.run_native_dtor(regs, dtor, &object)?;
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
            self.shutdown_names.remove(0);
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

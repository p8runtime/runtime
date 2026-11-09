//! Lazy objects (PHP 8.4): an object whose initialization waits until one
//! of its lazy properties is used. A ghost is initialized in place by its
//! initializer; a proxy's factory returns the real instance, which every
//! property access then goes to.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_lazy_objects.c

use crate::Context;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::object::{CLASS_ABSTRACT, CLASS_ENUM, CLASS_INTERFACE, CLASS_TRAIT, Class, LAZY_PROXY, LAZY_UNINIT, Lazy, Object, Slot};
use p8_runtime::value::Value;
use std::rc::Rc;

/// ReflectionClass::SKIP_INITIALIZATION_ON_SERIALIZE and SKIP_DESTRUCTOR.
pub(crate) const SKIP_INITIALIZATION_ON_SERIALIZE: i64 = 1 << 3;
pub(crate) const SKIP_DESTRUCTOR: i64 = 1 << 4;

fn text(s: &p8_runtime::string::PhpStr) -> String {
    String::from_utf8_lossy(s.as_bytes()).into_owned()
}

impl Context<'_> {
    /// Whether an access to property `name` of lazy `o` reaches its
    /// backing store: a lazy declared property, or one not declared and not
    /// set dynamically.
    pub(crate) fn lazy_affects(&self, o: &Object, name: &[u8]) -> bool {
        let scope = self.scope().0;
        match o.class.slot(name, scope.as_ref().map(|s| s.lower.as_slice())) {
            Some(i) => o.lazy_slot(i),
            None => !o.dynamic().contains_key(&Key::from_string(p8_runtime::string::PhpStr::new(name.to_vec()))),
        }
    }

    /// The uninitialized lazy object an access to `name` on `target` must
    /// initialize first.
    pub(crate) fn lazy_pending(&self, target: &Value, name: &Value) -> Option<Rc<Object>> {
        let o = target.as_object()?;
        if !o.is_uninit_lazy() {
            return None;
        }
        let name = name.to_php_string()?;
        self.lazy_affects(o, name.as_bytes()).then(|| Rc::clone(o))
    }

    /// Where an access to `name` on an initialized proxy goes: its real
    /// instance (followed through proxies of proxies).
    pub(crate) fn lazy_forward(&self, target: &Value, name: &Value) -> Option<Value> {
        let o = target.as_object()?;
        if o.lazy.get() != LAZY_PROXY {
            return None;
        }
        let name = name.to_php_string()?;
        if !self.lazy_affects(o, name.as_bytes()) {
            return None;
        }
        let mut inst = o.proxy_instance()?;
        while let Some(next) = inst.proxy_instance() {
            inst = next;
        }
        Some(Value::object(inst))
    }

    /// zend_object_make_lazy: `obj` (or a new instance of `class`) made
    /// lazy with `initializer`. Its declared properties become lazy.
    pub(crate) fn make_lazy(&mut self, class: &Rc<Class>, obj: Option<Rc<Object>>, initializer: Value, options: i64, proxy: bool) -> Result<Rc<Object>, Throw> {
        if class.flags & p8_runtime::object::CLASS_INTERNAL != 0 && class.lower.as_slice() != b"stdclass" {
            return Err(self.throw("Error", format!("Cannot make instance of internal class lazy: {} is internal", text(&class.name))));
        }
        let mut parent = class.parent.clone();
        while let Some(p) = parent {
            if p.flags & p8_runtime::object::CLASS_INTERNAL != 0 && p.lower.as_slice() != b"stdclass" {
                return Err(self.throw("Error", format!("Cannot make instance of internal class lazy: {} inherits internal class {}", text(&class.name), text(&p.name))));
            }
            parent = p.parent.clone();
        }
        let o = match obj {
            None => {
                let what = if class.flags & CLASS_INTERFACE != 0 {
                    Some("interface")
                } else if class.flags & CLASS_TRAIT != 0 {
                    Some("trait")
                } else if class.flags & CLASS_ENUM != 0 {
                    Some("enum")
                } else if class.flags & CLASS_ABSTRACT != 0 {
                    Some("abstract class")
                } else {
                    None
                };
                if let Some(w) = what {
                    return Err(self.throw("Error", format!("Cannot instantiate {w} {}", text(&class.name))));
                }
                p8_runtime::ext::Ctx::new_object(self, class)
            }
            Some(o) => {
                if o.lazy.get() == LAZY_PROXY {
                    // An initialized proxy becomes lazy again.
                    o.lazy.set(0);
                    *o.lazy_info_mut() = None;
                } else {
                    if o.lazy_info().is_some() {
                        return Err(self.throw("Error", "Can not reset an object while it is being initialized"));
                    }
                    if options & SKIP_DESTRUCTOR == 0 && !o.destructed.get() && o.class.method(b"__destruct").is_some() {
                        o.destructed.set(true);
                        let m = self.method_frame(&Value::object(Rc::clone(&o)), Some(p8_runtime::symbol::known::DESTRUCT), b"__destruct")?;
                        let mut regs = std::mem::take(&mut self.isolate.regs);
                        let r = self.run_nested(&mut regs, m, &mut []);
                        self.isolate.regs = regs;
                        r?;
                    }
                    o.destructed.set(false);
                }
                *o.dynamic_mut() = PhpArray::new();
                o
            }
        };
        // Declared properties become lazy (an initialized readonly property
        // of a final class or a final property stays).
        let n = o.class.properties.len();
        let mut lazy_slots = vec![false; n];
        let mut count = 0;
        {
            let slots = o.slots();
            for (i, p) in o.class.properties.iter().enumerate() {
                if p.is_internal() || p.hooks & p8_runtime::object::HOOK_VIRTUAL != 0 {
                    continue;
                }
                let keep = p.readonly && !slots[i].peek(Value::is_undef) && o.class.flags & p8_runtime::object::CLASS_FINAL != 0;
                if keep {
                    continue;
                }
                slots[i].set(Value::UNDEF);
                lazy_slots[i] = true;
                count += 1;
            }
        }
        if count == 0 {
            return Ok(o);
        }
        *o.lazy_info_mut() = Some(Box::new(Lazy { initializer, options, lazy_slots, instance: None }));
        o.lazy.set(LAZY_UNINIT | if proxy { LAZY_PROXY } else { 0 });
        Ok(o)
    }

    /// The defaults a lazy slot takes when initialized or skipped.
    fn slot_defaults(&mut self, class: &Rc<Class>) -> Result<Vec<Value>, Throw> {
        p8_runtime::ext::Ctx::class_defaults(self, class)
    }

    /// zend_lazy_object_mark_as_initialized: lazy properties take their
    /// defaults, without the initializer.
    pub(crate) fn lazy_mark_initialized(&mut self, o: &Rc<Object>) -> Result<(), Throw> {
        let defaults = self.slot_defaults(&o.class)?;
        let lazy = o.lazy_info_mut().take();
        if let Some(l) = lazy {
            let slots = o.slots();
            for (i, is_lazy) in l.lazy_slots.iter().enumerate() {
                if *is_lazy && let Some(d) = defaults.get(i) {
                    slots[i].set(d.clone());
                }
            }
        }
        o.lazy.set(0);
        Ok(())
    }

    /// Slot `i` stops being lazy (skipLazyInitialization, a raw write): the
    /// object is no longer lazy once no slot is.
    pub(crate) fn lazy_slot_done(&mut self, o: &Rc<Object>, i: usize) {
        let empty = {
            let mut info = o.lazy_info_mut();
            let Some(l) = info.as_mut() else { return };
            if let Some(s) = l.lazy_slots.get_mut(i) {
                *s = false;
            }
            !l.lazy_slots.iter().any(|&s| s)
        };
        if empty && o.is_uninit_lazy() {
            *o.lazy_info_mut() = None;
            o.lazy.set(0);
        }
    }

    /// zend_lazy_object_init: initialize `o` (with `regs` the running
    /// frames' registers): the object property accesses go to afterwards.
    pub(crate) fn lazy_init(&mut self, regs: &mut Vec<Value>, o: &Rc<Object>) -> Result<Rc<Object>, Throw> {
        if let Some(inst) = o.proxy_instance() {
            return if inst.is_uninit_lazy() { self.lazy_init(regs, &inst) } else { Ok(inst) };
        }
        if !o.is_uninit_lazy() {
            return Ok(Rc::clone(o));
        }
        let initializer = o.lazy_info().as_ref().map(|l| l.initializer.clone()).unwrap_or(Value::NULL);
        if o.lazy.get() & LAZY_PROXY != 0 {
            return self.lazy_init_proxy(regs, o, initializer);
        }
        // A ghost: lazy properties take their defaults, the initializer
        // fills them in; a failure puts everything back.
        let flags = o.lazy.get();
        o.lazy.set(0);
        let snapshot = o.slot_values();
        let dyn_snapshot = o.dynamic().clone();
        let defaults = self.slot_defaults(&o.class)?;
        {
            let info = o.lazy_info();
            let lazy_slots = info.as_ref().map(|l| l.lazy_slots.clone()).unwrap_or_default();
            let slots = o.slots();
            for (i, is_lazy) in lazy_slots.iter().enumerate() {
                if *is_lazy && let Some(d) = defaults.get(i) {
                    slots[i].set(d.clone());
                }
            }
        }
        let r = self.call_with_regs(regs, &initializer, &mut [Value::object(Rc::clone(o))]);
        let revert = |o: &Rc<Object>| {
            o.set_slot_values(&snapshot);
            *o.dynamic_mut() = dyn_snapshot.clone();
            o.lazy.set(flags);
        };
        match r {
            Err(t) => {
                revert(o);
                Err(t)
            }
            Ok(v) if !v.deref().is_null() => {
                revert(o);
                Err(self.throw("TypeError", "Lazy object initializer must return NULL or no value"))
            }
            Ok(_) => {
                *o.lazy_info_mut() = None;
                Ok(Rc::clone(o))
            }
        }
    }

    fn lazy_init_proxy(&mut self, regs: &mut Vec<Value>, o: &Rc<Object>, factory: Value) -> Result<Rc<Object>, Throw> {
        // No reentrant initialization while the factory runs.
        o.lazy.set(0);
        let r = self.call_with_regs(regs, &factory, &mut [Value::object(Rc::clone(o))]);
        let restore = |o: &Rc<Object>| o.lazy.set(LAZY_UNINIT | LAZY_PROXY);
        let v = match r {
            Ok(v) => v.deref(),
            Err(t) => {
                restore(o);
                return Err(t);
            }
        };
        let inst = match v.into_object() {
            Ok(inst) => inst,
            Err(v) => {
                restore(o);
                return Err(self.throw(
                    "TypeError",
                    format!(
                        "Lazy proxy factory must return an instance of a class compatible with {}, {} returned",
                        text(&o.class.name),
                        p8_runtime::args::value_name(&v)
                    ),
                ));
            }
        };
        let compatible = Rc::ptr_eq(&inst.class, &o.class)
            || (o.class.is_a(&inst.class.lower)
                && o.class.properties.len() == inst.class.properties.len()
                && o.class.method(b"__destruct").map(|m| m.declaring.clone()) == inst.class.method(b"__destruct").map(|m| m.declaring.clone())
                && o.class.method(b"__clone").map(|m| m.declaring.clone()) == inst.class.method(b"__clone").map(|m| m.declaring.clone()));
        if !compatible {
            restore(o);
            return Err(self.throw(
                "TypeError",
                format!(
                    "The real instance class {} is not compatible with the proxy class {}. The proxy must be a instance of the same class as the real instance, or a sub-class with no additional properties, and no overrides of the __destructor or __clone methods.",
                    text(&inst.class.name),
                    text(&o.class.name)
                ),
            ));
        }
        if Rc::ptr_eq(&inst, o) || inst.lazy.get() != 0 {
            restore(o);
            return Err(self.throw("Error", "Lazy proxy factory must return a non-lazy object"));
        }
        // The proxy's own properties are gone: every access goes to the instance.
        *o.dynamic_mut() = PhpArray::new();
        let n = o.class.properties.len();
        {
            let slots = o.slots();
            for (i, p) in o.class.properties.iter().enumerate() {
                if !p.is_internal() {
                    slots[i].set(Value::UNDEF);
                }
            }
        }
        if let Some(l) = o.lazy_info_mut().as_mut() {
            l.instance = Some(Rc::clone(&inst));
            l.initializer = Value::NULL;
            l.lazy_slots = vec![true; n];
        }
        o.lazy.set(LAZY_PROXY);
        Ok(inst)
    }

    /// For native paths ((array), foreach, clone, ...): initialize `o`
    /// from the registers lent to builtins.
    pub(crate) fn lazy_init_from_builtin(&mut self, o: &Rc<Object>) -> Result<Rc<Object>, Throw> {
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let r = self.lazy_init(&mut regs, o);
        self.isolate.regs = regs;
        r
    }
}

//! Making objects: what `new C` needs from a class, worked out once per
//! class and kept, so each `new` only copies it.
//!
//! PHP's object_init_ex copies the class's default property table into
//! the new object, and ZEND_NEW finds the constructor through the class
//! entry: nothing per object is looked up by name. Here a class's
//! `NewInfo` holds the same: its slots' starting values (a lazy class's
//! computed defaults in place), whether it is a Throwable (its objects
//! record where they were made), and its constructor with the class that
//! declares it. A `new` of a named class finds it through its site's
//! cache (no lookup at all); `new self`, `new static`, `new $name` and
//! the Construct that follows any `new` through the context's table,
//! keyed by the class's address.

use crate::classes::{Code, MethodFrame};
use crate::{Closure, Context};
use p8_runtime::object::{Class, Visibility};
use p8_runtime::value::Value;
use std::rc::{Rc, Weak};

/// What making an object of one class takes, as that class is linked in
/// this request.
pub(crate) struct NewInfo {
    /// The class (held: the table is keyed by its address).
    pub(crate) class: Rc<Class>,
    /// Each slot's starting value.
    pub(crate) defaults: Box<[Value]>,
    /// Its objects record where they were made (file, line, trace).
    pub(crate) throwable: bool,
    pub(crate) ctor: Option<Ctor>,
}

/// A class's constructor: its code, the class declaring it, and its
/// visibility (checked where each `new` is, against that code's class).
pub(crate) struct Ctor {
    pub(crate) func: Code,
    pub(crate) declaring: Rc<Class>,
    pub(crate) visibility: Visibility,
}

impl Ctor {
    /// The constructor's call on `object`.
    pub(crate) fn frame(&self, object: Rc<p8_runtime::object::Object>, class: &Rc<Class>) -> MethodFrame {
        let func = match &self.func {
            Code::User(c) => Code::User(Rc::clone(c)),
            Code::Native(n, name) => Code::Native(*n, name.clone()),
        };
        MethodFrame::method(func, Some(object), Rc::clone(&self.declaring), Rc::clone(class))
    }
}

/// A `new` site's cache (one per class the function names): the NewInfo
/// of the class it made last. Held weakly, so a function's cache keeps no
/// class or constructor alive (a constructor making its own class would
/// otherwise hold itself); the context's table holds it.
pub(crate) type NewSite = Weak<NewInfo>;

impl Context<'_> {
    /// `class`'s NewInfo, worked out the first time it is asked for and
    /// kept; the second value says whether it was kept. A lazy class whose
    /// default code has not finished gets one for now (its objects start
    /// from the declared defaults then, as they always have), not kept.
    pub(crate) fn new_info(&mut self, class: &Rc<Class>) -> (Rc<NewInfo>, bool) {
        let key = Rc::as_ptr(class) as usize;
        if let Some(info) = self.new_infos.get(&key) {
            return (Rc::clone(info), true);
        }
        let lazy = if class.flags & p8_runtime::object::CLASS_LAZY != 0 { Some(self.class_defaults.get(&key).cloned()) } else { None };
        let mut defaults: Vec<Value> = class.properties.iter().map(|p| p.default.clone()).collect();
        if let Some(Some(lazy)) = &lazy {
            for (i, v) in lazy.iter() {
                defaults[*i] = v.clone();
            }
        }
        let ctor = class.method_sym(p8_runtime::symbol::known::CONSTRUCT).map(|m| Ctor { func: self.code_of(m), declaring: self.declaring(class, &m.declaring), visibility: m.visibility });
        let info = Rc::new(NewInfo { class: Rc::clone(class), defaults: defaults.into_boxed_slice(), throwable: class.is_a(b"throwable"), ctor });
        let keep = !matches!(lazy, Some(None));
        if keep {
            self.new_infos.insert(key, Rc::clone(&info));
        }
        (info, keep)
    }

    /// The NewInfo a `new` of class reference `i` of `clo` used last,
    /// while it is there (a named class is always the same one).
    #[inline]
    pub(crate) fn cached_new(clo: &Closure, i: u16) -> Option<Rc<NewInfo>> {
        clo.news.get(i as usize)?.borrow().upgrade()
    }

    /// For the loop's New of a class reference its site does not cache
    /// (`new static`, `new self`, `new parent`): the class known without a
    /// lookup (`static_target`), and its NewInfo if one was kept. Out of
    /// line: a named class's `new` is the arm's straight path.
    #[inline(never)]
    pub(crate) fn new_info_kept(&self, clo: &Closure, i: u16) -> Option<Rc<NewInfo>> {
        let class = self.static_target(clo, i)?;
        self.new_infos.get(&(Rc::as_ptr(&class) as usize)).cloned()
    }

    pub(crate) fn remember_new(clo: &Closure, i: u16, info: &Rc<NewInfo>) {
        if let Some(site) = clo.news.get(i as usize) {
            *site.borrow_mut() = Rc::downgrade(info);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Isolate;
    use crate::tests::{Sink, compile};
    use p8_runtime::ext::Registry;
    use std::rc::Rc;
    use std::sync::Arc;

    /// A `new` of a named class keeps the class's NewInfo (made once) in
    /// its site, so the next one looks nothing up: here 1000 objects of
    /// two classes are made from one function's two sites, and each class
    /// has one NewInfo, its defaults and constructor prepared once.
    #[test]
    fn new_prepares_each_class_once_and_its_site_remembers_it() {
        let registry = Arc::new(Registry::default());
        let source = "<?php
            class A { public $list = [1, 2]; public $n = 0; public function __construct($n) { $this->n = $n; } }
            class B extends A { public $extra = 'b'; }
            function make($i) { $a = new A($i); $b = new B($i); $a->list[] = $i; return $a->n + $b->n + $b->list[1]; }
            $sum = 0;
            for ($i = 0; $i < 500; $i++) { $sum += make($i); }
            if ($sum !== 2 * (499 * 500 / 2) + 1000) { exit(3); }";
        let program = compile(source, &registry);
        let mut isolate = Isolate::new(Arc::clone(&registry));
        let mut host = Sink;
        let mut cx = isolate.new_context(&mut host);
        for f in &program.functions {
            cx.declare(Arc::clone(f)).expect("declares");
        }
        cx.declare_classes(&program.classes);
        assert_eq!(cx.run_main(Arc::clone(&program.main)), 0);
        let a = Rc::clone(cx.class_named(b"a").expect("A"));
        let b = Rc::clone(cx.class_named(b"b").expect("B"));
        let info = |c: &Rc<p8_runtime::object::Class>| Rc::clone(&cx.new_infos[&(Rc::as_ptr(c) as usize)]);
        let (ia, ib) = (info(&a), info(&b));
        assert_eq!(cx.new_infos.len(), 2, "one NewInfo per class");
        // B's constructor is A's, found through B once.
        assert!(Rc::ptr_eq(&ib.ctor.as_ref().expect("inherited").declaring, &a));
        assert_eq!(ib.defaults.len(), 3);
        // The defaults were copied, never changed by an object's writes.
        assert!(ia.defaults[0].as_array().is_some_and(|l| l.len() == 2));
        // make()'s two `new` sites hold their classes' NewInfo.
        let make = cx.closures.iter().find(|c| c.code.name.as_bytes() == b"make").expect("make");
        let held: Vec<_> = make.news.iter().filter_map(|s| s.borrow().upgrade()).collect();
        assert_eq!(held.len(), 2);
        assert!(held.iter().any(|i| Rc::ptr_eq(i, &ia)) && held.iter().any(|i| Rc::ptr_eq(i, &ib)));
    }
}

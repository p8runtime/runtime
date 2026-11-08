//! The call stack: a task's frames over its register file.

use super::*;

pub(crate) struct Frame {
    pub(crate) func: Rc<Closure>,
    /// Where the frame resumes (written when it calls out).
    pub(crate) pc: u32,
    pub(crate) base: u32,
    /// Absolute register of the caller's result (NO_RET for the main frame).
    pub(crate) ret: u32,
    pub(crate) argc: u32,
    /// What the frame has beyond its registers (`flag::*`): one word the
    /// common return tests once, as PHP's `zend_leave_helper` tests
    /// `call_info`. Set with the fields they describe, cleared by `push`.
    pub(crate) flags: u8,
    /// Arguments past the declared parameters (func_get_args()).
    /// and the variables only named at run time (`$$name`, extract()):
    /// rare, so one box that is usually none. Some iff `flag::EXTRA`.
    pub(crate) extra: Option<Box<Extra>>,
    /// A method's `$this`: Some iff `flag::THIS`.
    pub(crate) this: Option<Rc<Object>>,
    /// The class the code is in (visibility, self::) and the class it
    /// was called on (static::), read through `scope()` and `called()`:
    /// meaningful only with `flag::SCOPE`. Without it they are whatever
    /// the slot's last method frame left, kept (not released) so that
    /// the next method call at this depth of the same class finds its
    /// handles already here and touches no reference count (as `func`
    /// is kept, see `Frames`). A class lives for the request, so a kept
    /// handle costs nothing but the slot.
    scope: Option<Rc<Class>>,
    called: Option<Rc<Class>>,
    /// The object whose code this frame runs: in a generator function's
    /// code (`func.code.is_generator`) its Generator, which keeps the
    /// Closure it was made by; else the Closure object the frame was
    /// called as, if it was (Closure::getCurrent()). One field, as PHP
    /// keeps neither in the frame: there the closure is the frame's
    /// function, embedded in the zend_closure (ZEND_CLOSURE_OBJECT).
    /// Some iff `flag::OWNER`.
    pub(crate) owner: Option<Rc<Object>>,
}

/// `Frame::flags`.
pub(crate) mod flag {
    /// `this` is Some.
    pub(crate) const THIS: u8 = 1;
    /// `scope` and `called` are this frame's (a method's, or a closure's
    /// bound to a class).
    pub(crate) const SCOPE: u8 = 2;
    /// `owner` is Some.
    pub(crate) const OWNER: u8 = 4;
    /// Its variables are the global ones: the main script, and a file
    /// included at global scope (at any depth of includes).
    pub(crate) const GLOBAL: u8 = 8;
    /// `extra` is Some.
    pub(crate) const EXTRA: u8 = 16;
    /// What `leave` must do out of line: anything but releasing `$this`.
    pub(crate) const RARE: u8 = OWNER | EXTRA;
}

pub(crate) const NO_RET: u32 = u32::MAX;

/// A handle a call's frame needs: the one its slot kept from the last
/// call at this depth (the same function, the same class: no reference
/// count touched), or a new one.
pub(crate) enum Keep<T> {
    Slot,
    New(T),
}

/// A frame's arguments past its parameters and its run-time variables.
#[derive(Default)]
pub(crate) struct Extra {
    /// (A list: an Extra is kept for the next frame that needs one, room
    /// and all; a new one's list is a kept buffer, given back when the
    /// Extra goes.)
    pub(crate) args: Vec<Value>,
    pub(crate) vars: Vars,
    /// A file's code included in a function's scope: where each of its
    /// variables is the including frame's. Their bindings go back there
    /// when it ends: `global $x`, `$x = &$y`, `unset($x)` in the file
    /// rebind the function's variable, as one symbol table in PHP.
    pub(crate) shared: Option<Box<[Origin]>>,
}

impl Drop for Extra {
    fn drop(&mut self) {
        p8_runtime::array::keep_values_buffer(std::mem::take(&mut self.args));
    }
}

/// Where an included file's variable lives in the scope that included it.
pub(crate) enum Origin {
    /// That frame's variable register (absolute).
    Slot(u32),
    /// That frame's variable only named at run time.
    Var(Vec<u8>),
}

/// The ending frame's variable bindings, back to the scope that included
/// it (when it is a file's code sharing a function's scope).
pub(crate) fn give_back_shared(frames: &mut Frames, regs: &mut [Value]) {
    let n = frames.len;
    let Some(origins) = frames.slots[n - 1].extra.as_mut().and_then(|e| e.shared.take()) else { return };
    let base = frames.slots[n - 1].base as usize;
    for (i, origin) in origins.iter().enumerate() {
        let v = std::mem::take(&mut regs[base + i]);
        match origin {
            Origin::Slot(at) => regs[*at as usize] = v,
            Origin::Var(name) => {
                let vars = &mut frames.slots[n - 2].vars_mut().0;
                let at = vars.iter().position(|(n, _)| n == name);
                match (v.into_reference(), at) {
                    (Ok(cell), Some(at)) => vars[at].1 = cell,
                    (Ok(cell), None) => vars.push((name.clone(), cell)),
                    (Err(_), Some(at)) => {
                        vars.remove(at);
                    }
                    (Err(_), None) => {}
                }
            }
        }
    }
    // Its variables only named at run time (a file it included made them)
    // are the including scope's too.
    let made = frames.slots[n - 1].extra.as_mut().map(|e| std::mem::take(&mut e.vars.0)).unwrap_or_default();
    for (name, cell) in made {
        let below = &mut frames.slots[n - 2];
        match below.func.code.cv_names.iter().position(|c| c.as_bytes() == name.as_slice()) {
            Some(r) => regs[below.base as usize + r] = Value::reference(cell),
            None => {
                let vars = &mut below.vars_mut().0;
                match vars.iter().position(|(n, _)| *n == name) {
                    Some(at) => vars[at].1 = cell,
                    None => vars.push((name, cell)),
                }
            }
        }
    }
}

impl Frame {
    /// The Generator this frame runs, taken (its code returns): none in a
    /// frame that is not a generator's (its owner is a Closure, if any).
    pub(crate) fn take_generator(&mut self) -> Option<Rc<Object>> {
        if self.flags & flag::OWNER != 0 && self.func.code.is_generator {
            self.flags &= !flag::OWNER;
            self.owner.take()
        } else {
            None
        }
    }

    /// The arguments past the declared parameters.
    pub(crate) fn args(&self) -> &[Value] {
        self.extra.as_ref().map_or(&[], |e| &e.args)
    }

    pub(crate) fn vars(&self) -> Option<&Vars> {
        self.extra.as_ref().map(|e| &e.vars)
    }

    pub(crate) fn vars_mut(&mut self) -> &mut Vars {
        &mut self.extra_mut().vars
    }

    /// The frame's Extra, made if it has none.
    pub(crate) fn extra_mut(&mut self) -> &mut Extra {
        self.flags |= flag::EXTRA;
        self.extra.get_or_insert_with(Default::default)
    }

    /// The frame's Extra, taken (the flag goes with it).
    #[inline(always)]
    pub(crate) fn take_extra(&mut self) -> Option<Box<Extra>> {
        self.flags &= !flag::EXTRA;
        self.extra.take()
    }

    #[inline(always)]
    pub(crate) fn is_global(&self) -> bool {
        self.flags & flag::GLOBAL != 0
    }

    /// The class the frame's code is in, if it is in one.
    #[inline(always)]
    pub(crate) fn scope(&self) -> Option<&Rc<Class>> {
        if self.flags & flag::SCOPE != 0 { self.scope.as_ref() } else { None }
    }

    /// The class the method was called on (static::), if the frame is a
    /// method's.
    #[inline(always)]
    pub(crate) fn called(&self) -> Option<&Rc<Class>> {
        if self.flags & flag::SCOPE != 0 { self.called.as_ref() } else { None }
    }

    /// `(scope, called)`, cloned.
    pub(crate) fn classes(&self) -> (Option<Rc<Class>>, Option<Rc<Class>>) {
        (self.scope().cloned(), self.called().cloned())
    }

    /// The frame as a method's (or a class-bound closure's): its `$this`,
    /// its scope and its called class; None for each a function has not.
    pub(crate) fn set_classes(&mut self, this: Option<Rc<Object>>, scope: Option<Rc<Class>>, called: Option<Rc<Class>>) {
        if scope.is_some() || called.is_some() {
            self.flags |= flag::SCOPE;
            self.scope = scope;
            self.called = called;
        }
        self.set_this(this);
    }

    #[inline(always)]
    pub(crate) fn set_this(&mut self, this: Option<Rc<Object>>) {
        if this.is_some() {
            self.flags |= flag::THIS;
            self.this = this;
        }
    }

    pub(crate) fn set_owner(&mut self, owner: Option<Rc<Object>>) {
        if owner.is_some() {
            self.flags |= flag::OWNER;
            self.owner = owner;
        }
    }

    /// Whether the slot's kept scope handle is the class `declaring` (a
    /// call-site cache's weak handle): compared by address, which a kept
    /// handle keeps its own.
    #[inline(always)]
    pub(crate) fn keeps_scope(&self, declaring: &std::rc::Weak<Class>) -> bool {
        self.scope.as_ref().is_some_and(|s| Rc::as_ptr(s) == declaring.as_ptr())
    }

    /// The frame (just pushed) as an instance method's: `$this`, the
    /// declaring class as its scope (the slot's own when `keeps_scope`
    /// said so), `$this`'s class as the called class (the slot's own
    /// when it is the same, the usual case: the same call from the same
    /// depth touches no class reference count).
    #[inline(always)]
    pub(crate) fn adopt(&mut self, this: Rc<Object>, declaring: Keep<Rc<Class>>) {
        if let Keep::New(d) = declaring {
            self.scope = Some(d);
        }
        if self.called.as_ref().is_none_or(|c| !Rc::ptr_eq(c, &this.class)) {
            self.called = Some(Rc::clone(&this.class));
        }
        self.this = Some(this);
        self.flags |= flag::THIS | flag::SCOPE;
    }

    /// `$this` released (the frame ends): nothing else of the method's.
    #[inline(always)]
    pub(crate) fn drop_this(&mut self) {
        self.flags &= !flag::THIS;
        self.this = None;
    }
}

/// A frame's variables only named at run time, in the order made.
#[derive(Default)]
pub(crate) struct Vars(pub(crate) Vec<(Vec<u8>, Rc<RefCell<Value>>)>);

impl Vars {
    pub(crate) fn get(&self, name: &[u8]) -> Option<&Rc<RefCell<Value>>> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, c)| c)
    }

    /// The variable `name`, made (null) if there is none.
    pub(crate) fn cell(&mut self, name: &[u8]) -> Rc<RefCell<Value>> {
        if let Some(c) = self.get(name) {
            return Rc::clone(c);
        }
        let c = Rc::new(RefCell::new(Value::NULL));
        self.0.push((name.to_vec(), Rc::clone(&c)));
        c
    }

    pub(crate) fn remove(&mut self, name: &[u8]) {
        self.0.retain(|(n, _)| n != name);
    }

    /// The variable `name` made the reference `cell`.
    pub(crate) fn bind(&mut self, name: &[u8], cell: Rc<RefCell<Value>>) {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some((_, c)) => *c = cell,
            None => self.0.push((name.to_vec(), cell)),
        }
    }
}

/// PHP's error levels (E_*), for error_reporting.
pub(crate) const E_WARNING: i64 = 2;
pub(crate) const E_NOTICE: i64 = 8;
pub(crate) const E_USER_WARNING: i64 = 512;
pub(crate) const E_USER_NOTICE: i64 = 1024;
pub(crate) const E_DEPRECATED: i64 = 8192;
pub(crate) const E_USER_DEPRECATED: i64 = 16384;
/// The CLI's level with no php.ini: E_ALL except E_STRICT.
pub(crate) const E_DEFAULT: i64 = 30719;

/// The call stack. A popped frame keeps its slot and the closure in it, so
/// calling the same function again from the same depth (what a loop does)
/// reuses the slot without touching the closure's reference count.
#[derive(Default)]
pub(crate) struct Frames {
    pub(crate) slots: Vec<Frame>,
    /// Frames in use: `slots[..len]`.
    pub(crate) len: usize,
    /// The frame the running `execute` started at: a call from a builtin
    /// back into PHP runs its own `execute` above the frames below.
    pub(crate) floor: usize,
    /// Extras of finished frames, emptied, for the next frames that need
    /// one (a variadic call's): no allocation per call. (Boxes: a frame
    /// holds its Extra boxed, and these are those boxes, moved back.)
    #[allow(clippy::vec_box)] // the frames' own boxes, kept to be handed back
    pub(crate) spare: Vec<Box<Extra>>,
}

/// A finished frame's Extra: what it holds goes now, the box is kept (up
/// to a few).
#[inline(never)]
#[allow(clippy::vec_box)] // see `Frames::spare`
pub(crate) fn recycle_extra(spare: &mut Vec<Box<Extra>>, mut extra: Box<Extra>) {
    extra.args.clear();
    extra.vars.0.clear();
    extra.shared = None;
    if spare.len() < 16 {
        spare.push(extra);
    }
}

impl Frames {
    #[inline(always)]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub(crate) fn last(&self) -> Option<&Frame> {
        self.slots[..self.len].last()
    }

    #[inline(always)]
    pub(crate) fn last_mut(&mut self) -> Option<&mut Frame> {
        self.slots[..self.len].last_mut()
    }

    pub(crate) fn first(&self) -> Option<&Frame> {
        self.slots[..self.len].first()
    }

    /// Push a frame running `func`, its window at `base`.
    #[inline(always)]
    pub(crate) fn push(&mut self, func: &Rc<Closure>, base: usize, argc: usize, ret: u32) {
        match self.slots.get_mut(self.len) {
            Some(slot) => {
                if !Rc::ptr_eq(&slot.func, func) {
                    slot.func = Rc::clone(func);
                }
                (slot.pc, slot.base, slot.ret, slot.argc, slot.flags) = (0, base as u32, ret, argc as u32, 0);
            }
            None => self.slots.push(Frame {
                func: Rc::clone(func),
                pc: 0,
                base: base as u32,
                ret,
                argc: argc as u32,
                flags: 0,
                extra: None,
                this: None,
                scope: None,
                called: None,
                owner: None,
            }),
        }
        self.len += 1;
    }

    /// The slot the next pushed frame takes, if a frame has used it: what
    /// it kept (its function, its classes) is the last call's at this depth.
    #[inline(always)]
    pub(crate) fn next_slot(&self) -> Option<&Frame> {
        self.slots.get(self.len)
    }

    /// `push` of the function the next slot keeps (`next_slot`'s `func`):
    /// no handle touched.
    #[inline(always)]
    pub(crate) fn push_kept(&mut self, base: usize, argc: usize, ret: u32) -> &mut Frame {
        let slot = &mut self.slots[self.len];
        (slot.pc, slot.base, slot.ret, slot.argc, slot.flags) = (0, base as u32, ret, argc as u32, 0);
        self.len += 1;
        slot
    }

    /// Pop the top frame; its slot stays for the next push.
    #[inline(always)]
    pub(crate) fn pop(&mut self) -> &mut Frame {
        self.len -= 1;
        &mut self.slots[self.len]
    }

    /// Drop every frame and slot (and the closures they hold).
    pub(crate) fn clear(&mut self) {
        self.slots.clear();
        self.len = 0;
        self.floor = 0;
    }
}

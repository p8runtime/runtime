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
    /// Arguments past the declared parameters (func_get_args()).
    /// and the variables only named at run time (`$$name`, extract()):
    /// rare, so one box that is usually none.
    pub(crate) extra: Option<Box<Extra>>,
    /// A method's `$this`, the class its code is in (visibility, self::)
    /// and the class it was called on (static::); None in a function.
    pub(crate) this: Option<Rc<Object>>,
    pub(crate) scope: Option<Rc<Class>>,
    pub(crate) called: Option<Rc<Class>>,
    /// The object whose code this frame runs: in a generator function's
    /// code (`func.code.is_generator`) its Generator, which keeps the
    /// Closure it was made by; else the Closure object the frame was
    /// called as, if it was (Closure::getCurrent()). One field, as PHP
    /// keeps neither in the frame: there the closure is the frame's
    /// function, embedded in the zend_closure (ZEND_CLOSURE_OBJECT).
    pub(crate) owner: Option<Rc<Object>>,
    /// Its variables are the global ones: the main script, and a file
    /// included at global scope (at any depth of includes).
    pub(crate) global: bool,
}

pub(crate) const NO_RET: u32 = u32::MAX;

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
#[inline(always)]
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
    /// The class `static::` names: the one the frame was called on. A
    /// method frame with `$this` may leave `called` empty, as PHP takes it
    /// from `$this` (`Z_OBJCE(This)`): it is `$this`'s class.
    #[inline(always)]
    pub(crate) fn called_class(&self) -> Option<&Rc<Class>> {
        self.called.as_ref().or_else(|| self.this.as_ref().map(|t| &t.class))
    }

    /// The Generator this frame runs, taken (its code returns): none in a
    /// frame that is not a generator's (its owner is a Closure, if any).
    pub(crate) fn take_generator(&mut self) -> Option<Rc<Object>> {
        if self.owner.is_some() && self.func.code.is_generator { self.owner.take() } else { None }
    }

    /// The arguments past the declared parameters.
    pub(crate) fn args(&self) -> &[Value] {
        self.extra.as_ref().map_or(&[], |e| &e.args)
    }

    pub(crate) fn vars(&self) -> Option<&Vars> {
        self.extra.as_ref().map(|e| &e.vars)
    }

    pub(crate) fn vars_mut(&mut self) -> &mut Vars {
        &mut self.extra.get_or_insert_with(Default::default).vars
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

    /// The running frame, where there must be one (a call's caller): one
    /// bounds check, where `last_mut()` slices and tests for none.
    #[inline(always)]
    pub(crate) fn top_mut(&mut self) -> &mut Frame {
        let n = self.len - 1;
        &mut self.slots[n]
    }

    /// Push a frame running `func`, its window at `base`.
    #[inline(always)]
    pub(crate) fn push(&mut self, func: &Rc<Closure>, base: usize, argc: usize, ret: u32) -> &mut Frame {
        self.push_code(Some(func), base, argc, ret)
    }

    /// A call's frame above the running one, which goes on at `resume`
    /// when it returns: `func` as `site_code` gave it (None: the slot the
    /// push reuses runs it already). The frame pushed.
    #[inline(always)]
    pub(crate) fn call(&mut self, func: Option<&Rc<Closure>>, resume: usize, base: usize, argc: usize, ret: u32) -> &mut Frame {
        self.top_mut().pc = resume as u32;
        self.push_code(func, base, argc, ret)
    }

    /// The code a call site's cache keeps (a `Weak`, so that no closure
    /// keeps itself) for the frame a call pushes next: `Some(None)` when
    /// the slot that push reuses already runs it (no count is taken, as
    /// `push` takes none for the same function called again from the
    /// same depth), `Some(code)` otherwise, None when the code is gone.
    #[inline(always)]
    pub(crate) fn site_code(&self, code: &std::rc::Weak<Closure>) -> Option<Option<Rc<Closure>>> {
        match self.slots.get(self.len) {
            Some(slot) if std::ptr::eq(Rc::as_ptr(&slot.func), code.as_ptr()) => Some(None),
            _ => {
                p8_runtime::counts::bump(p8_runtime::counts::Count::CodeCounted);
                code.upgrade().map(Some)
            }
        }
    }

    /// The function `site_code` gave (`func`), for the checks before the
    /// push: that one, or the next slot's.
    #[inline(always)]
    pub(crate) fn next_code<'a>(&'a self, func: Option<&'a Rc<Closure>>) -> &'a Closure {
        match func {
            Some(f) => f,
            None => &self.slots[self.len].func,
        }
    }

    /// `push`, of `func` (None: the next slot's, as `site_code` found it).
    #[inline(always)]
    fn push_code(&mut self, func: Option<&Rc<Closure>>, base: usize, argc: usize, ret: u32) -> &mut Frame {
        let n = self.len;
        self.len = n + 1;
        if n >= self.slots.len() {
            return self.grow(func.expect("a new slot's function"), base, argc, ret);
        }
        let slot = &mut self.slots[n];
        if let Some(func) = func
            && !Rc::ptr_eq(&slot.func, func)
        {
            slot.func = Rc::clone(func);
        }
        (slot.pc, slot.base, slot.ret, slot.argc, slot.global) = (0, base as u32, ret, argc as u32, false);
        slot
    }

    /// A frame deeper than any so far: a new slot.
    #[cold]
    #[inline(never)]
    fn grow(&mut self, func: &Rc<Closure>, base: usize, argc: usize, ret: u32) -> &mut Frame {
        self.slots.push(Frame {
            func: Rc::clone(func),
            pc: 0,
            base: base as u32,
            ret,
            argc: argc as u32,
            extra: None,
            this: None,
            scope: None,
            called: None,
            owner: None,
            global: false,
        });
        self.slots.last_mut().expect("the slot just pushed")
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

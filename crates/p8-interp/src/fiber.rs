//! Fibers: a function run on its own call stack, which it can leave with
//! Fiber::suspend() and come back to with resume().
//!
//! A fiber runs in a nested `execute` above the frames that started (or
//! resumed) it. Fiber::suspend() sets a flag the loop stops at; the frames
//! above the fiber's first one, and their registers, move into the Fiber.
//! resume() puts them back above whatever frames are running then (their
//! register windows moved by the difference) and runs them again, the
//! value given becoming suspend()'s result.
//!
//! A fiber can't be suspended from PHP code a builtin called back (an
//! array_map callback, a generator being resumed): that code runs in a
//! further nested `execute` whose Rust frames can't be put aside.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_fibers.c

use crate::classes::{Code, MethodFrame};
use crate::{Context, Frame, NO_RET, Next};
use p8_bytecode::Instr;
use p8_runtime::ext::{ClassSpec, Ctx, Throw};
use p8_runtime::object::{CLASS_FINAL, CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Status {
    #[default]
    Init,
    Running,
    Suspended,
    Terminated,
}

#[derive(Default)]
pub(crate) struct FiberState {
    status: Status,
    callable: Value,
    /// While suspended: its frames (the first one is the fiber function's)
    /// and the registers from `at` on.
    frames: Vec<Frame>,
    regs: Vec<Value>,
    at: usize,
    returned: Option<Value>,
    threw: bool,
    /// Being destroyed while suspended: unwinding (finally blocks run, no
    /// catch sees it, suspending is an error).
    closing: bool,
}

pub(crate) struct FiberCell(RefCell<FiberState>);

pub(crate) fn make_fiber() -> Rc<dyn std::any::Any> {
    Rc::new(FiberCell(RefCell::new(FiberState::default())))
}

fn state(o: &Object) -> &RefCell<FiberState> {
    &o.native::<FiberCell>().expect("a Fiber").0
}

/// A fiber running now: where its frames and registers start.
pub(crate) struct FiberRun {
    fiber: Rc<Object>,
    base_frame: usize,
}

/// What a Fiber method asks the engine for (Ctx::fiber_op).
pub(crate) const START: u8 = 0;
pub(crate) const RESUME: u8 = 1;
pub(crate) const THROW: u8 = 2;
pub(crate) const SUSPEND: u8 = 3;
pub(crate) const CURRENT: u8 = 4;
pub(crate) const DESTROY: u8 = 5;

impl Context<'_> {
    /// A FiberError, made now: its trace shows the Fiber method it came from.
    fn fiber_error(&mut self, message: &str) -> Throw {
        let t = self.throw("FiberError", message.to_owned());
        self.exception(&|_| Value::Undef, t)
    }

    /// Where a new nested run's registers start: past the top frame's window.
    fn window_end(&self) -> usize {
        match self.frames.last() {
            Some(top) => top.base as usize + top.func.code.nregs as usize,
            None => 0,
        }
    }

    pub(crate) fn fiber_op(&mut self, fiber: Option<&Rc<Object>>, op: u8, args: &mut [Value]) -> Result<Value, Throw> {
        // Stack traces show the Fiber method (and the fiber's frames as
        // called from it), as PHP's do for internal calls.
        let name = match op {
            START => "Fiber->start",
            RESUME => "Fiber->resume",
            THROW => "Fiber->throw",
            SUSPEND => "Fiber::suspend",
            DESTROY => "Fiber->__destruct",
            _ => "Fiber::getCurrent",
        };
        self.natives.push(crate::NativeCall { depth: self.frames.len(), name, args: args.iter().map(Value::deref).collect(), top: true });
        let r = self.fiber_op_inner(fiber, op, args);
        self.natives.pop();
        r
    }

    fn fiber_op_inner(&mut self, fiber: Option<&Rc<Object>>, op: u8, args: &mut [Value]) -> Result<Value, Throw> {
        match op {
            SUSPEND => self.fiber_suspend(args.first().map(Value::deref).unwrap_or(Value::Null)),
            DESTROY => {
                let fiber = fiber.expect("a Fiber");
                if state(fiber).borrow().status != Status::Suspended {
                    return Ok(Value::Null);
                }
                // A suspended fiber destroyed: unwound by an exit no catch
                // matches (a plain object, not Throwable), its finally blocks run.
                state(fiber).borrow_mut().closing = true;
                let marker = Ctx::new_object(self, &Rc::clone(&self.classes[b"stdclass".as_slice()]));
                self.fiber_exit = Some(Rc::clone(&marker));
                let mut regs = std::mem::take(&mut self.isolate.regs);
                let r = self.fiber_resume(&mut regs, fiber, Value::Object(Rc::clone(&marker)), true);
                self.isolate.regs = regs;
                self.fiber_exit = None;
                match r {
                    Err(Throw::Object(o)) if Rc::ptr_eq(&o, &marker) => Ok(Value::Null),
                    other => other,
                }
            }
            CURRENT => Ok(self.fibers.last().map_or(Value::Null, |r| Value::Object(Rc::clone(&r.fiber)))),
            _ => {
                let fiber = fiber.expect("a Fiber method");
                let mut regs = std::mem::take(&mut self.isolate.regs);
                let r = match op {
                    START => self.fiber_start(&mut regs, fiber, args),
                    _ => {
                        let v = args.first().map(Value::deref).unwrap_or(Value::Null);
                        self.fiber_resume(&mut regs, fiber, v, op == THROW)
                    }
                };
                self.isolate.regs = regs;
                r
            }
        }
    }

    fn fiber_start(&mut self, regs: &mut Vec<Value>, fiber: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
        if state(fiber).borrow().status != Status::Init {
            return Err(self.fiber_error("Cannot start a fiber that has already been started"));
        }
        let callable = state(fiber).borrow().callable.clone();
        let target = self.resolve_callable(&callable)?;
        let m = match target {
            crate::callable::Target::Frame(m @ MethodFrame { func: Code::User(_), .. }) => m,
            _ => {
                // A builtin: it runs to its end (nothing in it suspends).
                state(fiber).borrow_mut().status = Status::Running;
                let r = self.call_with_regs(regs, &callable, args);
                let mut s = state(fiber).borrow_mut();
                s.status = Status::Terminated;
                return match r {
                    Ok(v) => {
                        s.returned = Some(v);
                        Ok(Value::Null)
                    }
                    Err(t) => {
                        s.threw = true;
                        Err(t)
                    }
                };
            }
        };
        let at = self.window_end();
        if regs.len() < at + args.len() + 1 {
            regs.resize(at + args.len() + 1, Value::Undef);
        }
        for (n, a) in args.iter().enumerate() {
            regs[at + n] = a.deref();
        }
        let base_frame = self.frames.len();
        let caller_pc = self.frames.last().map_or(0, |f| f.pc);
        state(fiber).borrow_mut().status = Status::Running;
        self.fibers.push(FiberRun { fiber: Rc::clone(fiber), base_frame });
        let next = self.call_method(regs, m, at, args.len() as u8, at as u32, caller_pc as usize);
        let r = match next {
            Err(t) => Err(t),
            Ok(Next::Go(_)) => Ok(()),
            Ok(_) => self.fiber_execute(regs),
        };
        self.fiber_finish(regs, fiber, at, r)
    }

    fn fiber_resume(&mut self, regs: &mut Vec<Value>, fiber: &Rc<Object>, value: Value, throw: bool) -> Result<Value, Throw> {
        if state(fiber).borrow().status != Status::Suspended {
            return Err(self.fiber_error("Cannot resume a fiber that is not suspended"));
        }
        let exception = if throw {
            match &value {
                Value::Object(e) if e.class.is_a(b"throwable") || self.fiber_exit.as_ref().is_some_and(|m| Rc::ptr_eq(m, e)) => Some(Rc::clone(e)),
                _ => return Err(self.throw("TypeError", "Fiber::throw(): Argument #1 ($exception) must be of type Throwable")),
            }
        } else {
            None
        };
        let (frames, saved, from) = {
            let mut s = state(fiber).borrow_mut();
            s.status = Status::Running;
            (std::mem::take(&mut s.frames), std::mem::take(&mut s.regs), s.at)
        };
        let at = self.window_end();
        if regs.len() < at + saved.len() + 1 {
            regs.resize(at + saved.len() + 1, Value::Undef);
        }
        for (i, v) in saved.into_iter().enumerate() {
            regs[at + i] = v;
        }
        let base_frame = self.frames.len();
        let caller_pc = self.frames.last().map_or(0, |f| f.pc);
        // The windows move with the registers.
        let delta = at as i64 - from as i64;
        for mut f in frames {
            f.base = (f.base as i64 + delta) as u32;
            if f.ret != NO_RET {
                f.ret = (f.ret as i64 + delta) as u32;
            }
            self.frames.restore(f);
        }
        // suspend()'s result: the value resume() gave (the call's dst).
        if exception.is_none()
            && let Some(top) = self.frames.last()
            && let Some(dst) = call_dst(&top.func.code.code, top.pc as usize)
        {
            let slot = top.base as usize + dst as usize;
            regs[slot] = value;
        }
        self.fiber_throw = exception;
        self.fibers.push(FiberRun { fiber: Rc::clone(fiber), base_frame });
        let r = self.fiber_execute(regs);
        if self.frames.len() == base_frame
            && let Some(caller) = self.frames.last_mut()
        {
            caller.pc = caller_pc;
        }
        self.fiber_finish(regs, fiber, at, r)
    }

    /// Runs the fiber's frames (above `floor`) until they return, throw,
    /// or suspend.
    fn fiber_execute(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        let (saved_pc, saved_floor) = (self.pc, self.frames.floor);
        let base = self.fibers.last().expect("running a fiber").base_frame;
        self.frames.floor = base;
        let r = self.execute(regs);
        self.frames.floor = saved_floor;
        self.pc = saved_pc;
        r
    }

    /// After a fiber's run: suspended (its frames put aside, suspend()'s
    /// value back to the resumer), returned, or thrown.
    fn fiber_finish(&mut self, regs: &mut [Value], fiber: &Rc<Object>, at: usize, r: Result<(), Throw>) -> Result<Value, Throw> {
        let run = self.fibers.pop().expect("running a fiber");
        if let Some(v) = self.fiber_suspended.take() {
            let frames = self.frames.take_above(run.base_frame);
            let end = frames.iter().map(|f| f.base as usize + f.func.code.nregs as usize).max().unwrap_or(at).max(at);
            let end = end.min(regs.len());
            let saved: Vec<Value> = regs[at..end].iter_mut().map(std::mem::take).collect();
            let mut s = state(fiber).borrow_mut();
            s.frames = frames;
            s.regs = saved;
            s.at = at;
            s.status = Status::Suspended;
            return Ok(v);
        }
        let mut s = state(fiber).borrow_mut();
        s.status = Status::Terminated;
        match r {
            Ok(()) => {
                s.returned = Some(std::mem::replace(&mut regs[at], Value::Undef).deref());
                Ok(Value::Null)
            }
            Err(t) => {
                s.threw = true;
                Err(t)
            }
        }
    }

    fn fiber_suspend(&mut self, value: Value) -> Result<Value, Throw> {
        let Some(run) = self.fibers.last() else {
            return Err(self.fiber_error("Cannot suspend outside of a fiber"));
        };
        if state(&run.fiber).borrow().closing {
            return Err(self.fiber_error("Cannot suspend in a force-closed fiber"));
        }
        let run = self.fibers.last().expect("checked");
        // Code a builtin called back into runs in its own nested execute:
        // its Rust frames can't be put aside.
        if self.frames.floor != run.base_frame {
            return Err(self.fiber_error("Cannot switch fibers in current execution context"));
        }
        self.fiber_suspended = Some(value);
        Ok(Value::Null)
    }
}

/// The result register of the call just before `pc` (where a frame that
/// called Fiber::suspend() goes on).
fn call_dst(code: &[Instr], pc: usize) -> Option<p8_bytecode::Reg> {
    match code.get(pc.checked_sub(1)?)? {
        Instr::StaticCall { dst, .. } | Instr::Call { dst, .. } | Instr::MethodCall { dst, .. } => Some(*dst),
        _ => None,
    }
}

// ---- the Fiber class ----

fn construct(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let callable = args.first().map(Value::deref).unwrap_or(Value::Null);
    if !ctx.is_callable(&callable) {
        return Err(ctx.throw("TypeError", format!("Fiber::__construct(): Argument #1 ($callback) must be a valid callback, {}", p8_runtime::args::value_name(&callable))));
    }
    if !matches!(state(this).borrow().callable, Value::Null) {
        return Err(ctx.throw("FiberError", "Cannot call constructor twice".into()));
    }
    state(this).borrow_mut().callable = callable;
    Ok(Value::Null)
}

fn start(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    ctx.fiber_op(Some(this), START, args)
}

fn resume(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    ctx.fiber_op(Some(this), RESUME, args)
}

fn throw(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    ctx.fiber_op(Some(this), THROW, args)
}

fn suspend(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctx.fiber_op(None, SUSPEND, args)
}

fn get_current(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    ctx.fiber_op(None, CURRENT, args)
}

fn get_return(ctx: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let s = state(this).borrow();
    let why = match (s.status, &s.returned, s.threw) {
        (Status::Terminated, Some(v), _) => return Ok(v.clone()),
        (Status::Terminated, _, true) => "The fiber threw an exception",
        (Status::Init, ..) => "The fiber has not been started",
        _ => "The fiber has not returned",
    };
    drop(s);
    Err(ctx.throw("FiberError", format!("Cannot get fiber return value: {why}")))
}

fn status_is(this: &Object, f: impl Fn(Status) -> bool) -> Value {
    Value::Bool(f(state(this).borrow().status))
}

fn is_started(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(status_is(this, |s| s != Status::Init))
}

fn is_suspended(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(status_is(this, |s| s == Status::Suspended))
}

fn is_running(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(status_is(this, |s| s == Status::Running))
}

fn is_terminated(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(status_is(this, |s| s == Status::Terminated))
}

/// The Fiber's cleanup when it is destroyed (before any __destruct).
pub(crate) fn destroy(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    ctx.fiber_op(Some(this), DESTROY, args)
}

fn fiber_error_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(ctx.throw("Error", "The \"FiberError\" class is reserved for internal use and cannot be manually instantiated".into()))
}

pub(crate) const CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "Fiber",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL | CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE,
        properties: &[],
        methods: &[
            ("__construct", Some(Native::Method(construct))),
            ("start", Some(Native::Method(start))),
            ("resume", Some(Native::Method(resume))),
            ("throw", Some(Native::Method(throw))),
            ("getReturn", Some(Native::Method(get_return))),
            ("isStarted", Some(Native::Method(is_started))),
            ("isSuspended", Some(Native::Method(is_suspended))),
            ("isRunning", Some(Native::Method(is_running))),
            ("isTerminated", Some(Native::Method(is_terminated))),
            ("suspend", Some(Native::Static(suspend))),
            ("getCurrent", Some(Native::Static(get_current))),
        ],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "FiberError",
        parent: Some("Error"),
        interfaces: &[],
        flags: CLASS_FINAL,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(fiber_error_construct)))],
        constants: &[],
        debug_info: None,
    },
];

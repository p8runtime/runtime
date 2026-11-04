//! Fibers: a function run on its own call stack, which it can leave with
//! Fiber::suspend() and come back to with resume().
//!
//! A fiber is a task of its own (task.rs): its frames over its own
//! register file. start() and resume() switch to the fiber's task and run
//! it in a nested `execute`; Fiber::suspend() sets a flag the loop stops
//! at, and the task switches back, the fiber's frames and registers left
//! where they are, in its Task. Nothing is copied either way: a switch is
//! a swap of the task's fields. The value resume() gives becomes
//! suspend()'s result.
//!
//! A fiber can't be suspended from PHP code a builtin called back (an
//! array_map callback, a generator being resumed): that code runs in a
//! further nested `execute` whose Rust frames can't be put aside.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_fibers.c

use crate::classes::{Code, MethodFrame};
use crate::task::{FIRST_REG, Task};
use crate::{Context, Next};
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
    /// While suspended: its task (its frames, the first one the fiber
    /// function's, and its registers).
    task: Task,
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

/// A fiber running now, and the task that started or resumed it (waiting
/// until the fiber returns, throws or suspends).
pub(crate) struct FiberRun {
    fiber: Rc<Object>,
    pub(crate) resumer: Task,
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
        self.push_native(name, None, args, true);
        let r = self.fiber_op_inner(fiber, op, args);
        self.pop_native();
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
                let marker = Ctx::new_object(self, &Rc::clone(&self.classes[&self.isolate.known.stdclass]));
                self.fiber_exit = Some(Rc::clone(&marker));
                let r = self.fiber_resume(fiber, Value::Object(Rc::clone(&marker)), true);
                self.fiber_exit = None;
                match r {
                    Err(Throw::Object(o)) if Rc::ptr_eq(&o, &marker) => Ok(Value::Null),
                    other => other,
                }
            }
            CURRENT => Ok(self.fibers.last().map_or(Value::Null, |r| Value::Object(Rc::clone(&r.fiber)))),
            START => self.fiber_start(fiber.expect("a Fiber method"), args),
            _ => {
                let fiber = fiber.expect("a Fiber method");
                let v = args.first().map(Value::deref).unwrap_or(Value::Null);
                self.fiber_resume(fiber, v, op == THROW)
            }
        }
    }

    fn fiber_start(&mut self, fiber: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
        if state(fiber).borrow().status != Status::Init {
            return Err(self.fiber_error("Cannot start a fiber that has already been started"));
        }
        let callable = state(fiber).borrow().callable.clone();
        // Resolved where start() was called (the registers are lent): a
        // class it names is autoloaded there.
        let m = match self.resolve_callable(&callable)? {
            crate::callable::Target::Frame(m @ MethodFrame { func: Code::User(_), .. }) => m,
            _ => {
                // A builtin: it runs to its end (nothing in it suspends).
                state(fiber).borrow_mut().status = Status::Running;
                let r = self.call_from_builtin(&callable, args);
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
        self.fiber_start_frame(fiber, m, args)
    }

    /// Starts `fiber` on a method's frame: a new task, its arguments in
    /// its own registers.
    fn fiber_start_frame(&mut self, fiber: &Rc<Object>, m: MethodFrame, args: &mut [Value]) -> Result<Value, Throw> {
        let at = FIRST_REG;
        let mut task = Task::default();
        task.regs.resize(at + args.len() + 1, Value::Undef);
        for (n, a) in args.iter().enumerate() {
            task.regs[at + n] = a.deref();
        }
        state(fiber).borrow_mut().status = Status::Running;
        self.enter_fiber(fiber, task);
        let mut regs = std::mem::take(&mut self.isolate.regs);
        let next = self.call_method(&mut regs, m, at, args.len() as u8, at as u32, 0);
        let r = match next {
            Err(t) => Err(t),
            Ok(Next::Go(_)) => Ok(()),
            Ok(_) => self.fiber_execute(&mut regs),
        };
        self.isolate.regs = regs;
        self.fiber_finish(fiber, r)
    }

    /// A destructor the cycle collector calls from inside a fiber runs in
    /// a fiber of its own, as PHP's do: if it suspends, that fiber is left
    /// to whoever kept it (Fiber::getCurrent()) and the collector goes on.
    pub(crate) fn destruct_in_fiber(&mut self, m: MethodFrame) -> Result<(), Throw> {
        let fiber = self.isolate.known.fiber;
        let class = match self.classes.get(&fiber).cloned().or_else(|| self.builtin_on_demand(fiber)) {
            Some(c) => c,
            None => return Ok(()),
        };
        let fiber = Ctx::new_object(self, &class);
        self.fiber_start_frame(&fiber, m, &mut []).map(|_| ())
    }

    fn fiber_resume(&mut self, fiber: &Rc<Object>, value: Value, throw: bool) -> Result<Value, Throw> {
        if state(fiber).borrow().status != Status::Suspended {
            return Err(self.fiber_error("Cannot resume a fiber that is not suspended"));
        }
        let exception = if throw {
            match &value {
                Value::Object(e) if e.class.is_a(b"throwable") || self.fiber_exit.as_ref().is_some_and(|m| Rc::ptr_eq(m, e)) => Some(Rc::clone(e)),
                _ => return Err(self.throw("TypeError", format!("Fiber::throw(): Argument #1 ($exception) must be of type Throwable, {} given", p8_runtime::args::value_name(&value)))),
            }
        } else {
            None
        };
        let task = {
            let mut s = state(fiber).borrow_mut();
            s.status = Status::Running;
            std::mem::take(&mut s.task)
        };
        self.enter_fiber(fiber, task);
        let mut regs = std::mem::take(&mut self.isolate.regs);
        // suspend()'s result: the value resume() gave (the call's dst).
        if exception.is_none()
            && let Some(top) = self.frames.last()
            && let Some(dst) = call_dst(&top.func.code.code, top.pc as usize)
        {
            regs[top.base as usize + dst as usize] = value;
        }
        self.fiber_throw = exception;
        let r = self.fiber_execute(&mut regs);
        self.isolate.regs = regs;
        self.fiber_finish(fiber, r)
    }

    /// Switch to `fiber`'s task; the running one waits in its FiberRun.
    fn enter_fiber(&mut self, fiber: &Rc<Object>, mut task: Task) {
        self.switch_task(&mut task);
        self.fibers.push(FiberRun { fiber: Rc::clone(fiber), resumer: task });
    }

    /// Runs the fiber's frames until they return, throw, or suspend.
    fn fiber_execute(&mut self, regs: &mut Vec<Value>) -> Result<(), Throw> {
        self.frames.floor = 0;
        self.execute(regs)
    }

    /// After a fiber's run: back to the task that started or resumed it.
    /// Suspended (its task kept in the Fiber, suspend()'s value back to
    /// the resumer), returned, or thrown.
    fn fiber_finish(&mut self, fiber: &Rc<Object>, r: Result<(), Throw>) -> Result<Value, Throw> {
        let mut task = self.fibers.pop().expect("running a fiber").resumer;
        self.switch_task(&mut task);
        if let Some(v) = self.fiber_suspended.take() {
            let mut s = state(fiber).borrow_mut();
            s.task = task;
            s.status = Status::Suspended;
            return Ok(v);
        }
        let mut s = state(fiber).borrow_mut();
        s.status = Status::Terminated;
        let r = match r {
            Ok(()) => {
                let v = task.regs.get_mut(FIRST_REG).map(std::mem::take).unwrap_or_default();
                s.returned = Some(v.deref());
                Ok(Value::Null)
            }
            Err(t) => {
                s.threw = true;
                Err(t)
            }
        };
        // The task goes after the state's borrow (what it releases may be
        // the fiber itself). Frames an exit left are unwound first, top
        // first, each one's variables in order, as PHP unwinds them.
        drop(s);
        while task.frames.len() > 0 {
            crate::throw::drop_frame(&mut task.frames, &mut task.regs);
        }
        drop(task);
        r
    }

    fn fiber_suspend(&mut self, value: Value) -> Result<Value, Throw> {
        let Some(run) = self.fibers.last() else {
            return Err(self.fiber_error("Cannot suspend outside of a fiber"));
        };
        if state(&run.fiber).borrow().closing {
            return Err(self.fiber_error("Cannot suspend in a force-closed fiber"));
        }
        // Code a builtin called back into runs in its own nested execute
        // (above the fiber's bottom one): its Rust frames can't be put aside.
        if self.frames.floor != 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Isolate;
    use crate::tests::{Sink, compile};
    use p8_runtime::ext::{Builtin, Registry};
    use std::sync::Arc;

    /// A suspended fiber's task, as `probe($fiber)` saw it: where its
    /// register file and frame stack are, how many registers, each
    /// frame's base.
    type Shape = (usize, usize, usize, Vec<u32>);

    thread_local! {
        static SHAPES: RefCell<Vec<Shape>> = const { RefCell::new(Vec::new()) };
    }

    fn probe(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
        let Value::Object(f) = args[0].deref() else { panic!("probe(a Fiber)") };
        let s = state(&f).borrow();
        assert_eq!(s.status, Status::Suspended);
        let t = &s.task;
        let bases = t.frames.slots[..t.frames.len()].iter().map(|fr| fr.base).collect();
        SHAPES.with(|v| v.borrow_mut().push((t.regs.as_ptr() as usize, t.regs.len(), t.frames.slots.as_ptr() as usize, bases)));
        Ok(Value::Null)
    }

    static PROBE: [Builtin; 1] = [Builtin { name: "probe", run: probe, by_ref: &[] }];

    /// A fiber suspended deep in a large stack keeps its task: resuming
    /// it (from callers at many depths) and suspending it again moves the
    /// same frame stack and register file in and out (the same
    /// allocations, the same frame bases), nothing copied or shifted.
    #[test]
    fn suspend_and_resume_switch_tasks_without_copying() {
        let mut registry = Registry::default();
        registry.add_functions(&PROBE);
        let registry = Arc::new(registry);
        let source = "<?php
            function deep($n) {
                $a = $n; $b = $n * 2; $c = [$a, $b]; $d = \"$a-$b\"; $e = $a + $b; $f = $e * 3;
                if ($n === 0) {
                    $x = 0;
                    while (true) { $x = Fiber::suspend($x); }
                }
                return deep($n - 1) + $a + $b + $e + $f + $c[1] + (int) $d;
            }
            function at_depth($fiber, $n, $v) { return $n > 0 ? at_depth($fiber, $n - 1, $v) : $fiber->resume($v); }
            $fiber = new Fiber(fn() => deep(200));
            $fiber->start();
            probe($fiber);
            for ($i = 1; $i <= 1000; $i++) {
                if (at_depth($fiber, $i % 50, $i) !== $i) { exit(3); }
                probe($fiber);
            }";
        let program = compile(source, &registry);
        let mut isolate = Isolate::new(Arc::clone(&registry));
        let mut host = Sink;
        let mut cx = isolate.new_context(&mut host);
        for f in &program.functions {
            cx.declare(Arc::clone(f)).expect("declares");
        }
        assert_eq!(cx.run_main(Arc::clone(&program.main)), 0);
        let shapes = SHAPES.with(|v| std::mem::take(&mut *v.borrow_mut()));
        assert_eq!(shapes.len(), 1001);
        let first = &shapes[0];
        // The fiber function's frame, then deep(200) .. deep(0): 202 frames
        // over the fiber's own registers, the first at FIRST_REG.
        assert_eq!(first.3.len(), 202);
        assert_eq!(first.3[0] as usize, FIRST_REG);
        assert!(first.1 > 202 * 6, "a large register file: {}", first.1);
        for s in &shapes {
            assert_eq!(s, first, "the task moved back as it was, not copied or shifted");
        }
    }
}

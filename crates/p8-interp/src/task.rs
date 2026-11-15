//! Tasks: a line of execution's state (its frames, its own register file
//! and the scratch state that goes with them), as one bundle.
//!
//! The running task's fields live in `Context` under their own names
//! (`frames`, `isolate.regs`, `pc`, ...), so the interpreter reads them
//! as it always has. A task that is not running is a `Task` value: a
//! fiber holds its own while it is suspended, and the task a fiber was
//! started or resumed from waits in `Context::fibers` while it runs.
//! Switching is a swap of each field: constant time, nothing copied, and
//! every task's frame bases are relative to its own register file.
//!
//! Only the request's first task has a main frame (the one whose window
//! starts at register 0, where the globals are). Every other task starts
//! its frames at `FIRST_REG`, so `base == 0` still means "the main
//! frame", and code that reaches the globals from another task goes to
//! the first task's registers (`Context::with_main`).

use crate::{Context, Frame, Frames, Natives};
use p8_runtime::array::PhpArray;
use p8_runtime::value::Value;

/// Where a task other than the request's first starts its frames.
pub(crate) const FIRST_REG: usize = 1;

/// A task that is not running.
#[derive(Default)]
pub(crate) struct Task {
    pub(crate) frames: Frames,
    pub(crate) regs: Vec<Value>,
    pub(crate) pc: usize,
    pub(crate) natives: Natives,
    pub(crate) running_builtin: Option<usize>,
    pub(crate) ref_wanted: Vec<usize>,
    pub(crate) named_extra: Option<(usize, PhpArray)>,
    pub(crate) result_reg: usize,
    pub(crate) deferred_write: Option<(usize, Value)>,
}

impl Context<'_> {
    /// Make `task` the running one; `task` becomes the one that was.
    /// The register file must be lent (in `isolate.regs`), as it is
    /// while a builtin runs.
    pub(crate) fn switch_task(&mut self, task: &mut Task) {
        std::mem::swap(&mut self.frames, &mut task.frames);
        std::mem::swap(&mut self.isolate.regs, &mut task.regs);
        std::mem::swap(&mut self.pc, &mut task.pc);
        std::mem::swap(&mut self.natives, &mut task.natives);
        std::mem::swap(&mut self.running_builtin, &mut task.running_builtin);
        std::mem::swap(&mut self.ref_wanted, &mut task.ref_wanted);
        std::mem::swap(&mut self.named_extra, &mut task.named_extra);
        std::mem::swap(&mut self.result_reg, &mut task.result_reg);
        std::mem::swap(&mut self.deferred_write, &mut task.deferred_write);
    }

    /// The tasks waiting under the running one, the request's first
    /// first: each started or resumed the fiber above it.
    pub(crate) fn waiting_tasks(&self) -> impl Iterator<Item = &Task> {
        self.fibers.iter().map(|run| &run.resumer)
    }

    /// The request's main frame (the main script's), wherever its task is.
    pub(crate) fn main_frame(&self) -> Option<&Frame> {
        match self.fibers.first() {
            Some(run) => run.resumer.frames.first(),
            None => self.frames.first(),
        }
    }

    /// Run `f` on the main frame's registers (its window starts at 0):
    /// `below`, the running task's registers under its running frame,
    /// when the running task is the request's first; else the first
    /// task's own, put aside while a fiber runs (taken out for the call,
    /// in constant time).
    pub(crate) fn with_main<R>(&mut self, below: &mut [Value], f: impl FnOnce(&mut Self, &mut [Value]) -> R) -> R {
        if self.fibers.is_empty() {
            return f(self, below);
        }
        let mut main = std::mem::take(&mut self.fibers[0].resumer.regs);
        let r = f(self, &mut main);
        self.fibers[0].resumer.regs = main;
        r
    }
}

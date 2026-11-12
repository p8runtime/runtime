//! Native code (#402, Phase 4): the interpreter's side of running a
//! function as machine code, in safe Rust.
//!
//! A p8-built executable (`p8 compile [dir] --native <exe>`) carries
//! machine code for the functions of its package. The machine code is
//! called through the one crate allowed `unsafe` (`p8-native`), which
//! implements [`NativeCode`]; this module is what that crate is given:
//! the frame to run on ([`NativeFrame`]), and the one way the machine
//! code calls back into the interpreter ([`NativeFrame::help`]). Nothing
//! here is `unsafe`.
//!
//! The contract: a native function runs the top frame, from its first
//! instruction, on the frame's own registers (the window). An instruction
//! its code has no inline form for, or whose operands its inline form does
//! not take (a string where it compiled for ints, an overflow to float, a
//! value holding memory), it has the interpreter run, here, as the loop's
//! `slow` runs it (`step`, with everything `step`'s caller does after:
//! unwinding to a handler, errors, destructors), then goes on wherever
//! that left the frame. So every case the inline code does not take keeps
//! one copy of its semantics, the interpreter's (#388). It hands the frame
//! back ([`Exit`]) at a return, and when what the interpreter ran left the
//! frame (an exception out of it, the end of the run).
//!
//! A function with native code is entered wherever the loop starts a
//! frame at its first instruction (`execute`'s frame head): every call
//! path, first calls, methods, static calls, closures and callbacks
//! included.

use super::*;
use p8_bytecode::Op;

/// Machine code for some of the program's functions: what the boundary
/// crate implements. One per process (it is the executable's).
pub trait NativeCode: Sync {
    /// Run native function `id` on the top frame from its first
    /// instruction, until it hands the frame back.
    fn run(&self, id: u32, frame: &mut NativeFrame<'_, '_>) -> Exit;
}

/// Where native code handed the frame back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Go on at this pc (a return, or an instruction the native code
    /// leaves to the interpreter), the registers as before it.
    At(u32),
    /// What a [`NativeFrame::help`] ran left the frame: the interpreter
    /// goes on from what it gave (kept in the frame).
    Left,
}

/// What [`NativeFrame::help`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Help {
    /// The instruction ran; the frame goes on at this pc.
    Next(u32),
    /// The instruction ran and the interpreter is to go on from what it
    /// gave (`Exit::Left`): it left the frame (an exception out of it, the
    /// end of the run, a fiber suspended), or pushed a callee it did not
    /// run nested.
    Left,
}

/// What [`NativeFrame::help`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpOp {
    /// An `Op::Call`: a user function the site resolved is called directly,
    /// anything else as `Step`.
    Call,
    /// Any instruction, as the loop's `slow` runs it.
    Step,
}

/// The frame native code runs on, and the interpreter it calls back.
pub struct NativeFrame<'a, 'c> {
    cx: &'a mut Context<'c>,
    regs: &'a mut Vec<Value>,
    /// What a help that left the frame gave (`Exit::Left`).
    left: Option<Result<Flow, Throw>>,
}

impl NativeFrame<'_, '_> {
    /// The top frame's registers: its window of `WINDOW` values from its
    /// base (every register operand of its code is inside it). What a
    /// help does may move the register file: this is to be asked again
    /// after every help.
    pub fn window(&mut self) -> &mut [Value] {
        let base = self.cx.frames.last().expect("a frame").base as usize;
        &mut self.regs[base..base + fast::WINDOW]
    }

    /// The isolate's interrupt word: not 0 when the run is to stop (a
    /// time limit), which native code tests at each backward jump, as the
    /// loop does, and has the interpreter run that jump (which raises it).
    pub fn interrupt(&self) -> &AtomicU64 {
        &self.cx.stop
    }

    /// Run the instruction at `pc` of the top frame on the interpreter
    /// (`op`), and say where the frame goes on. A call it makes runs its
    /// callee nested, as a callback a builtin makes does, natively when
    /// the callee has native code (`execute`'s frame head enters it).
    pub fn help(&mut self, op: HelpOp, pc: u32) -> Help {
        // A callee runs nested only while that is sound: below the stack
        // limit nested runs keep to (#444), and outside a fiber (a fiber
        // suspends only from its own run of the loop, whose Rust frames are
        // all it has). Else a call pushes its callee's frame and the frame
        // is left to the interpreter, which runs the callee on its own
        // frames, then the rest of this function.
        let nest = !self.cx.stack_exhausted() && self.cx.fibers.is_empty();
        let depth = self.cx.frames.len();
        if nest
            && op == HelpOp::Call
            && let Some(r) = self.call(pc)
        {
            return self.settle(depth, r);
        }
        let ran = self.cx.slow(self.regs, pc as usize);
        let r = match ran {
            // A call pushed its callee's frame: run it to its return.
            Ok(Flow::Go(next)) if nest && self.cx.frames.len() > depth => {
                self.cx.frames.last_mut().expect("the callee").pc = next as u32;
                let ran = self.nested();
                self.after(ran, pc as usize)
            }
            r => r,
        };
        self.settle(depth, r)
    }

    /// The loop's Call arm, for a user function the site resolved (None
    /// for anything else, which `step` runs).
    fn call(&mut self, pc: u32) -> Option<Result<Flow, Throw>> {
        let cx = &mut *self.cx;
        let top = cx.frames.last().expect("a frame");
        let base = top.base as usize;
        let insn = *top.func.code.insns.get(pc as usize)?;
        if insn.op != Op::Call {
            return None;
        }
        let (args, n, dst) = (usize::from(insn.a), usize::from(insn.b), usize::from(insn.c));
        // (The site is the low half of `imm`; the result goes to the
        // call's register, and the move after it runs.)
        let Some(&Callee::User(t)) = top.func.sites.get((insn.imm & 0xffff) as usize).and_then(|s| s.get()) else { return None };
        let target = &cx.closures[t as usize];
        // (The loop's Call arm's conditions.)
        if !fast::direct(target.shape, &target.code, insn.b) || cx.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
            return None;
        }
        cx.frames.call(Some(target), pc as usize + 1, base + args, n, (base + dst) as u32);
        fast::enter(&mut cx.frames, self.regs);
        let ran = self.nested();
        Some(self.after(ran, pc as usize))
    }

    /// Run the frame just pushed (at its pc) to its return, as the bottom
    /// frame of a run of its own (`run_nested_at`'s way): its return ends
    /// the run, its result in the call's register. `execute`'s frame head
    /// enters its native code.
    fn nested(&mut self) -> Result<(), Throw> {
        let cx = &mut *self.cx;
        let (saved_pc, saved_floor) = (cx.pc, cx.frames.floor);
        cx.frames.floor = cx.frames.len() - 1;
        let ran = cx.execute(self.regs);
        cx.frames.floor = saved_floor;
        cx.pc = saved_pc;
        ran
    }

    /// After a nested call made by the instruction at `at`: the caller
    /// goes on where the call left it, after the errors and destructors it
    /// made due; an exception out of the callee is raised at the call.
    fn after(&mut self, ran: Result<(), Throw>, at: usize) -> Result<Flow, Throw> {
        let cx = &mut *self.cx;
        match ran {
            // The callee suspended its fiber: its frames stay as they are.
            Ok(()) if cx.fiber_suspended.is_some() => Ok(Flow::Done),
            Ok(()) => {
                let next = cx.frames.last().expect("the caller").pc as usize;
                if cx.ids.has_pending() || !cx.pending_errors.is_empty() { cx.after_errors(self.regs, next, at).map(Flow::Go) } else { Ok(Flow::Go(next)) }
            }
            Err(t) => cx.unwind(self.regs, t, at).map(Flow::Go),
        }
    }

    /// The frame goes on at the pc `r` gives, if it is still the top one
    /// (as deep as before); else it left. A fiber suspended by what ran
    /// (in a callee run nested, too: its frames stay where they are, as
    /// the fiber left them) ends this run of the loop, as `went` ends it.
    fn settle(&mut self, depth: usize, r: Result<Flow, Throw>) -> Help {
        let r = if self.cx.fiber_suspended.is_some() && r.is_ok() { Ok(Flow::Done) } else { r };
        match r {
            Ok(Flow::Go(next)) if self.cx.frames.len() == depth => Help::Next(next as u32),
            r => {
                self.left = Some(r);
                Help::Left
            }
        }
    }
}

impl Context<'_> {
    /// Enter native function `id` on the top frame at pc 0: where the
    /// interpreter goes on.
    pub(crate) fn native_entry(&mut self, regs: &mut Vec<Value>, id: u32) -> Result<Flow, Throw> {
        let Some(code) = self.isolate.native else { return Ok(Flow::Go(0)) };
        let mut frame = NativeFrame { cx: self, regs, left: None };
        match code.run(id, &mut frame) {
            Exit::At(pc) => Ok(Flow::Go(pc as usize)),
            Exit::Left => frame.left.take().expect("the help that left kept what it gave"),
        }
    }
}

impl Isolate {
    /// Run the functions `code` has machine code for natively (a
    /// p8-built executable's, set before any request runs).
    pub fn set_native(&mut self, code: &'static dyn NativeCode) {
        self.native = Some(code);
    }
}

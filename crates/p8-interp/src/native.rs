//! Native code (#402, Phase 4): the interpreter's side of running a
//! function as machine code, in safe Rust.
//!
//! A p8-built executable (`php2 --compile <dir> --native <exe>`) carries
//! machine code for some of its package's functions. The machine code is
//! called through the one crate allowed `unsafe` (`p8-native`), which
//! implements [`NativeCode`]; this module is what that crate is given:
//! the frame to run on ([`NativeFrame`]), and the calls the machine code
//! makes back into the interpreter. Nothing here is `unsafe`.
//!
//! The contract: a native function runs the top frame from its first
//! instruction, on the frame's own registers (the window), and hands the
//! frame back to the interpreter at a pc ([`Exit`]): at a return (the
//! interpreter's return runs it), or at an instruction whose operands it
//! has no code for (a string where it compiled for ints, an overflow to
//! float, a reference), with the registers as they were before that
//! instruction. The interpreter goes on from there, so whatever the
//! native code does not take runs exactly as without it (#388). A
//! function is entered natively by a user call the loop makes (`Op::Call`)
//! and by the calls native code makes ([`NativeFrame::call`]); any other
//! call runs it on the interpreter.

use super::*;
use p8_bytecode::Op;

/// Machine code for some of the program's functions: what the boundary
/// crate implements. One per process (it is the executable's).
pub trait NativeCode: Sync {
    /// Run native function `id` on the top frame (just entered, its
    /// arguments in its first registers), until it hands the frame back.
    fn run(&self, id: u32, frame: &mut NativeFrame<'_, '_>) -> Exit;
}

/// Where native code handed the frame back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Go on at this pc (a return, or an instruction the native code
    /// leaves to the interpreter), the registers as before it.
    At(u32),
    /// The call at this pc ran, and left an error or a destructor due:
    /// they run, then the frame goes on after the call.
    Pending(u32),
    /// The call at this pc threw ([`NativeFrame::call`] kept what): the
    /// exception is raised there.
    Threw(u32),
}

/// What [`NativeFrame::call`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDone {
    /// The call ran; its result is in its register.
    Done,
    /// Nothing ran: the call is not one native code makes (not a user
    /// function the site resolved, too few arguments, a generator, an
    /// interrupt due). The interpreter is to run it.
    NotTaken,
    /// The call ran and left something due (`Exit::Pending`).
    Pending,
    /// The call threw (`Exit::Threw`).
    Threw,
}

/// The frame native code runs on, and the interpreter it calls back.
pub struct NativeFrame<'a, 'c> {
    cx: &'a mut Context<'c>,
    regs: &'a mut Vec<Value>,
    thrown: Option<Throw>,
}

impl NativeFrame<'_, '_> {
    /// The top frame's registers: its window of `WINDOW` values from its
    /// base (every register operand of its code is inside it). What a
    /// call does may move the register file: this is to be asked again
    /// after every call.
    pub fn window(&mut self) -> &mut [Value] {
        let base = self.cx.frames.last().expect("a frame").base as usize;
        &mut self.regs[base..base + fast::WINDOW]
    }

    /// The isolate's interrupt word: not 0 when the run is to stop (a
    /// time limit), which native code tests at each backward jump, as the
    /// loop does, and hands the frame back there.
    pub fn interrupt(&self) -> &AtomicU64 {
        &self.cx.stop
    }

    /// The `Op::Call` at `pc` of the top frame, run whole, as the loop's
    /// arm runs it: a user function the site has resolved, called with
    /// its required arguments, entered natively when it has native code,
    /// and run on the interpreter from wherever that hands it back. The
    /// result lands in the call's register. Anything else is not taken.
    pub fn call(&mut self, pc: u32) -> CallDone {
        let cx = &mut *self.cx;
        let top = cx.frames.last().expect("a frame");
        let base = top.base as usize;
        let Some(&insn) = top.func.code.insns.get(pc as usize) else { return CallDone::NotTaken };
        if insn.op != Op::Call {
            return CallDone::NotTaken;
        }
        let (args, n, dst) = (usize::from(insn.a), usize::from(insn.b), usize::from(insn.c));
        let Some(&Callee::User(t)) = top.func.sites.get(insn.imm as usize).and_then(|s| s.get()) else { return CallDone::NotTaken };
        let target = &cx.closures[t as usize];
        // (The loop's Call arm's conditions.)
        if n < target.code.required || target.code.is_generator || cx.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
            return CallDone::NotTaken;
        }
        let native = target.code.native;
        cx.frames.call(Some(target), pc as usize + 1, base + args, n, (base + dst) as u32);
        fast::enter(&mut cx.frames, self.regs);
        // The callee runs as the bottom frame of a run of its own, as a
        // callback a builtin makes does (`run_nested_at`): its return
        // ends the run, its result in the call's register.
        let (saved_pc, saved_floor) = (cx.pc, cx.frames.floor);
        cx.frames.floor = cx.frames.len() - 1;
        let ran = cx.run_entered(self.regs, native);
        cx.frames.floor = saved_floor;
        cx.pc = saved_pc;
        match ran {
            Ok(()) if cx.ids.has_pending() || !cx.pending_errors.is_empty() => CallDone::Pending,
            Ok(()) => CallDone::Done,
            Err(t) => {
                self.thrown = Some(t);
                CallDone::Threw
            }
        }
    }
}

impl Context<'_> {
    /// Enter native function `id` on the frame just pushed (pc 0): where
    /// the interpreter goes on in it.
    pub(crate) fn native_entry(&mut self, regs: &mut Vec<Value>, id: u32) -> Result<usize, Throw> {
        let Some(code) = self.isolate.native else { return Ok(0) };
        let mut frame = NativeFrame { cx: self, regs, thrown: None };
        let exit = code.run(id, &mut frame);
        let thrown = frame.thrown.take();
        match exit {
            Exit::At(pc) => Ok(pc as usize),
            Exit::Pending(pc) => self.after_errors(regs, pc as usize + 1, pc as usize),
            Exit::Threw(pc) => self.unwind(regs, thrown.expect("the call kept what it threw"), pc as usize),
        }
    }

    /// Run the frame just pushed to its return: natively first when it
    /// has native code, then on the interpreter from where that hands it
    /// back.
    fn run_entered(&mut self, regs: &mut Vec<Value>, native: Option<u32>) -> Result<(), Throw> {
        let start = match native {
            Some(id) => self.native_entry(regs, id)?,
            None => 0,
        };
        self.frames.last_mut().expect("the frame").pc = start as u32;
        self.execute(regs)
    }
}

impl Isolate {
    /// Run the functions `code` has machine code for natively (a
    /// p8-built executable's, set before any request runs).
    pub fn set_native(&mut self, code: &'static dyn NativeCode) {
        self.native = Some(code);
    }
}

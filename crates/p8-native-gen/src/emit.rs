//! What a template writes with: one PHP function's Cranelift function
//! under construction, at one instruction (`pc`), and the few shapes every
//! template is made of (a register's words, a guard that hands the frame
//! back, a jump, an exit).
//!
//! The function: `extern "C" fn(ctx) -> u64` (p8-native-abi). One block
//! per pc, entered with nothing: each loads the frame's window pointer
//! from `ctx` (a call may have moved it) and runs its instruction on the
//! registers in memory, so that wherever the code hands the frame back,
//! the registers are the interpreter's state at that pc. A guard comes
//! before an instruction's first store: a failed one leaves the
//! instruction undone.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{self, InstBuilder, MemFlagsData as MemFlags, types};
use cranelift_frontend::{FunctionBuilder, Variable};
use p8_bytecode::Function;
use p8_native_abi::{self as abi, exit, help, op, tag};
use p8_runtime::frozen::Frozen;

/// An instruction (or an operand of it) native code is not made for:
/// the function stays bytecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotNative;

pub type R = Result<(), NotNative>;

pub struct Emit<'a, 'f> {
    pub b: &'a mut FunctionBuilder<'f>,
    /// The PHP function being compiled.
    pub func: &'a Function,
    /// The instruction's pc, and one block per pc.
    pub pc: u32,
    pub blocks: &'a [ir::Block],
    pub ctx: ir::Value,
    /// The window's address, loaded at this pc's block.
    pub regs: ir::Value,
    /// The helper (`abi::HELP`).
    pub helper: ir::FuncRef,
    /// The block that goes on at the pc in `next` (a jump table over the
    /// pcs' blocks): where the code goes on after a help.
    pub dispatch: ir::Block,
    pub next: Variable,
}

const FLAGS: MemFlags = MemFlags::trusted();

/// A literal as a value's two words, if it is a plain one (a string or
/// an array literal is a value made per isolate: not native).
pub fn plain_words(k: &Frozen) -> Option<(u64, u64)> {
    Some(match k {
        Frozen::Null => (tag::NULL, 0),
        Frozen::Bool(b) => (tag::BOOL, u64::from(*b)),
        Frozen::Int(i) => (tag::INT, *i as u64),
        Frozen::Float(f) => (tag::FLOAT, f.to_bits()),
        Frozen::Str(_) | Frozen::Array(_) => return None,
    })
}

impl Emit<'_, '_> {
    /// Literal `k` of the function.
    pub fn konst(&self, k: u32) -> Result<&Frozen, NotNative> {
        self.func.consts.get(k as usize).ok_or(NotNative)
    }

    fn offset(r: u32, word: i32) -> i32 {
        (r & 0xff) as i32 * abi::VALUE_SIZE + word
    }

    /// Register `r`'s tag word.
    pub fn tag(&mut self, r: u32) -> ir::Value {
        self.b.ins().load(types::I64, FLAGS, self.regs, Self::offset(r, abi::TAG))
    }

    /// Register `r`'s payload word.
    pub fn payload(&mut self, r: u32) -> ir::Value {
        self.b.ins().load(types::I64, FLAGS, self.regs, Self::offset(r, abi::PAYLOAD))
    }

    /// Write a plain value's words into register `r` (whose old value a
    /// guard found plain: nothing to release).
    pub fn store(&mut self, r: u32, tag: ir::Value, payload: ir::Value) {
        self.b.ins().store(FLAGS, tag, self.regs, Self::offset(r, abi::TAG));
        self.b.ins().store(FLAGS, payload, self.regs, Self::offset(r, abi::PAYLOAD));
    }

    pub fn store_payload(&mut self, r: u32, payload: ir::Value) {
        self.b.ins().store(FLAGS, payload, self.regs, Self::offset(r, abi::PAYLOAD));
    }

    pub fn iconst(&mut self, v: u64) -> ir::Value {
        self.b.ins().iconst(types::I64, v as i64)
    }

    /// Whether `tag` is `want`.
    pub fn is(&mut self, tag: ir::Value, want: u64) -> ir::Value {
        self.b.ins().icmp_imm_u(IntCC::Equal, tag, want as i64)
    }

    /// Whether `tag` is a plain value's (nothing to release).
    pub fn is_plain(&mut self, tag: ir::Value) -> ir::Value {
        self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, tag, tag::FIRST_HEAP as i64)
    }

    /// Whether `tag` is a plain value that reads as itself: not undefined
    /// (whose read warns), nothing to release.
    pub fn is_readable(&mut self, tag: ir::Value) -> ir::Value {
        let plain = self.is_plain(tag);
        let defined = self.b.ins().icmp_imm_u(IntCC::NotEqual, tag, tag::UNDEF as i64);
        self.b.ins().band(plain, defined)
    }

    /// Go on only when `ok`; else the interpreter runs the instruction at
    /// `at` (`help`), and the code goes on where that left the frame.
    pub fn guard_at(&mut self, ok: ir::Value, at: u32) {
        let (cont, cold) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(cold);
        self.b.ins().brif(ok, cont, &[], cold, &[]);
        self.b.switch_to_block(cold);
        self.help(op::STEP, at);
        self.b.switch_to_block(cont);
    }

    /// Have the interpreter run the instruction at `at` (`op`, `abi::op`),
    /// then go on at the pc it gives (`dispatch`), or leave the frame to
    /// the interpreter when it says so. Fills the current block.
    pub fn help(&mut self, what: u32, at: u32) {
        let (o, p) = (self.b.ins().iconst(types::I32, i64::from(what)), self.b.ins().iconst(types::I32, i64::from(at)));
        let inst = self.b.ins().call(self.helper, &[self.ctx, o, p]);
        let r = self.b.inst_results(inst)[0];
        let status = self.b.ins().ushr_imm_u(r, 32);
        let next = self.b.ins().ireduce(types::I32, r);
        let is_next = self.b.ins().icmp_imm_u(IntCC::Equal, status, help::NEXT as i64);
        let (go, out) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is_next, go, &[], out, &[]);
        self.b.switch_to_block(go);
        self.b.def_var(self.next, next);
        self.b.ins().jump(self.dispatch, &[]);
        self.b.switch_to_block(out);
        self.b.set_cold_block(out);
        self.exit(exit::LEFT, 0);
    }

    /// `guard_at` this instruction.
    pub fn guard(&mut self, ok: ir::Value) {
        self.guard_at(ok, self.pc);
    }

    /// Return `reason << 32 | pc` (`exit`): the frame is the interpreter's
    /// from there.
    pub fn exit(&mut self, reason: u64, pc: u32) {
        let v = self.iconst(reason << 32 | u64::from(pc));
        self.b.ins().return_(&[v]);
    }

    /// Hand the frame back at this instruction (a return).
    pub fn hand_back(&mut self) -> R {
        self.exit(exit::RETURN, self.pc);
        Ok(())
    }

    /// The block of instruction `pc` (there is one).
    pub fn block(&self, pc: u32) -> Result<ir::Block, NotNative> {
        self.blocks.get(pc as usize).copied().ok_or(NotNative)
    }

    /// Go on at the next instruction.
    pub fn next(&mut self) -> R {
        let next = self.block(self.pc + 1)?;
        self.b.ins().jump(next, &[]);
        Ok(())
    }

    /// Jump to `target`. A backward jump is where an interrupt due (a time
    /// limit) is noticed, as in the loop: then the frame is handed back at
    /// `at`, whose instruction raises it on the interpreter.
    pub fn goto(&mut self, target: u32, at: u32) -> R {
        let to = self.block(target)?;
        if target <= self.pc {
            let word = self.b.ins().load(types::I64, FLAGS, self.ctx, abi::CTX_INTERRUPT);
            let due = self.b.ins().load(types::I64, FLAGS, word, 0);
            let none = self.b.ins().icmp_imm_u(IntCC::Equal, due, 0);
            self.guard_at(none, at);
        }
        self.b.ins().jump(to, &[]);
        Ok(())
    }

    /// Branch on `cond`: to `target` (`goto`) when it is set, else to the
    /// next instruction.
    pub fn branch(&mut self, cond: ir::Value, target: u32) -> R {
        let next = self.block(self.pc + 1)?;
        let taken = self.b.create_block();
        self.b.ins().brif(cond, taken, &[], next, &[]);
        self.b.switch_to_block(taken);
        self.goto(target, self.pc)
    }
}

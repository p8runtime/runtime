//! The native templates (#385): for each opcode whose `ops.def` entry
//! names one (`native=`), the machine code of its handler's inline cases,
//! on plain values only (ints, floats, bools, null; program-memory
//! strings copied). Each template is the native form of the interpreter's
//! fast path for the same instruction (named beside it), and hands the
//! frame back (a guard, `Emit::guard`) on everything else: an undefined
//! variable, a reference, a value holding memory, an int overflow, a
//! literal of another type. The interpreter then runs that instruction
//! as it always does, so those cases keep one copy, the interpreter's.
//!
//! The dispatch over opcodes is generated from `ops.def`
//! (`native_dispatch!`, p8-vmgen): a generated instruction's template
//! takes its operands decoded as its handler does (`args`, and its kinds
//! as `spec`); a hand-written one takes the `Insn`, read as the loop's
//! hand arm reads it.

use crate::emit::{Emit, NotNative, R, plain_words};
// (`native_dispatch!` names the templates `tpl::<name>`.)
use crate::tpl;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{self, InstBuilder, MemFlags, types};
use p8_bytecode::insn::kind::Kind;
use p8_bytecode::insn::kinds::*;
use p8_bytecode::insn::{self, Insn, Op, args, spec};
use p8_native_abi::{call as calls, exit, tag};
use p8_runtime::frozen::Frozen;
use p8_runtime::ops::BinOp;

include!(concat!(env!("OUT_DIR"), "/native_dispatch.rs"));

/// The instruction at `e.pc` as native code, or why not.
pub fn instruction(e: &mut Emit, insn: &Insn) -> R {
    native_dispatch!(e, insn)
}

/// An opcode without a template.
pub fn none(_: &mut Emit, _: &Insn) -> R {
    Err(NotNative)
}

const fn is_k(kind: u8) -> bool {
    kind == K || kind == KINT || kind == KSTR
}

/// A value operand's words: a register's (`Some(reg)`), or a literal's,
/// known now.
enum Operand {
    Reg(u32),
    Lit(u64, u64),
}

fn operand<S: Kind>(e: &Emit, x: u32) -> Result<Operand, NotNative> {
    if is_k(S::ID) {
        let (t, p) = plain_words(e.konst(x)?).ok_or(NotNative)?;
        Ok(Operand::Lit(t, p))
    } else {
        Ok(Operand::Reg(x))
    }
}

/// `dst = src` copied into a plain register, `src` a readable register or
/// a plain literal (`vmops::mov`, `vmops::assign`: a variable holding a
/// reference fails the plain test, so `assign`'s write through it is the
/// interpreter's).
fn copy(e: &mut Emit, dst: u32, src: Operand) -> R {
    let old = e.tag(dst);
    let plain = e.is_plain(old);
    let (t, p) = match src {
        Operand::Reg(r) => {
            let t = e.tag(r);
            let ok = e.is_readable(t);
            let both = e.b.ins().band(plain, ok);
            e.guard(both);
            (t, e.payload(r))
        }
        Operand::Lit(t, p) => {
            e.guard(plain);
            (e.iconst(t), e.iconst(p))
        }
    };
    e.store(dst, t, p);
    e.next()
}

/// `vmops::mov`.
pub fn mov<S: spec::Mov>(e: &mut Emit, args::Mov { dst, src }: args::Mov) -> R {
    let src = operand::<S::Src>(e, src)?;
    copy(e, dst, src)
}

/// `vmops::assign`.
pub fn assign<S: spec::Assign>(e: &mut Emit, args::Assign { var, src }: args::Assign) -> R {
    let src = operand::<S::Src>(e, src)?;
    copy(e, var, src)
}

/// `vmops::free`: a plain register made undefined.
pub fn free(e: &mut Emit, args::Free { reg }: args::Free) -> R {
    let t = e.tag(reg);
    let ok = e.is_plain(t);
    e.guard(ok);
    let (u, z) = (e.iconst(tag::UNDEF), e.iconst(0));
    e.store(reg, u, z);
    e.next()
}

/// `vmops::ret`: the interpreter's return runs it (what it reads and
/// moves out of the frame is its own).
pub fn ret<S: spec::Ret>(e: &mut Emit, _: args::Ret) -> R {
    e.hand_back()
}

/// `Op::ReturnNull`: as `ret`.
pub fn ret_null(e: &mut Emit, _: &Insn) -> R {
    e.hand_back()
}

/// `Op::LoadNull`.
pub fn load_null(e: &mut Emit, insn: &Insn) -> R {
    copy(e, u32::from(insn.a), Operand::Lit(tag::NULL, 0))
}

/// `Op::VerifyArgs`: with no parameter typed, every argument is plainly
/// of its type (`args_plain`), and there is nothing to do. A function
/// with a typed parameter is not native yet.
pub fn verify_args(e: &mut Emit, _: &Insn) -> R {
    if e.func.params.iter().any(|p| p.ty.is_some()) {
        return Err(NotNative);
    }
    e.next()
}

/// `Op::Jump`.
pub fn jump(e: &mut Emit, insn: &Insn) -> R {
    e.goto(insn.imm, e.pc)
}

/// `vmops::jump_cond`: a bool or an int condition (`truth`'s others,
/// null, a float, a string, are the interpreter's).
pub fn jump_cond<S: spec::JumpCond>(e: &mut Emit, args::JumpCond { cond, target }: args::JumpCond) -> R {
    let t = e.tag(cond);
    let is_bool = e.is(t, tag::BOOL);
    let is_int = e.is(t, tag::INT);
    let ok = e.b.ins().bor(is_bool, is_int);
    e.guard(ok);
    let p = e.payload(cond);
    let set = e.b.ins().icmp_imm(IntCC::NotEqual, p, 0);
    let go = if S::When::ID == T { set } else { e.b.ins().bxor_imm(set, 1) };
    e.branch(go, target)
}

/// `vmops::jump_cmp`: two ints (anything else is `test`'s, the
/// interpreter's).
pub fn jump_cmp<S: spec::JumpCmp>(e: &mut Emit, args::JumpCmp { a, b, target }: args::JumpCmp) -> R {
    let cc = match S::Cmp::ID {
        LT => IntCC::SignedLessThan,
        LE => IntCC::SignedLessThanOrEqual,
        GT => IntCC::SignedGreaterThan,
        GE => IntCC::SignedGreaterThanOrEqual,
        // EQ and ID: two ints are equal and identical alike.
        _ => IntCC::Equal,
    };
    let b = operand::<S::B>(e, b)?;
    if let Operand::Lit(t, _) = b
        && t != tag::INT
    {
        return Err(NotNative);
    }
    let ta = e.tag(a);
    let mut ok = e.is(ta, tag::INT);
    if let Operand::Reg(r) = b {
        let tb = e.tag(r);
        let int_b = e.is(tb, tag::INT);
        ok = e.b.ins().band(ok, int_b);
    }
    e.guard(ok);
    let x = e.payload(a);
    let y = match b {
        Operand::Reg(r) => e.payload(r),
        Operand::Lit(_, p) => e.iconst(p),
    };
    let holds = e.b.ins().icmp(cc, x, y);
    let go = if S::When::ID == T { holds } else { e.b.ins().bxor_imm(holds, 1) };
    e.branch(go, target)
}

/// The arithmetic the templates make native: `+`, `-`, `*`.
fn op_of(op: BinOp) -> Result<BinOp, NotNative> {
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul => Ok(op),
        _ => Err(NotNative),
    }
}

/// `dst = a op b` on numbers, as the loop's `arith!` and `arith` take
/// them: two ints checked (an overflow, whose result is a float, is the
/// interpreter's), else as floats when both are ints or floats (an int
/// made a float first, as PHP's `ZEND_ADD` does). `dst` must hold a plain
/// value (a variable holding a reference is written through by the
/// interpreter).
fn numbers(e: &mut Emit, op: BinOp, dst: u32, a: u32, b: Operand) -> R {
    let ta = e.tag(a);
    let old = e.tag(dst);
    let plain = e.is_plain(old);
    let (int_a, float_a) = (e.is(ta, tag::INT), e.is(ta, tag::FLOAT));
    let num_a = e.b.ins().bor(int_a, float_a);
    let (int_b, num_b) = match b {
        Operand::Reg(r) => {
            let tb = e.tag(r);
            let (i, f) = (e.is(tb, tag::INT), e.is(tb, tag::FLOAT));
            let n = e.b.ins().bor(i, f);
            (i, n)
        }
        Operand::Lit(t, _) if t == tag::INT || t == tag::FLOAT => {
            let i = e.b.ins().iconst(types::I8, i64::from(t == tag::INT));
            let one = e.b.ins().iconst(types::I8, 1);
            (i, one)
        }
        Operand::Lit(..) => return Err(NotNative),
    };
    let nums = e.b.ins().band(num_a, num_b);
    let ok = e.b.ins().band(nums, plain);
    e.guard(ok);
    let x = e.payload(a);
    let y = match b {
        Operand::Reg(r) => e.payload(r),
        Operand::Lit(_, p) => e.iconst(p),
    };
    let both_int = e.b.ins().band(int_a, int_b);
    let (ints, floats, join) = (e.b.create_block(), e.b.create_block(), e.b.create_block());
    e.b.ins().brif(both_int, ints, &[], floats, &[]);
    // Two ints: checked.
    e.b.switch_to_block(ints);
    let (n, over) = match op {
        BinOp::Add => e.b.ins().sadd_overflow(x, y),
        BinOp::Sub => e.b.ins().ssub_overflow(x, y),
        _ => e.b.ins().smul_overflow(x, y),
    };
    let fits = e.b.ins().icmp_imm(IntCC::Equal, over, 0);
    e.guard(fits);
    let it = e.iconst(tag::INT);
    e.store(dst, it, n);
    e.b.ins().jump(join, &[]);
    // A float on either side: both as floats.
    e.b.switch_to_block(floats);
    let fx = as_float(e, x, int_a);
    let fy = as_float(e, y, int_b);
    let r = match op {
        BinOp::Add => e.b.ins().fadd(fx, fy),
        BinOp::Sub => e.b.ins().fsub(fx, fy),
        _ => e.b.ins().fmul(fx, fy),
    };
    let bits = e.b.ins().bitcast(types::I64, MemFlags::new(), r);
    let ft = e.iconst(tag::FLOAT);
    e.store(dst, ft, bits);
    e.b.ins().jump(join, &[]);
    e.b.switch_to_block(join);
    e.next()
}

/// A number's payload as a float: an int's converted, a float's bits.
fn as_float(e: &mut Emit, payload: ir::Value, is_int: ir::Value) -> ir::Value {
    let converted = e.b.ins().fcvt_from_sint(types::F64, payload);
    let bits = e.b.ins().bitcast(types::F64, MemFlags::new(), payload);
    e.b.ins().select(is_int, converted, bits)
}

/// `Op::Add`, `Sub`, `Mul` and their constant forms (`AddC`: the literal
/// in `imm`): the loop's `arith!`.
pub fn arith(e: &mut Emit, insn: &Insn) -> R {
    let (op, lit) = match insn.op {
        Op::Add => (BinOp::Add, false),
        Op::Sub => (BinOp::Sub, false),
        Op::Mul => (BinOp::Mul, false),
        Op::AddC => (BinOp::Add, true),
        Op::SubC => (BinOp::Sub, true),
        Op::MulC => (BinOp::Mul, true),
        _ => return Err(NotNative),
    };
    let b = if lit {
        let (t, p) = plain_words(e.konst(insn.imm)?).ok_or(NotNative)?;
        Operand::Lit(t, p)
    } else {
        Operand::Reg(u32::from(insn.c))
    };
    numbers(e, op, u32::from(insn.a), u32::from(insn.b), b)
}

/// `Op::AssignOp` (`a (op imm) = b`) and `AssignOpC` (`a (op imm & 15) =
/// K (imm >> 8)`): the loop's `assign_op!`, for `+=`, `-=`, `*=`.
pub fn assign_op(e: &mut Emit, insn: &Insn) -> R {
    let var = u32::from(insn.a);
    let op = op_of(insn::binop(insn.imm))?;
    let b = match insn.op {
        Op::AssignOp => Operand::Reg(u32::from(insn.b)),
        Op::AssignOpC => {
            let (t, p) = plain_words(e.konst(insn.imm >> 8)?).ok_or(NotNative)?;
            Operand::Lit(t, p)
        }
        _ => return Err(NotNative),
    };
    numbers(e, op, var, var, b)
}

/// `Op::IncVar` and `DecVar`: an int stepped in place, unless it would
/// overflow.
pub fn inc_dec_var(e: &mut Emit, insn: &Insn) -> R {
    let var = u32::from(insn.a);
    let t = e.tag(var);
    let int = e.is(t, tag::INT);
    e.guard(int);
    let x = e.payload(var);
    let one = e.iconst(1);
    let (n, over) = if insn.op == Op::IncVar { e.b.ins().sadd_overflow(x, one) } else { e.b.ins().ssub_overflow(x, one) };
    let fits = e.b.ins().icmp_imm(IntCC::Equal, over, 0);
    e.guard(fits);
    e.store_payload(var, n);
    e.next()
}

/// The fused step and test (`IncJumpLt` and co., and their constant
/// forms): the loop's `step_jump!`. An int variable (else, or on an
/// overflow, the interpreter runs it); then, the bound an int, the jump
/// or the instruction after the test (the test is the next instruction,
/// which a bound of another type leaves to run: the frame is handed back
/// there, the step done).
pub fn step_jump(e: &mut Emit, insn: &Insn) -> R {
    let (inc, cc, lit) = match insn.op {
        Op::IncJumpLt => (true, IntCC::SignedLessThan, false),
        Op::IncJumpLe => (true, IntCC::SignedLessThanOrEqual, false),
        Op::DecJumpGt => (false, IntCC::SignedGreaterThan, false),
        Op::DecJumpGe => (false, IntCC::SignedGreaterThanOrEqual, false),
        Op::IncJumpLtC => (true, IntCC::SignedLessThan, true),
        Op::IncJumpLeC => (true, IntCC::SignedLessThanOrEqual, true),
        Op::DecJumpGtC => (false, IntCC::SignedGreaterThan, true),
        Op::DecJumpGeC => (false, IntCC::SignedGreaterThanOrEqual, true),
        _ => return Err(NotNative),
    };
    let (var, target, pc) = (u32::from(insn.a), insn.imm, e.pc);
    // The test after it, and where the loop goes past it.
    let after = e.block(pc + 2)?;
    let bound = if lit {
        match e.konst(insn::wide(insn) as u32)? {
            Frozen::Int(b) => Some(Operand::Lit(tag::INT, *b as u64)),
            _ => None,
        }
    } else {
        Some(Operand::Reg(u32::from(insn.b)))
    };
    let t = e.tag(var);
    let int = e.is(t, tag::INT);
    e.guard(int);
    let x = e.payload(var);
    let one = e.iconst(1);
    let (n, over) = if inc { e.b.ins().sadd_overflow(x, one) } else { e.b.ins().ssub_overflow(x, one) };
    let fits = e.b.ins().icmp_imm(IntCC::Equal, over, 0);
    e.guard(fits);
    let y = match bound {
        // A literal bound of another type: the step, then the test.
        None => {
            e.store_payload(var, n);
            e.exit(exit::GUARD, pc + 1);
            return Ok(());
        }
        Some(Operand::Lit(_, p)) => {
            e.store_payload(var, n);
            e.iconst(p)
        }
        Some(Operand::Reg(r)) => {
            let tb = e.tag(r);
            let int_b = e.is(tb, tag::INT);
            // (Read before the store: the bound may be the variable.)
            let y = e.payload(r);
            e.store_payload(var, n);
            e.guard_at(int_b, pc + 1);
            y
        }
    };
    let holds = e.b.ins().icmp(cc, n, y);
    let taken = e.b.create_block();
    e.b.ins().brif(holds, taken, &[], after, &[]);
    e.b.switch_to_block(taken);
    // A backward jump with an interrupt due: the step is done, the test
    // (next) sees it.
    e.goto(target, pc + 1)
}

/// `Op::Call`: the call helper runs it whole (`NativeFrame::call`); what
/// it does not take, the interpreter runs at the call.
pub fn call(e: &mut Emit, _: &Insn) -> R {
    let next = e.block(e.pc + 1)?;
    let pc = e.b.ins().iconst(types::I32, i64::from(e.pc));
    let inst = e.b.ins().call(e.call, &[e.ctx, pc]);
    let status = e.b.inst_results(inst)[0];
    let done = e.b.ins().icmp_imm(IntCC::Equal, status, i64::from(calls::DONE));
    let other = e.b.create_block();
    e.b.set_cold_block(other);
    e.b.ins().brif(done, next, &[], other, &[]);
    e.b.switch_to_block(other);
    // The other answers are the exit reasons of the same numbers.
    let reason = e.b.ins().uextend(types::I64, status);
    let high = e.b.ins().ishl_imm(reason, 32);
    let code = e.b.ins().bor_imm(high, i64::from(e.pc));
    e.b.ins().return_(&[code]);
    Ok(())
}

const _: () = {
    assert!(calls::NOT_TAKEN as u64 == exit::GUARD);
    assert!(calls::PENDING as u64 == exit::PENDING);
    assert!(calls::THREW as u64 == exit::THREW);
};

/// `Op::CallRegs` not taken as a frameless builtin call: its first Move
/// (the loop's fallback): argument register `imm >> 16` = register `a`,
/// or literal `a` (ARG_CONST). The call after it is the call helper's.
pub fn call_regs(e: &mut Emit, insn: &Insn) -> R {
    let dst = (insn.imm >> 16) & 0xff;
    let src = if insn.imm & insn::ARG_CONST != 0 {
        let (t, p) = plain_words(e.konst(u32::from(insn.a))?).ok_or(NotNative)?;
        Operand::Lit(t, p)
    } else {
        Operand::Reg(u32::from(insn.a))
    };
    copy(e, dst, src)
}

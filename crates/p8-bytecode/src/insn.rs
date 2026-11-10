//! The executed form of the code: 8-byte instructions (`Insn`), one per
//! `Instr` at the same pc, made from the function's `Instr`s once, when
//! the function is made (`lower`).
//!
//! `Instr` stays the compiler's output and the slow path's input (`step`
//! reads `code[pc]`); the interpreter's loop reads only the `Insn`s. Each
//! is an opcode and three `u8` register operands into the frame's
//! 256-register window (so an operand is in range by its type: no bounds
//! check, no mask) and a 32-bit immediate: a jump target, a cache or site
//! index, a count, or the flags and sub-operator of an opcode that has
//! them. An instruction the loop does not run inline, or one whose
//! registers do not fit in a byte (a function with more than 256), is
//! `Op::Slow`: its operands are the `Instr` at the same pc, the side table,
//! which `step` runs. So the stream stays dense whatever the cold
//! instructions need.
//!
//! The opcodes are generated (`ops.def`, p8-vmgen): the hand-written ones,
//! then one per specialisation of each generated instruction,
//! `P8_<OP>_<KINDS>`. Lowering classifies each operand (`Opnd`: a
//! temporary, a variable, a literal of a type, a flag it knows) and the
//! instruction's encoder (`enc`) picks the exact opcode and packs it.

use crate::{ClassRef, Cmp, Function, Instr, Rare, Reg, THIS, const_index};
use p8_runtime::frozen::Frozen;
use p8_runtime::ops::BinOp;

include!(concat!(env!("OUT_DIR"), "/ops.rs"));

/// One executed instruction: 8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insn {
    pub op: Op,
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub imm: u32,
}

const _: () = assert!(std::mem::size_of::<Insn>() == 8);
const _: () = assert!(std::mem::align_of::<Insn>() == 4);
const _: () = assert!(std::mem::size_of::<Op>() == 1);

/// An operand as lowering classifies it, for an encoder (`enc`): the
/// value is the register, the constant's index or the immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opnd {
    /// A temporary (a register at or past `ncv`).
    Tmp(u32),
    /// A compiled variable (below `ncv`).
    Cv(u32),
    /// A literal: of any other type, an int, a string.
    K(u32),
    KInt(u32),
    KStr(u32),
    Imm(u32),
    /// The frame's `$this`.
    This,
    /// A flag (`kinds::USED`, ..).
    Flag(u8),
}

fn val(o: Opnd) -> u32 {
    match o {
        Opnd::Tmp(v) | Opnd::Cv(v) | Opnd::K(v) | Opnd::KInt(v) | Opnd::KStr(v) | Opnd::Imm(v) => v,
        Opnd::This | Opnd::Flag(_) => 0,
    }
}

/// An operand's value in a byte field.
fn byte(o: Opnd) -> Option<u8> {
    u8::try_from(val(o)).ok()
}

/// An operand's value in an `n`-bit field.
fn bits(o: Opnd, n: u32) -> Option<u32> {
    let v = val(o);
    (n >= 32 || v < 1 << n).then_some(v)
}

/// The flag in `imm` of ArrayPush and ArraySet: the value operand is a
/// temporary read for the last time (moved, not copied).
pub const TAKE: u32 = 1 << 31;

/// IncDec's `imm`: increment (else decrement), and post (the old value).
pub const INC: u32 = 1;
pub const POST: u32 = 2;

/// BinaryC's `imm` bit: the constant is the left operand (`K op b`), not
/// the right (`b op K`).
pub const CONST_LEFT: u32 = 1 << 4;

/// ArraySetC's `imm` bits: the key is the constant in bits 0..14 (else
/// register b); the value is the constant in bits 14..28 (else register c).
pub const KEY_CONST: u32 = 1 << 28;
pub const VAL_CONST: u32 = 1 << 29;

/// CallRegs' `imm` bit: the one argument is constant `a` (an index below
/// 256), not the register.
pub const ARG_CONST: u32 = 1 << 30;

/// The constant operand of a fused step (the `b` and `c` bytes).
#[inline(always)]
pub fn wide(insn: &Insn) -> usize {
    usize::from(insn.b) | usize::from(insn.c) << 8
}

/// `a cmp b` as `b cmp' a`.
pub fn swapped(cmp: Cmp) -> Cmp {
    match cmp {
        Cmp::Lt => Cmp::Gt,
        Cmp::Le => Cmp::Ge,
        Cmp::Gt => Cmp::Lt,
        Cmp::Ge => Cmp::Le,
        other => other,
    }
}

/// The intrinsics' order in `Op::Intrinsic`'s immediate.
pub const INTRINSICS: [Rare; 10] =
    [Rare::Strlen, Rare::Count, Rare::IsNull, Rare::IsBool, Rare::IsInt, Rare::IsFloat, Rare::IsString, Rare::IsArray, Rare::IsObject, Rare::ArrayKeyExists];

/// The BinOp in an immediate (`lower`'s numbering).
#[inline(always)]
pub fn binop(imm: u32) -> BinOp {
    match imm & 15 {
        0 => BinOp::Add,
        1 => BinOp::Sub,
        2 => BinOp::Mul,
        3 => BinOp::Div,
        4 => BinOp::Mod,
        5 => BinOp::Pow,
        6 => BinOp::Concat,
        7 => BinOp::BitAnd,
        8 => BinOp::BitOr,
        9 => BinOp::BitXor,
        10 => BinOp::Shl,
        _ => BinOp::Shr,
    }
}

fn binop_imm(op: BinOp) -> u32 {
    match op {
        BinOp::Add => 0,
        BinOp::Sub => 1,
        BinOp::Mul => 2,
        BinOp::Div => 3,
        BinOp::Mod => 4,
        BinOp::Pow => 5,
        BinOp::Concat => 6,
        BinOp::BitAnd => 7,
        BinOp::BitOr => 8,
        BinOp::BitXor => 9,
        BinOp::Shl => 10,
        BinOp::Shr => 11,
    }
}

/// The Cmp in an immediate (`lower`'s numbering).
#[inline(always)]
pub fn cmp(imm: u32) -> Cmp {
    match imm & 15 {
        0 => Cmp::Eq,
        1 => Cmp::NotEq,
        2 => Cmp::Identical,
        3 => Cmp::NotIdentical,
        4 => Cmp::Lt,
        5 => Cmp::Le,
        6 => Cmp::Gt,
        7 => Cmp::Ge,
        _ => Cmp::Spaceship,
    }
}

fn cmp_imm(c: Cmp) -> u32 {
    match c {
        Cmp::Eq => 0,
        Cmp::NotEq => 1,
        Cmp::Identical => 2,
        Cmp::NotIdentical => 3,
        Cmp::Lt => 4,
        Cmp::Le => 5,
        Cmp::Gt => 6,
        Cmp::Ge => 7,
        Cmp::Spaceship => 8,
    }
}

/// A compare-and-branch's comparison as JUMP_CMP's flag.
fn cmp_flag(c: Cmp) -> Option<u8> {
    Some(match c {
        Cmp::Lt => kinds::LT,
        Cmp::Le => kinds::LE,
        Cmp::Gt => kinds::GT,
        Cmp::Ge => kinds::GE,
        Cmp::Eq => kinds::EQ,
        Cmp::Identical => kinds::ID,
        _ => return None,
    })
}

const SLOW: Insn = Insn { op: Op::Slow, a: 0, b: 0, c: 0, imm: 0 };

/// A register operand as a byte, if it is in the window.
fn r(reg: Reg) -> Option<u8> {
    u8::try_from(reg).ok()
}

/// The executed form of `func`'s code: one `Insn` per `Instr`.
pub fn lower(func: &Function) -> Vec<Insn> {
    let code = &func.code;
    (0..code.len()).map(|pc| lower_in(code, pc, &func.consts, func.ncv, &func.classes).unwrap_or(SLOW)).collect()
}

/// A class reference as the flag of an instruction that names a class:
/// NAMED for a class the code names (a name, once declared, is always the
/// same class, `self`/`parent` in a class included), STATIC for the called
/// class, SCOPE for the class a trait's code runs in; None for the rest
/// (a trait's `parent`, a class in a register).
fn class_flag(classes: &[ClassRef], class: u16) -> Option<Opnd> {
    Some(Opnd::Flag(match classes.get(usize::from(class))? {
        ClassRef::Named(..) | ClassRef::Forward(..) => kinds::NAMED,
        ClassRef::Static => kinds::STATIC,
        ClassRef::Scope => kinds::SCOPE,
        ClassRef::ParentScope | ClassRef::Dynamic(_) => return None,
    }))
}

/// The property cache entry of a name operand: its constant's index.
fn prop_site(name: Reg, nconsts: usize) -> Option<u32> {
    let i = const_index(name)?;
    (i < nconsts).then_some(i as u32)
}

/// A constant operand's index, when it is one and it is in the table.
fn k(reg: Reg, nconsts: usize) -> Option<u32> {
    let i = const_index(reg)?;
    (i < nconsts).then_some(i as u32)
}

/// Constant `i` as an operand: of its type. `KStr` is a string that is not
/// a canonical integer, so a KSTR handler looks it up as a string key
/// with no numeric test; a numeric one is `K` (a dimension's is its int:
/// `key_opnd`).
fn konst(consts: &[Frozen], i: usize) -> Option<Opnd> {
    Some(match consts.get(i)? {
        Frozen::Int(_) => Opnd::KInt(i as u32),
        Frozen::Str(s) if p8_runtime::array::int_key(s.as_bytes()).is_none() => Opnd::KStr(i as u32),
        _ => Opnd::K(i as u32),
    })
}

/// A register or constant operand, classified: a temporary or a variable
/// (by `ncv`), a literal by its type. None for a constant past the table.
fn opnd(reg: Reg, consts: &[Frozen], ncv: Reg) -> Option<Opnd> {
    match const_index(reg) {
        Some(i) => konst(consts, i),
        None if reg < ncv => Some(Opnd::Cv(u32::from(reg))),
        None => Some(Opnd::Tmp(u32::from(reg))),
    }
}

/// A dimension's key operand: a literal naming an integer (`'5'`) is that
/// int's constant (the compiler interns it), as PHP's compiler makes the
/// key (zend_handle_numeric_dim); None (not inline) without one. The
/// `Instr` keeps the string, for an ArrayAccess object on the cold path.
fn key_opnd(reg: Reg, consts: &[Frozen], ncv: Reg) -> Option<Opnd> {
    let int = match const_index(reg).and_then(|i| consts.get(i)) {
        Some(Frozen::Str(s)) => p8_runtime::array::int_key(s.as_bytes()),
        _ => None,
    };
    match int {
        Some(n) => consts.iter().position(|c| matches!(c, Frozen::Int(x) if *x == n)).map(|j| Opnd::KInt(j as u32)),
        None => opnd(reg, consts, ncv),
    }
}

/// UNUSED when the instruction after `pc` frees `dst` (the handler then
/// writes no result and goes past that Free), else USED.
fn res(code: &[Instr], pc: usize, dst: Reg) -> Opnd {
    Opnd::Flag(if matches!(code.get(pc + 1), Some(&Instr::Free { reg }) if reg == dst) { kinds::UNUSED } else { kinds::USED })
}

/// `lower_in` of code that names no class (the unit tests').
#[cfg(test)]
fn lower_at(code: &[Instr], pc: usize, consts: &[Frozen], ncv: Reg) -> Option<Insn> {
    lower_in(code, pc, consts, ncv, &[])
}

fn lower_in(code: &[Instr], pc: usize, consts: &[Frozen], ncv: Reg, classes: &[ClassRef]) -> Option<Insn> {
    let nconsts = consts.len();
    let i = |op: Op, a: Reg, b: Reg, c: Reg, imm: u32| -> Option<Insn> { Some(Insn { op, a: r(a)?, b: r(b)?, c: r(c)?, imm }) };
    let o = |reg: Reg| opnd(reg, consts, ncv);
    let ko = |reg: Reg| key_opnd(reg, consts, ncv);
    // A constant operand: its index, when the instruction has one there.
    let k = |reg: Reg| k(reg, nconsts);
    // The constant forms: `a` a register and K in the b and c bytes (a
    // fused step, whose imm is the target).
    let wide = |op: Op, a: Reg, kk: u32, imm: u32| -> Option<Insn> { Some(Insn { op, a: r(a)?, b: kk as u8, c: (kk >> 8) as u8, imm }) };
    // A comparison, one side a register and the other K: the test as
    // `reg cmp K`.
    let cmp_c = |cmp: Cmp, a: Reg, b: Reg| -> Option<(Cmp, Reg, u32)> {
        match (k(a), k(b)) {
            (None, Some(kb)) => Some((cmp, a, kb)),
            (Some(ka), None) => Some((swapped(cmp), b, ka)),
            _ => None,
        }
    };
    // `dst = a op K`, or `K op b` (CONST_LEFT).
    let bin_c = |op: BinOp, dst: Reg, a: Reg, b: Reg| -> Option<Insn> {
        match (k(a), k(b)) {
            (None, Some(kb)) => i(Op::BinaryC, dst, a, 0, binop_imm(op) | kb << 8),
            (Some(ka), None) => i(Op::BinaryC, dst, b, 0, binop_imm(op) | CONST_LEFT | ka << 8),
            _ => None,
        }
    };
    // A block of registers (arguments, a concatenation's parts) is in the
    // window whole: the loop indexes it from its first.
    let block = |start: Reg, n: u8| (n == 0 || r(start + Reg::from(n) - 1).is_some()).then_some(());
    match code[pc] {
        Instr::Rare { op, b, c, .. } if op.is_intrinsic() => block(b, u8::try_from(c).ok()?)?,
        Instr::ConcatN { start, n, .. } => block(start, n)?,
        Instr::Call { base, argc, .. } | Instr::MethodCall { base, argc, .. } | Instr::Construct { base, argc, .. } | Instr::StaticCall { base, argc, .. } => block(base, argc)?,
        // (The position is the register after the iterator's.)
        Instr::IterNext { iter, .. } => block(iter, 2)?,
        _ => {}
    }
    let take = |t: bool| if t { TAKE } else { 0 };
    match code[pc] {
        // The generated instructions (`ops.def`): the encoder picks the
        // specialisation of the operands' kinds.
        Instr::This { dst } => enc::this(enc::This { dst: o(dst)? }),
        Instr::FetchProp { dst, obj, name, .. } => {
            let site = Opnd::Imm(prop_site(name, nconsts)?);
            enc::fetch_prop(enc::FetchProp { dst: o(dst)?, obj: if obj == THIS { Opnd::This } else { o(obj)? }, site })
        }
        Instr::AssignProp { obj, name, val, dst, take: t } => {
            let site = Opnd::Imm(prop_site(name, nconsts)?);
            let obj = if obj == THIS { Opnd::This } else { o(obj)? };
            let mode = Opnd::Flag(if t { kinds::TAKE } else { kinds::COPY });
            enc::assign_prop(enc::AssignProp { obj, val: o(val)?, mode, res: res(code, pc, dst), dst: o(dst)?, site })
        }
        Instr::JumpLt { .. }
        | Instr::JumpNotLt { .. }
        | Instr::JumpLe { .. }
        | Instr::JumpNotLe { .. }
        | Instr::JumpEq { .. }
        | Instr::JumpNotEq { .. }
        | Instr::JumpIdentical { .. }
        | Instr::JumpNotIdentical { .. } => {
            let (cmp, a, b, when, target) = code[pc].as_jump_cmp()?;
            // A literal on the left goes to the right.
            let (cmp, a, b) = if const_index(a).is_some() && const_index(b).is_none() { (swapped(cmp), b, a) } else { (cmp, a, b) };
            let when = Opnd::Flag(if when { kinds::T } else { kinds::F });
            enc::jump_cmp(enc::JumpCmp { cmp: Opnd::Flag(cmp_flag(cmp)?), when, a: o(a)?, b: o(b)?, target: Opnd::Imm(target) })
        }
        Instr::Move { dst, src } => call_regs(code, pc).or_else(|| enc::mov(enc::Mov { dst: o(dst)?, src: o(src)? })),
        Instr::LoadConst { dst, k: kk } => call_regs(code, pc).or_else(|| enc::mov(enc::Mov { dst: o(dst)?, src: konst(consts, usize::from(kk))? })),
        Instr::Assign { var, src } => enc::assign(enc::Assign { var: o(var)?, src: o(src)? }),
        Instr::AssignTake { var, src } => enc::assign_take(enc::AssignTake { var: o(var)?, src: o(src)? }),
        Instr::Free { reg } => enc::free(enc::Free { reg: o(reg)? }),
        Instr::Return { src } => enc::ret(enc::Ret { src: o(src)? }),
        Instr::InstanceOf { dst, src, class } => {
            enc::instance_of(enc::InstanceOf { cls: class_flag(classes, class)?, dst: o(dst)?, src: o(src)?, class: Opnd::Imm(u32::from(class)) })
        }
        Instr::ClassConst { dst, class, name } => {
            let site = Opnd::Imm(prop_site(name, nconsts)?);
            enc::class_const(enc::ClassConst { cls: class_flag(classes, class)?, dst: o(dst)?, class: Opnd::Imm(u32::from(class)), site })
        }
        Instr::JumpIf { cond, target } => enc::jump_cond(enc::JumpCond { when: Opnd::Flag(kinds::T), cond: o(cond)?, target: Opnd::Imm(target) }),
        Instr::JumpIfNot { cond, target } => enc::jump_cond(enc::JumpCond { when: Opnd::Flag(kinds::F), cond: o(cond)?, target: Opnd::Imm(target) }),
        // (empty()'s last dimension is a read here: its misses are cold.)
        Instr::FetchDim { dst, arr, key, quiet, empty } => {
            let mode = Opnd::Flag(if quiet && !empty { kinds::IS } else { kinds::R });
            enc::fetch_dim(enc::FetchDim { mode, dst: o(dst)?, arr: o(arr)?, key: ko(key)? })
        }
        Instr::AssignDim { var, n: 0, append: true, val, dst, .. } => {
            enc::append_dim(enc::AppendDim { var: o(var)?, val: o(val)?, res: res(code, pc, dst), dst: o(dst)? })
        }
        Instr::IssetDim { dst, var, keys, n: 1 } => enc::isset_dim(enc::IssetDim { dst: o(dst)?, var: o(var)?, key: ko(keys)? }),
        Instr::AssignDim { var, keys, n: 1, append: false, val, dst } => {
            enc::assign_dim(enc::AssignDim { var: o(var)?, key: ko(keys)?, val: o(val)?, res: res(code, pc, dst), dst: o(dst)? })
        }

        // The hand-written ones.
        Instr::Rare { op: Rare::VerifyArgs, .. } => i(Op::VerifyArgs, 0, 0, 0, 0),
        Instr::Rare { op: Rare::VerifyReturn, a, .. } => i(Op::VerifyReturn, a, 0, 0, 0),
        Instr::Rare { op, a, b, c, class } if op.is_intrinsic() => {
            let kind = INTRINSICS.iter().position(|&k| k == op)? as u32;
            i(Op::Intrinsic, a, b, c, u32::from(class) | kind << 16)
        }
        Instr::SendVarMethod { obj, method, n, arg, var } => i(Op::SendVarMethod, obj, arg, var, u32::from(method) | u32::from(n) << 16),
        Instr::Jump { target } => i(Op::Jump, 0, 0, 0, target),
        Instr::Compare { cmp, dst, a, b } => match cmp_c(cmp, a, b) {
            Some((cmp, a, kk)) => i(Op::CompareC, dst, a, 0, cmp_imm(cmp) | kk << 8),
            None => i(Op::Compare, dst, a, b, cmp_imm(cmp)),
        },
        Instr::IncVar { var } => i(Op::IncVar, var, 0, 0, 0),
        Instr::DecVar { var } => i(Op::DecVar, var, 0, 0, 0),
        Instr::IncJumpLt { var, b, target } => match k(b) {
            Some(kk) => wide(Op::IncJumpLtC, var, kk, target),
            None => i(Op::IncJumpLt, var, b, 0, target),
        },
        Instr::IncJumpLe { var, b, target } => match k(b) {
            Some(kk) => wide(Op::IncJumpLeC, var, kk, target),
            None => i(Op::IncJumpLe, var, b, 0, target),
        },
        Instr::DecJumpGt { var, b, target } => match k(b) {
            Some(kk) => wide(Op::DecJumpGtC, var, kk, target),
            None => i(Op::DecJumpGt, var, b, 0, target),
        },
        Instr::DecJumpGe { var, b, target } => match k(b) {
            Some(kk) => wide(Op::DecJumpGeC, var, kk, target),
            None => i(Op::DecJumpGe, var, b, 0, target),
        },
        Instr::Binary { op, dst, a, b } => bin_c(op, dst, a, b).or_else(|| i(Op::Binary, dst, a, b, binop_imm(op))),
        // (`+` and `*` commute: a literal on the left goes to the right.)
        Instr::Add { dst, a, b } => match (k(a), k(b)) {
            (None, Some(kk)) => i(Op::AddC, dst, a, 0, kk),
            (Some(kk), None) => i(Op::AddC, dst, b, 0, kk),
            _ => i(Op::Add, dst, a, b, 0),
        },
        Instr::Sub { dst, a, b } => match k(b) {
            Some(kk) => i(Op::SubC, dst, a, 0, kk),
            None => i(Op::Sub, dst, a, b, 0),
        },
        Instr::Mul { dst, a, b } => match (k(a), k(b)) {
            (None, Some(kk)) => i(Op::MulC, dst, a, 0, kk),
            (Some(kk), None) => i(Op::MulC, dst, b, 0, kk),
            _ => i(Op::Mul, dst, a, b, 0),
        },
        Instr::ConcatN { dst, start, n } => i(Op::ConcatN, dst, start, Reg::from(n), 0),
        Instr::AssignDimOp { op, var, keys, n: 1, val, dst } => i(Op::AssignDimOp, var, keys, val, u32::from(r(dst)?) | binop_imm(op) << 8),
        Instr::IterNext { iter, val, key, has_key, end } => i(if has_key { Op::IterNextKey } else { Op::IterNext }, iter, val, if has_key { key } else { 0 }, end),
        Instr::AssignOp { op, var, src } => match k(src) {
            Some(kk) => i(Op::AssignOpC, var, 0, 0, binop_imm(op) | kk << 8),
            None => i(Op::AssignOp, var, src, 0, binop_imm(op)),
        },
        Instr::IncDec { var, dst, inc, post } => i(Op::IncDec, var, dst, 0, if inc { INC } else { 0 } | if post { POST } else { 0 }),
        Instr::LoadNull { dst } => i(Op::LoadNull, dst, 0, 0, 0),
        Instr::Not { dst, src } => i(Op::Not, dst, src, 0, 0),
        Instr::ToBool { dst, src } => i(Op::ToBool, dst, src, 0, 0),
        Instr::IssetVar { dst, var } => i(Op::IssetVar, dst, var, 0, 0),
        Instr::JumpIfSet { src, dst, target } => i(Op::JumpIfSet, src, dst, 0, target),
        Instr::NewArray { dst, n } => i(Op::NewArray, dst, 0, 0, u32::from(n)),
        Instr::ArrayPush { arr, val, take: t } => match k(val) {
            Some(kk) => i(Op::ArrayPushC, arr, 0, 0, kk),
            None => i(Op::ArrayPush, arr, val, 0, take(t)),
        },
        Instr::ArraySet { arr, key, val, take: t } => match (k(key), k(val)) {
            (None, None) => i(Op::ArraySet, arr, key, val, take(t)),
            (Some(kk), None) => i(Op::ArraySetC, arr, 0, val, take(t) | KEY_CONST | kk),
            (None, Some(kv)) => i(Op::ArraySetC, arr, key, 0, VAL_CONST | kv << 14),
            (Some(kk), Some(kv)) => i(Op::ArraySetC, arr, 0, 0, KEY_CONST | VAL_CONST | kk | kv << 14),
        },
        Instr::FetchConst { site, dst } => i(Op::FetchConst, dst, 0, 0, u32::from(site)),
        Instr::IterInit { src, iter, end } => {
            r(iter + 1)?;
            i(Op::IterInit, src, iter, 0, end)
        }
        Instr::SendVar { site, n, arg, var } => i(Op::SendVar, arg, var, 0, u32::from(site) | u32::from(n) << 16),
        Instr::Call { site, base, argc, dst } => i(Op::Call, base, Reg::from(argc), dst, u32::from(site)),
        Instr::MethodCall { dst, obj, method, base, argc } => i(Op::MethodCall, dst, obj, base, u32::from(method) | u32::from(argc) << 16),
        Instr::New { dst, class } => i(Op::New, dst, 0, 0, u32::from(class)),
        Instr::Construct { obj, base, argc } => i(Op::Construct, obj, base, Reg::from(argc), 0),
        Instr::StaticCall { dst, class, method, base, argc, vars: crate::NO_VARS } => i(Op::StaticCall, dst, base, Reg::from(argc), u32::from(method) | u32::from(class) << 16),
        Instr::ReturnNull => i(Op::ReturnNull, 0, 0, 0, 0),
        _ => None,
    }
}

/// The most arguments `Op::CallRegs` passes.
pub const CALL_REGS: usize = 3;

/// `Op::CallRegs` at `pc`: 1 to CALL_REGS `Move`s of consecutive source
/// registers (any one for a single argument) into the consecutive argument
/// registers of the `Call` right after them, or one `LoadConst` of a
/// constant below 256 into a one-argument call's (ARG_CONST: `a` is the
/// constant). The `Instr`s stay as they are (the Moves and the Call keep
/// their pcs), so a jump into the middle runs them one by one.
fn call_regs(code: &[Instr], pc: usize) -> Option<Insn> {
    if let Instr::LoadConst { dst: base, k } = code[pc] {
        let Some(&Instr::Call { site, base: b, argc: 1, dst }) = code.get(pc + 1) else { return None };
        if b != base {
            return None;
        }
        return Some(Insn { op: Op::CallRegs, a: u8::try_from(k).ok()?, b: 1, c: r(dst)?, imm: u32::from(site) | u32::from(r(base)?) << 16 | ARG_CONST });
    }
    let Instr::Move { dst: base, src: first } = code[pc] else { return None };
    for n in 1..=CALL_REGS {
        match *code.get(pc + n)? {
            Instr::Move { dst, src } if dst == base + n as Reg && src == first + n as Reg => continue,
            // The sources are not the argument registers themselves (those
            // are consumed by the call; a source stays as it was). The
            // result may be a source: it is written after the call.
            Instr::Call { site, base: b, argc, dst } if b == base && argc as usize == n && (first + n as Reg <= base || first >= base + n as Reg) => {
                r(first + n as Reg - 1)?;
                r(base + n as Reg - 1)?;
                return Some(Insn { op: Op::CallRegs, a: r(first)?, b: n as u8, c: r(dst)?, imm: u32::from(site) | u32::from(r(base)?) << 16 });
            }
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediates_round_trip() {
        for op in [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Mod, BinOp::Pow, BinOp::Concat, BinOp::BitAnd, BinOp::BitOr, BinOp::BitXor, BinOp::Shl, BinOp::Shr] {
            assert_eq!(binop(binop_imm(op)), op);
        }
        for c in [Cmp::Eq, Cmp::NotEq, Cmp::Identical, Cmp::NotIdentical, Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge, Cmp::Spaceship] {
            assert_eq!(cmp(cmp_imm(c)), c);
        }
    }

    /// 512 constants: ints, but 7 is a string and 9 a float.
    fn consts() -> Vec<Frozen> {
        (0..512)
            .map(|i| match i {
                7 => Frozen::Str("k".into()),
                8 => Frozen::Str("5".into()),
                10 => Frozen::Str("600".into()),
                9 => Frozen::Float(0.5),
                _ => Frozen::Int(i),
            })
            .collect()
    }

    #[test]
    fn moves_into_a_call_become_one_call() {
        let code = [
            Instr::Move { dst: 10, src: 2 },
            Instr::Move { dst: 11, src: 3 },
            Instr::Call { site: 4, base: 10, argc: 2, dst: 12 },
            Instr::Move { dst: 10, src: 7 },
            Instr::Call { site: 5, base: 10, argc: 1, dst: 7 },
            Instr::Move { dst: 10, src: 2 },
            Instr::Move { dst: 11, src: 5 },
            Instr::Call { site: 6, base: 10, argc: 2, dst: 12 },
        ];
        let at = |pc| lower_at(&code, pc, &[], 0).unwrap();
        assert_eq!(at(0), Insn { op: Op::CallRegs, a: 2, b: 2, c: 12, imm: 4 | 10 << 16 });
        assert_eq!(at(1).op, Op::P8_MOVE_REG);
        assert_eq!(at(3), Insn { op: Op::CallRegs, a: 7, b: 1, c: 7, imm: 5 | 10 << 16 });
        // Not consecutive: the Moves stay.
        assert_eq!(at(5).op, Op::P8_MOVE_REG);
        assert_eq!(at(6).op, Op::P8_MOVE_REG);
    }

    #[test]
    fn wide_registers_are_slow() {
        let code = [Instr::Add { dst: 300, a: 1, b: 2 }, Instr::Add { dst: 3, a: 1, b: 2 }, Instr::Move { dst: 300, src: 1 }, Instr::Free { reg: 256 }];
        assert!(lower_at(&code, 0, &[], 0).is_none());
        assert_eq!(lower_at(&code, 1, &[], 0).unwrap().op, Op::Add);
        assert!(lower_at(&code, 2, &[], 0).is_none());
        assert!(lower_at(&code, 3, &[], 0).is_none());
    }

    #[test]
    fn constant_operands_lower_to_constant_forms() {
        use crate::const_operand as c;
        let code = [
            Instr::JumpLt { a: 2, b: c(3), target: 9 },
            Instr::JumpLt { a: c(300), b: 2, target: 9 },
            Instr::JumpNotIdentical { a: c(1), b: 4, target: 9 },
            Instr::Compare { cmp: Cmp::Le, dst: 5, a: c(1), b: 4 },
            Instr::Add { dst: 5, a: c(7), b: 4 },
            Instr::Sub { dst: 5, a: 4, b: c(7) },
            Instr::Binary { op: BinOp::Concat, dst: 5, a: c(7), b: 4 },
            Instr::ArraySet { arr: 5, key: c(1), val: c(2), take: false },
            Instr::LoadConst { dst: 10, k: 7 },
            Instr::Call { site: 4, base: 10, argc: 1, dst: 12 },
            Instr::Assign { var: 1, src: c(2) },
            Instr::IncJumpLt { var: 1, b: c(258), target: 0 },
            Instr::Return { src: c(2) },
            // Both operands constants: no form (the compiler loads one).
            Instr::JumpEq { a: c(1), b: c(2), target: 9 },
        ];
        let consts = consts();
        let at = |pc| lower_at(&code, pc, &consts, 0).unwrap();
        assert_eq!(at(0), Insn { op: Op::P8_JUMP_CMP_LT_T_K, a: 2, b: 3, c: 0, imm: 9 });
        assert_eq!(at(1), Insn { op: Op::P8_JUMP_CMP_GT_T_K, a: 2, b: 44, c: 1, imm: 9 });
        assert_eq!(at(2), Insn { op: Op::P8_JUMP_CMP_ID_F_K, a: 4, b: 1, c: 0, imm: 9 });
        assert_eq!(at(3), Insn { op: Op::CompareC, a: 5, b: 4, c: 0, imm: cmp_imm(Cmp::Ge) | 1 << 8 });
        assert_eq!(at(4), Insn { op: Op::AddC, a: 5, b: 4, c: 0, imm: 7 });
        assert_eq!(at(5), Insn { op: Op::SubC, a: 5, b: 4, c: 0, imm: 7 });
        assert_eq!(at(6), Insn { op: Op::BinaryC, a: 5, b: 4, c: 0, imm: binop_imm(BinOp::Concat) | CONST_LEFT | 7 << 8 });
        assert_eq!(at(7), Insn { op: Op::ArraySetC, a: 5, b: 0, c: 0, imm: KEY_CONST | VAL_CONST | 1 | 2 << 14 });
        assert_eq!(at(8), Insn { op: Op::CallRegs, a: 7, b: 1, c: 12, imm: 4 | 10 << 16 | ARG_CONST });
        assert_eq!(at(10), Insn { op: Op::P8_ASSIGN_K, a: 1, b: 0, c: 0, imm: 2 });
        assert_eq!(at(11), Insn { op: Op::IncJumpLtC, a: 1, b: 2, c: 1, imm: 0 });
        assert_eq!(at(12), Insn { op: Op::P8_RETURN_K, a: 0, b: 0, c: 0, imm: 2 });
        assert!(lower_at(&code, 13, &consts, 0).is_none());
        // A constant past the table is no form either.
        assert!(lower_at(&code, 0, &consts[..3], 0).is_none());
    }

    #[test]
    fn the_kinds_pick_the_handler() {
        use crate::const_operand as c;
        let code = [
            Instr::FetchDim { dst: 5, arr: 1, key: c(7), quiet: false, empty: false },
            Instr::FetchDim { dst: 5, arr: 1, key: c(3), quiet: false, empty: false },
            Instr::FetchDim { dst: 5, arr: 1, key: 2, quiet: false, empty: false },
            // A float key: no specialisation (`step`'s).
            Instr::FetchDim { dst: 5, arr: 1, key: c(9), quiet: false, empty: false },
            // The result freed next: UNUSED, and the site and the value in imm.
            Instr::AssignProp { obj: THIS, name: c(4), val: c(5), dst: 6, take: false },
            Instr::Free { reg: 6 },
            Instr::AssignProp { obj: 1, name: c(4), val: 3, dst: 6, take: true },
            Instr::JumpIfNot { cond: 1, target: 77 },
            Instr::AssignDim { var: 1, keys: c(7), n: 1, append: false, val: 3, dst: 6 },
            Instr::Free { reg: 6 },
            // A literal key and a literal value: none.
            Instr::AssignDim { var: 1, keys: c(7), n: 1, append: false, val: c(3), dst: 6 },
            // `$a['k'] ?? ..` and `$a[] = $v;`.
            Instr::FetchDim { dst: 5, arr: 1, key: c(7), quiet: true, empty: false },
            Instr::AssignDim { var: 1, keys: 0, n: 0, append: true, val: 3, dst: 6 },
            Instr::Free { reg: 6 },
            // A numeric string literal key is no KSTR: its int's constant
            // (5 here), or none when the compiler interned none.
            Instr::FetchDim { dst: 5, arr: 1, key: c(8), quiet: false, empty: false },
            // `$o->p = K` (a literal value the compiler leaves in place).
            Instr::AssignProp { obj: 1, name: c(4), val: c(7), dst: 6, take: false },
            Instr::IssetDim { dst: 5, var: 1, keys: c(10), n: 1 },
            // As a value (not a key) a numeric string is K.
            Instr::Move { dst: 5, src: c(8) },
        ];
        let consts = consts();
        let at = |pc| lower_at(&code, pc, &consts, 2);
        assert_eq!(at(0).unwrap(), Insn { op: Op::P8_FETCH_DIM_R_KSTR, a: 5, b: 1, c: 0, imm: 7 });
        assert_eq!(at(1).unwrap().op, Op::P8_FETCH_DIM_R_KINT);
        assert_eq!(at(2).unwrap(), Insn { op: Op::P8_FETCH_DIM_R_REG, a: 5, b: 1, c: 2, imm: 0 });
        assert_eq!(at(11).unwrap().op, Op::P8_FETCH_DIM_IS_KSTR);
        assert_eq!(at(12).unwrap(), Insn { op: Op::P8_APPEND_DIM_REG_UNUSED, a: 1, b: 3, c: 6, imm: 0 });
        assert!(at(3).is_none());
        assert_eq!(at(4).unwrap(), Insn { op: Op::P8_ASSIGN_PROP_THIS_K_COPY_UNUSED, a: 6, b: 0, c: 0, imm: 5 | 4 << 16 });
        assert_eq!(at(6).unwrap(), Insn { op: Op::P8_ASSIGN_PROP_REG_REG_TAKE_USED, a: 1, b: 3, c: 6, imm: 4 });
        assert_eq!(at(7).unwrap(), Insn { op: Op::P8_JUMP_COND_F, a: 1, b: 0, c: 0, imm: 77 });
        assert_eq!(at(8).unwrap(), Insn { op: Op::P8_ASSIGN_DIM_KSTR_REG_UNUSED, a: 1, b: 3, c: 6, imm: 7 });
        assert!(at(10).is_none());
        assert_eq!(at(14).unwrap(), Insn { op: Op::P8_FETCH_DIM_R_KINT, a: 5, b: 1, c: 0, imm: 5 });
        assert_eq!(at(15).unwrap(), Insn { op: Op::P8_ASSIGN_PROP_REG_K_COPY_USED, a: 1, b: 6, c: 0, imm: 7 | 4 << 16 });
        assert!(at(16).is_none());
        assert_eq!(at(17).unwrap(), Insn { op: Op::P8_MOVE_K, a: 5, b: 0, c: 0, imm: 8 });
    }

    #[test]
    fn every_generated_opcode_is_listed() {
        // SPECS and the enum agree, in order, after the hand opcodes.
        let first = SPECS[0].0 as usize;
        for (n, (op, _, _)) in SPECS.iter().enumerate() {
            assert_eq!(*op as usize, first + n);
        }
        assert!(SPECS.len() > 40);
    }
}

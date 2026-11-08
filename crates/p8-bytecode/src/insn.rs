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

use crate::{Cmp, Function, Instr, Rare, Reg, THIS};
use p8_runtime::ops::BinOp;

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

/// The flag in `imm` of AssignProp, ArrayPush and ArraySet: the value
/// operand is a temporary read for the last time (moved, not copied).
pub const TAKE: u32 = 1 << 31;

/// IncDec's `imm`: increment (else decrement), and post (the old value).
pub const INC: u32 = 1;
pub const POST: u32 = 2;

/// An opcode. Operands are listed as `a`, `b`, `c` (registers) and `imm`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Not run by the loop: `step` runs the `Instr` at this pc.
    Slow,
    /// a = `$this`
    This,
    /// a = b->(property site imm)
    FetchProp,
    /// a = `$this`->(property site imm)
    FetchPropThis,
    /// a->(site imm & !TAKE) = b; c = the value
    AssignProp,
    /// `$this`->(site) = b; c = the value
    AssignPropThis,
    VerifyArgs,
    /// a: the value returned
    VerifyReturn,
    /// a = intrinsic (imm >> 16, `INTRINSICS`) of the c arguments at b; call site imm & 0xffff
    Intrinsic,
    /// pass b's variable c as argument imm >> 16 of method site imm & 0xffff on the object in a
    SendVarMethod,
    /// pc = imm
    Jump,
    /// compare a and b, jump to imm
    JumpLt,
    JumpNotLt,
    JumpLe,
    JumpNotLe,
    JumpEq,
    JumpNotEq,
    JumpIdentical,
    JumpNotIdentical,
    /// a = b (cmp imm, `cmp`) c
    Compare,
    IncVar,
    DecVar,
    /// step a, test against b, jump to imm
    IncJumpLt,
    IncJumpLe,
    DecJumpGt,
    DecJumpGe,
    /// a = b (op imm, `binop`) c
    Binary,
    Add,
    Sub,
    Mul,
    /// a = the c registers from b concatenated
    ConcatN,
    /// a = b
    Move,
    Assign,
    AssignTake,
    Free,
    /// test a, jump to imm
    JumpIf,
    JumpIfNot,
    /// a = b[c]
    FetchDim,
    /// a = isset(b[c])
    IssetDim,
    /// a[b] = c; imm = the result register
    AssignDim,
    /// a[b] (op imm >> 8) = c; imm & 0xff = the result register
    AssignDimOp,
    /// next of iterator a: value to b (and key to c for IterNextKey), or jump to imm
    IterNext,
    IterNextKey,
    /// a (op imm) = b
    AssignOp,
    /// a's step (imm: INC, POST) into b
    IncDec,
    LoadNull,
    /// a = !b
    Not,
    ToBool,
    /// a = isset(b)
    IssetVar,
    /// a ?? : b = a and jump to imm when set
    JumpIfSet,
    /// a = [] with room for imm
    NewArray,
    /// a[] = b (imm: TAKE)
    ArrayPush,
    /// a[b] = c (imm: TAKE)
    ArraySet,
    /// a = constant of site imm
    FetchConst,
    /// a = class constant: class reference imm >> 16, the name the
    /// constant imm & 0xffff (its cache entry, as a property name's)
    ClassConst,
    /// foreach over a: the array (b, b + 1 = it and its position 0), or to
    /// imm when it is empty
    IterInit,
    /// argument n (imm >> 16) of call site imm & 0xffff: a = b, when the
    /// function takes it by value
    SendVar,
    /// call site imm with the b arguments at a, result to c
    Call,
    /// `f(x0, .., xn-1)`, x the b registers from a (one for any register,
    /// several when they are consecutive), whose `Instr`s are the b `Move`s
    /// into the call's argument registers (from imm >> 16) and the `Call`
    /// (site imm & 0xffff, result to c) that follow: a frameless builtin
    /// call reads its arguments where they are, and pc goes past the call.
    /// Anything else runs the first Move (and the code after it) as it is.
    CallRegs,
    /// a = b->(method site imm & 0xffff) with the imm >> 16 arguments at c
    MethodCall,
    /// return a
    Return,
    ReturnNull,
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

const SLOW: Insn = Insn { op: Op::Slow, a: 0, b: 0, c: 0, imm: 0 };

/// A register operand as a byte, if it is in the window.
fn r(reg: Reg) -> Option<u8> {
    u8::try_from(reg).ok()
}

/// The executed form of `func`'s code: one `Insn` per `Instr`.
pub fn lower(func: &Function) -> Vec<Insn> {
    let code = &func.code;
    let consts = (func.consts_at as usize, func.consts.len());
    (0..code.len()).map(|pc| lower_at(code, pc, consts).unwrap_or(SLOW)).collect()
}

/// The property cache entry of a name register: its constant's index.
fn prop_site(name: Reg, (consts_at, nconsts): (usize, usize)) -> Option<u32> {
    let i = (name as usize).checked_sub(consts_at)?;
    (i < nconsts).then_some(i as u32)
}

fn lower_at(code: &[Instr], pc: usize, consts: (usize, usize)) -> Option<Insn> {
    let i = |op: Op, a: Reg, b: Reg, c: Reg, imm: u32| -> Option<Insn> { Some(Insn { op, a: r(a)?, b: r(b)?, c: r(c)?, imm }) };
    // A block of registers (arguments, a concatenation's parts) is in the
    // window whole: the loop indexes it from its first.
    let block = |start: Reg, n: u8| (n == 0 || r(start + Reg::from(n) - 1).is_some()).then_some(());
    match code[pc] {
        Instr::Rare { op, b, c, .. } if op.is_intrinsic() => block(b, u8::try_from(c).ok()?)?,
        Instr::ConcatN { start, n, .. } => block(start, n)?,
        Instr::Call { base, argc, .. } | Instr::MethodCall { base, argc, .. } => block(base, argc)?,
        // (The position is the register after the iterator's.)
        Instr::IterNext { iter, .. } => block(iter, 2)?,
        _ => {}
    }
    let take = |t: bool| if t { TAKE } else { 0 };
    match code[pc] {
        Instr::This { dst } => i(Op::This, dst, 0, 0, 0),
        Instr::FetchProp { dst, obj, name, .. } => {
            let site = prop_site(name, consts)?;
            if obj == THIS { i(Op::FetchPropThis, dst, 0, 0, site) } else { i(Op::FetchProp, dst, obj, 0, site) }
        }
        Instr::AssignProp { obj, name, val, dst, take: t } => {
            let site = prop_site(name, consts)?;
            if obj == THIS { i(Op::AssignPropThis, 0, val, dst, site | take(t)) } else { i(Op::AssignProp, obj, val, dst, site | take(t)) }
        }
        Instr::Rare { op: Rare::VerifyArgs, .. } => i(Op::VerifyArgs, 0, 0, 0, 0),
        Instr::Rare { op: Rare::VerifyReturn, a, .. } => i(Op::VerifyReturn, a, 0, 0, 0),
        Instr::Rare { op, a, b, c, class } if op.is_intrinsic() => {
            let kind = INTRINSICS.iter().position(|&k| k == op)? as u32;
            i(Op::Intrinsic, a, b, c, u32::from(class) | kind << 16)
        }
        Instr::SendVarMethod { obj, method, n, arg, var } => i(Op::SendVarMethod, obj, arg, var, u32::from(method) | u32::from(n) << 16),
        Instr::Jump { target } => i(Op::Jump, 0, 0, 0, target),
        Instr::JumpLt { a, b, target } => i(Op::JumpLt, a, b, 0, target),
        Instr::JumpNotLt { a, b, target } => i(Op::JumpNotLt, a, b, 0, target),
        Instr::JumpLe { a, b, target } => i(Op::JumpLe, a, b, 0, target),
        Instr::JumpNotLe { a, b, target } => i(Op::JumpNotLe, a, b, 0, target),
        Instr::JumpEq { a, b, target } => i(Op::JumpEq, a, b, 0, target),
        Instr::JumpNotEq { a, b, target } => i(Op::JumpNotEq, a, b, 0, target),
        Instr::JumpIdentical { a, b, target } => i(Op::JumpIdentical, a, b, 0, target),
        Instr::JumpNotIdentical { a, b, target } => i(Op::JumpNotIdentical, a, b, 0, target),
        Instr::Compare { cmp, dst, a, b } => i(Op::Compare, dst, a, b, cmp_imm(cmp)),
        Instr::IncVar { var } => i(Op::IncVar, var, 0, 0, 0),
        Instr::DecVar { var } => i(Op::DecVar, var, 0, 0, 0),
        Instr::IncJumpLt { var, b, target } => i(Op::IncJumpLt, var, b, 0, target),
        Instr::IncJumpLe { var, b, target } => i(Op::IncJumpLe, var, b, 0, target),
        Instr::DecJumpGt { var, b, target } => i(Op::DecJumpGt, var, b, 0, target),
        Instr::DecJumpGe { var, b, target } => i(Op::DecJumpGe, var, b, 0, target),
        Instr::Binary { op, dst, a, b } => i(Op::Binary, dst, a, b, binop_imm(op)),
        Instr::Add { dst, a, b } => i(Op::Add, dst, a, b, 0),
        Instr::Sub { dst, a, b } => i(Op::Sub, dst, a, b, 0),
        Instr::Mul { dst, a, b } => i(Op::Mul, dst, a, b, 0),
        Instr::ConcatN { dst, start, n } => i(Op::ConcatN, dst, start, Reg::from(n), 0),
        Instr::Move { dst, src } => call_regs(code, pc).or_else(|| i(Op::Move, dst, src, 0, 0)),
        Instr::Assign { var, src } => i(Op::Assign, var, src, 0, 0),
        Instr::AssignTake { var, src } => i(Op::AssignTake, var, src, 0, 0),
        Instr::Free { reg } => i(Op::Free, reg, 0, 0, 0),
        Instr::JumpIf { cond, target } => i(Op::JumpIf, cond, 0, 0, target),
        Instr::JumpIfNot { cond, target } => i(Op::JumpIfNot, cond, 0, 0, target),
        Instr::FetchDim { dst, arr, key, .. } => i(Op::FetchDim, dst, arr, key, 0),
        Instr::IssetDim { dst, var, keys, n: 1 } => i(Op::IssetDim, dst, var, keys, 0),
        Instr::AssignDim { var, keys, n: 1, append: false, val, dst } => i(Op::AssignDim, var, keys, val, u32::from(r(dst)?)),
        Instr::AssignDimOp { op, var, keys, n: 1, val, dst } => i(Op::AssignDimOp, var, keys, val, u32::from(r(dst)?) | binop_imm(op) << 8),
        Instr::IterNext { iter, val, key, has_key, end } => i(if has_key { Op::IterNextKey } else { Op::IterNext }, iter, val, if has_key { key } else { 0 }, end),
        Instr::AssignOp { op, var, src } => i(Op::AssignOp, var, src, 0, binop_imm(op)),
        Instr::IncDec { var, dst, inc, post } => i(Op::IncDec, var, dst, 0, if inc { INC } else { 0 } | if post { POST } else { 0 }),
        Instr::LoadNull { dst } => i(Op::LoadNull, dst, 0, 0, 0),
        Instr::Not { dst, src } => i(Op::Not, dst, src, 0, 0),
        Instr::ToBool { dst, src } => i(Op::ToBool, dst, src, 0, 0),
        Instr::IssetVar { dst, var } => i(Op::IssetVar, dst, var, 0, 0),
        Instr::JumpIfSet { src, dst, target } => i(Op::JumpIfSet, src, dst, 0, target),
        Instr::NewArray { dst, n } => i(Op::NewArray, dst, 0, 0, u32::from(n)),
        Instr::ArrayPush { arr, val, take: t } => i(Op::ArrayPush, arr, val, 0, take(t)),
        Instr::ArraySet { arr, key, val, take: t } => i(Op::ArraySet, arr, key, val, take(t)),
        Instr::FetchConst { site, dst } => i(Op::FetchConst, dst, 0, 0, u32::from(site)),
        Instr::IterInit { src, iter, end } => {
            r(iter + 1)?;
            i(Op::IterInit, src, iter, 0, end)
        }
        Instr::SendVar { site, n, arg, var } => i(Op::SendVar, arg, var, 0, u32::from(site) | u32::from(n) << 16),
        Instr::ClassConst { dst, class, name } => {
            let site = prop_site(name, consts).filter(|&s| s <= 0xffff)?;
            i(Op::ClassConst, dst, 0, 0, site | u32::from(class) << 16)
        }
        Instr::Call { site, base, argc, dst } => i(Op::Call, base, Reg::from(argc), dst, u32::from(site)),
        Instr::MethodCall { dst, obj, method, base, argc } => i(Op::MethodCall, dst, obj, base, u32::from(method) | u32::from(argc) << 16),
        Instr::Return { src } => i(Op::Return, src, 0, 0, 0),
        Instr::ReturnNull => i(Op::ReturnNull, 0, 0, 0, 0),
        _ => None,
    }
}

/// The most arguments `Op::CallRegs` passes.
pub const CALL_REGS: usize = 3;

/// `Op::CallRegs` at `pc`: 1 to CALL_REGS `Move`s of consecutive source
/// registers (any one for a single argument) into the consecutive argument
/// registers of the `Call` right after them. The `Instr`s stay as they
/// are (the Moves and the Call keep their pcs), so a jump into the middle
/// runs them one by one.
fn call_regs(code: &[Instr], pc: usize) -> Option<Insn> {
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
        let at = |pc| lower_at(&code, pc, (0, 0)).unwrap();
        assert_eq!(at(0), Insn { op: Op::CallRegs, a: 2, b: 2, c: 12, imm: 4 | 10 << 16 });
        assert_eq!(at(1).op, Op::Move);
        assert_eq!(at(3), Insn { op: Op::CallRegs, a: 7, b: 1, c: 7, imm: 5 | 10 << 16 });
        // Not consecutive: the Moves stay.
        assert_eq!(at(5).op, Op::Move);
        assert_eq!(at(6).op, Op::Move);
    }

    #[test]
    fn wide_registers_are_slow() {
        let code = [Instr::Add { dst: 300, a: 1, b: 2 }, Instr::Add { dst: 3, a: 1, b: 2 }];
        assert!(lower_at(&code, 0, (0, 0)).is_none());
        assert_eq!(lower_at(&code, 1, (0, 0)).unwrap().op, Op::Add);
    }
}

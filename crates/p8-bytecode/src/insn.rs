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

use crate::{Cmp, Function, Instr, Rare, Reg, THIS, const_index};
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

/// The constant operand of a compare-and-branch or a fused step (the
/// `b` and `c` bytes).
#[inline(always)]
pub fn wide(insn: &Insn) -> usize {
    usize::from(insn.b) | usize::from(insn.c) << 8
}

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
    /// a = a new object of class reference imm (its constructor not run)
    New,
    /// the constructor of the object in a, with the c arguments at b
    Construct,
    /// a = class reference imm >> 16's method (site imm & 0xffff) with
    /// the c arguments at b; sends no variables itself (`vars` is NO_VARS)
    StaticCall,
    /// return a
    Return,
    ReturnNull,

    // The constant forms: the same operations with one operand a literal,
    // read in place from the closure's constants (`Function::consts`), as
    // PHP's handlers read a CONST operand. `K` below is that constant.
    /// a = K (imm)
    LoadConst,
    /// variable a = K (imm)
    AssignC,
    /// return K (imm)
    ReturnC,
    /// compare a with K (`wide`: b | c << 8), jump to imm; K is the right
    /// operand (`a < K`), so `Gt`/`Ge` cover a literal on the left
    JumpLtC,
    JumpNotLtC,
    JumpLeC,
    JumpNotLeC,
    JumpGtC,
    JumpNotGtC,
    JumpGeC,
    JumpNotGeC,
    JumpEqC,
    JumpNotEqC,
    JumpIdenticalC,
    JumpNotIdenticalC,
    /// a = b (cmp imm & 15, `cmp`) K (imm >> 8)
    CompareC,
    /// a = b op K (imm)
    AddC,
    SubC,
    MulC,
    /// a = b (op imm & 15, `binop`) K (imm >> 8), or K op b with CONST_LEFT
    BinaryC,
    /// a (op imm & 15) = K (imm >> 8)
    AssignOpC,
    /// step a, test against K (`wide`), jump to imm
    IncJumpLtC,
    IncJumpLeC,
    DecJumpGtC,
    DecJumpGeC,
    /// a = b[K] (imm)
    FetchDimC,
    /// a = isset(b[K]) (imm)
    IssetDimC,
    /// a[K (imm >> 8)] = c; imm & 0xff = the result register
    AssignDimCK,
    /// a[b] = K (imm >> 8); imm & 0xff = the result register
    AssignDimCV,
    /// a[] = K (imm)
    ArrayPushC,
    /// a[key] = val with the key, the value or both constants (KEY_CONST,
    /// VAL_CONST; the constants in imm bits 0..14 and 14..28; TAKE as
    /// ArraySet's)
    ArraySetC,
}

/// The constant-form opcode of a compare-and-branch: jump when `a cmp K`
/// is `when`, for the conditions a branch tests.
fn jump_c(cmp: Cmp, when: bool) -> Option<Op> {
    Some(match (cmp, when) {
        (Cmp::Lt, true) => Op::JumpLtC,
        (Cmp::Lt, false) => Op::JumpNotLtC,
        (Cmp::Le, true) => Op::JumpLeC,
        (Cmp::Le, false) => Op::JumpNotLeC,
        (Cmp::Gt, true) => Op::JumpGtC,
        (Cmp::Gt, false) => Op::JumpNotGtC,
        (Cmp::Ge, true) => Op::JumpGeC,
        (Cmp::Ge, false) => Op::JumpNotGeC,
        (Cmp::Eq, true) => Op::JumpEqC,
        (Cmp::Eq, false) => Op::JumpNotEqC,
        (Cmp::Identical, true) => Op::JumpIdenticalC,
        (Cmp::Identical, false) => Op::JumpNotIdenticalC,
        _ => return None,
    })
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

const SLOW: Insn = Insn { op: Op::Slow, a: 0, b: 0, c: 0, imm: 0 };

/// A register operand as a byte, if it is in the window.
fn r(reg: Reg) -> Option<u8> {
    u8::try_from(reg).ok()
}

/// The executed form of `func`'s code: one `Insn` per `Instr`.
pub fn lower(func: &Function) -> Vec<Insn> {
    let code = &func.code;
    (0..code.len()).map(|pc| lower_at(code, pc, func.consts.len()).unwrap_or(SLOW)).collect()
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

fn lower_at(code: &[Instr], pc: usize, nconsts: usize) -> Option<Insn> {
    let i = |op: Op, a: Reg, b: Reg, c: Reg, imm: u32| -> Option<Insn> { Some(Insn { op, a: r(a)?, b: r(b)?, c: r(c)?, imm }) };
    // A constant operand: its index, when the instruction has one there.
    let k = |reg: Reg| k(reg, nconsts);
    // The constant forms: `a` a register and K in the b and c bytes (a
    // branch or a fused step, whose imm is the target), or K in imm.
    let wide = |op: Op, a: Reg, kk: u32, imm: u32| -> Option<Insn> { Some(Insn { op, a: r(a)?, b: kk as u8, c: (kk >> 8) as u8, imm }) };
    // A comparison, one side a register and the other K: the opcode
    // (and the test as `reg cmp K`).
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
        Instr::This { dst } => i(Op::This, dst, 0, 0, 0),
        Instr::FetchProp { dst, obj, name, .. } => {
            let site = prop_site(name, nconsts)?;
            if obj == THIS { i(Op::FetchPropThis, dst, 0, 0, site) } else { i(Op::FetchProp, dst, obj, 0, site) }
        }
        Instr::AssignProp { obj, name, val, dst, take: t } => {
            let site = prop_site(name, nconsts)?;
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
        Instr::JumpLt { .. }
        | Instr::JumpNotLt { .. }
        | Instr::JumpLe { .. }
        | Instr::JumpNotLe { .. }
        | Instr::JumpEq { .. }
        | Instr::JumpNotEq { .. }
        | Instr::JumpIdentical { .. }
        | Instr::JumpNotIdentical { .. } => {
            let (cmp, a, b, when, target) = code[pc].as_jump_cmp()?;
            match cmp_c(cmp, a, b) {
                Some((cmp, a, kk)) => wide(jump_c(cmp, when)?, a, kk, target),
                None => {
                    let op = match (cmp, when) {
                        (Cmp::Lt, true) => Op::JumpLt,
                        (Cmp::Lt, false) => Op::JumpNotLt,
                        (Cmp::Le, true) => Op::JumpLe,
                        (Cmp::Le, false) => Op::JumpNotLe,
                        (Cmp::Eq, true) => Op::JumpEq,
                        (Cmp::Eq, false) => Op::JumpNotEq,
                        (Cmp::Identical, true) => Op::JumpIdentical,
                        (Cmp::Identical, false) => Op::JumpNotIdentical,
                        _ => return None,
                    };
                    i(op, a, b, 0, target)
                }
            }
        }
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
        Instr::Move { dst, src } => call_regs(code, pc).or_else(|| i(Op::Move, dst, src, 0, 0)),
        Instr::LoadConst { dst, k: kk } => call_regs(code, pc).or_else(|| i(Op::LoadConst, dst, 0, 0, u32::from(kk))),
        Instr::Assign { var, src } => match k(src) {
            Some(kk) => i(Op::AssignC, var, 0, 0, kk),
            None => i(Op::Assign, var, src, 0, 0),
        },
        Instr::AssignTake { var, src } => i(Op::AssignTake, var, src, 0, 0),
        Instr::Free { reg } => i(Op::Free, reg, 0, 0, 0),
        Instr::JumpIf { cond, target } => i(Op::JumpIf, cond, 0, 0, target),
        Instr::JumpIfNot { cond, target } => i(Op::JumpIfNot, cond, 0, 0, target),
        Instr::FetchDim { dst, arr, key, .. } => match k(key) {
            Some(kk) => i(Op::FetchDimC, dst, arr, 0, kk),
            None => i(Op::FetchDim, dst, arr, key, 0),
        },
        Instr::IssetDim { dst, var, keys, n: 1 } => match k(keys) {
            Some(kk) => i(Op::IssetDimC, dst, var, 0, kk),
            None => i(Op::IssetDim, dst, var, keys, 0),
        },
        Instr::AssignDim { var, keys, n: 1, append: false, val, dst } => match (k(keys), k(val)) {
            (Some(kk), None) => i(Op::AssignDimCK, var, 0, val, u32::from(r(dst)?) | kk << 8),
            (None, Some(kk)) => i(Op::AssignDimCV, var, keys, 0, u32::from(r(dst)?) | kk << 8),
            _ => i(Op::AssignDim, var, keys, val, u32::from(r(dst)?)),
        },
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
        Instr::ClassConst { dst, class, name } => {
            let site = prop_site(name, nconsts).filter(|&s| s <= 0xffff)?;
            i(Op::ClassConst, dst, 0, 0, site | u32::from(class) << 16)
        }
        Instr::Call { site, base, argc, dst } => i(Op::Call, base, Reg::from(argc), dst, u32::from(site)),
        Instr::MethodCall { dst, obj, method, base, argc } => i(Op::MethodCall, dst, obj, base, u32::from(method) | u32::from(argc) << 16),
        Instr::New { dst, class } => i(Op::New, dst, 0, 0, u32::from(class)),
        Instr::Construct { obj, base, argc } => i(Op::Construct, obj, base, Reg::from(argc), 0),
        Instr::StaticCall { dst, class, method, base, argc, vars: crate::NO_VARS } => i(Op::StaticCall, dst, base, Reg::from(argc), u32::from(method) | u32::from(class) << 16),
        Instr::Return { src } => match k(src) {
            Some(kk) => i(Op::ReturnC, 0, 0, 0, kk),
            None => i(Op::Return, src, 0, 0, 0),
        },
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
        let at = |pc| lower_at(&code, pc, 0).unwrap();
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
        assert!(lower_at(&code, 0, 0).is_none());
        assert_eq!(lower_at(&code, 1, 0).unwrap().op, Op::Add);
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
        let at = |pc| lower_at(&code, pc, 512).unwrap();
        assert_eq!(at(0), Insn { op: Op::JumpLtC, a: 2, b: 3, c: 0, imm: 9 });
        assert_eq!(at(1), Insn { op: Op::JumpGtC, a: 2, b: 44, c: 1, imm: 9 });
        assert_eq!(at(2), Insn { op: Op::JumpNotIdenticalC, a: 4, b: 1, c: 0, imm: 9 });
        assert_eq!(at(3), Insn { op: Op::CompareC, a: 5, b: 4, c: 0, imm: cmp_imm(Cmp::Ge) | 1 << 8 });
        assert_eq!(at(4), Insn { op: Op::AddC, a: 5, b: 4, c: 0, imm: 7 });
        assert_eq!(at(5), Insn { op: Op::SubC, a: 5, b: 4, c: 0, imm: 7 });
        assert_eq!(at(6), Insn { op: Op::BinaryC, a: 5, b: 4, c: 0, imm: binop_imm(BinOp::Concat) | CONST_LEFT | 7 << 8 });
        assert_eq!(at(7), Insn { op: Op::ArraySetC, a: 5, b: 0, c: 0, imm: KEY_CONST | VAL_CONST | 1 | 2 << 14 });
        assert_eq!(at(8), Insn { op: Op::CallRegs, a: 7, b: 1, c: 12, imm: 4 | 10 << 16 | ARG_CONST });
        assert_eq!(at(10), Insn { op: Op::AssignC, a: 1, b: 0, c: 0, imm: 2 });
        assert_eq!(at(11), Insn { op: Op::IncJumpLtC, a: 1, b: 2, c: 1, imm: 0 });
        assert_eq!(at(12), Insn { op: Op::ReturnC, a: 0, b: 0, c: 0, imm: 2 });
        assert!(lower_at(&code, 13, 512).is_none());
        // A constant past the table is no form either.
        assert!(lower_at(&code, 0, 3).is_none());
    }
}

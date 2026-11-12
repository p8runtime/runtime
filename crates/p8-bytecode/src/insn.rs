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

use crate::{CastKind, ClassRef, Cmp, Function, Instr, Rare, Reg, THIS, const_index};
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

/// DimIncDec's `imm` bits (above the key count): INC and POST.
pub const DIM_INC: u32 = INC << 8;
pub const DIM_POST: u32 = POST << 8;

/// PropIncDec's `imm` bits (above the property site): INC and POST; and
/// PROP_PATH_THIS (the object is `$this`).
pub const PROP_INC: u32 = INC << 16;
pub const PROP_POST: u32 = POST << 16;

/// Cast's `imm`: the kind (`(unset)` is `Op::Slow`).
pub const CAST_INT: u32 = 0;
pub const CAST_FLOAT: u32 = 1;
pub const CAST_STRING: u32 = 2;
pub const CAST_BOOL: u32 = 3;
pub const CAST_ARRAY: u32 = 4;

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

/// Call's `imm` bit: the instruction after it moves the result into a
/// variable (`$v = f(..)`), and bits 16..24 name that variable; without
/// it they name the result register (`c`, which always does). A frame
/// called with it stores its result in the variable and goes past the
/// move (`leave_fast`).
pub const RET_VAR: u32 = 1 << 31;

/// IterNextRef's `imm` bit (above the end's pc): the key is assigned too.
pub const ITER_KEY: u32 = 1 << 31;

/// FetchList's `imm`: the key is register c (else it is constant imm).
pub const LIST_KEY_REG: u32 = 1 << 31;

/// AssignPropOp's `imm` bit (with PROP_PATH_THIS; the operator in bits
/// 16..20): the value is constant b, not register b.
pub const PROP_OP_K: u32 = 1 << 30;

/// RefDim's and DimSetRef's `imm`: the key count (bits 0..8), REF_APPEND
/// (a last `[]`), REF_KEY_K (the one key is constant `imm >> 10`, not
/// register c).
pub const REF_APPEND: u32 = 1 << 8;
pub const REF_KEY_K: u32 = 1 << 9;

/// Yield's `imm`: the value constant `imm & 0x3fff` with YIELD_VALUE_K
/// (else register b), the key constant `imm >> 14 & 0x3fff` with
/// YIELD_KEY_K (else register c), YIELD_HAS_KEY (`yield k => v`).
/// YieldFrom's: its source constant `imm & 0x3fff` with YIELD_VALUE_K.
pub const YIELD_VALUE_K: u32 = 1 << 28;
pub const YIELD_KEY_K: u32 = 1 << 29;
pub const YIELD_HAS_KEY: u32 = 1 << 30;

/// Include's and FromCallable's `imm` bit: the operand is constant
/// `imm >> 8 & 0x3fff` (else register b); Include's kind in bits 0..2
/// (`include_kind`).
pub const SRC_K: u32 = 1 << 31;

/// Include's kind in an immediate (`lower`'s numbering).
pub fn include_kind(imm: u32) -> crate::IncludeKind {
    match imm & 3 {
        0 => crate::IncludeKind::Include,
        1 => crate::IncludeKind::IncludeOnce,
        2 => crate::IncludeKind::Require,
        _ => crate::IncludeKind::RequireOnce,
    }
}

/// AssignStaticPropOp's `imm`: the name constant (bits 0..14), the
/// operator (14..18), STATIC_OP_K (the value is constant b, else
/// register b).
pub const STATIC_OP_K: u32 = 1 << 18;

/// IssetProp's `imm`: the name is register c (else `imm` is the name's
/// property site).
pub const ISSET_PROP_DYN: u32 = 1 << 31;

/// Capture's `imm` bits: by reference (`use (&$x)`); quiet (no warning
/// for an undefined variable).
pub const CAPTURE_REF: u32 = 1;
pub const CAPTURE_QUIET: u32 = 2;

/// The most keys `Op::AssignDimPath` writes through.
pub const PATH_KEYS: u8 = 4;

/// AssignDimPath's `imm` bit: the result is freed by the next instruction
/// (none is written, and that Free is skipped).
pub const PATH_UNUSED: u32 = 1 << 31;

/// AssignDimPath's `imm` bit: the value is constant c (else register c).
pub const PATH_VAL_K: u32 = 1 << 16;

/// AssignPropDim's `imm` bit: the object is the frame's `$this`.
pub const PROP_PATH_THIS: u32 = 1 << 29;

/// FetchPropDyn's `imm` bit: a quiet read (`??`, `?->`: no warning).
pub const PROP_QUIET: u32 = 1;

/// AssignDimPath's and AssignPropDim's `imm` bit: a `[]` after the keys
/// (the value appended there).
pub const PATH_APPEND: u32 = 1 << 30;

/// A path write's PATH_UNUSED and PATH_APPEND.
fn path_flags(code: &[Instr], pc: usize, dst: Reg, append: bool) -> u32 {
    let unused = if res(code, pc, dst) == Opnd::Flag(kinds::UNUSED) { PATH_UNUSED } else { 0 };
    unused | if append { PATH_APPEND } else { 0 }
}

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
    let mut insns: Vec<Insn> = (0..code.len()).map(|pc| lower_in(code, pc, &func.consts, func.ncv, &func.classes, &func.arg_vars).unwrap_or(SLOW)).collect();
    if !func.frees_temps {
        // A temporary of code that does not free each where it dies may
        // still hold a dead value, which a call's result releases as it
        // lands there: such a call stores in its register.
        for insn in &mut insns {
            *insn = to_register(*insn);
        }
    }
    insns
}

/// A call as one that stores its result in its register (no RET_VAR).
fn to_register(insn: Insn) -> Insn {
    match insn.op {
        Op::Call => Insn { imm: insn.imm & 0xffff | u32::from(insn.c) << 16, ..insn },
        _ => insn,
    }
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

/// `$x::class`: the register of class reference `class` when it is a
/// class in a register (`Dynamic`) and `name` is the literal `class`.
fn dynamic_class_name(classes: &[ClassRef], class: u16, name: Reg, consts: &[Frozen]) -> Option<Reg> {
    let ClassRef::Dynamic(src) = *classes.get(usize::from(class))? else { return None };
    match consts.get(const_index(name)?)? {
        Frozen::Str(s) if s.as_bytes().eq_ignore_ascii_case(b"class") => Some(src),
        _ => None,
    }
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
    lower_in(code, pc, consts, ncv, &[], &[])
}

/// The instruction at `pc` lowered; `classes` and `arg_vars` are the
/// function's (`Function::classes`, `Function::arg_vars`).
fn lower_in(code: &[Instr], pc: usize, consts: &[Frozen], ncv: Reg, classes: &[ClassRef], arg_vars: &[Vec<(u8, Reg)>]) -> Option<Insn> {
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
            // (`K <=> $b` is not `$b <=> K`: it keeps its order, in `step`.)
            (Some(ka), None) if cmp != Cmp::Spaceship => Some((swapped(cmp), b, ka)),
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
    // An instruction with one source a register (b) or a constant (SRC_K,
    // its index in `imm >> 8`), `imm`'s low byte its own.
    let src_or_k = |op: Op, dst: Reg, src: Reg, low: u32| -> Option<Insn> {
        match k(src) {
            Some(kk) if kk < 1 << 14 => i(op, dst, 0, 0, low | kk << 8 | SRC_K),
            Some(_) => None,
            None => i(op, dst, src, 0, low),
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
        Instr::FetchProp { dst, obj, name, quiet } => match prop_site(name, nconsts) {
            Some(site) => {
                let mode = Opnd::Flag(if quiet { kinds::IS } else { kinds::R });
                enc::fetch_prop(enc::FetchProp { mode, dst: o(dst)?, obj: if obj == THIS { Opnd::This } else { o(obj)? }, site: Opnd::Imm(site) })
            }
            // `$o->$name`: the name in a register (a hand arm).
            None => i(Op::FetchPropDyn, dst, if obj == THIS { 0 } else { obj }, name, if obj == THIS { PROP_PATH_THIS } else { 0 } | if quiet { PROP_QUIET } else { 0 }),
        },
        Instr::AssignProp { obj, name, val, dst, take: t } => match prop_site(name, nconsts) {
            Some(site) => {
                let obj = if obj == THIS { Opnd::This } else { o(obj)? };
                let mode = Opnd::Flag(if t { kinds::TAKE } else { kinds::COPY });
                enc::assign_prop(enc::AssignProp { obj, val: o(val)?, mode, res: res(code, pc, dst), dst: o(dst)?, site: Opnd::Imm(site) })
            }
            // `$o->$name = $v`: the name and the value in registers (a hand
            // arm; the result always written, the Free after run as it is).
            None => {
                let flags = if obj == THIS { PROP_PATH_THIS } else { 0 } | if t { TAKE } else { 0 };
                let (v, vk) = match k(val) {
                    Some(kk) if kk < 256 => (kk as Reg, PROP_OP_K),
                    Some(_) => return None,
                    None => (val, 0),
                };
                i(Op::AssignPropDyn, if obj == THIS { 0 } else { obj }, v, name, flags | vk | u32::from(r(dst)?) << 8)
            }
        },
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
            // (A class in a register, `parent`: OTHER, always out of line.)
            classes.get(usize::from(class))?;
            let cls = class_flag(classes, class).unwrap_or(Opnd::Flag(kinds::OTHER));
            enc::instance_of(enc::InstanceOf { cls, dst: o(dst)?, src: o(src)?, class: Opnd::Imm(u32::from(class)) })
        }
        Instr::ClassConst { dst, class, name } => match dynamic_class_name(classes, class, name, consts) {
            // `$obj::class`.
            Some(src) => i(Op::ClassName, dst, src, 0, u32::from(class)),
            None => {
                let site = Opnd::Imm(prop_site(name, nconsts)?);
                enc::class_const(enc::ClassConst { cls: class_flag(classes, class)?, dst: o(dst)?, class: Opnd::Imm(u32::from(class)), site })
            }
        },
        Instr::FetchStaticProp { dst, class, name } => {
            let site = Opnd::Imm(prop_site(name, nconsts)?);
            enc::fetch_static_prop(enc::FetchStaticProp { cls: class_flag(classes, class)?, dst: o(dst)?, class: Opnd::Imm(u32::from(class)), site })
        }
        Instr::AssignStaticProp { class, name, val, dst } => {
            let site = Opnd::Imm(prop_site(name, nconsts)?);
            enc::assign_static_prop(enc::AssignStaticProp { cls: class_flag(classes, class)?, val: o(val)?, dst: o(dst)?, class: Opnd::Imm(u32::from(class)), site })
        }
        Instr::JumpIf { cond, target } => enc::jump_cond(enc::JumpCond { when: Opnd::Flag(kinds::T), cond: o(cond)?, target: Opnd::Imm(target) }),
        Instr::JumpIfNot { cond, target } => enc::jump_cond(enc::JumpCond { when: Opnd::Flag(kinds::F), cond: o(cond)?, target: Opnd::Imm(target) }),
        // (empty()'s last dimension, EMPTY, the handler reads as IS: on an
        // array, null, a string's int offset, a miss is null as for `??`;
        // an ArrayAccess object, where the two differ, and empty()'s key
        // conversions are out of line, which tells them apart.)
        Instr::FetchDim { dst, arr, key, quiet, empty } => {
            let mode = Opnd::Flag(if empty {
                kinds::EMPTY
            } else if quiet {
                kinds::IS
            } else {
                kinds::R
            });
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
        // (b: 0 a value returned, 1 none, for the TypeError's text.)
        Instr::Rare { op: Rare::VerifyReturn, a, b, .. } => i(Op::VerifyReturn, a, u16::from(b != 0), 0, 0),
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
        Instr::Call { site, base, argc, dst } => {
            let (ret, flag) = match ret_var(code, pc, dst, ncv) {
                Some(var) => (var, RET_VAR),
                None => (r(dst)?, 0),
            };
            i(Op::Call, base, Reg::from(argc), dst, u32::from(site) | u32::from(ret) << 16 | flag)
        }
        Instr::MethodCall { dst, obj, method, base, argc } => i(Op::MethodCall, dst, obj, base, u32::from(method) | u32::from(argc) << 16),
        Instr::New { dst, class } => i(Op::New, dst, 0, 0, u32::from(class)),
        Instr::Construct { obj, base, argc } => i(Op::Construct, obj, base, Reg::from(argc), 0),
        Instr::StaticCall { dst, class, method, base, argc, vars: crate::NO_VARS } => i(Op::StaticCall, dst, base, Reg::from(argc), u32::from(method) | u32::from(class) << 16),
        // (The variables it sends are in the window too.)
        Instr::StaticCall { dst, class, method, base, argc, vars } => {
            for &(_, var) in arg_vars.get(usize::from(vars))? {
                r(var)?;
            }
            i(Op::StaticCallVars, dst, base, Reg::from(argc), u32::from(method) | u32::from(class) << 16)
        }
        Instr::MakeClosure { dst, func, is_static } => i(Op::MakeClosure, dst, 0, 0, u32::from(func) | u32::from(is_static) << 16),
        Instr::CollectVariadic { var } => i(Op::CollectVariadic, var, 0, 0, 0),
        Instr::ArrayUnpack { arr, src } => i(Op::ArrayUnpack, arr, src, 0, 0),
        // `$o->p[k0]..[kn-1] = v` and `$o->p[k0]..[] = v` (`$this->p[..]`
        // with DIM_THIS): the keys from base + 2, a block in the window.
        Instr::AssignPropDim { op: None, base, name, n, append, val, dst } if const_index(val).is_none() => {
            let (this, n) = (n & crate::DIM_THIS != 0, n & !crate::DIM_THIS);
            if n > PATH_KEYS || (n == 0 && !append) {
                return None;
            }
            let site = prop_site(name, nconsts).filter(|&s| s < 1 << 16)?;
            block(base + 2, n)?;
            let flags = path_flags(code, pc, dst, append) | if this { PROP_PATH_THIS } else { 0 };
            let obj = if this { 0 } else { base };
            i(Op::AssignPropDim, obj, base + 2, val, site | u32::from(r(dst)?) << 16 | u32::from(n) << 24 | flags)
        }
        Instr::ConstructSpread { obj, args } => i(Op::ConstructSpread, obj, args, 0, 0),
        Instr::SendVarStatic { class, method, n, arg, var } => i(Op::SendVarStatic, arg, var, Reg::from(n), u32::from(method) | u32::from(class) << 16),
        Instr::BindGlobal { var, name } if usize::from(name) < nconsts => i(Op::BindGlobal, var, 0, 0, u32::from(name)),
        // Two keys or more, or one and a `[]` after it (`ASSIGN_DIM` and
        // `APPEND_DIM` are the others). (Keys registers, a block in the
        // window; the value a register, or a constant of the first 256:
        // PATH_VAL_K.)
        Instr::AssignDim { var, keys, n: n @ 1..=PATH_KEYS, append, val, dst } if (n >= 2 || append) && const_index(keys).is_none() => {
            block(keys, n)?;
            let flags = u32::from(n) | u32::from(r(dst)?) << 8 | path_flags(code, pc, dst, append);
            if const_index(val).is_none() {
                return i(Op::AssignDimPath, var, keys, val, flags);
            }
            let kk = k(val).filter(|&kk| kk < 1 << 8)?;
            i(Op::AssignDimPath, var, keys, kk as Reg, flags | PATH_VAL_K)
        }
        // `unset($a[k0]..)` and `$a[k0]..++`: the keys a block of
        // registers in the window.
        Instr::UnsetDim { var, keys, n: n @ 1..=PATH_KEYS } if const_index(keys).is_none() => {
            block(keys, n)?;
            i(Op::UnsetDim, var, keys, 0, u32::from(n))
        }
        Instr::DimIncDec { var, keys, n: n @ 1..=PATH_KEYS, dst, inc, post } if const_index(keys).is_none() => {
            block(keys, n)?;
            i(Op::DimIncDec, var, keys, dst, u32::from(n) | if inc { DIM_INC } else { 0 } | if post { DIM_POST } else { 0 })
        }
        Instr::Cast { kind, dst, src } => {
            let kind = match kind {
                CastKind::Int => CAST_INT,
                CastKind::Float => CAST_FLOAT,
                CastKind::String => CAST_STRING,
                CastKind::Bool => CAST_BOOL,
                CastKind::Array => CAST_ARRAY,
                CastKind::Unset => return None,
            };
            i(Op::Cast, dst, src, 0, kind)
        }
        // `$o->p++`, `$this->p++` (the name a constant: its cache entry).
        Instr::PropIncDec { obj, name, dst, inc, post } => {
            let site = prop_site(name, nconsts).filter(|&s| s < 1 << 16)?;
            let this = obj == THIS;
            let flags = if inc { PROP_INC } else { 0 } | if post { PROP_POST } else { 0 } | if this { PROP_PATH_THIS } else { 0 };
            i(Op::PropIncDec, if this { 0 } else { obj }, dst, 0, site | flags)
        }
        Instr::MethodArgByRef { dst, obj, method, n } => i(Op::MethodArgByRef, dst, obj, Reg::from(n), u32::from(method)),
        // `&$o->p` (the name a constant: its cache entry).
        Instr::Rare { op: Rare::PropRef, a, b, c, .. } => i(Op::PropRef, a, b, 0, prop_site(c, nconsts)?),
        Instr::StaticArgByRef { dst, class, method, n } => i(Op::StaticArgByRef, dst, 0, Reg::from(n), u32::from(method) | u32::from(class) << 16),
        // (Parameter n is register n.)
        Instr::ArgDefault { n, target } => i(Op::ArgDefault, Reg::from(n), 0, 0, target),
        Instr::FuncArgByRef { dst, site, n } => i(Op::FuncArgByRef, dst, 0, Reg::from(n), u32::from(site)),
        Instr::StaticVar { var, index, skip } => i(Op::StaticVar, var, index & 0xff, index >> 8, skip),
        // `$f(..)`: the arguments a block in the window.
        Instr::CallValue { dst, callee, base, argc, vars } => {
            block(base, argc)?;
            i(Op::CallValue, dst, callee, base, u32::from(argc) | u32::from(vars) << 8)
        }
        Instr::CallSpread { dst, callee, args } => i(Op::CallSpread, dst, callee, args, 0),
        Instr::BindRef { var, src } => i(Op::BindRef, var, src, 0, 0),
        Instr::UnsetVar { var } => i(Op::UnsetVar, var, 0, 0, 0),
        // (The name in a register; VarGet's flags in imm.)
        Instr::Rare { op: Rare::ToObject, a, b, .. } => i(Op::ToObject, a, b, 0, 0),
        // (The argument's position in imm: u16::MAX for a named one, its
        // name in register c.)
        Instr::Rare { op: Rare::CallableArgByRef, a, b, c, class } => i(Op::CallableArgByRef, a, b, if class == u16::MAX { c } else { 0 }, u32::from(class)),
        Instr::Rare { op: Rare::VarGet, a, b, c, .. } => i(Op::VarGet, a, b, 0, u32::from(c)),
        Instr::Rare { op: Rare::VarRef, a, b, c, .. } => i(Op::VarRef, a, b, 0, u32::from(c)),
        Instr::UnsetProp { obj, name } => i(Op::UnsetProp, obj, 0, 0, u32::from(name)),
        Instr::Rare { op: Rare::AutoGlobal, a, b, .. } => i(Op::AutoGlobal, a, b, 0, 0),
        Instr::Rare { op: Rare::ArrayPushRef, a, b, .. } => i(Op::ArrayPushRef, a, b, 0, 0),
        // `C::$name op= v` (class reference c below 256, the name a constant,
        // the value a register or a constant below 256).
        Instr::AssignStaticPropOp { op, class, name, val, dst } if class < 256 => {
            let nk = prop_site(name, nconsts).filter(|&s| s < 1 << 14)?;
            let (v, vk) = match k(val) {
                Some(kk) if kk < 256 => (kk as Reg, STATIC_OP_K),
                Some(_) => return None,
                None => (val, 0),
            };
            i(Op::AssignStaticPropOp, dst, v, class, nk | binop_imm(op) << 14 | vk)
        }
        Instr::DeclareClass { class } => i(Op::DeclareClass, 0, 0, 0, u32::from(class)),
        Instr::DeclareFunction { func } => i(Op::DeclareFunction, 0, 0, 0, u32::from(func)),
        // `include $path`, `f(...)`: the operand a register or a constant.
        Instr::Include { dst, src, kind } => {
            let kind = match kind {
                crate::IncludeKind::Include => 0,
                crate::IncludeKind::IncludeOnce => 1,
                crate::IncludeKind::Require => 2,
                crate::IncludeKind::RequireOnce => 3,
            };
            src_or_k(Op::Include, dst, src, kind)
        }
        Instr::FromCallable { dst, src } => src_or_k(Op::FromCallable, dst, src, 0),
        // `yield k => v`, `yield from src` (an operand a register or a
        // constant).
        Instr::Yield { dst, value, key, has_key } => {
            let (vb, vk) = match k(value) {
                Some(kk) if kk < 1 << 14 => (0, kk | YIELD_VALUE_K),
                Some(_) => return None,
                None => (value, 0),
            };
            let (kc, kf) = match k(key) {
                _ if !has_key => (0, 0),
                Some(kk) if kk < 1 << 14 => (0, kk << 14 | YIELD_KEY_K),
                Some(_) => return None,
                None => (key, 0),
            };
            i(Op::Yield, dst, vb, kc, vk | kf | if has_key { YIELD_HAS_KEY } else { 0 })
        }
        Instr::YieldFrom { dst, src } => match k(src) {
            Some(kk) if kk < 1 << 14 => i(Op::YieldFrom, dst, 0, 0, kk | YIELD_VALUE_K),
            Some(_) => None,
            None => i(Op::YieldFrom, dst, src, 0, 0),
        },
        // `&$root[k0]..` and `$root[k0].. = &$src`: the keys a block of
        // registers in the window, or one constant.
        Instr::RefDim { var, root, keys, n, append } | Instr::DimSetRef { root, keys, n, append, src: var } => {
            let op = if matches!(code[pc], Instr::RefDim { .. }) { Op::RefDim } else { Op::DimSetRef };
            let flags = u32::from(n) | if append { REF_APPEND } else { 0 };
            match k(keys) {
                Some(kk) if n == 1 && kk < 1 << 14 => i(op, var, root, 0, flags | REF_KEY_K | kk << 10),
                Some(_) => None,
                None => {
                    block(keys, n)?;
                    i(op, var, root, if n == 0 { 0 } else { keys }, flags)
                }
            }
        }
        // `$x = &C::$name` (the name loaded into register b).
        Instr::Rare { op: Rare::StaticRef, a, b, class, .. } => i(Op::StaticRef, a, b, 0, u32::from(class)),
        // `isset($a[k0][k1]..)`: the keys a block of registers in the window.
        Instr::IssetDim { dst, var, keys, n: n @ 2.. } if const_index(keys).is_none() => {
            block(keys, n)?;
            i(Op::IssetDimPath, dst, var, keys, u32::from(n))
        }
        // `isset($o->p)` (the name a constant: its cache entry).
        Instr::IssetProp { dst, obj, name } => match prop_site(name, nconsts) {
            Some(site) => i(Op::IssetProp, dst, obj, 0, site),
            // `isset($o->$name)`: the name in register c.
            None => i(Op::IssetProp, dst, obj, name, ISSET_PROP_DYN),
        },
        // `$o->p op= v`, `$this->p op= v` (the name a constant: its cache
        // entry; the value a register, or a constant below 256).
        Instr::AssignPropOp { op, obj, name, val, dst } => {
            let site = prop_site(name, nconsts).filter(|&s| s < 1 << 16)?;
            let this = obj == THIS;
            let (v, kflag) = match k(val) {
                Some(kk) if kk < 256 => (kk as Reg, PROP_OP_K),
                Some(_) => return None,
                None => (val, 0),
            };
            i(Op::AssignPropOp, if this { 0 } else { obj }, v, dst, site | binop_imm(op) << 16 | kflag | if this { PROP_PATH_THIS } else { 0 })
        }
        Instr::Silence { saved } => i(Op::Silence, saved, 0, 0, 0),
        Instr::Unsilence { saved } => i(Op::Unsilence, saved, 0, 0, 0),
        Instr::Neg { dst, src } => i(Op::Neg, dst, src, 0, 0),
        Instr::BitNot { dst, src } => i(Op::BitNot, dst, src, 0, 0),
        Instr::Capture { closure, var, by_ref, quiet } => {
            i(Op::Capture, closure, var, 0, if by_ref { CAPTURE_REF } else { 0 } | if quiet { CAPTURE_QUIET } else { 0 })
        }
        // `[$a, 'k' => $b] = $src`: the key a literal or a register.
        Instr::FetchList { dst, arr, key } => match k(key) {
            Some(kk) => i(Op::FetchList, dst, arr, 0, kk),
            None => i(Op::FetchList, dst, arr, key, LIST_KEY_REG),
        },
        // (The iterator and its state: a block in the window.)
        Instr::IterInitRef { src, iter, end } => {
            block(iter, 2)?;
            i(Op::IterInitRef, src, iter, 0, end)
        }
        Instr::IterNextRef { iter, val, key, has_key, end } if end < ITER_KEY => {
            block(iter, 2)?;
            i(Op::IterNextRef, iter, val, if has_key { key } else { 0 }, end | if has_key { ITER_KEY } else { 0 })
        }
        Instr::ReturnNull => i(Op::ReturnNull, 0, 0, 0, 0),
        // Out of line on their operands, whole (an operand that may be a
        // literal kept in imm as the `Instr` has it).
        Instr::Echo { src } => i(Op::Echo, 0, 0, 0, u32::from(src)),
        Instr::Clone { dst, src } => i(Op::Clone, dst, 0, 0, u32::from(src)),
        Instr::Throw { src } => i(Op::Throw, 0, 0, 0, u32::from(src)),
        // (The class reference in b | c << 8.)
        Instr::CatchIf { src, class, target } => Some(Insn { op: Op::CatchIf, a: r(src)?, b: class as u8, c: (class >> 8) as u8, imm: target }),
        Instr::Rare { op: Rare::IssetStatic, a, b, class, .. } => i(Op::IssetStatic, a, b, 0, u32::from(class)),
        _ => None,
    }
}

/// The variable the result of the call at `pc` (in `dst`, a temporary) is
/// moved into by the next instruction, `$v = f(..)`: a variable of the
/// window. (The move keeps its pc: a call that does not store there
/// itself runs it.)
fn ret_var(code: &[Instr], pc: usize, dst: Reg, ncv: Reg) -> Option<u8> {
    match code.get(pc + 1) {
        Some(&Instr::AssignTake { var, src }) if src == dst && var < ncv && dst >= ncv => r(var),
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
    fn a_call_names_the_variable_its_result_is_moved_into() {
        let code = [
            // `$v = f();`: the variable 1 takes the temporary 5.
            Instr::Call { site: 4, base: 6, argc: 0, dst: 5 },
            Instr::AssignTake { var: 1, src: 5 },
            // Another temporary's move, a move into a temporary, no move.
            Instr::Call { site: 4, base: 6, argc: 0, dst: 5 },
            Instr::AssignTake { var: 1, src: 4 },
            Instr::Call { site: 4, base: 6, argc: 0, dst: 5 },
            Instr::AssignTake { var: 3, src: 5 },
            Instr::Call { site: 4, base: 6, argc: 0, dst: 5 },
        ];
        let at = |pc| lower_at(&code, pc, &[], 3).unwrap();
        assert_eq!(at(0), Insn { op: Op::Call, a: 6, b: 0, c: 5, imm: 4 | 1 << 16 | RET_VAR });
        // The move is there as it was, for the calls that leave it to run.
        assert_eq!(at(1).op, Op::P8_ASSIGN_TAKE);
        assert_eq!(at(2), Insn { op: Op::Call, a: 6, b: 0, c: 5, imm: 4 | 5 << 16 });
        assert_eq!(at(4), Insn { op: Op::Call, a: 6, b: 0, c: 5, imm: 4 | 5 << 16 });
        assert_eq!(at(6), Insn { op: Op::Call, a: 6, b: 0, c: 5, imm: 4 | 5 << 16 });
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
    fn the_second_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            // `empty($a['k'])`: EMPTY, which the handler reads as `??`'s IS.
            Instr::FetchDim { dst: 5, arr: 1, key: c(7), quiet: true, empty: true },
            // `$o::class` (class 0 is Dynamic(3)); `$o::K` is not CLASS_NAME.
            Instr::ClassConst { dst: 5, class: 0, name: c(11) },
            Instr::ClassConst { dst: 5, class: 0, name: c(7) },
            Instr::MakeClosure { dst: 5, func: 2, is_static: true },
            Instr::CollectVariadic { var: 1 },
            Instr::ArrayUnpack { arr: 5, src: 1 },
            // A static call sending variable 1 (entry 0 of arg_vars), and
            // one whose variable is past the window.
            Instr::StaticCall { dst: 5, class: 1, method: 3, base: 10, argc: 1, vars: 0 },
            Instr::StaticCall { dst: 5, class: 1, method: 3, base: 10, argc: 1, vars: 1 },
        ];
        let mut consts = consts();
        consts[11] = Frozen::Str("CLASS".into());
        let classes = [ClassRef::Dynamic(3), ClassRef::Static];
        let arg_vars = [vec![(0, 1)], vec![(0, 300)]];
        let at = |pc| lower_in(&code, pc, &consts, 2, &classes, &arg_vars);
        assert_eq!(at(0).unwrap().op, Op::P8_FETCH_DIM_EMPTY_KSTR);
        assert_eq!(at(1).unwrap(), Insn { op: Op::ClassName, a: 5, b: 3, c: 0, imm: 0 });
        assert!(at(2).is_none());
        assert_eq!(at(3).unwrap(), Insn { op: Op::MakeClosure, a: 5, b: 0, c: 0, imm: 2 | 1 << 16 });
        assert_eq!(at(4).unwrap().op, Op::CollectVariadic);
        assert_eq!(at(5).unwrap(), Insn { op: Op::ArrayUnpack, a: 5, b: 1, c: 0, imm: 0 });
        assert_eq!(at(6).unwrap(), Insn { op: Op::StaticCallVars, a: 5, b: 10, c: 1, imm: 3 | 1 << 16 });
        assert!(at(7).is_none());
    }

    #[test]
    fn the_third_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            Instr::BindGlobal { var: 1, name: 7 },
            // A path of two keys (registers 10, 11), its result freed next.
            Instr::AssignDim { var: 1, keys: 10, n: 2, append: false, val: 3, dst: 6 },
            Instr::Free { reg: 6 },
            // Five keys: not inline. A literal value (PATH_VAL_K, since
            // #375's eighth batch), keys then `[]`: inline.
            Instr::AssignDim { var: 1, keys: 10, n: 5, append: false, val: 3, dst: 6 },
            Instr::AssignDim { var: 1, keys: 10, n: 2, append: false, val: c(3), dst: 6 },
            Instr::AssignDim { var: 1, keys: 10, n: 2, append: true, val: 3, dst: 6 },
            // `$this->p['k'] = v` (DIM_THIS), and `$o->p[a][b] = v` with
            // the object in 20 and its keys from 22.
            Instr::AssignPropDim { op: None, base: 20, name: c(4), n: 1 | crate::DIM_THIS, append: false, val: 3, dst: 6 },
            Instr::AssignPropDim { op: None, base: 20, name: c(4), n: 2, append: false, val: 3, dst: 6 },
            Instr::Free { reg: 6 },
            Instr::AssignPropDim { op: Some(BinOp::Add), base: 20, name: c(4), n: 1, append: false, val: 3, dst: 6 },
            Instr::SendVarStatic { class: 2, method: 5, n: 1, arg: 11, var: 1 },
            // `$this->p[] = v`; no keys and no `[]` is no path.
            Instr::AssignPropDim { op: None, base: 20, name: c(4), n: crate::DIM_THIS, append: true, val: 3, dst: 6 },
            Instr::AssignPropDim { op: None, base: 20, name: c(4), n: 0, append: false, val: 3, dst: 6 },
        ];
        let consts = consts();
        let at = |pc| lower_at(&code, pc, &consts, 2);
        assert_eq!(at(0).unwrap(), Insn { op: Op::BindGlobal, a: 1, b: 0, c: 0, imm: 7 });
        assert_eq!(at(1).unwrap(), Insn { op: Op::AssignDimPath, a: 1, b: 10, c: 3, imm: 2 | 6 << 8 | PATH_UNUSED });
        assert!(at(3).is_none());
        assert_eq!(at(4).unwrap(), Insn { op: Op::AssignDimPath, a: 1, b: 10, c: 3, imm: 2 | 6 << 8 | PATH_VAL_K });
        assert_eq!(at(5).unwrap(), Insn { op: Op::AssignDimPath, a: 1, b: 10, c: 3, imm: 2 | 6 << 8 | PATH_APPEND });
        assert_eq!(at(6).unwrap(), Insn { op: Op::AssignPropDim, a: 0, b: 22, c: 3, imm: 4 | 6 << 16 | 1 << 24 | PROP_PATH_THIS });
        assert_eq!(at(7).unwrap(), Insn { op: Op::AssignPropDim, a: 20, b: 22, c: 3, imm: 4 | 6 << 16 | 2 << 24 | PATH_UNUSED });
        assert!(at(9).is_none());
        assert_eq!(at(10).unwrap(), Insn { op: Op::SendVarStatic, a: 11, b: 1, c: 1, imm: 5 | 2 << 16 });
        assert_eq!(at(11).unwrap(), Insn { op: Op::AssignPropDim, a: 0, b: 22, c: 3, imm: 4 | 6 << 16 | PROP_PATH_THIS | PATH_APPEND });
        assert!(at(12).is_none());
    }

    #[test]
    fn the_fourth_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            Instr::UnsetDim { var: 1, keys: 10, n: 2 },
            Instr::UnsetDim { var: 1, keys: 10, n: 5 },
            Instr::DimIncDec { var: 1, keys: 10, n: 1, dst: 6, inc: true, post: true },
            Instr::DimIncDec { var: 1, keys: 10, n: 2, dst: 6, inc: false, post: false },
            Instr::Cast { kind: CastKind::String, dst: 6, src: 1 },
            Instr::Cast { kind: CastKind::Unset, dst: 6, src: 1 },
            Instr::PropIncDec { obj: THIS, name: c(4), dst: 6, inc: true, post: false },
            Instr::PropIncDec { obj: 20, name: c(4), dst: 6, inc: false, post: true },
            // A name that is no constant: not inline.
            Instr::PropIncDec { obj: 20, name: 3, dst: 6, inc: true, post: true },
            Instr::MethodArgByRef { dst: 6, obj: 20, method: 5, n: 2 },
        ];
        let consts = consts();
        let at = |pc| lower_at(&code, pc, &consts, 2);
        assert_eq!(at(0).unwrap(), Insn { op: Op::UnsetDim, a: 1, b: 10, c: 0, imm: 2 });
        assert!(at(1).is_none());
        assert_eq!(at(2).unwrap(), Insn { op: Op::DimIncDec, a: 1, b: 10, c: 6, imm: 1 | DIM_INC | DIM_POST });
        assert_eq!(at(3).unwrap(), Insn { op: Op::DimIncDec, a: 1, b: 10, c: 6, imm: 2 });
        assert_eq!(at(4).unwrap(), Insn { op: Op::Cast, a: 6, b: 1, c: 0, imm: CAST_STRING });
        assert!(at(5).is_none());
        assert_eq!(at(6).unwrap(), Insn { op: Op::PropIncDec, a: 0, b: 6, c: 0, imm: 4 | PROP_INC | PROP_PATH_THIS });
        assert_eq!(at(7).unwrap(), Insn { op: Op::PropIncDec, a: 20, b: 6, c: 0, imm: 4 | PROP_POST });
        assert!(at(8).is_none());
        assert_eq!(at(9).unwrap(), Insn { op: Op::MethodArgByRef, a: 6, b: 20, c: 2, imm: 5 });
    }

    #[test]
    fn the_fifth_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            Instr::ArgDefault { n: 2, target: 9 },
            Instr::StaticArgByRef { dst: 6, class: 1, method: 5, n: 2 },
            // `&$o->p`, the name a constant; a name in a register is not inline.
            Instr::rare(Rare::PropRef, 6, 20, c(4), 0),
            Instr::rare(Rare::PropRef, 6, 20, 3, 0),
            // `self::$x = $v` in a trait (class 0), `static::$x = 5`; a
            // class in a register (class 2) or a name in a register: not
            // inline.
            Instr::AssignStaticProp { class: 0, name: c(7), val: 3, dst: 6 },
            Instr::AssignStaticProp { class: 1, name: c(7), val: c(5), dst: 6 },
            Instr::AssignStaticProp { class: 2, name: c(7), val: 3, dst: 6 },
            Instr::AssignStaticProp { class: 0, name: 4, val: 3, dst: 6 },
        ];
        let consts = consts();
        let classes = [ClassRef::Scope, ClassRef::Static, ClassRef::Dynamic(3)];
        let at = |pc| lower_in(&code, pc, &consts, 2, &classes, &[]);
        assert_eq!(at(0).unwrap(), Insn { op: Op::ArgDefault, a: 2, b: 0, c: 0, imm: 9 });
        assert_eq!(at(1).unwrap(), Insn { op: Op::StaticArgByRef, a: 6, b: 0, c: 2, imm: 5 | 1 << 16 });
        assert_eq!(at(2).unwrap(), Insn { op: Op::PropRef, a: 6, b: 20, c: 0, imm: 4 });
        assert!(at(3).is_none());
        assert_eq!(at(4).unwrap().op, Op::P8_ASSIGN_STATIC_PROP_SCOPE_REG);
        assert_eq!(at(5).unwrap().op, Op::P8_ASSIGN_STATIC_PROP_STATIC_K);
        assert!(at(6).is_none());
        assert!(at(7).is_none());
    }

    #[test]
    fn the_sixth_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            Instr::FuncArgByRef { dst: 6, site: 3, n: 1 },
            Instr::StaticVar { var: 4, index: 0x0102, skip: 9 },
            Instr::IterInitRef { src: 5, iter: 10, end: 20 },
            Instr::IterNextRef { iter: 10, val: 6, key: 7, has_key: true, end: 20 },
            Instr::IterNextRef { iter: 10, val: 6, key: 7, has_key: false, end: 20 },
            // `[$a, 'k' => $b] = ..`: a literal key, a key in a register;
            // a register past the window is not inline.
            Instr::FetchList { dst: 6, arr: 5, key: c(7) },
            Instr::FetchList { dst: 6, arr: 5, key: 8 },
            Instr::FetchList { dst: 300, arr: 5, key: 8 },
            Instr::CallValue { dst: 6, callee: 5, base: 10, argc: 2, vars: crate::NO_VARS },
            Instr::CallValue { dst: 6, callee: 5, base: 10, argc: 2, vars: 3 },
            // `$this->p .= 'k'` (a constant value below 256), `$o->p += $v`;
            // a value constant past 256 is not inline.
            Instr::AssignPropOp { op: BinOp::Concat, obj: THIS, name: c(7), val: c(8), dst: 6 },
            Instr::AssignPropOp { op: BinOp::Add, obj: 5, name: c(7), val: 3, dst: 6 },
            Instr::AssignPropOp { op: BinOp::Add, obj: 5, name: c(7), val: c(300), dst: 6 },
            Instr::Capture { closure: 5, var: 3, by_ref: true, quiet: false },
            Instr::Neg { dst: 6, src: 3 },
            Instr::CallSpread { dst: 6, callee: 5, args: 4 },
        ];
        let consts = consts();
        let at = |pc| lower_in(&code, pc, &consts, 2, &[], &[]);
        assert_eq!(at(0).unwrap(), Insn { op: Op::FuncArgByRef, a: 6, b: 0, c: 1, imm: 3 });
        assert_eq!(at(1).unwrap(), Insn { op: Op::StaticVar, a: 4, b: 2, c: 1, imm: 9 });
        assert_eq!(at(2).unwrap(), Insn { op: Op::IterInitRef, a: 5, b: 10, c: 0, imm: 20 });
        assert_eq!(at(3).unwrap(), Insn { op: Op::IterNextRef, a: 10, b: 6, c: 7, imm: 20 | ITER_KEY });
        assert_eq!(at(4).unwrap(), Insn { op: Op::IterNextRef, a: 10, b: 6, c: 0, imm: 20 });
        assert_eq!(at(5).unwrap(), Insn { op: Op::FetchList, a: 6, b: 5, c: 0, imm: 7 });
        assert_eq!(at(6).unwrap(), Insn { op: Op::FetchList, a: 6, b: 5, c: 8, imm: LIST_KEY_REG });
        assert!(at(7).is_none());
        assert_eq!(at(8).unwrap(), Insn { op: Op::CallValue, a: 6, b: 5, c: 10, imm: 2 | u32::from(crate::NO_VARS) << 8 });
        assert_eq!(at(9).unwrap(), Insn { op: Op::CallValue, a: 6, b: 5, c: 10, imm: 2 | 3 << 8 });
        assert_eq!(at(10).unwrap(), Insn { op: Op::AssignPropOp, a: 0, b: 8, c: 6, imm: 7 | 6 << 16 | PROP_OP_K | PROP_PATH_THIS });
        assert_eq!(at(11).unwrap(), Insn { op: Op::AssignPropOp, a: 5, b: 3, c: 6, imm: 7 });
        assert!(at(12).is_none());
        assert_eq!(at(13).unwrap(), Insn { op: Op::Capture, a: 5, b: 3, c: 0, imm: CAPTURE_REF });
        assert_eq!(at(14).unwrap(), Insn { op: Op::Neg, a: 6, b: 3, c: 0, imm: 0 });
        assert_eq!(at(15).unwrap(), Insn { op: Op::CallSpread, a: 6, b: 5, c: 4, imm: 0 });
    }

    #[test]
    fn the_seventh_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            // A name in a register: the dynamic forms.
            Instr::FetchProp { dst: 6, obj: THIS, name: 3, quiet: false },
            Instr::AssignProp { obj: 5, name: 3, val: c(8), dst: 6, take: false },
            Instr::IssetProp { dst: 6, obj: 5, name: 3 },
            Instr::IssetProp { dst: 6, obj: 5, name: c(7) },
            Instr::IssetDim { dst: 6, var: 5, keys: 10, n: 3 },
            Instr::BindRef { var: 4, src: 5 },
            Instr::RefDim { var: 4, root: 5, keys: c(7), n: 1, append: false },
            Instr::DimSetRef { root: 5, keys: 10, n: 2, append: true, src: 4 },
            Instr::rare(Rare::StaticRef, 6, 3, 0, 2),
            Instr::AssignStaticPropOp { op: BinOp::Add, class: 2, name: c(7), val: 3, dst: 6 },
            Instr::Yield { dst: 6, value: 3, key: c(7), has_key: true },
            Instr::Include { dst: 6, src: 3, kind: crate::IncludeKind::RequireOnce },
            Instr::UnsetVar { var: 4 },
            Instr::DeclareClass { class: 9 },
        ];
        let consts = consts();
        let at = |pc| lower_in(&code, pc, &consts, 2, &[], &[]);
        assert_eq!(at(0).unwrap(), Insn { op: Op::FetchPropDyn, a: 6, b: 0, c: 3, imm: PROP_PATH_THIS });
        assert_eq!(at(1).unwrap(), Insn { op: Op::AssignPropDyn, a: 5, b: 8, c: 3, imm: PROP_OP_K | 6 << 8 });
        assert_eq!(at(2).unwrap(), Insn { op: Op::IssetProp, a: 6, b: 5, c: 3, imm: ISSET_PROP_DYN });
        assert_eq!(at(3).unwrap(), Insn { op: Op::IssetProp, a: 6, b: 5, c: 0, imm: 7 });
        assert_eq!(at(4).unwrap(), Insn { op: Op::IssetDimPath, a: 6, b: 5, c: 10, imm: 3 });
        assert_eq!(at(5).unwrap(), Insn { op: Op::BindRef, a: 4, b: 5, c: 0, imm: 0 });
        assert_eq!(at(6).unwrap(), Insn { op: Op::RefDim, a: 4, b: 5, c: 0, imm: 1 | REF_KEY_K | 7 << 10 });
        assert_eq!(at(7).unwrap(), Insn { op: Op::DimSetRef, a: 4, b: 5, c: 10, imm: 2 | REF_APPEND });
        assert_eq!(at(8).unwrap(), Insn { op: Op::StaticRef, a: 6, b: 3, c: 0, imm: 2 });
        assert_eq!(at(9).unwrap(), Insn { op: Op::AssignStaticPropOp, a: 6, b: 3, c: 2, imm: 7 });
        assert_eq!(at(10).unwrap(), Insn { op: Op::Yield, a: 6, b: 3, c: 0, imm: 7 << 14 | YIELD_KEY_K | YIELD_HAS_KEY });
        assert_eq!(at(11).unwrap(), Insn { op: Op::Include, a: 6, b: 3, c: 0, imm: 3 });
        assert_eq!(at(12).unwrap(), Insn { op: Op::UnsetVar, a: 4, b: 0, c: 0, imm: 0 });
        assert_eq!(at(13).unwrap(), Insn { op: Op::DeclareClass, a: 0, b: 0, c: 0, imm: 9 });
    }

    #[test]
    fn the_eighth_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            // A path write of a literal value (one of the first 256
            // constants), and of one past them.
            Instr::AssignDim { var: 1, keys: 10, n: 2, append: false, val: c(8), dst: 6 },
            Instr::AssignDim { var: 1, keys: 10, n: 1, append: true, val: c(7), dst: 6 },
            Instr::AssignDim { var: 1, keys: 10, n: 2, append: false, val: c(300), dst: 6 },
            // `$x::class`, class reference 1 a class in register 3.
            Instr::ClassConst { dst: 5, class: 1, name: c(11) },
        ];
        let mut consts = consts();
        consts[11] = Frozen::Str("class".into());
        let classes = [ClassRef::Static, ClassRef::Dynamic(3)];
        let at = |pc| lower_in(&code, pc, &consts, 2, &classes, &[]);
        assert_eq!(at(0).unwrap(), Insn { op: Op::AssignDimPath, a: 1, b: 10, c: 8, imm: 2 | 6 << 8 | PATH_VAL_K });
        assert_eq!(at(1).unwrap(), Insn { op: Op::AssignDimPath, a: 1, b: 10, c: 7, imm: 1 | 6 << 8 | PATH_VAL_K | PATH_APPEND });
        assert!(at(2).is_none());
        assert_eq!(at(3).unwrap(), Insn { op: Op::ClassName, a: 5, b: 3, c: 0, imm: 1 });
    }

    #[test]
    fn the_ninth_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            // `$x instanceof $cls` (class reference 1 a class in register
            // 3), and of `parent` (2).
            Instr::InstanceOf { dst: 5, src: 4, class: 1 },
            Instr::InstanceOf { dst: 5, src: 4, class: 2 },
            // `~$x`.
            Instr::BitNot { dst: 5, src: 4 },
            // `$x?->p` / `$x->p ?? …`: a quiet read; and a plain one.
            Instr::FetchProp { dst: 5, obj: 4, name: c(3), quiet: true },
            Instr::FetchProp { dst: 5, obj: 4, name: c(3), quiet: false },
            // A return of none (`return;`): the TypeError's text.
            Instr::Rare { op: Rare::VerifyReturn, a: 5, b: 1, c: 0, class: 0 },
            // `K <=> $b` keeps its order (not CompareC); `K < $b` swaps.
            Instr::Compare { cmp: Cmp::Spaceship, dst: 5, a: c(1), b: 4 },
            Instr::Compare { cmp: Cmp::Lt, dst: 5, a: c(1), b: 4 },
        ];
        let consts = consts();
        let classes = [ClassRef::Static, ClassRef::Dynamic(3), ClassRef::ParentScope];
        let at = |pc| lower_in(&code, pc, &consts, 2, &classes, &[]);
        assert_eq!(at(0).unwrap().op, Op::P8_INSTANCEOF_OTHER);
        assert_eq!(at(1).unwrap().op, Op::P8_INSTANCEOF_OTHER);
        assert_eq!(at(2).unwrap(), Insn { op: Op::BitNot, a: 5, b: 4, c: 0, imm: 0 });
        assert_eq!(at(3).unwrap().op, Op::P8_FETCH_PROP_IS_REG);
        assert_eq!(at(4).unwrap().op, Op::P8_FETCH_PROP_R_REG);
        assert_eq!(at(5).unwrap(), Insn { op: Op::VerifyReturn, a: 5, b: 1, c: 0, imm: 0 });
        assert!(at(6).is_none());
        assert_eq!(at(7).unwrap(), Insn { op: Op::CompareC, a: 5, b: 4, c: 0, imm: cmp_imm(Cmp::Gt) | 1 << 8 });
    }

    #[test]
    fn the_tenth_batch_lowers_to_the_loop() {
        use crate::const_operand as c;
        let code = [
            // `echo`, `clone`, `throw` of a register and of a literal.
            Instr::Echo { src: 4 },
            Instr::Echo { src: c(7) },
            Instr::Clone { dst: 5, src: 4 },
            Instr::Throw { src: 4 },
            // A catch's test: class reference 300 (past a byte).
            Instr::CatchIf { src: 4, class: 300, target: 9 },
            Instr::Rare { op: Rare::IssetStatic, a: 5, b: 4, c: 0, class: 2 },
            // empty()'s last dimension: EMPTY, not IS.
            Instr::FetchDim { dst: 5, arr: 1, key: 2, quiet: true, empty: true },
            Instr::FetchDim { dst: 5, arr: 1, key: 2, quiet: true, empty: false },
            // `$o->$name ?? ..`: the quiet bit.
            Instr::FetchProp { dst: 5, obj: 4, name: 3, quiet: true },
            Instr::FetchProp { dst: 5, obj: THIS, name: 3, quiet: false },
        ];
        let consts = consts();
        let at = |pc| lower_at(&code, pc, &consts, 2);
        assert_eq!(at(0).unwrap(), Insn { op: Op::Echo, a: 0, b: 0, c: 0, imm: 4 });
        assert_eq!(at(1).unwrap(), Insn { op: Op::Echo, a: 0, b: 0, c: 0, imm: u32::from(c(7)) });
        assert_eq!(at(2).unwrap(), Insn { op: Op::Clone, a: 5, b: 0, c: 0, imm: 4 });
        assert_eq!(at(3).unwrap(), Insn { op: Op::Throw, a: 0, b: 0, c: 0, imm: 4 });
        assert_eq!(at(4).unwrap(), Insn { op: Op::CatchIf, a: 4, b: 300u16 as u8, c: (300u16 >> 8) as u8, imm: 9 });
        assert_eq!(at(5).unwrap(), Insn { op: Op::IssetStatic, a: 5, b: 4, c: 0, imm: 2 });
        assert_eq!(at(6).unwrap().op, Op::P8_FETCH_DIM_EMPTY_REG);
        assert_eq!(at(7).unwrap().op, Op::P8_FETCH_DIM_IS_REG);
        assert_eq!(at(8).unwrap(), Insn { op: Op::FetchPropDyn, a: 5, b: 4, c: 3, imm: PROP_QUIET });
        assert_eq!(at(9).unwrap(), Insn { op: Op::FetchPropDyn, a: 5, b: 0, c: 3, imm: PROP_PATH_THIS });
    }

    #[test]
    fn every_generated_opcode_is_listed() {
        // SPECS and the enum agree, in order, after the hand opcodes.
        let first = SPECS[0].0 as usize;
        for (n, (op, _, _)) in SPECS.iter().enumerate() {
            assert_eq!(*op as usize, first + n);
        }
        assert!(SPECS.len() > 40);
        // ALL lists every opcode at its number, the generated ones last.
        for (n, op) in Op::ALL.iter().enumerate() {
            assert_eq!(*op as usize, n);
        }
        assert_eq!(Op::ALL.len(), first + SPECS.len());
    }
}

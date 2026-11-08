//! Operand kinds and the other `const` parameters of the handlers. Each
//! name here is also a kind the generator (`build.rs`) names in a
//! handler's name: `P8_MOVE_TMP_CONST` is `ops::mv::<{TMP}, {CONST}>`.

/// A literal: read in place from the instruction (`Insn::k0`/`k1`), never
/// undefined, never a reference (PHP's CONST).
pub const CONST: u8 = 0;
/// A temporary: a register the compiler owns; never undefined where it is
/// read, never a reference (PHP's TMP).
pub const TMP: u8 = 1;
/// A compiled variable (`$x`): a register that may be undefined (a
/// warning, then null) or hold a reference (read and written through it).
pub const CV: u8 = 2;
/// The running method's `$this`, from the frame (PHP's UNUSED op1).
pub const THIS: u8 = 3;
/// No operand (`return;`).
pub const NONE: u8 = 4;
/// An array key that is an int literal.
pub const CINT: u8 = 5;
/// An array key that is a string literal (never a canonical integer: the
/// translator refuses those, as PHP's compiler normalizes them).
pub const CSTR: u8 = 6;

/// The instruction's result is read (by the next instructions) / is freed
/// right after (the `Free` is skipped).
pub const USED: u8 = 0;
pub const UNUSED: u8 = 1;

/// The value operand is copied / moved out of its temporary (read for the
/// last time: p8's `AssignTake`, `take`).
pub const COPY: u8 = 0;
pub const TAKE: u8 = 1;

/// Branch when the condition holds / does not.
pub const T: u8 = 1;
pub const F: u8 = 0;

/// Comparisons (`Cmp` without `<=>`).
pub const LT: u8 = 0;
pub const LE: u8 = 1;
pub const GT: u8 = 2;
pub const GE: u8 = 3;
pub const EQ: u8 = 4;
pub const NE: u8 = 5;
pub const ID: u8 = 6;
pub const NI: u8 = 7;

/// Arithmetic.
pub const ADD: u8 = 0;
pub const SUB: u8 = 1;
pub const MUL: u8 = 2;

/// The fused step-and-test (`IncJumpLt` and kin).
pub const INC_LT: u8 = 0;
pub const INC_LE: u8 = 1;
pub const DEC_GT: u8 = 2;
pub const DEC_GE: u8 = 3;

/// `foreach` without / with a key.
pub const NOKEY: u8 = 0;
pub const KEY: u8 = 1;

/// `new C`: the class has no constructor (the `Construct` after it is
/// skipped) / has one.
pub const PLAIN: u8 = 0;
pub const CTOR: u8 = 1;

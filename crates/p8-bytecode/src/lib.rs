#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8 bytecode: register instructions over a function's frame. This is the
//! contract between the compiler and the interpreter (and any later tier).
//!
//! A frame's registers are, in order: the parameters, the other compiled
//! variables (CVs: every `$name` the function uses), the function's
//! constants (copied in when the frame starts), then temporaries. All
//! frames live in one register file; a call's arguments are evaluated into
//! the caller's top temporaries, which become the callee's first registers,
//! so passing arguments copies nothing.

use p8_runtime::frozen::{Frozen, Text};
use p8_runtime::ops::BinOp;
use std::sync::Arc;

/// A register index within the current frame.
pub type Reg = u16;

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    NotEq,
    Identical,
    NotIdentical,
    Lt,
    Le,
    Gt,
    Ge,
    Spaceship,
}

/// `(int)` and friends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastKind {
    Int,
    Float,
    String,
    Bool,
    Array,
    Unset,
}

#[derive(Debug, Clone, Copy)]
pub enum Instr {
    /// dst = src's value (a copy; references are looked through)
    Move {
        dst: Reg,
        src: Reg,
    },
    /// The variable `var` = src's value, written through a reference
    Assign {
        var: Reg,
        src: Reg,
    },
    LoadNull {
        dst: Reg,
    },
    Binary {
        op: BinOp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    /// `var op= src` (the expression's value is the variable itself)
    AssignOp {
        op: BinOp,
        var: Reg,
        src: Reg,
    },
    Compare {
        cmp: Cmp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Not {
        dst: Reg,
        src: Reg,
    },
    Neg {
        dst: Reg,
        src: Reg,
    },
    BitNot {
        dst: Reg,
        src: Reg,
    },
    ToBool {
        dst: Reg,
        src: Reg,
    },
    Cast {
        kind: CastKind,
        dst: Reg,
        src: Reg,
    },
    /// `++$var`/`--$var` (dst gets the new value) or `$var++`/`$var--`
    /// (dst gets the old one)
    IncDec {
        var: Reg,
        dst: Reg,
        inc: bool,
        post: bool,
    },
    /// `++$var` whose value is not used
    IncVar {
        var: Reg,
    },
    /// `--$var` whose value is not used
    DecVar {
        var: Reg,
    },
    /// A comparison and its branch in one, one instruction per test (so
    /// the interpreter dispatches once): jump when `a < b` holds as PHP's
    /// `<` decides it, or for the `Not` forms when it does not. `>` and
    /// `>=` swap their operands, as PHP compiles them. See `jump_cmp`.
    JumpLt {
        a: Reg,
        b: Reg,
        target: u32,
    },
    JumpNotLt {
        a: Reg,
        b: Reg,
        target: u32,
    },
    JumpLe {
        a: Reg,
        b: Reg,
        target: u32,
    },
    JumpNotLe {
        a: Reg,
        b: Reg,
        target: u32,
    },
    JumpEq {
        a: Reg,
        b: Reg,
        target: u32,
    },
    JumpNotEq {
        a: Reg,
        b: Reg,
        target: u32,
    },
    JumpIdentical {
        a: Reg,
        b: Reg,
        target: u32,
    },
    JumpNotIdentical {
        a: Reg,
        b: Reg,
        target: u32,
    },
    Jump {
        target: u32,
    },
    JumpIf {
        cond: Reg,
        target: u32,
    },
    JumpIfNot {
        cond: Reg,
        target: u32,
    },
    /// `a ?? b`: if `src` is set and not null, dst = it and jump
    JumpIfSet {
        src: Reg,
        dst: Reg,
        target: u32,
    },
    NewArray {
        dst: Reg,
    },
    /// arr[] = val (array literal building)
    ArrayPush {
        arr: Reg,
        val: Reg,
    },
    /// arr[key] = val (array literal building)
    ArraySet {
        arr: Reg,
        key: Reg,
        val: Reg,
    },
    /// dst = arr[key]; `quiet` for isset/?? (no warning)
    FetchDim {
        dst: Reg,
        arr: Reg,
        key: Reg,
        quiet: bool,
    },
    /// var[k1]..[kn] = val with keys in `keys..keys+n`; when `append`, the
    /// last level is `[]`. dst gets the assigned value.
    AssignDim {
        var: Reg,
        keys: Reg,
        n: u8,
        append: bool,
        val: Reg,
        dst: Reg,
    },
    /// var[k1]..[kn] op= val
    AssignDimOp {
        op: BinOp,
        var: Reg,
        keys: Reg,
        n: u8,
        val: Reg,
        dst: Reg,
    },
    /// isset(var[k1]..[kn])
    IssetDim {
        dst: Reg,
        var: Reg,
        keys: Reg,
        n: u8,
    },
    /// isset($var)
    IssetVar {
        dst: Reg,
        var: Reg,
    },
    /// unset($var)
    UnsetVar {
        var: Reg,
    },
    /// unset(var[k1]..[kn])
    UnsetDim {
        var: Reg,
        keys: Reg,
        n: u8,
    },
    /// Start a foreach over `src`: iter gets the iterated value, pos the
    /// position; an empty iterable jumps to `end`
    IterInit {
        src: Reg,
        iter: Reg,
        end: u32,
    },
    /// Next element: value into `val` (and key into `key` when has_key),
    /// or jump to `end` when done
    IterNext {
        iter: Reg,
        val: Reg,
        key: Reg,
        has_key: bool,
        end: u32,
    },
    /// Call site `site` with `argc` arguments in base..base+argc; result to dst
    Call {
        site: u16,
        base: Reg,
        argc: u8,
        dst: Reg,
    },
    /// Pass variable `var` as argument `n` of `site` into `arg`: by
    /// reference if the callee takes it so, else its value
    SendVar {
        site: u16,
        n: u8,
        arg: Reg,
        var: Reg,
    },
    Return {
        src: Reg,
    },
    ReturnNull,
    Echo {
        src: Reg,
    },
    /// dst = the constant named at site (cached once found)
    FetchConst {
        site: u16,
        dst: Reg,
    },
    /// dst = a . b . … for n registers from start (interpolation)
    ConcatN {
        dst: Reg,
        start: Reg,
        n: u8,
    },
    /// `global $name`: bind CV `var` to the global variable named const `name`
    BindGlobal {
        var: Reg,
        name: u16,
    },
    /// Parameter `n` was passed: skip its default's code (at the next
    /// instruction) by jumping to target
    ArgDefault {
        n: u8,
        target: u32,
    },
    /// exit/die with status or message in src (or none)
    Exit {
        src: Reg,
        has: bool,
    },
}

impl Instr {
    /// The compare-and-branch that jumps to `target` when `a cmp b` is
    /// `when`. `<=>` is not a branch condition.
    pub fn jump_cmp(cmp: Cmp, a: Reg, b: Reg, when: bool, target: u32) -> Instr {
        let (cmp, a, b) = match cmp {
            Cmp::Gt => (Cmp::Lt, b, a),
            Cmp::Ge => (Cmp::Le, b, a),
            other => (other, a, b),
        };
        let (cmp, when) = match cmp {
            Cmp::NotEq => (Cmp::Eq, !when),
            Cmp::NotIdentical => (Cmp::Identical, !when),
            other => (other, when),
        };
        match (cmp, when) {
            (Cmp::Lt, true) => Instr::JumpLt { a, b, target },
            (Cmp::Lt, false) => Instr::JumpNotLt { a, b, target },
            (Cmp::Le, true) => Instr::JumpLe { a, b, target },
            (Cmp::Le, false) => Instr::JumpNotLe { a, b, target },
            (Cmp::Eq, true) => Instr::JumpEq { a, b, target },
            (Cmp::Eq, false) => Instr::JumpNotEq { a, b, target },
            (Cmp::Identical, true) => Instr::JumpIdentical { a, b, target },
            (Cmp::Identical, false) => Instr::JumpNotIdentical { a, b, target },
            (other, _) => unreachable!("{other:?} is not a branch condition"),
        }
    }

    /// What a compare-and-branch tests: (cmp, a, b, when, target).
    pub fn as_jump_cmp(&self) -> Option<(Cmp, Reg, Reg, bool, u32)> {
        Some(match *self {
            Instr::JumpLt { a, b, target } => (Cmp::Lt, a, b, true, target),
            Instr::JumpNotLt { a, b, target } => (Cmp::Lt, a, b, false, target),
            Instr::JumpLe { a, b, target } => (Cmp::Le, a, b, true, target),
            Instr::JumpNotLe { a, b, target } => (Cmp::Le, a, b, false, target),
            Instr::JumpEq { a, b, target } => (Cmp::Eq, a, b, true, target),
            Instr::JumpNotEq { a, b, target } => (Cmp::Eq, a, b, false, target),
            Instr::JumpIdentical { a, b, target } => (Cmp::Identical, a, b, true, target),
            Instr::JumpNotIdentical { a, b, target } => (Cmp::Identical, a, b, false, target),
            _ => return None,
        })
    }

    /// The jump target of any instruction that has one.
    pub fn target_mut(&mut self) -> Option<&mut u32> {
        match self {
            Instr::Jump { target: t }
            | Instr::JumpIf { target: t, .. }
            | Instr::JumpIfNot { target: t, .. }
            | Instr::JumpIfSet { target: t, .. }
            | Instr::JumpLt { target: t, .. }
            | Instr::JumpNotLt { target: t, .. }
            | Instr::JumpLe { target: t, .. }
            | Instr::JumpNotLe { target: t, .. }
            | Instr::JumpEq { target: t, .. }
            | Instr::JumpNotEq { target: t, .. }
            | Instr::JumpIdentical { target: t, .. }
            | Instr::JumpNotIdentical { target: t, .. }
            | Instr::IterInit { end: t, .. }
            | Instr::IterNext { end: t, .. }
            | Instr::ArgDefault { target: t, .. } => Some(t),
            _ => None,
        }
    }
}

/// A name a call or constant fetch resolves (each context caches what it
/// resolved to, in its closure of the function).
#[derive(Debug)]
pub struct Site {
    /// Lowercased for functions; as written for constants.
    pub name: Text,
    /// The global name to try if `name` (namespaced) is not found.
    pub fallback: Option<Text>,
    /// The name as written, for messages.
    pub display: Text,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: Text,
    pub by_ref: bool,
    pub has_default: bool,
    pub variadic: bool,
}

/// A compiled function (or a file's top-level code). Immutable, and shared
/// by every isolate (`Send + Sync`).
#[derive(Debug)]
pub struct Function {
    pub name: Text,
    pub params: Vec<Param>,
    pub returns_ref: bool,
    pub code: Vec<Instr>,
    /// Source line of each instruction.
    pub lines: Vec<u32>,
    pub consts: Vec<Frozen>,
    /// First constant register (constants occupy consts_at..consts_at+consts.len()).
    pub consts_at: Reg,
    /// Registers the frame needs.
    pub nregs: Reg,
    /// Names of the compiled variables, by register (params first).
    pub cv_names: Vec<Text>,
    pub sites: Vec<Site>,
    pub file: Arc<str>,
    /// Arguments a call must pass: up to the last parameter without a
    /// default (an optional one before it is required too, as in PHP).
    pub required: usize,
    /// The declaration's line (where a missing argument is reported).
    pub line: u32,
}

impl Function {
    /// Whether parameter `n` is passed by reference (variadic: the last one's mode).
    pub fn param_by_ref(&self, n: usize) -> bool {
        match self.params.get(n) {
            Some(p) => p.by_ref,
            None => self.params.last().is_some_and(|p| p.variadic && p.by_ref),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiled_code_is_shared_across_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<super::Function>();
    }

    #[test]
    fn instructions_stay_small() {
        // Dispatch reads one instruction per step; keep them compact.
        assert!(
            std::mem::size_of::<super::Instr>() <= 12,
            "{}",
            std::mem::size_of::<super::Instr>()
        );
    }
}

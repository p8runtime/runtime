//! What each register of a function may hold before each instruction:
//! a set of kinds (`Kinds`) per register of the window, found by a forward
//! dataflow pass over the function's code when it is lowered (`walk`).
//! `insn::lower` reads it to pick a specialised opcode where the kinds
//! are proven.
//!
//! The facts are sound, not hints: a kind not in a register's set is
//! never there. So they are kept only where every write is known:
//!
//! - A compiled variable (CV) is followed only while it is *clean*: no
//!   instruction the pass does not model names it (those can bind it to
//!   a reference, `global`, `static`, `&`, a by-reference argument, a
//!   by-reference `foreach`, a capture by reference, `catch`), it is not
//!   a by-reference or variadic parameter, a closure's bound variable,
//!   a call's `arg_vars`, or a handler's slot, and the function reads no
//!   variable by name (`$$x`, `extract()`, `include`, `eval`, the global
//!   scope's code). A clean CV changes only at the instructions that
//!   name it, which the pass models one by one.
//! - A register that may hold a reference (REF) may change at any
//!   instruction (through another holder of the reference): it becomes
//!   ANY before each one.
//! - Temporaries are written only by the instructions that name them; an
//!   instruction the pass does not model makes every temporary ANY.
//! - A handler's target (a `catch`, a `finally`) starts with every
//!   register but the constants ANY.

use crate::{CastKind, Cmp, Frozen, Function, Instr, Rare, Reg};
use p8_runtime::ops::BinOp;
use p8_runtime::types::{T_ARRAY, T_BOOL, T_CALLABLE, T_FALSE, T_FLOAT, T_INT, T_MIXED, T_NULL, T_OBJECT, T_STATIC, T_STRING, T_TRUE, TypeDecl};

/// A set of kinds a register may hold.
pub type Kinds = u16;
pub const NULL: Kinds = 1;
pub const BOOL: Kinds = 2;
pub const INT: Kinds = 4;
pub const FLOAT: Kinds = 8;
pub const STR: Kinds = 16;
pub const ARR: Kinds = 32;
pub const OBJ: Kinds = 64;
/// A reference (to anything).
pub const REF: Kinds = 128;
pub const UNDEF: Kinds = 256;
/// Any defined value that is not a reference.
pub const VALUE: Kinds = NULL | BOOL | INT | FLOAT | STR | ARR | OBJ;
pub const ANY: Kinds = VALUE | REF | UNDEF;
/// What a store over the register needs not release.
pub const PLAIN: Kinds = NULL | BOOL | INT | FLOAT | UNDEF;
pub const NUM: Kinds = INT | FLOAT;

/// The registers followed: the loop's window (wider ones escape anyway).
const TRACKED: usize = 256;

/// The kinds of a constant.
fn frozen_kinds(c: &Frozen) -> Kinds {
    match c {
        Frozen::Null => NULL,
        Frozen::Bool(_) => BOOL,
        Frozen::Int(_) => INT,
        Frozen::Float(_) => FLOAT,
        Frozen::Str(_) => STR,
        Frozen::Array(_) => ARR,
    }
}

/// What a parameter of declared type `ty` holds once VerifyArgs passed:
/// the value as accepted (`quick`), or as converted to one of the types
/// (an int to a float when the type has float but not int, and the weak
/// mode's scalar conversions).
fn type_kinds(ty: &TypeDecl) -> Kinds {
    let m = ty.mask;
    if m & T_MIXED != 0 {
        return VALUE;
    }
    let mut k = 0;
    if m & (T_OBJECT | T_STATIC) != 0 || !ty.classes.is_empty() {
        k |= OBJ;
    }
    if m & T_CALLABLE != 0 {
        k |= STR | ARR | OBJ;
    }
    if m & T_ARRAY != 0 {
        k |= ARR;
    }
    if m & T_STRING != 0 {
        k |= STR;
    }
    if m & T_INT != 0 {
        k |= INT;
    }
    if m & T_FLOAT != 0 {
        k |= FLOAT;
    }
    // (Weak mode: a float, int or numeric string for `int` alone may be
    // converted to that; one for `int|float` stays what it is, a kind
    // the type has.)
    if m & (T_BOOL | T_FALSE | T_TRUE) != 0 {
        k |= BOOL;
    }
    if m & T_NULL != 0 {
        k |= NULL;
    }
    k
}

/// The value a copy of a register reads (`copied`): through a reference;
/// an undefined one reads as null (with its warning).
fn read(k: Kinds) -> Kinds {
    if k & REF != 0 {
        VALUE
    } else {
        (k & VALUE) | if k & UNDEF != 0 { NULL } else { 0 }
    }
}

/// `x op y`'s result, for the values `x` and `y`.
fn arith(op: BinOp, x: Kinds, y: Kinds) -> Kinds {
    let both = x | y;
    if op == BinOp::Concat {
        return STR;
    }
    // Operator overloading (GMP, BcMath\Number), or an error.
    if both & OBJ != 0 {
        return VALUE;
    }
    match op {
        BinOp::Add if both & ARR != 0 => NUM | ARR,
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => NUM,
        BinOp::Mod | BinOp::Shl | BinOp::Shr => INT,
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => INT | if x & y & STR != 0 { STR } else { 0 },
        BinOp::Concat => STR,
    }
}

/// `++`/`--` of a value (both: the union covers either).
fn step(x: Kinds) -> Kinds {
    x | if x & NULL != 0 { INT } else { 0 } | if x & INT != 0 { FLOAT } else { 0 } | if x & STR != 0 { NUM } else { 0 }
}

/// Which registers are what, for one function.
struct Layout {
    /// Registers followed (the window, or fewer).
    n: usize,
    /// CVs are below this; constants from it up to `temps`.
    consts_at: usize,
    temps: usize,
    /// Per CV (below `n`): never followed (always ANY).
    dirty: Vec<bool>,
    /// A call in the code may leave a reference in its result (WantRef).
    wants_refs: bool,
}

impl Layout {
    fn is_temp(&self, r: usize) -> bool {
        r >= self.temps
    }
}

/// The instructions whose effect on every register they name the pass
/// models (`transfer`); any other one makes the CVs it names dirty.
fn modelled(instr: &Instr) -> bool {
    use Instr::*;
    match instr {
        Move { .. }
        | Assign { .. }
        | AssignTake { .. }
        | Free { .. }
        | LoadNull { .. }
        | Binary { .. }
        | Add { .. }
        | Sub { .. }
        | Mul { .. }
        | AssignOp { .. }
        | Compare { .. }
        | Not { .. }
        | ToBool { .. }
        | Neg { .. }
        | BitNot { .. }
        | Cast { .. }
        | IncDec { .. }
        | IncVar { .. }
        | DecVar { .. }
        | IncJumpLt { .. }
        | IncJumpLe { .. }
        | DecJumpGt { .. }
        | DecJumpGe { .. }
        | JumpLt { .. }
        | JumpNotLt { .. }
        | JumpLe { .. }
        | JumpNotLe { .. }
        | JumpEq { .. }
        | JumpNotEq { .. }
        | JumpIdentical { .. }
        | JumpNotIdentical { .. }
        | Jump { .. }
        | JumpIf { .. }
        | JumpIfNot { .. }
        | NewArray { .. }
        | ArrayPush { .. }
        | ArraySet { .. }
        | FetchDim { .. }
        | AssignDim { .. }
        | AssignDimOp { .. }
        | IssetDim { .. }
        | IssetVar { .. }
        | UnsetVar { .. }
        | UnsetDim { .. }
        | IterInit { .. }
        | IterNext { .. }
        | Call { .. }
        | Return { .. }
        | ReturnNull
        | Echo { .. }
        | FetchConst { .. }
        | ConcatN { .. }
        | This { .. }
        | FetchProp { .. }
        | AssignProp { .. }
        | MethodCall { .. }
        | InstanceOf { .. } => true,
        Rare { op, .. } => op.is_intrinsic() || matches!(op, crate::Rare::VerifyArgs | crate::Rare::VerifyReturn),
        _ => false,
    }
}

/// A function the pass does not follow any CV of: one that reads or binds
/// variables by name, or shares them (the global scope's code, which an
/// included file and `global` see; `include`, `eval`).
fn by_name(func: &Function) -> bool {
    func.name.as_bytes() == b"{main}"
        || func.code.iter().any(|i| {
            matches!(
                i,
                Instr::Include { .. }
                    | Instr::Eval { .. }
                    | Instr::Rare { op: Rare::VarRef | Rare::VarGet | Rare::VarUnset | Rare::VarBind | Rare::Globals | Rare::GlobalBind, .. }
            )
        })
        || func.sites.iter().any(|s| {
            let global = s.fallback.as_ref().map_or(s.name.as_bytes(), |(g, _)| g.as_bytes());
            // (They set the caller's variables by name.)
            matches!(global, b"extract" | b"parse_str" | b"mb_parse_str")
        })
}

fn layout(func: &Function) -> Layout {
    let n = usize::from(func.nregs).min(TRACKED);
    let ncv = func.cv_names.len();
    let consts_at = func.consts_at as usize;
    let temps = consts_at + func.consts.len();
    let wants_refs = func.code.iter().any(|i| matches!(i, Instr::Rare { op: Rare::WantRef, .. }));
    let mut dirty = vec![false; ncv.min(n)];
    let mut mark = |r: Reg| {
        if let Some(d) = dirty.get_mut(usize::from(r)) {
            *d = true;
        }
    };
    if by_name(func) {
        dirty.iter_mut().for_each(|d| *d = true);
        return Layout { n, consts_at, temps, dirty, wants_refs };
    }
    for (i, p) in func.params.iter().enumerate() {
        if p.by_ref || p.variadic {
            mark(i as Reg);
        }
    }
    for &r in &func.bound {
        mark(r);
    }
    for vars in &func.arg_vars {
        for &(_, r) in vars {
            mark(r);
        }
    }
    for h in &func.handlers {
        mark(h.slot);
    }
    for (_, _, saved) in &func.silences {
        mark(*saved);
    }
    // (file_get_contents() and the other http wrappers set it in the caller.)
    for (r, name) in func.cv_names.iter().enumerate() {
        if name.as_bytes() == b"http_response_header" {
            mark(r as Reg);
        }
    }
    for instr in &func.code {
        if !modelled(instr) {
            crate::regs::each_reg(instr, &mut mark);
        }
    }
    Layout { n, consts_at, temps, dirty, wants_refs }
}

/// The kinds of the followed registers at one point.
#[derive(Clone, PartialEq)]
struct State {
    k: Vec<Kinds>,
}

impl State {
    fn get(&self, r: Reg) -> Kinds {
        self.k.get(usize::from(r)).copied().unwrap_or(ANY)
    }

    fn join(&mut self, other: &State) -> bool {
        let mut changed = false;
        for (a, b) in self.k.iter_mut().zip(&other.k) {
            let j = *a | *b;
            changed |= j != *a;
            *a = j;
        }
        changed
    }
}

/// The pcs `instr` at `pc` may go to next.
fn successors(instr: &Instr, pc: usize, mut each: impl FnMut(usize)) {
    use Instr::*;
    let next = pc + 1;
    match *instr {
        Jump { target } => each(target as usize),
        Return { .. } | ReturnNull | Throw { .. } | Exit { .. } | MatchError { .. } => {}
        // The fused step and test go past the test (`step_jump`).
        IncJumpLt { target, .. } | IncJumpLe { target, .. } | DecJumpGt { target, .. } | DecJumpGe { target, .. } => {
            each(next);
            each(next + 1);
            each(target as usize);
        }
        JumpLt { target, .. }
        | JumpNotLt { target, .. }
        | JumpLe { target, .. }
        | JumpNotLe { target, .. }
        | JumpEq { target, .. }
        | JumpNotEq { target, .. }
        | JumpIdentical { target, .. }
        | JumpNotIdentical { target, .. }
        | JumpIf { target, .. }
        | JumpIfNot { target, .. }
        | JumpIfSet { target, .. }
        | CatchIf { target, .. }
        | ArgDefault { target, .. } => {
            each(next);
            each(target as usize);
        }
        IterInit { end, .. } | IterNext { end, .. } | IterInitRef { end, .. } | IterNextRef { end, .. } => {
            each(next);
            each(end as usize);
        }
        StaticVar { skip, .. } => {
            each(next);
            each(skip as usize);
        }
        _ => each(next),
    }
}

struct Pass<'f> {
    func: &'f Function,
    lay: Layout,
}

impl Pass<'_> {
    /// A store that replaces the register (not through a reference).
    fn set(&self, s: &mut State, r: Reg, k: Kinds) {
        let r = usize::from(r);
        if r >= self.lay.n || (r >= self.lay.consts_at && r < self.lay.temps) {
            return;
        }
        // A reference may change at any instruction (through another
        // holder): one that may be there makes the register ANY.
        s.k[r] = if k & REF != 0 || self.lay.dirty.get(r).copied().unwrap_or(false) { ANY } else { k };
    }

    /// A store into a variable (`assign`): through the reference it holds.
    /// (Not a temporary's: the compiler never makes one that holds a
    /// reference the destination of a value, `reset_temps` frees them.)
    fn write(&self, s: &mut State, r: Reg, k: Kinds) {
        let old = s.get(r);
        self.set(s, r, if old & REF != 0 && !self.lay.is_temp(usize::from(r)) { ANY } else { k });
    }

    /// The registers of a block from `start` that are temporaries: ANY.
    fn spoil(&self, s: &mut State, start: Reg, n: usize) {
        for r in usize::from(start)..usize::from(start) + n {
            if self.lay.is_temp(r) && r < self.lay.n {
                s.k[r] = ANY;
            }
        }
    }

    fn all_temps(&self, s: &mut State) {
        for r in self.lay.temps..self.lay.n {
            s.k[r] = ANY;
        }
    }

    fn entry(&self) -> State {
        let f = self.func;
        // Temporaries: what `leave` and `drop_frame` left (cleared) or
        // a frame below did (values: a temporary that held a reference
        // was freed, `reset_temps`).
        let mut k = vec![VALUE | UNDEF; self.lay.n];
        for (r, slot) in k.iter_mut().enumerate().take(self.lay.consts_at) {
            // Not passed (or not a parameter): cleared by `enter`, or a
            // constant default.
            // A by-value parameter holds the value passed (`call` reads
            // a reference's value); one not passed and before `required`
            // is an ArgumentCountError; a later one holds its constant
            // default, or nothing until its default's code runs.
            *slot = match f.params.get(r) {
                Some(_) if r < f.required => VALUE,
                Some(p) if p.default.is_some() => VALUE,
                Some(_) => VALUE | UNDEF,
                None => UNDEF,
            };
            if self.lay.dirty.get(r).copied().unwrap_or(true) {
                *slot = ANY;
            }
        }
        for (i, c) in f.consts.iter().enumerate().take(f.value_consts as usize) {
            if let Some(slot) = k.get_mut(self.lay.consts_at + i) {
                *slot = frozen_kinds(c);
            }
        }
        State { k }
    }

    /// What a handler's target starts with: nothing known but constants.
    fn handler_entry(&self) -> State {
        let mut s = self.entry();
        for r in 0..self.lay.consts_at.min(self.lay.n) {
            s.k[r] = ANY;
        }
        self.all_temps(&mut s);
        s
    }

    /// A conditional jump at `pc` on a type test of a clean variable
    /// (`is_int($x)`, `$x instanceof C`, the test and its argument's copy
    /// right before it, with no way in between): the variable, what it
    /// is where the jump is taken, and where it is not.
    fn guard(&self, start: &[bool], pc: usize) -> Option<(Reg, Kinds, Kinds)> {
        let code = &self.func.code;
        let (cond, when) = match code[pc] {
            Instr::JumpIf { cond, .. } => (cond, true),
            Instr::JumpIfNot { cond, .. } => (cond, false),
            _ => return None,
        };
        if pc < 1 || start[pc] {
            return None;
        }
        // (yes, no): what the variable is when the test is true, false.
        let (arg, yes, no) = match code[pc - 1] {
            Instr::Rare { op, a, b, c: 1, .. } if a == cond => {
                let k = match op {
                    Rare::IsNull => NULL | UNDEF,
                    Rare::IsBool => BOOL,
                    Rare::IsInt => INT,
                    Rare::IsFloat => FLOAT,
                    Rare::IsString => STR,
                    Rare::IsArray => ARR,
                    Rare::IsObject => OBJ,
                    _ => return None,
                };
                (b, k, ANY & !k)
            }
            Instr::InstanceOf { dst, src, .. } if dst == cond => (src, OBJ, ANY),
            _ => return None,
        };
        // The argument is the variable, or its copy just before.
        let var = match code.get(pc.wrapping_sub(2)) {
            Some(&Instr::Move { dst, src }) if dst == arg && !start[pc - 1] => src,
            _ => arg,
        };
        let v = usize::from(var);
        if v >= self.lay.consts_at || self.lay.dirty.get(v).copied().unwrap_or(true) || var == cond {
            return None;
        }
        Some(if when { (var, yes, no) } else { (var, no, yes) })
    }

    /// `s` after the instruction at `pc`.
    fn transfer(&self, s: &mut State, pc: usize) {
        use Instr::*;
        let f = self.func;
        let instr = &f.code[pc];
        // (A register that may hold a reference is ANY, `set`: whatever
        // runs may change it through another holder.)
        match *instr {
            Move { dst, src } => {
                let v = read(s.get(src));
                self.set(s, dst, v);
            }
            Assign { var, src } => {
                let v = read(s.get(src));
                self.write(s, var, v);
            }
            AssignTake { var, src } => {
                let v = read(s.get(src));
                self.set(s, src, UNDEF);
                self.write(s, var, v);
            }
            // (AssignProp's fused form skips the Free of its unused result,
            // which then keeps what it held.)
            Free { reg } => {
                let kept = match pc.checked_sub(1).map(|p| &f.code[p]) {
                    Some(AssignProp { dst, .. }) if *dst == reg => s.get(reg),
                    _ => 0,
                };
                self.set(s, reg, UNDEF | kept);
            }
            LoadNull { dst } => self.set(s, dst, NULL),
            Binary { op, dst, a, b } => {
                let v = arith(op, read(s.get(a)), read(s.get(b)));
                self.write(s, dst, v);
            }
            Add { dst, a, b } | Sub { dst, a, b } | Mul { dst, a, b } => {
                let v = arith(BinOp::Add, read(s.get(a)), read(s.get(b)));
                self.write(s, dst, v);
            }
            AssignOp { op, var, src } => {
                let v = arith(op, read(s.get(var)), read(s.get(src)));
                self.write(s, var, v);
            }
            Compare { cmp, dst, .. } => self.write(s, dst, if cmp == Cmp::Spaceship { INT } else { BOOL }),
            Not { dst, .. } | ToBool { dst, .. } | IssetVar { dst, .. } | IssetDim { dst, .. } | InstanceOf { dst, .. } => self.write(s, dst, BOOL),
            Neg { dst, src } => {
                let v = arith(BinOp::Mul, read(s.get(src)), INT);
                self.write(s, dst, v);
            }
            BitNot { dst, src } => {
                let x = read(s.get(src));
                self.write(s, dst, arith(BinOp::BitXor, x, x));
            }
            Cast { kind, dst, .. } => {
                let k = match kind {
                    CastKind::Int => INT,
                    CastKind::Float => FLOAT,
                    CastKind::String => STR,
                    CastKind::Bool => BOOL,
                    CastKind::Array => ARR,
                    CastKind::Unset => NULL,
                };
                self.write(s, dst, k);
            }
            IncVar { var } | DecVar { var } | IncJumpLt { var, .. } | IncJumpLe { var, .. } | DecJumpGt { var, .. } | DecJumpGe { var, .. } => {
                let v = step(read(s.get(var)));
                self.write(s, var, v);
            }
            IncDec { var, dst, post, .. } => {
                let old = read(s.get(var));
                let new = step(old);
                self.write(s, var, new);
                self.write(s, dst, if post { old } else { new });
            }
            JumpLt { .. }
            | JumpNotLt { .. }
            | JumpLe { .. }
            | JumpNotLe { .. }
            | JumpEq { .. }
            | JumpNotEq { .. }
            | JumpIdentical { .. }
            | JumpNotIdentical { .. }
            | Jump { .. }
            | JumpIf { .. }
            | JumpIfNot { .. }
            | Return { .. }
            | ReturnNull
            | Echo { .. }
            | UnsetDim { .. } => {}
            NewArray { dst, .. } => self.write(s, dst, ARR),
            ArrayPush { val, take, .. } => {
                if take {
                    self.set(s, val, UNDEF);
                }
            }
            ArraySet { val, take, .. } => {
                if take {
                    self.set(s, val, UNDEF);
                }
            }
            FetchDim { dst, .. } | FetchConst { dst, .. } => self.write(s, dst, VALUE),
            AssignDim { var, dst, .. } | AssignDimOp { var, dst, .. } => {
                // In place: null (or undefined, or false) becomes an array;
                // a string, an object (ArrayAccess) stay what they are.
                let old = s.get(var);
                self.write(s, var, (old & !(NULL | UNDEF)) | ARR);
                self.write(s, dst, VALUE);
            }
            UnsetVar { var } => self.set(s, var, UNDEF),
            IterInit { iter, .. } => self.spoil(s, iter, 2),
            IterNext { iter, val, key, has_key, .. } => {
                self.spoil(s, iter + 1, 1);
                self.write(s, val, VALUE);
                if has_key {
                    self.write(s, key, VALUE);
                }
            }
            // A call's result is a value (`returned` reads a reference's,
            // an undefined one is `step`'s warning and null, a builtin
            // returns one, a function returning by reference returns its
            // value), but in a reference context (`$x = &f()`, WantRef):
            // the reference.
            Call { base, argc, dst, .. } | MethodCall { dst, base, argc, .. } => {
                self.spoil(s, base, usize::from(argc));
                self.write(s, dst, if self.lay.wants_refs { ANY } else { VALUE });
            }
            ConcatN { dst, start, n } => {
                self.spoil(s, start, usize::from(n));
                self.write(s, dst, STR);
            }
            This { dst } => self.write(s, dst, OBJ),
            FetchProp { dst, .. } => self.write(s, dst, VALUE | UNDEF),
            AssignProp { val, dst, take, .. } => {
                // (The fused form leaves the result as it was.)
                let old = s.get(dst);
                self.write(s, dst, VALUE | old);
                if take {
                    self.set(s, val, UNDEF);
                }
            }
            Rare { op: crate::Rare::VerifyArgs, .. } => {
                for (i, p) in f.params.iter().enumerate() {
                    let Some(ty) = &p.ty else { continue };
                    let mut k = type_kinds(ty);
                    if k == 0 {
                        continue;
                    }
                    // One not passed is not checked: its default, or
                    // nothing yet (its default's code comes after).
                    if p.has_default {
                        k |= UNDEF | p.default.as_ref().map_or(VALUE, frozen_kinds);
                    }
                    let r = i as Reg;
                    let now = s.get(r);
                    if now & REF == 0 {
                        self.set(s, r, now & k);
                    }
                }
            }
            Rare { op: crate::Rare::VerifyReturn, .. } => {}
            Rare { op, a, b, c, .. } if op.is_intrinsic() => {
                self.spoil(s, b, usize::from(c));
                self.write(s, a, if matches!(op, crate::Rare::Strlen | crate::Rare::Count) { INT } else { BOOL });
            }
            _ => self.all_temps(s),
        }
    }
}

/// Calls `each(pc, kinds)` for every pc of `func` in order, `kinds(r)`
/// being what register `r` may hold before the instruction there.
pub fn walk(func: &Function, mut each: impl FnMut(usize, &dyn Fn(Reg) -> Kinds)) {
    let len = func.code.len();
    let pass = Pass { func, lay: layout(func) };
    // Block starts: the entry, jump targets, handler targets, and after
    // every instruction that may go elsewhere.
    let mut start = vec![false; len + 2];
    start[0] = true;
    for (pc, instr) in func.code.iter().enumerate() {
        let mut n = 0;
        successors(instr, pc, |t| {
            n += 1;
            if t != pc + 1 && t < start.len() {
                start[t] = true;
            }
        });
        if n != 1 {
            start[pc + 1] = true;
        }
    }
    for h in &func.handlers {
        if let Some(s) = start.get_mut(h.target as usize) {
            *s = true;
        }
    }
    let mut at: Vec<Option<State>> = vec![None; len + 2];
    at[0] = Some(pass.entry());
    for h in &func.handlers {
        let t = h.target as usize;
        if t < len {
            match &mut at[t] {
                Some(s) => {
                    s.join(&pass.handler_entry());
                }
                None => at[t] = Some(pass.handler_entry()),
            }
        }
    }
    // To a fixpoint: a worklist of block starts.
    let mut work: Vec<usize> = (0..len).filter(|&pc| at[pc].is_some()).collect();
    let mut queued = vec![false; len + 2];
    work.iter().for_each(|&pc| queued[pc] = true);
    while let Some(b) = work.pop() {
        queued[b] = false;
        let Some(mut s) = at[b].clone() else { continue };
        let mut pc = b;
        loop {
            pass.transfer(&mut s, pc);
            let mut flow = |t: usize, s: &State, at: &mut Vec<Option<State>>, work: &mut Vec<usize>| {
                if t >= len {
                    return;
                }
                let changed = match &mut at[t] {
                    Some(old) => old.join(s),
                    None => {
                        at[t] = Some(s.clone());
                        true
                    }
                };
                if changed && !queued[t] {
                    queued[t] = true;
                    work.push(t);
                }
            };
            let mut targets = [usize::MAX; 3];
            let mut n = 0;
            successors(&func.code[pc], pc, |t| {
                targets[n] = t;
                n += 1;
            });
            let falls = targets[..n].contains(&(pc + 1));
            let guard = pass.guard(&start, pc);
            for &t in &targets[..n] {
                if t != pc + 1 || start[t] {
                    match guard {
                        // A type test's branches: what the variable is on each.
                        Some((var, taken, other)) if n == 2 => {
                            let mut e = s.clone();
                            let k = e.get(var) & if t == pc + 1 { other } else { taken };
                            pass.set(&mut e, var, k);
                            flow(t, &e, &mut at, &mut work);
                        }
                        _ => flow(t, &s, &mut at, &mut work),
                    }
                }
            }
            if let Some((var, _, other)) = guard
                && falls
            {
                let k = s.get(var) & other;
                pass.set(&mut s, var, k);
            }
            if !falls || pc + 1 >= len || start[pc + 1] {
                break;
            }
            pc += 1;
        }
    }
    // The pcs in order, with the state before each.
    let any = |_: Reg| ANY;
    let mut cur: Option<State> = None;
    for pc in 0..len {
        if start[pc] {
            cur = at[pc].clone();
        }
        match &mut cur {
            Some(s) => {
                each(pc, &|r| s.get(r));
                pass.transfer(s, pc);
                if let Some((var, _, other)) = pass.guard(&start, pc) {
                    let k = s.get(var) & other;
                    pass.set(s, var, k);
                }
            }
            None => each(pc, &any),
        }
        if !successors_fall(&func.code[pc], pc) {
            cur = None;
        }
    }
}

fn successors_fall(instr: &Instr, pc: usize) -> bool {
    let mut falls = false;
    successors(instr, pc, |t| falls |= t == pc + 1);
    falls
}

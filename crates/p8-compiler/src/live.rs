//! Temporaries freed at their last use, as PHP frees a TMP operand in the
//! instruction that reads it (and its live ranges free what an exception
//! leaves behind).
//!
//! One pass over a function's finished code (registers numbered): a
//! backward liveness analysis of the temporaries, then a forward one of
//! which may hold a counted value (a string, an array, an object, a
//! reference). Where such a temporary is no longer live, a `Free` of it
//! goes in front of the instruction there (a jump to that instruction
//! runs it too); where it dies as the value an assignment or an array or
//! property write stores, that instruction takes it instead. So no
//! temporary holds anything at a return, and frame exit clears only the
//! variables (`Function::frees_temps`).
//!
//! What each instruction reads and writes (`Frame::ops`) errs one way only:
//! a register it might read is a use (it stays live: freed later, never
//! early), and only a write known to happen before any read ends a live
//! range; a register it might write is one that may hold a counted value.

use p8_bytecode::{CastKind, ClassRef, Handler, Instr, NO_VARS, PartialSite, Rare as RareOp, Reg};
use p8_runtime::frozen::Frozen;
use p8_runtime::ops::BinOp;

/// What one instruction does to the temporaries, one bit each, and where
/// control goes after it (worked out once: the passes read it per round).
#[derive(Clone, Copy, Default)]
struct Ops {
    /// Read (or possibly read).
    uses: u64,
    /// Written before anything reads them: live ranges end here, going back.
    kills: u64,
    /// May hold a counted value after it.
    dirty: u64,
    /// Hold a plain value (or nothing) after it.
    plain: u64,
    /// Left empty by it (moved out, consumed by a call, freed).
    clean: u64,
    /// Goes on to the next instruction.
    falls: bool,
    /// A jump's target (`NO_TARGET`: none).
    target: u32,
}

const NO_TARGET: u32 = u32::MAX;

/// The function being analysed: where its temporaries start, how many,
/// and the tables instructions name registers through.
struct Frame<'a> {
    tbase: usize,
    ntemps: usize,
    classes: &'a [ClassRef],
    arg_vars: &'a [Vec<(u8, Reg)>],
    partials: &'a [PartialSite],
    /// The constants `LoadConst` loads.
    consts: &'a [Frozen],
}

/// The most temporaries a function may have for the analysis (one word of
/// bits); one with more keeps clearing them all at exit.
pub(crate) const MAX_TEMPS: usize = 64;

/// Scratch the compiler keeps for every function's analysis.
#[derive(Default)]
pub(crate) struct Scratch {
    ops: Vec<Ops>,
    live: Vec<u64>,
    /// Live after each instruction (its handlers' needs included).
    out: Vec<u64>,
    inflow: Vec<u64>,
    frees: Vec<u64>,
    exc: Vec<u64>,
    /// Read by a jump (or a handler) from at or after it: a backward
    /// pass that changes one of these needs another round.
    back_target: Vec<bool>,
}

impl Scratch {
    /// The entries its vectors have room for.
    pub(crate) fn room(&self) -> usize {
        self.ops.capacity() + self.live.capacity() + self.out.capacity() + self.inflow.capacity() + self.frees.capacity() + self.exc.capacity() + self.back_target.capacity()
    }
}

impl Frame<'_> {
    fn bit(&self, r: Reg) -> u64 {
        let r = r as usize;
        if r >= self.tbase && r - self.tbase < self.ntemps { 1 << (r - self.tbase) } else { 0 }
    }

    fn range(&self, start: Reg, n: usize) -> u64 {
        (0..n).fold(0, |m, k| m | self.bit(start.wrapping_add(k as Reg)))
    }

    fn class(&self, i: u16) -> u64 {
        match self.classes.get(i as usize) {
            Some(ClassRef::Dynamic(r)) => self.bit(*r),
            _ => 0,
        }
    }

    fn vars(&self, i: u16) -> u64 {
        if i == NO_VARS {
            return 0;
        }
        self.arg_vars.get(i as usize).map_or(0, |vs| vs.iter().fold(0, |m, (_, r)| m | self.bit(*r)))
    }

    fn ops(&self, instr: &Instr) -> Ops {
        use Instr::*;
        let mut o = Ops::default();
        let b = |r: Reg| self.bit(r);
        // Roles: read; written first (a value that may be counted, or a
        // plain one); read and written (or written only sometimes);
        // emptied.
        macro_rules! u { ($($r:expr),*) => {{ $(o.uses |= b($r);)* }} }
        macro_rules! def { ($r:expr) => {{ let m = b($r); o.kills |= m; o.dirty |= m; }} }
        macro_rules! defp { ($r:expr) => {{ let m = b($r); o.kills |= m; o.plain |= m; }} }
        macro_rules! w { ($($r:expr),*) => {{ $(o.dirty |= b($r);)* }} }
        match *instr {
            Move { dst, src } => {
                u!(src);
                def!(dst);
            }
            Assign { var, src } => {
                u!(var, src);
                w!(var);
            }
            AssignTake { var, src } => {
                u!(var, src);
                o.clean |= b(src);
                w!(var);
            }
            Free { reg } => {
                u!(reg);
                o.clean |= b(reg);
            }
            LoadNull { dst } => defp!(dst),
            // A scalar holds nothing to release; a string literal of
            // program memory neither, but one that could not be interned
            // (or eval's) is counted, so a string is taken as counted.
            LoadConst { dst, k } => match self.consts.get(k as usize) {
                Some(Frozen::Null | Frozen::Bool(_) | Frozen::Int(_) | Frozen::Float(_)) => defp!(dst),
                _ => def!(dst),
            },
            Binary { op, dst, a, b: y } => {
                u!(a, y);
                if matches!(op, BinOp::Div | BinOp::Mod | BinOp::Pow | BinOp::Shl | BinOp::Shr | BinOp::Sub | BinOp::Mul) {
                    defp!(dst);
                } else {
                    def!(dst);
                }
            }
            // `+` of two arrays is an array.
            Add { dst, a, b: y } => {
                u!(a, y);
                def!(dst);
            }
            Sub { dst, a, b: y } | Mul { dst, a, b: y } | Compare { dst, a, b: y, .. } => {
                u!(a, y);
                defp!(dst);
            }
            AssignOp { var, src, .. } => {
                u!(var, src);
                w!(var);
            }
            Not { dst, src } | Neg { dst, src } | ToBool { dst, src } => {
                u!(src);
                defp!(dst);
            }
            BitNot { dst, src } => {
                u!(src);
                def!(dst);
            }
            Cast { kind, dst, src } => {
                u!(src);
                if matches!(kind, CastKind::Int | CastKind::Float | CastKind::Bool | CastKind::Unset) {
                    defp!(dst);
                } else {
                    def!(dst);
                }
            }
            IncDec { var, dst, .. } => {
                u!(var);
                w!(var);
                def!(dst);
            }
            IncVar { var } | DecVar { var } => {
                u!(var);
                w!(var);
            }
            IncJumpLt { var, b: y, .. } | IncJumpLe { var, b: y, .. } | DecJumpGt { var, b: y, .. } | DecJumpGe { var, b: y, .. } => {
                u!(var, y);
                w!(var);
            }
            JumpLt { a, b: y, .. }
            | JumpNotLt { a, b: y, .. }
            | JumpLe { a, b: y, .. }
            | JumpNotLe { a, b: y, .. }
            | JumpEq { a, b: y, .. }
            | JumpNotEq { a, b: y, .. }
            | JumpIdentical { a, b: y, .. }
            | JumpNotIdentical { a, b: y, .. } => u!(a, y),
            Jump { .. } | ReturnNull | ArgDefault { .. } | DeclareClass { .. } | DeclareFunction { .. } => {}
            JumpIf { cond, .. } | JumpIfNot { cond, .. } => u!(cond),
            // dst is written only when the jump is taken (`empty()` reads
            // the null it held before when it is not).
            JumpIfSet { src, dst, .. } => {
                u!(src, dst);
                w!(dst);
            }
            NewArray { dst, .. } => def!(dst),
            ArrayPush { arr, val, take } => {
                u!(arr, val);
                w!(arr);
                if take {
                    o.clean |= b(val);
                }
            }
            ArraySet { arr, key, val, take } => {
                u!(arr, key, val);
                w!(arr);
                if take {
                    o.clean |= b(val);
                }
            }
            FetchDim { dst, arr, key, .. } | FetchList { dst, arr, key } => {
                u!(arr, key);
                def!(dst);
            }
            // (The keys' block, with one more for an append: a use too many
            // is only a later free.)
            AssignDim { var, keys, n, val, dst, .. } | AssignDimOp { var, keys, n, val, dst, .. } => {
                u!(var, val);
                o.uses |= self.range(keys, n as usize + 1);
                w!(var, dst);
            }
            IssetDim { dst, var, keys, n } => {
                u!(var);
                o.uses |= self.range(keys, n as usize + 1);
                defp!(dst);
            }
            IssetVar { dst, var } => {
                u!(var);
                defp!(dst);
            }
            UnsetVar { var } => u!(var),
            UnsetDim { var, keys, n } => {
                u!(var);
                o.uses |= self.range(keys, n as usize + 1);
                w!(var);
            }
            // The iterator's two registers (the value, its position): what
            // was there before is never read again, whether it is written
            // or the loop is skipped (its end only frees them).
            IterInit { src, iter, .. } => {
                u!(src);
                def!(iter);
                def!(iter.wrapping_add(1));
            }
            IterInitRef { src, iter, .. } => {
                u!(src);
                w!(src);
                def!(iter);
                def!(iter.wrapping_add(1));
            }
            IterNext { iter, val, key, .. } | IterNextRef { iter, val, key, .. } => {
                u!(iter, iter.wrapping_add(1));
                w!(iter.wrapping_add(1), val, key);
            }
            // A call consumes its arguments: a PHP function's become its
            // frame's variables (cleared when it ends), a builtin's are
            // cleared after it runs.
            Call { base, argc, dst, .. } => {
                let args = self.range(base, argc as usize);
                o.uses |= args;
                o.clean |= args;
                def!(dst);
            }
            CallValue { dst, callee, base, argc, vars } => {
                let args = self.range(base, argc as usize);
                u!(callee);
                o.uses |= args | self.vars(vars);
                o.clean |= args;
                def!(dst);
            }
            // (The receiver InitMethod captured goes into the call's frame
            // or is released by the call; the word beside it is plain.)
            MethodCall { dst, obj, base, argc, .. } => {
                let args = self.range(base, argc as usize);
                let held = self.range(obj, 2);
                o.uses |= args | held;
                o.clean |= args | held;
                def!(dst);
            }
            // (A temporary receiver is released as the call takes it.)
            MethodCallNow { dst, obj, base, argc, .. } => {
                let args = self.range(base, argc as usize);
                if obj != p8_bytecode::THIS {
                    u!(obj);
                    o.clean |= b(obj);
                }
                o.uses |= args;
                o.clean |= args;
                def!(dst);
            }
            InitMethod { recv, obj, .. } => {
                if obj != p8_bytecode::THIS {
                    u!(obj);
                }
                def!(recv);
                defp!(recv + 1);
            }
            StaticCall { dst, class, base, argc, vars, .. } => {
                let args = self.range(base, argc as usize);
                o.uses |= args | self.class(class) | self.vars(vars);
                o.clean |= args;
                def!(dst);
            }
            Construct { obj, base, argc } => {
                let args = self.range(base, argc as usize);
                u!(obj);
                o.uses |= args;
                o.clean |= args;
            }
            ConstructSpread { obj, args } => u!(obj, args),
            CallSpread { dst, callee, args } => {
                u!(callee, args);
                def!(dst);
            }
            New { dst, class } => {
                o.uses |= self.class(class);
                def!(dst);
            }
            SendVar { arg, var, .. } => {
                u!(var);
                w!(var);
                def!(arg);
            }
            SendVarMethod { word, arg, var, .. } => {
                u!(word, var);
                w!(var);
                def!(arg);
            }
            SendVarStatic { class, arg, var, .. } => {
                o.uses |= self.class(class);
                u!(var);
                w!(var);
                def!(arg);
            }
            MethodArgByRef { dst, word, .. } => {
                u!(word);
                defp!(dst);
            }
            StaticArgByRef { dst, class, .. } => {
                o.uses |= self.class(class);
                defp!(dst);
            }
            FuncArgByRef { dst, .. } => defp!(dst),
            Return { src } | Echo { src } | Exit { src, .. } | MatchError { src } | Throw { src } => u!(src),
            CatchIf { src, class, .. } => {
                u!(src);
                o.uses |= self.class(class);
            }
            FetchConst { dst, .. } | MakeClosure { dst, .. } | This { dst } => def!(dst),
            // Its parts are its own temporaries, released once it is made.
            ConcatN { dst, start, n } => {
                let parts = self.range(start, n as usize);
                o.uses |= parts;
                o.clean |= parts;
                def!(dst);
            }
            BindGlobal { var, .. } | StaticVar { var, .. } | CollectVariadic { var } => {
                u!(var);
                w!(var);
            }
            Silence { saved } => defp!(saved),
            Unsilence { saved } => u!(saved),
            Capture { closure, var, .. } => {
                u!(closure, var);
                w!(closure, var);
            }
            BindRef { var, src } => {
                u!(var, src);
                w!(var, src);
            }
            RefDim { var, root, keys, n, .. } => {
                u!(var, root);
                o.uses |= self.range(keys, n as usize + 1);
                w!(var, root);
            }
            DimSetRef { root, keys, n, src, .. } => {
                u!(root, src);
                o.uses |= self.range(keys, n as usize + 1);
                w!(root, src);
            }
            Yield { dst, value, key, .. } => {
                u!(value, key);
                def!(dst);
            }
            YieldFrom { dst, src } | Eval { dst, src } | FromCallable { dst, src } | Include { dst, src, .. } | Clone { dst, src } => {
                u!(src);
                def!(dst);
            }
            DimIncDec { var, keys, n, dst, .. } => {
                u!(var);
                o.uses |= self.range(keys, n as usize + 1);
                w!(var, dst);
            }
            ArrayUnpack { arr, src } => {
                u!(arr, src);
                w!(arr);
            }
            FetchProp { dst, obj, name, .. } => {
                u!(obj, name);
                def!(dst);
            }
            AssignProp { obj, name, val, dst, take } => {
                u!(obj, name, val);
                if take {
                    o.clean |= b(val);
                }
                w!(dst);
            }
            AssignPropOp { obj, name, val, dst, .. } => {
                u!(obj, name, val);
                w!(dst);
            }
            PropIncDec { obj, name, dst, .. } => {
                u!(obj, name);
                w!(dst);
            }
            // The object (or nothing), the name's place, the keys.
            // (The name's place, base + 1, only when the name is there,
            // and base only when the object is not the frame's `$this`:
            // a slot it leaves alone may hold another expression's
            // temporary, which must be freed after that one's last use,
            // not kept until this write: a copy of the array it writes
            // would make the write copy the array.)
            AssignPropDim { base, name, n, val, dst, .. } => {
                let mut block = self.range(base, (n & !p8_bytecode::DIM_THIS) as usize + 3);
                if name != base.wrapping_add(1) {
                    block &= !b(base.wrapping_add(1));
                }
                if n & p8_bytecode::DIM_THIS != 0 {
                    block &= !b(base);
                }
                u!(name, val);
                o.uses |= block;
                o.dirty |= block;
                w!(dst);
            }
            IssetProp { dst, obj, name } => {
                u!(obj, name);
                defp!(dst);
            }
            UnsetProp { obj, name } => u!(obj, name),
            ClassConst { dst, class, name } | FetchStaticProp { dst, class, name } => {
                o.uses |= self.class(class);
                u!(name);
                def!(dst);
            }
            AssignStaticProp { class, name, val, dst } | AssignStaticPropOp { class, name, val, dst, .. } => {
                o.uses |= self.class(class);
                u!(name, val);
                w!(dst);
            }
            InstanceOf { dst, src, class } => {
                u!(src);
                o.uses |= self.class(class);
                defp!(dst);
            }
            Rare { op, a, b: y, c, class } => {
                // `class` is a class reference for some (a call site, a
                // count for others): naming a register through it is only
                // a use too many.
                o.uses |= self.class(class);
                if op.is_intrinsic() {
                    // The c arguments from b, consumed as a call's are; an
                    // int or a bool into a.
                    let args = self.range(y, c as usize);
                    o.uses |= args;
                    o.clean |= args;
                    defp!(a);
                } else {
                    match op {
                        RareOp::VerifyArgs => {}
                        RareOp::IssetStatic | RareOp::CallableArgByRef => {
                            u!(y, c);
                            defp!(a);
                        }
                        RareOp::Partial => {
                            let n = self.partials.get(class as usize).map_or(0, |p| p.args.len());
                            u!(y);
                            o.uses |= self.range(c, n);
                            w!(a);
                        }
                        _ => {
                            u!(a, y, c);
                            w!(a, y, c);
                        }
                    }
                }
            }
        }
        let (falls, target) = flow(instr);
        o.falls = falls;
        o.target = target.unwrap_or(NO_TARGET);
        o
    }
}

/// Where control goes after `instr` at `pc`, besides its handlers: on to
/// the next instruction (`falls`), and to a jump's target.
fn flow(instr: &Instr) -> (bool, Option<u32>) {
    let mut c = *instr;
    let target = c.target_mut().map(|t| *t);
    let falls = !matches!(
        instr,
        Instr::Jump { .. }
            | Instr::Return { .. }
            | Instr::ReturnNull
            | Instr::Throw { .. }
            | Instr::MatchError { .. }
            | Instr::Exit { .. }
            | Instr::Rare { op: RareOp::ReturnRef | RareOp::ThrowError, .. }
    );
    (falls, target)
}

/// The analysed function's code and the tables that hold pcs, rewritten
/// with the frees in place. Returns whether the function frees its
/// temporaries itself (false: too many to analyse, and nothing changed).
#[allow(clippy::too_many_arguments)] // (each is one of the function's parts the frees move)
pub(crate) fn free_temps(
    code: &mut Vec<Instr>,
    lines: &mut Vec<u32>,
    handlers: &mut [Handler],
    silences: &mut [(u32, u32, Reg)],
    (tbase, ntemps): (usize, usize),
    classes: &[ClassRef],
    arg_vars: &[Vec<(u8, Reg)>],
    partials: &[PartialSite],
    consts: &[Frozen],
    s: &mut Scratch,
) -> bool {
    if ntemps == 0 {
        return true;
    }
    if ntemps > MAX_TEMPS {
        return false;
    }
    let frame = Frame { tbase, ntemps, classes, arg_vars, partials, consts };
    let n = code.len();
    s.ops.clear();
    s.ops.extend(code.iter().map(|i| frame.ops(i)));
    let has_handlers = !handlers.is_empty();
    let handler_bits = |h: &Handler| frame.bit(h.slot);
    // Going backward, a forward jump's target (and a handler after its
    // range) is done before what jumps there. A target at or before the
    // jump is read before it is done: a round that changes one of those
    // needs another.
    s.back_target.clear();
    s.back_target.resize(n, false);
    for (pc, o) in s.ops.iter().enumerate() {
        if o.target != NO_TARGET && o.target as usize <= pc {
            s.back_target[o.target as usize] = true;
        }
    }
    for h in handlers.iter() {
        if h.target < h.end {
            s.back_target[h.target as usize] = true;
        }
    }

    // Liveness, backward: live[pc] is what is live going into pc, out[pc]
    // what is live after it (its handlers' needs included).
    s.live.clear();
    s.live.resize(n + 1, 0);
    s.out.clear();
    s.out.resize(n, 0);
    s.exc.clear();
    s.exc.resize(n, 0);
    loop {
        let mut again = false;
        for (pc, o) in s.ops.iter().enumerate().rev() {
            let mut out = if o.falls { s.live[pc + 1] } else { 0 };
            if o.target != NO_TARGET {
                out |= s.live[o.target as usize];
            }
            // What a handler's code needs is live wherever it may be
            // entered from (the instruction may throw before it writes
            // anything: not ended by its kills).
            let mut exc = 0;
            if has_handlers {
                for h in handlers.iter().filter(|h| h.start as usize <= pc && pc < h.end as usize) {
                    exc |= s.live[h.target as usize] & !handler_bits(h);
                }
                s.exc[pc] = exc;
            }
            s.out[pc] = out | exc;
            let live = o.uses | (out & !o.kills) | exc;
            if live != s.live[pc] {
                s.live[pc] = live;
                again |= s.back_target[pc];
            }
        }
        if !again {
            break;
        }
    }

    // A value stored from a temporary that dies there is moved (taken).
    for (pc, instr) in code.iter_mut().enumerate() {
        let out = s.out[pc];
        let dies = |r: Reg| frame.bit(r) != 0 && out & frame.bit(r) == 0;
        let taken = match *instr {
            Instr::Assign { var, src } if var != src && dies(src) => Instr::AssignTake { var, src },
            Instr::ArrayPush { arr, val, take: false } if arr != val && dies(val) => Instr::ArrayPush { arr, val, take: true },
            Instr::ArraySet { arr, key, val, take: false } if arr != val && key != val && dies(val) => Instr::ArraySet { arr, key, val, take: true },
            Instr::AssignProp { obj, name, val, dst, take: false } if obj != val && name != val && dst != val && dies(val) => {
                Instr::AssignProp { obj, name, val, dst, take: true }
            }
            _ => continue,
        };
        *instr = taken;
        s.ops[pc] = frame.ops(&taken);
    }

    // What may hold a counted value, forward; a temporary that does and is
    // no longer live is freed going into the instruction.
    s.inflow.clear();
    s.inflow.resize(n + 1, 0);
    s.frees.clear();
    s.frees.resize(n, 0);
    // A handler's code is entered with its slot just written.
    for h in handlers.iter() {
        s.inflow[h.target as usize] |= handler_bits(h);
    }
    loop {
        let mut changed = false;
        for (pc, o) in s.ops.iter().enumerate() {
            let e = s.inflow[pc];
            let free = e & !s.live[pc];
            s.frees[pc] = free;
            let e = e & s.live[pc];
            let out = (e & !o.clean & !o.plain) | o.dirty;
            let mut merge = |at: usize, m: u64| {
                let new = s.inflow[at] | m;
                if new != s.inflow[at] {
                    // Only a merge into code already gone through needs another round.
                    changed |= at <= pc;
                    s.inflow[at] = new;
                }
            };
            if o.falls {
                merge(pc + 1, out);
            }
            if o.target != NO_TARGET {
                merge(o.target as usize, out);
            }
            // An exception may leave it anywhere between its start and its end.
            if has_handlers && e | out != 0 {
                for h in handlers.iter().filter(|h| h.start as usize <= pc && pc < h.end as usize) {
                    merge(h.target as usize, e | out);
                }
            }
        }
        if !changed {
            break;
        }
    }

    let count: usize = s.frees.iter().map(|m| m.count_ones() as usize).sum();
    if count == 0 {
        return true;
    }
    // The frees go in: `at[pc]` is where pc's frees start in the new code
    // (pc's own instruction is after them).
    let mut at = Vec::with_capacity(n + 1);
    let mut new_code = Vec::with_capacity(n + count);
    let mut new_lines = Vec::with_capacity(n + count);
    for pc in 0..n {
        at.push(new_code.len() as u32);
        let mut m = s.frees[pc];
        // A free belongs to what came before it (the last use).
        let line = lines[pc.saturating_sub(1)];
        while m != 0 {
            let i = m.trailing_zeros() as usize;
            m &= m - 1;
            new_code.push(Instr::Free { reg: (tbase + i) as Reg });
            new_lines.push(line);
        }
        new_code.push(code[pc]);
        new_lines.push(lines[pc]);
    }
    at.push(new_code.len() as u32);
    for i in &mut new_code {
        if let Some(t) = i.target_mut() {
            *t = at[*t as usize];
        }
    }
    for h in handlers.iter_mut() {
        h.start = at[h.start as usize];
        h.end = at[h.end as usize];
        h.target = at[h.target as usize];
    }
    for (silence, unsilence, _) in silences.iter_mut() {
        // The Silence itself (after its frees) starts the range.
        let s0 = *silence as usize;
        *silence = at[s0 + 1] - 1;
        *unsilence = at[*unsilence as usize];
    }
    *code = new_code;
    *lines = new_lines;
    true
}

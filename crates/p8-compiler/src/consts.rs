//! Constant operands placed: a literal stays a constant operand
//! (`p8_bytecode::const_index`), read in place when the code runs, where
//! the instruction can read one there (the loop's constant forms,
//! `insn::Op::*C`, and `step` for the same operands); every other
//! constant operand becomes a `LoadConst` into a scratch temporary right
//! before its instruction. So a literal costs nothing until the line that
//! reads it runs, and a call copies no constants into its frame, as PHP's
//! handlers read a CONST operand from the op_array's literals.
//!
//! `rewrite` is the one list of which operands stay: `insn::lower` gives
//! those a constant form (or leaves the instruction to `step`, which reads
//! them through `Context::operand` and its kin), and a debug build checks
//! here that nothing else is left constant.

use p8_bytecode::{Cmp, Handler, Instr, Reg, const_index};

/// Scratch the compiler keeps for every function's pass.
#[derive(Default)]
pub(crate) struct Scratch {
    at: Vec<u32>,
    code: Vec<Instr>,
    lines: Vec<u32>,
}

/// Place the constants of `code`: the loads go in before their
/// instructions, into the temporaries from `scratch` on (one per load an
/// instruction needs, reused by the next instruction), and every pc the
/// tables hold moves with the code. Returns how many scratch temporaries
/// the loads use (0: nothing changed).
pub(crate) fn place_consts(code: &mut Vec<Instr>, lines: &mut Vec<u32>, handlers: &mut [Handler], silences: &mut [(u32, u32, Reg)], scratch: usize, s: &mut Scratch) -> usize {
    // Most functions need no load: the first pass only counts.
    let mut total = 0;
    let mut most = 0;
    for instr in code.iter() {
        let mut n = 0;
        rewrite(&mut { *instr }, &mut |_| {
            n += 1;
            0
        });
        total += n;
        most = most.max(n);
    }
    if total == 0 {
        for instr in code.iter_mut() {
            // (A `Move` of a constant is a `LoadConst` as it is: no load.)
            rewrite(instr, &mut |r| r);
        }
        check(code);
        return 0;
    }
    let n = code.len();
    s.at.clear();
    s.code.clear();
    s.lines.clear();
    for pc in 0..n {
        s.at.push(s.code.len() as u32);
        let line = lines[pc];
        let mut next = 0;
        let mut instr = code[pc];
        rewrite(&mut instr, &mut |r| {
            let dst = (scratch + next) as Reg;
            next += 1;
            s.code.push(Instr::LoadConst { dst, k: const_index(r).expect("a constant operand") as u16 });
            s.lines.push(line);
            dst
        });
        s.code.push(instr);
        s.lines.push(line);
    }
    s.at.push(s.code.len() as u32);
    for i in &mut s.code {
        if let Some(t) = i.target_mut() {
            *t = s.at[*t as usize];
        }
    }
    for h in handlers.iter_mut() {
        h.start = s.at[h.start as usize];
        h.end = s.at[h.end as usize];
        h.target = s.at[h.target as usize];
    }
    for (silence, unsilence, _) in silences.iter_mut() {
        // The Silence itself (after its loads) starts the range.
        *silence = s.at[*silence as usize + 1] - 1;
        *unsilence = s.at[*unsilence as usize];
    }
    std::mem::swap(code, &mut s.code);
    std::mem::swap(lines, &mut s.lines);
    check(code);
    most
}

/// Debug builds: no constant operand is left where the instruction cannot
/// read it in place.
fn check(code: &[Instr]) {
    if cfg!(debug_assertions) {
        for instr in code {
            rewrite(&mut { *instr }, &mut |r| panic!("constant operand {r:#x} left in {instr:?}"));
        }
    }
}

/// `instr` with every constant operand it cannot read in place passed to
/// `load`, which gives the register to read instead. The operands that
/// stay are the ones the loop's constant forms cover (`insn::lower`): one
/// side of a comparison or a branch, the right side of `-` (both sides of
/// `+`, `*` and `Binary`: a literal on the left is swapped, or CONST_LEFT),
/// an assignment's or `op=`'s value, a property assignment's value
/// (`$o->p = K`, `C::$p = K`), a fused step's bound, a return, an
/// echo, a dimension's one key (or the value written, when the key is a
/// register), an array literal's keys and values, and the name operands
/// (a property's, a class constant's, a static property's), which `step`
/// has always read in place.
fn rewrite(instr: &mut Instr, load: &mut impl FnMut(Reg) -> Reg) {
    use Instr::*;
    let c = |r: Reg| const_index(r).is_some();
    macro_rules! ld {
        ($($r:expr),*) => {{ $( if c(*$r) { *$r = load(*$r); } )* }};
    }
    match instr {
        // One literal is read in place; of two, the left is loaded.
        JumpLt { a, b, .. }
        | JumpNotLt { a, b, .. }
        | JumpLe { a, b, .. }
        | JumpNotLe { a, b, .. }
        | JumpEq { a, b, .. }
        | JumpNotEq { a, b, .. }
        | JumpIdentical { a, b, .. }
        | JumpNotIdentical { a, b, .. }
        | Add { a, b, .. }
        | Mul { a, b, .. }
        | Binary { a, b, .. } => {
            if c(*a) && c(*b) {
                ld!(a);
            }
        }
        // (`<=>` swaps into nothing: its literal on the left is loaded.)
        Compare { cmp, a, b, .. } => {
            if c(*a) && (c(*b) || *cmp == Cmp::Spaceship) {
                ld!(a);
            }
        }
        Sub { a, .. } => ld!(a),
        AssignOp { .. }
        | IncJumpLt { .. }
        | IncJumpLe { .. }
        | DecJumpGt { .. }
        | DecJumpGe { .. }
        | Assign { .. }
        | Return { .. }
        | Echo { .. }
        | ArraySet { .. }
        | ArrayPush { .. }
        | LoadConst { .. } => {}
        Move { dst, src } => {
            if let Some(k) = const_index(*src) {
                *instr = LoadConst { dst: *dst, k: k as u16 };
            }
        }
        FetchDim { arr, .. } => ld!(arr),
        IssetDim { var, keys, n, .. } => {
            ld!(var);
            if *n != 1 {
                ld!(keys);
            }
        }
        // One key in place, or the value when the key is a register (the
        // two would not fit one instruction); a path of keys is a block
        // of temporaries, an append has no key.
        AssignDim { var, keys, n, append, val, .. } => {
            ld!(var);
            if *n >= 1 && (*n != 1 || *append) {
                ld!(keys);
            }
            if c(*keys) && c(*val) {
                ld!(val);
            }
        }
        // Name operands stay; what else they name is loaded.
        FetchProp { obj, .. } | IssetProp { obj, .. } | UnsetProp { obj, .. } | PropIncDec { obj, .. } => ld!(obj),
        // A literal value stays (PHP's ASSIGN_OBJ with OP_DATA CONST):
        // `$o->p = 'x'` is P8_ASSIGN_PROP_*_K_*, read in place.
        AssignProp { obj, .. } => ld!(obj),
        AssignPropOp { obj, val, .. } => ld!(obj, val),
        AssignPropDim { val, .. } => ld!(val),
        // `&$o->p`: the name stays (the loop's PropRef reads its cache
        // entry by it).
        Rare { op: p8_bytecode::Rare::PropRef, b, .. } => ld!(b),
        // A literal value stays (`C::$x = 'x'`: P8_ASSIGN_STATIC_PROP_*_K).
        ClassConst { .. } | FetchStaticProp { .. } | AssignStaticProp { .. } => {}
        AssignStaticPropOp { val, .. } => ld!(val),
        _ => crate::map_regs(instr, |r| if c(r) { load(r) } else { r }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p8_bytecode::const_operand as k;

    #[test]
    fn literals_stay_where_the_loop_reads_them() {
        let mut code = vec![
            Instr::JumpLt { a: 1, b: k(0), target: 3 },
            Instr::Add { dst: 2, a: k(1), b: 1 },
            Instr::Assign { var: 1, src: k(2) },
            Instr::FetchProp { dst: 2, obj: 1, name: k(3), quiet: false },
            Instr::ArraySet { arr: 2, key: k(0), val: k(1), take: false },
            Instr::AssignProp { obj: 1, name: k(3), val: k(2), dst: 2, take: false },
        ];
        let mut lines = vec![1; 6];
        let before = code.clone();
        assert_eq!(place_consts(&mut code, &mut lines, &mut [], &mut [], 10, &mut Scratch::default()), 0);
        assert_eq!(format!("{code:?}"), format!("{before:?}"));
    }

    #[test]
    fn other_literals_are_loaded_before_their_instruction() {
        let mut code = vec![
            Instr::Jump { target: 2 },
            Instr::Move { dst: 2, src: k(5) },
            // Two literals: the left one loaded, the right in place.
            Instr::JumpEq { a: k(1), b: k(2), target: 1 },
            Instr::Sub { dst: 2, a: k(1), b: k(2) },
            // Two loads for one instruction: two scratch temporaries.
            Instr::Rare { op: p8_bytecode::Rare::VarGet, a: 2, b: k(1), c: k(2), class: 0 },
            Instr::Echo { src: k(0) },
        ];
        let mut lines = vec![10, 11, 12, 13, 14, 15];
        let mut handlers = [Handler { start: 1, end: 5, target: 5, slot: 2, depth: 0 }];
        let mut silences = [(0u32, 5u32, 2 as Reg)];
        let loads = place_consts(&mut code, &mut lines, &mut handlers, &mut silences, 10, &mut Scratch::default());
        assert_eq!(loads, 2);
        // A jump to an instruction runs its loads too: it lands on them.
        let want = [
            Instr::Jump { target: 2 },
            Instr::LoadConst { dst: 2, k: 5 },
            Instr::LoadConst { dst: 10, k: 1 },
            Instr::JumpEq { a: 10, b: k(2), target: 1 },
            Instr::LoadConst { dst: 10, k: 1 },
            Instr::Sub { dst: 2, a: 10, b: k(2) },
            Instr::LoadConst { dst: 10, k: 1 },
            Instr::LoadConst { dst: 11, k: 2 },
            Instr::Rare { op: p8_bytecode::Rare::VarGet, a: 2, b: 10, c: 11, class: 0 },
            Instr::Echo { src: k(0) },
        ];
        assert_eq!(format!("{code:?}"), format!("{want:?}"));
        assert_eq!(lines, [10, 11, 12, 12, 13, 13, 14, 14, 14, 15]);
        assert_eq!((handlers[0].start, handlers[0].end, handlers[0].target), (1, 9, 9));
        assert_eq!(silences[0], (0, 9, 2));
    }
}

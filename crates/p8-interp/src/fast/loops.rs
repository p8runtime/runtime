//! #352's prototype: the interpreter loop in three other shapes, beside
//! `run` (the product's loop), to measure what the shape costs per
//! bytecode. A measurement, not a loop to ship as it is: the shape is
//! chosen by the environment (`P8_LOOP`), which the product never reads.
//!
//! Every shape runs the same handler bodies (`Context::p_arm`, the hand
//! arms of `run` copied with its macros redefined, and the generated
//! opcodes' handlers of `vmops.rs`):
//!
//! - a **P** handler does not change the frame stack: it takes the context
//!   shared, the frame's window, the frame, its closure, the pc and the
//!   instruction, all in registers, and gives back one word, the next pc.
//!   Anything else it has to say (the rare path, a destructor due, a
//!   return, "this one needs the context mutably") is the same word with
//!   a kind in its top byte (`SLOW`, `PEND`, `RET`, `F_CALL`, ...), so the
//!   loop's bounds check on `code[pc]` is also the test for it;
//! - an **F** handler (`f_call`, `f_method`, ...) takes the context
//!   mutably and may change frames; the loop reads the frame again after
//!   it;
//! - an opcode with no handler here yet runs `run`'s own arm, one
//!   instruction of it (`run::<true>`).
//!
//! The shapes: `loop_a`, one table of P handlers; `loop_b`, the hottest
//! straight-line P handlers inline in a small match and the table for the
//! rest; `loop_c`, every P handler inline in one match; `loop_d`, a big
//! inline list; `loop_e` to `loop_g`, B, D and C with the F handlers
//! inlined in the loop's function; `loop_i` and `loop_j`, E with a longer
//! inline list; `loop_h` and `loop_k`, one loop (no inner one) whose call
//! arms are the F handlers' bodies, the frame read again after them.

use super::*;

include!(concat!(env!("OUT_DIR"), "/gen_ops.rs"));

/// The loop `execute` runs: 0 `run` (the product's), 1 `loop_a`, 2
/// `loop_b`, 3 `loop_c`. (Read once from `P8_LOOP`: the prototype's
/// switch, not a product setting.)
pub(super) fn shape() -> u8 {
    static SHAPE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *SHAPE.get_or_init(|| match std::env::var("P8_LOOP").as_deref() {
        Ok("a") => 1,
        Ok("b") => 2,
        Ok("c") => 3,
        Ok("d") => 4,
        Ok("e") => 5,
        Ok("f") => 6,
        Ok("g") => 7,
        Ok("h") => 8,
        Ok("i") => 9,
        Ok("j") => 10,
        Ok("k") => 11,
        #[cfg(p8_proto_tail)]
        Ok("t") => 12,
        _ => 0,
    })
}

// A handler's word: the next pc, or a kind in the top byte with the pc of
// the instruction in the low 28 bits (and, for `PEND`, the instruction's
// own pc in the 28 above them).
const KIND: u32 = 56;
const AUX: u32 = 28;
const PC_MASK: usize = (1 << AUX) - 1;
const K_SLOW: u8 = 1;
const K_DEFER: u8 = 2;
const K_COLD: u8 = 3;
const K_PEND: u8 = 4;
const K_RET: u8 = 5;
const K_CALL: u8 = 6;
const K_METHOD: u8 = 7;
const K_NEW: u8 = 8;
const K_CONSTRUCT: u8 = 9;
const K_CALL_REGS: u8 = 10;
/// `slow(pc)`: the instruction is `step`'s.
const SLOW: usize = (K_SLOW as usize) << KIND;
/// `run::<true>(pc)`: `run`'s arm for the instruction.
const DEFER: usize = (K_DEFER as usize) << KIND;
/// The generated instruction's cold method.
const COLD: usize = (K_COLD as usize) << KIND;
/// `after_errors(next, at)`: a destructor is due.
const PEND: usize = (K_PEND as usize) << KIND;
/// A return: the value is in `Context::ret_slot`.
const RET: usize = (K_RET as usize) << KIND;
const F_CALL: usize = (K_CALL as usize) << KIND;
const F_METHOD: usize = (K_METHOD as usize) << KIND;
const F_NEW: usize = (K_NEW as usize) << KIND;
const F_CONSTRUCT: usize = (K_CONSTRUCT as usize) << KIND;
const F_CALL_REGS: usize = (K_CALL_REGS as usize) << KIND;

type P = fn(&Context<'_>, &mut [Value; WINDOW], &Frame, &Closure, usize, &Insn) -> usize;
type C = fn(&mut Context<'_>, &mut Vec<Value>, usize) -> Result<Flow, Throw>;

/// An opcode with no P handler: `run`'s arm.
fn p_defer(_: &Context<'_>, _: &mut [Value; WINDOW], _: &Frame, _: &Closure, pc: usize, _: &Insn) -> usize {
    DEFER | pc
}

/// The opcodes `p_arm` has a hand arm for (`loop_a`'s table entries).
macro_rules! hand_ops {
    ($m:ident) => {
        $m!(Slow VerifyArgs VerifyReturn Intrinsic SendVarMethod Jump Compare CompareC IncVar DecVar
            IncJumpLt IncJumpLe DecJumpGt DecJumpGe IncJumpLtC IncJumpLeC DecJumpGtC DecJumpGeC
            Binary BinaryC Add Sub Mul AddC SubC MulC ConcatN AssignDimOp IterNext IterNextKey
            AssignOp AssignOpC IncDec LoadNull Not ToBool IssetVar JumpIfSet NewArray ArrayPush
            ArrayPushC ArraySet ArraySetC FetchConst IterInit SendVar Call CallRegs MethodCall New
            Construct ReturnNull);
    };
}

/// `loop_a`'s table: an opcode's P handler (`p_arm` with the opcode
/// known, so each entry is that one arm).
static P_TABLE: [P; 256] = {
    let mut t: [P; 256] = [p_defer; 256];
    macro_rules! entry {
        ($op:ident) => {{
            fn e(cx: &Context<'_>, w: &mut [Value; WINDOW], top: &Frame, clo: &Closure, pc: usize, insn: &Insn) -> usize {
                cx.p_arm(w, top, clo, pc, insn, Op::$op)
            }
            e
        }};
    }
    macro_rules! hand {
        ($($op:ident)*) => { $( t[Op::$op as usize] = entry!($op); )* };
    }
    macro_rules! generated {
        ($(($op:ident, $cold:ident, |$hx:ident, $insn:ident| $call:expr);)*) => { $( t[Op::$op as usize] = entry!($op); )* };
    }
    hand_ops!(hand);
    gen_ops!(generated);
    t
};

/// The tail-call loop's table: an opcode's P handler, then the next
/// instruction's entry by a tail call (`become`), so no handler returns to
/// a loop until a word is not a pc of the code. A ceiling to measure, not
/// a proposal: `become` is a nightly feature.
#[cfg(p8_proto_tail)]
static T_TABLE: [P; 256] = {
    let mut t: [P; 256] = [p_defer; 256];
    macro_rules! entry {
        ($op:ident) => {{
            fn e(cx: &Context<'_>, w: &mut [Value; WINDOW], top: &Frame, clo: &Closure, pc: usize, insn: &Insn) -> usize {
                let next = cx.p_arm(&mut *w, top, clo, pc, insn, Op::$op);
                match clo.code.insns.get(next) {
                    Some(insn) => become T_TABLE[insn.op as u8 as usize](cx, w, top, clo, next, insn),
                    None => next,
                }
            }
            e
        }};
    }
    macro_rules! hand {
        ($($op:ident)*) => { $( t[Op::$op as usize] = entry!($op); )* };
    }
    macro_rules! generated {
        ($(($op:ident, $cold:ident, |$hx:ident, $insn:ident| $call:expr);)*) => { $( t[Op::$op as usize] = entry!($op); )* };
    }
    hand_ops!(hand);
    gen_ops!(generated);
    t
};

/// A generated opcode's cold method (`cold!` in `run`).
static COLD_TABLE: [C; 256] = {
    fn none(cx: &mut Context<'_>, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
        cx.run::<true>(regs, pc)
    }
    let mut t: [C; 256] = [none; 256];
    macro_rules! generated {
        ($(($op:ident, $cold:ident, |$hx:ident, $insn:ident| $call:expr);)*) => {
            $( t[Op::$op as usize] = {
                fn c(cx: &mut Context<'_>, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
                    cx.cold(regs, pc, Context::$cold)
                }
                c
            }; )*
        };
    }
    gen_ops!(generated);
    t
};

/// The frame's window of `regs` (the loop keeps the register file a
/// window ahead of the frame it runs).
#[inline(always)]
fn window(regs: &mut [Value], base: usize) -> &mut [Value; WINDOW] {
    (&mut regs[base..base + WINDOW]).try_into().expect("a window's worth of registers")
}

/// The window of a frame `regs` does not reach a window past yet.
#[cold]
#[inline(never)]
fn grown_window(regs: &mut Vec<Value>, base: usize) -> &mut [Value; WINDOW] {
    regs.resize(base + WINDOW, Value::UNDEF);
    window(regs, base)
}

/// A prototype loop: the frame read once per run of it, P handlers
/// (`$dispatch`, an expression giving the next word) until one gives a
/// word that is not a pc of the code, then what that word asks for
/// (`$special`: the F handlers called, or inlined here).
macro_rules! proto_loop {
    ($(#[$doc:meta])* $name:ident, $special:ident, |$cx:ident, $w:ident, $top:ident, $clo:ident, $pc:ident, $insn:ident| $dispatch:expr) => {
        $(#[$doc])*
        #[inline(never)]
        fn $name(&mut self, regs: &mut Vec<Value>, mut pc: usize) -> Result<(), Throw> {
            loop {
                let word = {
                    let $cx: &Self = &*self;
                    let $top = $cx.frames.last().expect("a frame");
                    let $clo = &*$top.func;
                    let base = $top.base as usize;
                    let code = &$clo.code.insns[..];
                    let $w: &mut [Value; WINDOW] = match regs.get_mut(base..base + WINDOW) {
                        Some(w) => w.try_into().expect("a window's worth of registers"),
                        None => grown_window(regs, base),
                    };
                    let mut $pc = pc;
                    loop {
                        let Some($insn) = code.get($pc) else { break $pc };
                        $pc = $dispatch;
                    }
                };
                match self.$special(regs, word)? {
                    Flow::Go(next) => pc = next,
                    Flow::Done => return Ok(()),
                }
            }
        }
    };
}

/// The frame as a loop holds it: the frame, its closure, its code and its
/// window of the registers.
macro_rules! frame {
    ($s:ident, $regs:ident) => {{
        let top = $s.frames.last().expect("a frame");
        let clo = &*top.func;
        let base = top.base as usize;
        let code = &clo.code.insns[..];
        let w: &mut [Value; WINDOW] = match $regs.get_mut(base..base + WINDOW) {
            Some(w) => w.try_into().expect("a window's worth of registers"),
            None => grown_window($regs, base),
        };
        (top, clo, code, w)
    }};
}

/// In a direct loop, what took the context mutably (`$flow`, a `Flow`):
/// the frame read again, the next pc given.
macro_rules! direct_f {
    ($s:ident, $regs:ident, $w:ident, $top:ident, $clo:ident, $code:ident; $flow:expr) => {
        match $flow? {
            Flow::Go(next) => {
                ($top, $clo, $code, $w) = frame!($s, $regs);
                next
            }
            Flow::Done => return Ok(()),
        }
    };
}

/// A direct loop's match: the listed opcodes' P handlers inline, the call
/// opcodes' arms as `run` has them (the F handlers' bodies, the frame read
/// again after one; `New` and a `Construct` with no constructor in place),
/// the table for `$rest`.
macro_rules! direct_arms {
    ($s:ident, $regs:ident, $w:ident, $top:ident, $clo:ident, $code:ident, $pc:ident, $insn:ident; $rest:pat => [$($op:ident)*]) => {
        match $insn.op {
            $(Op::$op => $s.p_arm(&mut *$w, $top, $clo, $pc, $insn, Op::$op),)*
            Op::Call => direct_f!($s, $regs, $w, $top, $clo, $code; $s.call_body($regs, $pc)),
            Op::MethodCall => direct_f!($s, $regs, $w, $top, $clo, $code; $s.method_body($regs, $pc)),
            Op::CallRegs => {
                if let Some(&Callee::Builtin(_)) = $clo.sites[($insn.imm & 0xffff) as usize].get() {
                    direct_f!($s, $regs, $w, $top, $clo, $code; $s.call_regs_body($regs, $pc))
                } else {
                    $s.p_arm(&mut *$w, $top, $clo, $pc, $insn, Op::CallRegs)
                }
            }
            Op::New => 'new: {
                let info = match Self::cached_new($clo, $insn.imm as u16) {
                    Some(info) => info,
                    None => match $s.new_info_kept($clo, $insn.imm as u16) {
                        Some(info) => info,
                        None => break 'new direct_f!($s, $regs, $w, $top, $clo, $code; $s.run::<true>($regs, $pc)),
                    },
                };
                let Some(object) = $s.make_plain(&info) else {
                    break 'new direct_f!($s, $regs, $w, $top, $clo, $code; $s.run::<true>($regs, $pc));
                };
                object.destructed.set(true);
                $s.last_new = Some(info);
                if put(&mut $w[usize::from($insn.a)], Value::object(object)) && $s.ids.has_pending() {
                    break 'new direct_f!($s, $regs, $w, $top, $clo, $code; $s.after_errors($regs, $pc + 1, $pc).map(Flow::Go));
                }
                $pc + 1
            }
            Op::Construct => 'construct: {
                let (obj, args, n) = ($insn.a, $insn.b, usize::from($insn.c));
                let Some(object) = $w[usize::from(obj)].as_object() else {
                    break 'construct direct_f!($s, $regs, $w, $top, $clo, $code; $s.slow($regs, $pc));
                };
                let info = match $s.last_new.as_ref() {
                    Some(i) if Rc::ptr_eq(&i.class, &object.class) => i,
                    _ => match $s.new_infos.get(&(Rc::as_ptr(&object.class) as usize)) {
                        Some(i) => i,
                        None => break 'construct direct_f!($s, $regs, $w, $top, $clo, $code; $s.run::<true>($regs, $pc)),
                    },
                };
                match &info.ctor {
                    None => {
                        object.destructed.set(false);
                        $s.last_new = None;
                        let start = usize::from(args);
                        let mut released = false;
                        for r in &mut $w[start..start + n] {
                            released |= clear(r);
                        }
                        if released && $s.ids.has_pending() {
                            break 'construct direct_f!($s, $regs, $w, $top, $clo, $code; $s.after_errors($regs, $pc + 1, $pc).map(Flow::Go));
                        }
                        $pc + 1
                    }
                    Some(_) => direct_f!($s, $regs, $w, $top, $clo, $code; $s.construct_body($regs, $pc)),
                }
            }
            $rest => P_TABLE[$insn.op as u8 as usize]($s, &mut *$w, $top, $clo, $pc, $insn),
        }
    };
}

/// A direct loop: one loop, the frame held across P handlers and read
/// again after anything that took the context mutably; `$list` is its
/// inline list (without the calls), `$rest` every other opcode.
macro_rules! direct_loop {
    ($(#[$doc:meta])* $name:ident, $list:ident, $rest:pat) => {
        $(#[$doc])*
        #[inline(never)]
        fn $name(&mut self, regs: &mut Vec<Value>, mut pc: usize) -> Result<(), Throw> {
            let (mut top, mut clo, mut code, mut w) = frame!(self, regs);
            loop {
                let Some(insn) = code.get(pc) else {
                    pc = direct_f!(self, regs, w, top, clo, code; self.special_direct(regs, pc));
                    continue;
                };
                pc = $list!(direct_arms!(self, regs, w, top, clo, code, pc, insn; $rest =>));
            }
        }
    };
}

/// A hybrid loop's match: the listed opcodes' handlers inline, the table
/// for `$rest` (every other opcode, so the match has no default).
macro_rules! hybrid {
    ($cx:ident, $w:ident, $top:ident, $clo:ident, $pc:ident, $insn:ident; $rest:pat => [$($op:ident)*]) => {
        match $insn.op {
            $(Op::$op => $cx.p_arm(&mut *$w, $top, $clo, $pc, $insn, Op::$op),)*
            $rest => P_TABLE[$insn.op as u8 as usize]($cx, &mut *$w, $top, $clo, $pc, $insn),
        }
    };
}

/// The simple inline list: moves, jumps, int arithmetic and compares, and
/// the returns (whose handlers are a word for the F handler); then `$more`.
macro_rules! simple_list {
    ($m:ident!($($args:tt)*) $($more:ident)*) => {
        $m!($($args)* [
            Slow VerifyArgs VerifyReturn Jump Compare CompareC IncVar DecVar
            IncJumpLt IncJumpLe DecJumpGt DecJumpGe IncJumpLtC IncJumpLeC DecJumpGtC DecJumpGeC
            Add Sub Mul AddC SubC MulC LoadNull ReturnNull
            P8_MOVE_REG P8_MOVE_K P8_ASSIGN_REG P8_ASSIGN_K P8_ASSIGN_TAKE P8_FREE P8_THIS
            P8_JUMP_COND_T P8_JUMP_COND_F
            P8_JUMP_CMP_LT_T_REG P8_JUMP_CMP_LT_T_K P8_JUMP_CMP_LT_F_REG P8_JUMP_CMP_LT_F_K
            P8_JUMP_CMP_LE_T_REG P8_JUMP_CMP_LE_T_K P8_JUMP_CMP_LE_F_REG P8_JUMP_CMP_LE_F_K
            P8_JUMP_CMP_GT_T_K P8_JUMP_CMP_GT_F_K P8_JUMP_CMP_GE_T_K P8_JUMP_CMP_GE_F_K
            P8_JUMP_CMP_EQ_T_REG P8_JUMP_CMP_EQ_T_K P8_JUMP_CMP_EQ_F_REG P8_JUMP_CMP_EQ_F_K
            P8_JUMP_CMP_ID_T_REG P8_JUMP_CMP_ID_T_K P8_JUMP_CMP_ID_F_REG P8_JUMP_CMP_ID_F_K
            P8_RETURN_TMP P8_RETURN_CV P8_RETURN_K
            $($more)*
        ])
    };
}

/// The small inline list: the simple one and the calls (whose handlers are
/// a word for the F handler).
macro_rules! small_list {
    ($($m:tt)*) => {
        simple_list!($($m)* Call CallRegs MethodCall New Construct)
    };
}

/// The simple list and the generated property and array handlers; then
/// `$more`.
macro_rules! data_list {
    ($m:ident!($($args:tt)*) $($more:ident)*) => {
        simple_list!($m!($($args)*)
            P8_FETCH_DIM_R_REG P8_FETCH_DIM_R_KINT P8_FETCH_DIM_R_KSTR
            P8_FETCH_DIM_IS_REG P8_FETCH_DIM_IS_KINT P8_FETCH_DIM_IS_KSTR
            P8_ISSET_DIM_REG P8_ISSET_DIM_KINT P8_ISSET_DIM_KSTR
            P8_ASSIGN_DIM_REG_REG_USED P8_ASSIGN_DIM_REG_REG_UNUSED P8_ASSIGN_DIM_REG_K_USED
            P8_ASSIGN_DIM_REG_K_UNUSED P8_ASSIGN_DIM_KINT_REG_USED P8_ASSIGN_DIM_KINT_REG_UNUSED
            P8_ASSIGN_DIM_KSTR_REG_USED P8_ASSIGN_DIM_KSTR_REG_UNUSED
            P8_APPEND_DIM_REG_USED P8_APPEND_DIM_REG_UNUSED P8_APPEND_DIM_K_USED P8_APPEND_DIM_K_UNUSED
            P8_FETCH_PROP_REG P8_FETCH_PROP_THIS
            P8_ASSIGN_PROP_REG_REG_COPY_USED P8_ASSIGN_PROP_REG_REG_COPY_UNUSED
            P8_ASSIGN_PROP_REG_REG_TAKE_USED P8_ASSIGN_PROP_REG_REG_TAKE_UNUSED
            P8_ASSIGN_PROP_REG_K_COPY_USED P8_ASSIGN_PROP_REG_K_COPY_UNUSED
            P8_ASSIGN_PROP_THIS_REG_COPY_USED P8_ASSIGN_PROP_THIS_REG_COPY_UNUSED
            P8_ASSIGN_PROP_THIS_REG_TAKE_USED P8_ASSIGN_PROP_THIS_REG_TAKE_UNUSED
            P8_ASSIGN_PROP_THIS_K_COPY_USED P8_ASSIGN_PROP_THIS_K_COPY_UNUSED
            $($more)*
        )
    };
}

/// The medium inline list: the small one and the generated property and
/// array handlers.
macro_rules! medium_list {
    ($($m:tt)*) => {
        data_list!($($m)* Call CallRegs MethodCall New Construct)
    };
}

/// The medium list and the argument handlers.
macro_rules! sends_list {
    ($($m:tt)*) => {
        data_list!($($m)* Call CallRegs MethodCall New Construct SendVarMethod SendVar)
    };
}

/// The medium list and the argument handlers, without the calls (a loop
/// whose call arms are its own).
macro_rules! direct_sends_list {
    ($($m:tt)*) => {
        data_list!($($m)* SendVarMethod SendVar)
    };
}

/// The big inline list: the small one and the property, array, argument
/// and operator handlers.
macro_rules! big_list {
    ($m:ident!($($args:tt)*)) => {
        $m!($($args)* [
            Slow VerifyArgs VerifyReturn Jump Compare CompareC IncVar DecVar
            IncJumpLt IncJumpLe DecJumpGt DecJumpGe IncJumpLtC IncJumpLeC DecJumpGtC DecJumpGeC
            Add Sub Mul AddC SubC MulC LoadNull Call CallRegs MethodCall New Construct ReturnNull
            P8_MOVE_REG P8_MOVE_K P8_ASSIGN_REG P8_ASSIGN_K P8_ASSIGN_TAKE P8_FREE P8_THIS
            P8_JUMP_COND_T P8_JUMP_COND_F
            P8_JUMP_CMP_LT_T_REG P8_JUMP_CMP_LT_T_K P8_JUMP_CMP_LT_F_REG P8_JUMP_CMP_LT_F_K
            P8_JUMP_CMP_LE_T_REG P8_JUMP_CMP_LE_T_K P8_JUMP_CMP_LE_F_REG P8_JUMP_CMP_LE_F_K
            P8_JUMP_CMP_GT_T_K P8_JUMP_CMP_GT_F_K P8_JUMP_CMP_GE_T_K P8_JUMP_CMP_GE_F_K
            P8_JUMP_CMP_EQ_T_REG P8_JUMP_CMP_EQ_T_K P8_JUMP_CMP_EQ_F_REG P8_JUMP_CMP_EQ_F_K
            P8_JUMP_CMP_ID_T_REG P8_JUMP_CMP_ID_T_K P8_JUMP_CMP_ID_F_REG P8_JUMP_CMP_ID_F_K
            P8_RETURN_TMP P8_RETURN_CV P8_RETURN_K
            Intrinsic SendVarMethod Binary BinaryC ConcatN AssignDimOp IterNext IterNextKey
            AssignOp AssignOpC IncDec Not ToBool IssetVar JumpIfSet NewArray ArrayPush ArrayPushC
            ArraySet ArraySetC FetchConst IterInit SendVar
            P8_FETCH_DIM_R_REG P8_FETCH_DIM_R_KINT P8_FETCH_DIM_R_KSTR
            P8_FETCH_DIM_IS_REG P8_FETCH_DIM_IS_KINT P8_FETCH_DIM_IS_KSTR
            P8_ISSET_DIM_REG P8_ISSET_DIM_KINT P8_ISSET_DIM_KSTR
            P8_ASSIGN_DIM_REG_REG_USED P8_ASSIGN_DIM_REG_REG_UNUSED P8_ASSIGN_DIM_REG_K_USED
            P8_ASSIGN_DIM_REG_K_UNUSED P8_ASSIGN_DIM_KINT_REG_USED P8_ASSIGN_DIM_KINT_REG_UNUSED
            P8_ASSIGN_DIM_KSTR_REG_USED P8_ASSIGN_DIM_KSTR_REG_UNUSED
            P8_APPEND_DIM_REG_USED P8_APPEND_DIM_REG_UNUSED P8_APPEND_DIM_K_USED P8_APPEND_DIM_K_UNUSED
            P8_FETCH_PROP_REG P8_FETCH_PROP_THIS
            P8_ASSIGN_PROP_REG_REG_COPY_USED P8_ASSIGN_PROP_REG_REG_COPY_UNUSED
            P8_ASSIGN_PROP_REG_REG_TAKE_USED P8_ASSIGN_PROP_REG_REG_TAKE_UNUSED
            P8_ASSIGN_PROP_REG_K_COPY_USED P8_ASSIGN_PROP_REG_K_COPY_UNUSED
            P8_ASSIGN_PROP_THIS_REG_COPY_USED P8_ASSIGN_PROP_THIS_REG_COPY_UNUSED
            P8_ASSIGN_PROP_THIS_REG_TAKE_USED P8_ASSIGN_PROP_THIS_REG_TAKE_UNUSED
            P8_ASSIGN_PROP_THIS_K_COPY_USED P8_ASSIGN_PROP_THIS_K_COPY_UNUSED
        ])
    };
}

/// The argument opcodes.
macro_rules! send_ops {
    () => {
        Op::SendVarMethod | Op::SendVar
    };
}

/// The opcodes the big list adds to the small one.
macro_rules! data_ops {
    () => {
        send_ops!() | hand_data_ops!() | generated_data_ops!()
    };
}

/// The hand opcodes the big list adds, but for the argument ones.
macro_rules! hand_data_ops {
    () => {
        Op::Intrinsic
            | Op::Binary
            | Op::BinaryC
            | Op::ConcatN
            | Op::AssignDimOp
            | Op::IterNext
            | Op::IterNextKey
            | Op::AssignOp
            | Op::AssignOpC
            | Op::IncDec
            | Op::Not
            | Op::ToBool
            | Op::IssetVar
            | Op::JumpIfSet
            | Op::NewArray
            | Op::ArrayPush
            | Op::ArrayPushC
            | Op::ArraySet
            | Op::ArraySetC
            | Op::FetchConst
            | Op::IterInit
    };
}

/// The generated property and array opcodes (the medium list's).
macro_rules! generated_data_ops {
    () => {
        Op::P8_FETCH_DIM_R_REG
            | Op::P8_FETCH_DIM_R_KINT
            | Op::P8_FETCH_DIM_R_KSTR
            | Op::P8_FETCH_DIM_IS_REG
            | Op::P8_FETCH_DIM_IS_KINT
            | Op::P8_FETCH_DIM_IS_KSTR
            | Op::P8_ISSET_DIM_REG
            | Op::P8_ISSET_DIM_KINT
            | Op::P8_ISSET_DIM_KSTR
            | Op::P8_ASSIGN_DIM_REG_REG_USED
            | Op::P8_ASSIGN_DIM_REG_REG_UNUSED
            | Op::P8_ASSIGN_DIM_REG_K_USED
            | Op::P8_ASSIGN_DIM_REG_K_UNUSED
            | Op::P8_ASSIGN_DIM_KINT_REG_USED
            | Op::P8_ASSIGN_DIM_KINT_REG_UNUSED
            | Op::P8_ASSIGN_DIM_KSTR_REG_USED
            | Op::P8_ASSIGN_DIM_KSTR_REG_UNUSED
            | Op::P8_APPEND_DIM_REG_USED
            | Op::P8_APPEND_DIM_REG_UNUSED
            | Op::P8_APPEND_DIM_K_USED
            | Op::P8_APPEND_DIM_K_UNUSED
            | Op::P8_FETCH_PROP_REG
            | Op::P8_FETCH_PROP_THIS
            | Op::P8_ASSIGN_PROP_REG_REG_COPY_USED
            | Op::P8_ASSIGN_PROP_REG_REG_COPY_UNUSED
            | Op::P8_ASSIGN_PROP_REG_REG_TAKE_USED
            | Op::P8_ASSIGN_PROP_REG_REG_TAKE_UNUSED
            | Op::P8_ASSIGN_PROP_REG_K_COPY_USED
            | Op::P8_ASSIGN_PROP_REG_K_COPY_UNUSED
            | Op::P8_ASSIGN_PROP_THIS_REG_COPY_USED
            | Op::P8_ASSIGN_PROP_THIS_REG_COPY_UNUSED
            | Op::P8_ASSIGN_PROP_THIS_REG_TAKE_USED
            | Op::P8_ASSIGN_PROP_THIS_REG_TAKE_UNUSED
            | Op::P8_ASSIGN_PROP_THIS_K_COPY_USED
            | Op::P8_ASSIGN_PROP_THIS_K_COPY_UNUSED
    };
}

/// The opcodes in neither list: the rare generated ones and the hand
/// ones `p_arm` has no arm for.
macro_rules! rare_ops {
    () => {
        Op::P8_INSTANCEOF_NAMED
            | Op::P8_INSTANCEOF_STATIC
            | Op::P8_INSTANCEOF_SCOPE
            | Op::P8_CLASS_CONST_NAMED
            | Op::P8_CLASS_CONST_STATIC
            | Op::P8_CLASS_CONST_SCOPE
            | Op::P8_FETCH_STATIC_PROP_NAMED
            | Op::P8_FETCH_STATIC_PROP_STATIC
            | Op::P8_FETCH_STATIC_PROP_SCOPE
            | Op::P8_ASSIGN_STATIC_PROP_NAMED_REG
            | Op::P8_ASSIGN_STATIC_PROP_NAMED_K
            | Op::P8_ASSIGN_STATIC_PROP_STATIC_REG
            | Op::P8_ASSIGN_STATIC_PROP_STATIC_K
            | Op::P8_ASSIGN_STATIC_PROP_SCOPE_REG
            | Op::P8_ASSIGN_STATIC_PROP_SCOPE_K
            | Op::StaticCall
            | Op::StaticCallVars
            | Op::MakeClosure
            | Op::CollectVariadic
            | Op::ArrayUnpack
            | Op::ConstructSpread
            | Op::SendVarStatic
            | Op::BindGlobal
            | Op::AssignDimPath
            | Op::AssignPropDim
            | Op::UnsetDim
            | Op::DimIncDec
            | Op::Cast
            | Op::PropIncDec
            | Op::MethodArgByRef
            | Op::PropRef
            | Op::StaticArgByRef
            | Op::ArgDefault
            | Op::FuncArgByRef
            | Op::StaticVar
            | Op::IterInitRef
            | Op::IterNextRef
            | Op::FetchList
            | Op::CallValue
            | Op::AssignPropOp
            | Op::BindRef
            | Op::UnsetVar
            | Op::ToObject
            | Op::CallableArgByRef
            | Op::VarGet
            | Op::AutoGlobal
            | Op::ArrayPushRef
            | Op::AssignStaticPropOp
            | Op::DeclareClass
            | Op::DeclareFunction
            | Op::Include
            | Op::FromCallable
            | Op::Yield
            | Op::YieldFrom
            | Op::RefDim
            | Op::DimSetRef
            | Op::StaticRef
            | Op::FetchPropDyn
            | Op::AssignPropDyn
            | Op::IssetProp
            | Op::IssetDimPath
            | Op::CallSpread
            | Op::Silence
            | Op::Unsilence
            | Op::Neg
            | Op::Capture
            | Op::ClassName
    };
}

/// A word's F handler (`$f`: the handler called, or its body inlined).
macro_rules! special_fn {
    ($(#[$doc:meta])* $name:ident, $ret:ident, $call:ident, $method:ident, $new:ident, $construct:ident, $call_regs:ident) => {
        $(#[$doc])*
        #[inline(always)]
        fn $name(&mut self, regs: &mut Vec<Value>, word: usize) -> Result<Flow, Throw> {
            let at = word & PC_MASK;
            match (word >> KIND) as u8 {
                K_SLOW => self.slow(regs, at),
                K_DEFER => self.run::<true>(regs, at),
                K_COLD => self.f_cold(regs, at),
                K_PEND => self.after_errors(regs, at, word >> AUX & PC_MASK).map(Flow::Go),
                K_RET => self.$ret(regs, at),
                K_CALL => self.$call(regs, at),
                K_METHOD => self.$method(regs, at),
                K_NEW => self.$new(regs, at),
                K_CONSTRUCT => self.$construct(regs, at),
                K_CALL_REGS => self.$call_regs(regs, at),
                _ => panic!("pc {word} is not in the code"),
            }
        }
    };
}

/// An F handler out of line.
macro_rules! out_of_line {
    ($($name:ident = $body:ident;)*) => {
        $(
            #[inline(never)]
            fn $name(&mut self, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
                self.$body(regs, pc)
            }
        )*
    };
}

impl Context<'_> {
    pub(super) fn run_shape(&mut self, regs: &mut Vec<Value>, pc: usize, shape: u8) -> Result<(), Throw> {
        match shape {
            1 => self.loop_a(regs, pc),
            2 => self.loop_b(regs, pc),
            3 => self.loop_c(regs, pc),
            4 => self.loop_d(regs, pc),
            5 => self.loop_e(regs, pc),
            6 => self.loop_f(regs, pc),
            7 => self.loop_g(regs, pc),
            8 => self.loop_h(regs, pc),
            9 => self.loop_i(regs, pc),
            10 => self.loop_j(regs, pc),
            #[cfg(p8_proto_tail)]
            12 => self.loop_t(regs, pc),
            _ => self.loop_k(regs, pc),
        }
    }

    proto_loop!(
        /// Shape T (nightly, a ceiling): every P handler tail-calls the next.
        #[cfg(p8_proto_tail)]
        loop_t,
        special_inline,
        |cx, w, top, clo, pc, insn| T_TABLE[insn.op as u8 as usize](cx, &mut *w, top, clo, pc, insn)
    );

    direct_loop!(
        /// Shape H: E's inline list in one loop whose call arms are its own.
        loop_h,
        simple_list,
        data_ops!() | rare_ops!()
    );

    proto_loop!(
        /// Shape I: E with the generated property and array handlers inline.
        loop_i,
        special_inline,
        |cx, w, top, clo, pc, insn| medium_list!(hybrid!(cx, w, top, clo, pc, insn; send_ops!() | hand_data_ops!() | rare_ops!() =>))
    );

    proto_loop!(
        /// Shape J: I with the argument handlers inline.
        loop_j,
        special_inline,
        |cx, w, top, clo, pc, insn| sends_list!(hybrid!(cx, w, top, clo, pc, insn; hand_data_ops!() | rare_ops!() =>))
    );

    direct_loop!(
        /// Shape K: J's inline list in one loop whose call arms are its own.
        loop_k,
        direct_sends_list,
        hand_data_ops!() | rare_ops!()
    );

    /// What a handler's word asks for in a direct loop (no call's word:
    /// the calls are the loop's own arms).
    #[inline(always)]
    fn special_direct(&mut self, regs: &mut Vec<Value>, word: usize) -> Result<Flow, Throw> {
        let at = word & PC_MASK;
        match (word >> KIND) as u8 {
            K_SLOW => self.slow(regs, at),
            K_DEFER => self.run::<true>(regs, at),
            K_COLD => self.f_cold(regs, at),
            K_PEND => self.after_errors(regs, at, word >> AUX & PC_MASK).map(Flow::Go),
            K_RET => self.ret_body(regs, at),
            _ => panic!("pc {word} is not in the code"),
        }
    }

    proto_loop!(
        /// Shape A: every P handler through one table.
        loop_a,
        special,
        |cx, w, top, clo, pc, insn| P_TABLE[insn.op as u8 as usize](cx, &mut *w, top, clo, pc, insn)
    );

    proto_loop!(
        /// Shape B: the small list inline, the table for the rest.
        loop_b,
        special,
        |cx, w, top, clo, pc, insn| small_list!(hybrid!(cx, w, top, clo, pc, insn; data_ops!() | rare_ops!() =>))
    );

    proto_loop!(
        /// Shape C: every P handler inline in one match.
        loop_c,
        special,
        |cx, w, top, clo, pc, insn| cx.p_arm(&mut *w, top, clo, pc, insn, insn.op)
    );

    proto_loop!(
        /// Shape D: the big list inline, the table for the rest.
        loop_d,
        special,
        |cx, w, top, clo, pc, insn| big_list!(hybrid!(cx, w, top, clo, pc, insn; rare_ops!() =>))
    );

    proto_loop!(
        /// Shape E: B with the F handlers inlined in the loop's function.
        loop_e,
        special_inline,
        |cx, w, top, clo, pc, insn| small_list!(hybrid!(cx, w, top, clo, pc, insn; data_ops!() | rare_ops!() =>))
    );

    proto_loop!(
        /// Shape F: D with the F handlers inlined in the loop's function.
        loop_f,
        special_inline,
        |cx, w, top, clo, pc, insn| big_list!(hybrid!(cx, w, top, clo, pc, insn; rare_ops!() =>))
    );

    proto_loop!(
        /// Shape G: C with the F handlers inlined in the loop's function.
        loop_g,
        special_inline,
        |cx, w, top, clo, pc, insn| cx.p_arm(&mut *w, top, clo, pc, insn, insn.op)
    );

    special_fn!(
        /// What a handler's word asks for when it is not a pc of the code:
        /// the F handlers called.
        special, f_ret, f_call, f_method, f_new, f_construct, f_call_regs
    );

    special_fn!(
        /// `special`, the F handlers inlined.
        special_inline, ret_body, call_body, method_body, new_body, construct_body, call_regs_body
    );

    out_of_line! {
        f_ret = ret_body;
        f_call = call_body;
        f_method = method_body;
        f_new = new_body;
        f_construct = construct_body;
        f_call_regs = call_regs_body;
    }

    #[inline(never)]
    fn f_cold(&mut self, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
        let op = self.frames.last().expect("a frame").func.code.insns[pc].op;
        COLD_TABLE[op as u8 as usize](self, regs, pc)
    }

    /// A return of the value a handler left in `ret_slot` (`ret!` in `run`).
    #[inline(always)]
    fn ret_body(&mut self, regs: &mut Vec<Value>, at: usize) -> Result<Flow, Throw> {
        let value = self.ret_slot.take();
        if self.frames.len() == self.frames.floor + 1 {
            self.pc = at + 1;
            leave(&mut self.frames, regs, value);
            if self.ids.has_pending() {
                self.run_destructors(regs)?;
            }
            return Ok(Flow::Done);
        }
        leave(&mut self.frames, regs, value);
        let pc = self.frames.last().expect("the caller's frame").pc as usize;
        if self.ids.has_pending() {
            return self.after_errors(regs, pc, pc.saturating_sub(1)).map(Flow::Go);
        }
        Ok(Flow::Go(pc))
    }

    /// A frameless builtin call (`frameless!` in `run`), the frame's
    /// window read from `regs`.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)] // (`frameless!`'s operands, as `run` names them.)
    fn f_frameless(&mut self, regs: &mut Vec<Value>, base: usize, i: usize, run: Imp, (start, n, dst): (usize, usize, usize), temps: bool, at: usize) -> Result<Flow, Throw> {
        self.pc = at + 1;
        let w = window(regs, base);
        let r = self.frameless_run(i, run, &mut w[start..start + n]);
        match r {
            Some(v) if self.diags.is_empty() && self.pending_errors.is_empty() => {
                if temps {
                    for r in &mut w[start..start + n] {
                        let _ = clear(r);
                    }
                }
                let _ = put(&mut w[dst], v);
                if self.ids.has_pending() {
                    return self.after_errors(regs, at + 1, at).map(Flow::Go);
                }
                Ok(Flow::Go(at + 1))
            }
            r => self.frameless_tail(regs, at, i, r, (base + start, n, base + dst, temps)).map(Flow::Go),
        }
    }

    /// `Op::Call` (`run`'s arm): a resolved user function's frame pushed,
    /// a resolved builtin called (frameless when it can be).
    #[inline(always)]
    fn call_body(&mut self, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
        let top = self.frames.last().expect("a frame");
        let clo = &*top.func;
        let base = top.base as usize;
        let insn = clo.code.insns[pc];
        let (args, n, dst, imm) = (insn.a, insn.b, insn.c, insn.imm);
        match clo.sites[imm as usize].get() {
            Some(&Callee::User(target)) => {
                let target = &self.closures[target as usize];
                if (n as usize) < target.code.required || target.code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
                    return self.run::<true>(regs, pc);
                }
                let native = target.code.native;
                self.frames.call(Some(target), pc + 1, base + args as usize, n as usize, (base + dst as usize) as u32);
                enter(&mut self.frames, regs);
                match native {
                    Some(id) => self.native_entry(regs, id).map(Flow::Go),
                    None => Ok(Flow::Go(0)),
                }
            }
            Some(&Callee::Builtin(i)) => {
                let (start, n) = (usize::from(args), usize::from(n));
                let call = self.isolate.registry.call(i);
                if let Some(run) = call.frameless
                    && call.plain(&regs[base + start..base + start + n])
                    && (!self.header_callback_set || self.out.started)
                {
                    self.f_frameless(regs, base, i, run, (start, n, usize::from(dst)), true, pc)
                } else {
                    self.builtin_call(regs, pc, base, i, (Reg::from(args), n as u8, Reg::from(dst))).map(Flow::Go)
                }
            }
            _ => self.run::<true>(regs, pc),
        }
    }

    /// `Op::CallRegs` on a resolved builtin (`run`'s arm): frameless on
    /// the variables themselves when it can be, else the first Move.
    #[inline(always)]
    fn call_regs_body(&mut self, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
        let top = self.frames.last().expect("a frame");
        let clo = &*top.func;
        let base = top.base as usize;
        let insn = clo.code.insns[pc];
        let (a, b, c, imm) = (insn.a, insn.b, insn.c, insn.imm);
        let (start, n) = (usize::from(a), usize::from(b));
        let Some(&Callee::Builtin(i)) = clo.sites[(imm & 0xffff) as usize].get() else { return self.run::<true>(regs, pc) };
        let call = self.isolate.registry.call(i);
        if imm & insn::ARG_CONST != 0 {
            // (The one constant argument lent: `run`'s arm.)
            return self.run::<true>(regs, pc);
        }
        if let Some(run) = call.frameless
            && matches!(run, Imp::Lent(_))
            && call.plain(&regs[base + start..base + start + n])
            && (!self.header_callback_set || self.out.started)
        {
            return self.f_frameless(regs, base, i, run, (start, n, usize::from(c)), false, pc + n);
        }
        let w = window(regs, base);
        let Some(v) = copied(&w[start]) else { return self.slow(regs, pc) };
        let _ = put(&mut w[(imm >> 16) as usize & (WINDOW - 1)], v);
        if self.ids.has_pending() {
            return self.after_errors(regs, pc + 1, pc).map(Flow::Go);
        }
        Ok(Flow::Go(pc + 1))
    }

    /// `Op::MethodCall` (`run`'s arm): the cached user method's frame
    /// pushed.
    #[inline(always)]
    fn method_body(&mut self, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
        let top = self.frames.last().expect("a frame");
        let clo = &*top.func;
        let base = top.base as usize;
        let insn = clo.code.insns[pc];
        let (dst, obj, args, method, n) = (insn.a, insn.b, insn.c, (insn.imm & 0xffff) as usize, (insn.imm >> 16) as usize);
        let w = window(regs, base);
        let held;
        let o = match w[usize::from(obj)].as_object() {
            Some(o) => o,
            None => {
                held = ref_object(&w[usize::from(obj)]);
                match &held {
                    Some(o) => o,
                    None => return self.run::<true>(regs, pc),
                }
            }
        };
        let scope = top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
        let found = {
            let site = clo.methods[method].borrow();
            match site.as_ref() {
                Some(s) if s.class == Rc::as_ptr(&o.class) as usize && s.scope == scope && !s.is_static => match &s.code {
                    // (Not `zip`: with this many loops in one binary it is a call.)
                    MethodCode::User(code) => match (self.frames.site_code(code), s.declaring.upgrade()) {
                        (Some(target), Some(declaring)) => Some((target, declaring)),
                        _ => None,
                    },
                    MethodCode::Native(..) => None,
                },
                _ => None,
            }
        };
        let Some((target, declaring)) = found else { return self.run::<true>(regs, pc) };
        let code = &self.frames.next_code(target.as_ref()).code;
        if n < code.required || code.is_generator || self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT {
            return self.run::<true>(regs, pc);
        }
        let this = Rc::clone(o);
        if is_temp(&clo.code, Reg::from(obj)) {
            clear(&mut w[usize::from(obj)]);
        }
        let frame = self.frames.call(target.as_ref(), pc + 1, base + args as usize, n, (base + dst as usize) as u32);
        frame.this = Some(this);
        frame.scope = Some(declaring);
        enter(&mut self.frames, regs);
        Ok(Flow::Go(0))
    }

    /// `Op::New` (`run`'s arm): an object that needs only its slots.
    #[inline(always)]
    fn new_body(&mut self, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
        let top = self.frames.last().expect("a frame");
        let clo = &*top.func;
        let base = top.base as usize;
        let insn = clo.code.insns[pc];
        let info = match Self::cached_new(clo, insn.imm as u16) {
            Some(info) => info,
            None => match self.new_info_kept(clo, insn.imm as u16) {
                Some(info) => info,
                None => return self.run::<true>(regs, pc),
            },
        };
        let Some(object) = self.make_plain(&info) else { return self.run::<true>(regs, pc) };
        object.destructed.set(true);
        self.last_new = Some(info);
        if put(&mut window(regs, base)[usize::from(insn.a)], Value::object(object)) && self.ids.has_pending() {
            return self.after_errors(regs, pc + 1, pc).map(Flow::Go);
        }
        Ok(Flow::Go(pc + 1))
    }

    /// `Op::Construct` (`run`'s arm): the PHP constructor's frame pushed;
    /// none, the arguments released.
    #[inline(always)]
    fn construct_body(&mut self, regs: &mut Vec<Value>, pc: usize) -> Result<Flow, Throw> {
        let top = self.frames.last().expect("a frame");
        let base = top.base as usize;
        let insn = top.func.code.insns[pc];
        let (obj, args, n) = (insn.a, insn.b, usize::from(insn.c));
        let w = window(regs, base);
        let Some(object) = w[usize::from(obj)].as_object() else { return self.slow(regs, pc) };
        let info = match self.last_new.as_ref() {
            Some(i) if Rc::ptr_eq(&i.class, &object.class) => i,
            _ => match self.new_infos.get(&(Rc::as_ptr(&object.class) as usize)) {
                Some(i) => i,
                None => return self.run::<true>(regs, pc),
            },
        };
        match &info.ctor {
            Some(crate::instances::Ctor { func: classes::Code::User(f), declaring, .. })
                if !f.code.is_generator && n >= f.code.required && self.stop.load(AtomicOrdering::Relaxed) == NO_INTERRUPT =>
            {
                let (f, declaring) = (Rc::clone(f), Rc::clone(declaring));
                let object = Rc::clone(object);
                self.last_new = None;
                object.destructed.set(false);
                let frame = self.frames.call(Some(&f), pc + 1, base + args as usize, n, NO_RET);
                frame.this = Some(object);
                frame.scope = Some(declaring);
                enter(&mut self.frames, regs);
                Ok(Flow::Go(0))
            }
            None => {
                object.destructed.set(false);
                self.last_new = None;
                let start = usize::from(args);
                let mut released = false;
                for r in &mut w[start..start + n] {
                    released |= clear(r);
                }
                if released && self.ids.has_pending() {
                    return self.after_errors(regs, pc + 1, pc).map(Flow::Go);
                }
                Ok(Flow::Go(pc + 1))
            }
            Some(_) => self.run::<true>(regs, pc),
        }
    }

    /// The P handler of `op` (the instruction `insn` at `pc`): `run`'s arm
    /// for it, the arm's macros giving a word back where `run`'s go out of
    /// the loop. Inlined where `op` is a constant (a table entry, a match
    /// arm), it is that one arm.
    #[inline(always)]
    fn p_arm(&self, w: &mut [Value; WINDOW], top: &Frame, clo: &Closure, mut pc: usize, insn: &Insn, op: Op) -> usize {
        macro_rules! back {
            ($target:expr) => {
                $target as usize <= pc && self.stop.load(AtomicOrdering::Relaxed) != NO_INTERRUPT
            };
        }
        macro_rules! slow {
            () => {{
                return SLOW | pc;
            }};
        }
        // `run`'s arm, from its start (nothing is written before this).
        macro_rules! defer {
            () => {{
                return DEFER | pc;
            }};
        }
        macro_rules! pending_at {
            ($next:expr, $at:expr) => {{
                return PEND | ($at) << AUX | ($next);
            }};
        }
        macro_rules! pending {
            () => {
                pending_at!(pc, pc.saturating_sub(1))
            };
        }
        macro_rules! done {
            ($released:expr) => {{
                let released: bool = $released;
                pc += 1;
                if released && self.ids.has_pending() {
                    pending!();
                }
            }};
        }
        macro_rules! rx {
            ($e:expr) => {
                usize::from($e)
            };
        }
        macro_rules! rk {
            ($e:expr) => {
                ($e) & (WINDOW - 1)
            };
        }
        macro_rules! cst {
            ($k:expr) => {
                &clo.consts[$k as usize]
            };
        }
        macro_rules! reg_src {
            ($r:expr) => {
                Src::Reg(Reg::from($r))
            };
        }
        macro_rules! cst_src {
            ($k:expr) => {
                Src::Const(cst!($k))
            };
        }
        macro_rules! arith {
            ($op:expr, $dst:expr, $a:expr, $b:expr, $int:expr) => {{
                let b: Src = $b;
                let released = if let (Some(x), Some(y)) = (w[rx!($a)].as_int(), src(&w[..], b).as_int())
                    && let Some(n) = $int(x, y)
                {
                    put_var(&mut w[rx!($dst)], Value::int(n))
                } else if arith(&mut w[..], $op, Reg::from($dst), Reg::from($a), b) {
                    true
                } else {
                    slow!()
                };
                done!(released);
            }};
        }
        macro_rules! assign_op {
            ($op:expr, $var:expr, $b:expr) => {{
                let b: Src = $b;
                let released = if let (Some(x), Some(y)) = (w[rx!($var)].as_int(), src(&w[..], b).as_int())
                    && let Some(n) = int_op($op, x, y)
                {
                    put(&mut w[rx!($var)], Value::int(n))
                } else if assign_op(&mut w[..], $op, Reg::from($var), b) {
                    true
                } else {
                    slow!()
                };
                done!(released);
            }};
        }
        macro_rules! compare {
            ($cmp:expr, $dst:expr, $a:expr, $y:expr) => {{
                let cmp: Cmp = $cmp;
                let (x, y): (&Value, &Value) = (&w[rx!($a)], $y);
                let t = if let (Some(x), Some(y)) = (x.as_int(), y.as_int())
                    && cmp != Cmp::Spaceship
                {
                    int_test(cmp, x, y)
                } else if cmp == Cmp::Spaceship {
                    slow!()
                } else {
                    match test(cmp, x, y) {
                        Some(t) => t,
                        None => slow!(),
                    }
                };
                done!(put_var(&mut w[rx!($dst)], Value::bool(t)));
            }};
        }
        macro_rules! array_push {
            ($arr:expr, $val:expr, $take:expr) => {{
                let val: Src = $val;
                let Some(arr) = w[rx!($arr)].as_array() else { slow!() };
                if arr.next_key().is_none() {
                    slow!();
                }
                let Some(v) = taken(&mut w[..], val, $take) else { slow!() };
                let _ = w[rx!($arr)].array_mut().expect("checked above").push(v);
                done!(true);
            }};
        }
        macro_rules! array_set {
            ($arr:expr, $key:expr, $val:expr, $take:expr) => {{
                let (key, val): (Src, Src) = ($key, $val);
                if !w[rx!($arr)].is_array() {
                    slow!();
                }
                let key = src(&w[..], key);
                let k = if let Some(k) = key.as_int() {
                    Key::Int(k)
                } else if let Some(s) = key.cloned_str() {
                    Key::from_string(s)
                } else {
                    slow!()
                };
                let Some(v) = taken(&mut w[..], val, $take) else { slow!() };
                w[rx!($arr)].array_mut().expect("checked above").insert(k, v);
                done!(true);
            }};
        }
        macro_rules! step_jump {
            ($var:expr, $b:expr, $target:expr, $step:expr, $holds:expr) => {{
                let r = &mut w[rx!($var)];
                let n = if let Some(i) = r.as_int_mut() {
                    match $step(*i, 1) {
                        Some(n) => {
                            *i = n;
                            n
                        }
                        None => slow!(),
                    }
                } else if let Some(cell) = r.as_reference() {
                    match step_ref(cell, $step) {
                        Some(n) => n,
                        None => slow!(),
                    }
                } else {
                    slow!()
                };
                match int_of($b) {
                    Some(b) if $holds(n, b) => {
                        if back!($target) {
                            // The step is done: the test (next) sees the interrupt.
                            pc += 1;
                            slow!();
                        }
                        pc = $target as usize;
                    }
                    Some(_) => pc += 2,
                    None => pc += 1,
                }
            }};
        }
        macro_rules! hx {
            () => {
                Hx { w: &mut *w, clo, top, frames: &self.frames, reinit: !self.reinit.is_empty() }
            };
        }
        macro_rules! ret {
            ($v:expr) => {{
                // (The slot holds nothing between returns: the F handler
                // takes the value at once.)
                std::mem::forget(self.ret_slot.replace($v));
                return RET | pc;
            }};
        }
        macro_rules! gen_arm {
            ($m:ident, $call:expr) => {{
                match $call {
                    Ctl::Next(released) => done!(released),
                    Ctl::Skip(released) => {
                        let at = pc;
                        pc += 2;
                        if released && self.ids.has_pending() {
                            pending_at!(pc, at);
                        }
                    }
                    Ctl::Jump(target) => {
                        if back!(target) {
                            return COLD | pc;
                        }
                        pc = target as usize;
                    }
                    Ctl::Return(value) => ret!(value),
                    Ctl::Cold => return COLD | pc,
                }
            }};
        }
        macro_rules! ops {
            ($a:ident, $b:ident, $c:ident, $imm:ident) => {
                #[allow(unused_variables)]
                let ($a, $b, $c, $imm) = (insn.a, insn.b, insn.c, insn.imm);
            };
        }
        macro_rules! iter_next {
            ($key:expr) => {{
                ops!(a, b, c, imm);
                if !w[rx!(a)].is_array() {
                    if w[rx!(a)].is_object() {
                        defer!();
                    }
                    slow!();
                }
                let next = if iter_next(&mut w[..], Reg::from(a), Reg::from(b), Reg::from(c), $key) { pc + 1 } else { imm as usize };
                if self.ids.has_pending() {
                    pending_at!(next, pc);
                }
                pc = next;
            }};
        }
        macro_rules! truth {
            ($not:expr) => {{
                ops!(a, b, c, imm);
                let v = &w[rx!(b)];
                let t = match v.as_bool() {
                    Some(b) => b,
                    None => match truth(v) {
                        Some(t) => t,
                        None => slow!(),
                    },
                };
                done!(put(&mut w[rx!(a)], Value::bool(t != $not)));
            }};
        }
        dispatch!(op, insn, {
            Op::Slow => slow!(),
            Op::VerifyArgs => {
                if !args_plain(clo, &w[..], top.argc as usize) {
                    slow!();
                }
                pc += 1;
            }
            Op::VerifyReturn => match &clo.code.ret_type {
                Some(ty)
                    if clo.code.is_generator
                        || !(ClassMemo::quick(clo.type_memo.last(), ty, &w[rx!(insn.a)])
                            || w[rx!(insn.a)].as_reference().is_some_and(|c| quick_ref(clo.type_memo.last(), ty, c))) =>
                {
                    slow!()
                }
                _ => pc += 1,
            },
            Op::Intrinsic => {
                ops!(a, b, c, imm);
                let (dst, args, argc) = (a, rx!(b), c);
                let Some(&Callee::Builtin(_)) = clo.sites[(imm & 0xffff) as usize].get() else { slow!() };
                // (The builtin's call: `run`'s arm, from its start.)
                macro_rules! call {
                    () => {
                        defer!()
                    };
                }
                let x = &w[args];
                let r = match imm >> 16 {
                    0 => match x.as_str() {
                        Some(s) => Value::int(s.len() as i64),
                        None => call!(),
                    },
                    1 => match x.as_array() {
                        Some(arr) => Value::int(arr.len() as i64),
                        None => call!(),
                    },
                    9 => {
                        let Some(arr) = w[rk!(args + 1)].as_array() else { call!() };
                        if let Some(k) = x.as_int() {
                            Value::bool(arr.contains_key(&Key::Int(k)))
                        } else if let Some(s) = x.as_str() {
                            Value::bool(arr.get_str(s).is_some())
                        } else {
                            call!()
                        }
                    }
                    _ if x.is_undef() || x.is_reference() => call!(),
                    2 => Value::bool(x.is_null()),
                    3 => Value::bool(x.is_bool()),
                    4 => Value::bool(x.is_int()),
                    5 => Value::bool(x.is_float()),
                    6 => Value::bool(x.is_str()),
                    7 => Value::bool(x.is_array()),
                    8 => Value::bool(x.as_object().is_some_and(|o| o.class.flags & p8_runtime::object::CLASS_RESOURCE == 0)),
                    _ => call!(),
                };
                let mut released = false;
                for i in 0..usize::from(argc) {
                    released |= clear(&mut w[rk!(args + i)]);
                }
                released |= put(&mut w[rx!(dst)], r);
                done!(released);
            }
            Op::SendVarMethod => {
                ops!(a, b, c, imm);
                let (obj, arg, var, method, n) = (a, b, c, (imm & 0xffff) as usize, (imm >> 16) as usize);
                let held;
                let o = match w[rx!(obj)].as_object() {
                    Some(o) => o,
                    None => {
                        held = ref_object(&w[rx!(obj)]);
                        match &held {
                            Some(o) => o,
                            None => slow!(),
                        }
                    }
                };
                let (class, scope) = (Rc::as_ptr(&o.class) as usize, top.scope.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize));
                let by_ref = clo.methods[method].borrow().as_ref().and_then(|s| match &s.code {
                    MethodCode::User(code) if s.class == class && s.scope == scope => {
                        self.frames.site_code(code).map(|t| self.frames.next_code(t.as_ref()).code.param_by_ref(n))
                    }
                    _ => None,
                });
                let by_ref = match by_ref {
                    Some(by_ref) => by_ref,
                    None => match self.cached_takes_ref_out(&clo.methods[method], &o.class, n) {
                        Some(by_ref) => by_ref,
                        None => self.method_takes_ref_out(&o.class, clo.code.sites[method].sym, n),
                    },
                };
                if by_ref {
                    let cell = make_ref(&mut w[..], Reg::from(var));
                    done!(put(&mut w[rx!(arg)], Value::reference(cell)));
                } else {
                    let Some(v) = copied(&w[rx!(var)]) else { slow!() };
                    done!(put(&mut w[rx!(arg)], v));
                }
            }
            Op::Jump => {
                let imm = insn.imm;
                if back!(imm) {
                    slow!();
                }
                pc = imm as usize;
            }
            Op::Compare => {
                ops!(a, b, c, imm);
                compare!(insn::cmp(imm), a, b, &w[rx!(c)]);
            }
            Op::CompareC => {
                ops!(a, b, c, imm);
                compare!(insn::cmp(imm), a, b, cst!(imm >> 8));
            }
            Op::IncVar => {
                let r = &mut w[rx!(insn.a)];
                if let Some(i) = r.as_int_mut()
                    && *i != i64::MAX
                {
                    *i += 1;
                    pc += 1;
                } else if let Some(cell) = r.as_reference()
                    && step_ref(cell, i64::checked_add).is_some()
                {
                    pc += 1;
                } else {
                    slow!();
                }
            }
            Op::IncJumpLt => step_jump!(insn.a, &w[rx!(insn.b)], insn.imm, i64::checked_add, |n, b| n < b),
            Op::IncJumpLe => step_jump!(insn.a, &w[rx!(insn.b)], insn.imm, i64::checked_add, |n, b| n <= b),
            Op::DecJumpGt => step_jump!(insn.a, &w[rx!(insn.b)], insn.imm, i64::checked_sub, |n, b| n > b),
            Op::DecJumpGe => step_jump!(insn.a, &w[rx!(insn.b)], insn.imm, i64::checked_sub, |n, b| n >= b),
            Op::IncJumpLtC => step_jump!(insn.a, cst!(insn::wide(insn)), insn.imm, i64::checked_add, |n, b| n < b),
            Op::IncJumpLeC => step_jump!(insn.a, cst!(insn::wide(insn)), insn.imm, i64::checked_add, |n, b| n <= b),
            Op::DecJumpGtC => step_jump!(insn.a, cst!(insn::wide(insn)), insn.imm, i64::checked_sub, |n, b| n > b),
            Op::DecJumpGeC => step_jump!(insn.a, cst!(insn::wide(insn)), insn.imm, i64::checked_sub, |n, b| n >= b),
            Op::DecVar => {
                let r = &mut w[rx!(insn.a)];
                if let Some(i) = r.as_int_mut()
                    && *i != i64::MIN
                {
                    *i -= 1;
                    pc += 1;
                } else if let Some(cell) = r.as_reference()
                    && step_ref(cell, i64::checked_sub).is_some()
                {
                    pc += 1;
                } else {
                    slow!();
                }
            }
            Op::Binary => {
                ops!(a, b, c, imm);
                if !arith(&mut w[..], insn::binop(imm), Reg::from(a), Reg::from(b), reg_src!(c)) {
                    slow!();
                }
                done!(true);
            }
            Op::BinaryC => {
                ops!(a, b, c, imm);
                let done = if imm & insn::CONST_LEFT != 0 {
                    arith_left(&mut w[..], insn::binop(imm), Reg::from(a), cst!(imm >> 8), Reg::from(b))
                } else {
                    arith(&mut w[..], insn::binop(imm), Reg::from(a), Reg::from(b), cst_src!(imm >> 8))
                };
                if !done {
                    slow!();
                }
                done!(true);
            }
            Op::Add => arith!(BinOp::Add, insn.a, insn.b, reg_src!(insn.c), i64::checked_add),
            Op::Sub => arith!(BinOp::Sub, insn.a, insn.b, reg_src!(insn.c), i64::checked_sub),
            Op::Mul => arith!(BinOp::Mul, insn.a, insn.b, reg_src!(insn.c), i64::checked_mul),
            Op::AddC => arith!(BinOp::Add, insn.a, insn.b, cst_src!(insn.imm), i64::checked_add),
            Op::SubC => arith!(BinOp::Sub, insn.a, insn.b, cst_src!(insn.imm), i64::checked_sub),
            Op::MulC => arith!(BinOp::Mul, insn.a, insn.b, cst_src!(insn.imm), i64::checked_mul),
            Op::ConcatN => {
                ops!(a, b, c, imm);
                if !concat_n(&mut w[..], Reg::from(a), Reg::from(b), c) {
                    slow!();
                }
                done!(true);
            }
            Op::AssignDimOp => {
                ops!(a, b, c, imm);
                if !dim_op(&mut w[..], insn::binop(imm >> 8), Reg::from(a), Reg::from(b), Reg::from(c), Reg::from(imm as u8)) {
                    slow!();
                }
                done!(true);
            }
            Op::IterNext => iter_next!(false),
            Op::IterNextKey => iter_next!(true),
            Op::AssignOp => {
                ops!(a, b, c, imm);
                assign_op!(insn::binop(imm), a, reg_src!(b));
            }
            Op::AssignOpC => {
                ops!(a, b, c, imm);
                assign_op!(insn::binop(imm), a, cst_src!(imm >> 8));
            }
            Op::IncDec => {
                ops!(a, b, c, imm);
                let inc = imm & insn::INC != 0;
                let (i, n) = if let Some(slot) = w[rx!(a)].as_int_mut() {
                    let i = *slot;
                    let Some(n) = (if inc { i.checked_add(1) } else { i.checked_sub(1) }) else { slow!() };
                    *slot = n;
                    (i, n)
                } else if let Some(cell) = w[rx!(a)].as_reference()
                    && let Some(n) = step_ref(cell, if inc { i64::checked_add } else { i64::checked_sub })
                {
                    (if inc { n - 1 } else { n + 1 }, n)
                } else {
                    slow!()
                };
                done!(put(&mut w[rx!(b)], Value::int(if imm & insn::POST != 0 { i } else { n })));
            }
            Op::LoadNull => {
                done!(put(&mut w[rx!(insn.a)], Value::NULL));
            }
            Op::Not => truth!(true),
            Op::ToBool => truth!(false),
            Op::IssetVar => {
                ops!(a, b, c, imm);
                let set = is_set(&w[rx!(b)]);
                done!(put(&mut w[rx!(a)], Value::bool(set)));
            }
            Op::JumpIfSet => {
                ops!(a, b, c, imm);
                let s = &w[rx!(a)];
                let v = if s.is_null_or_undef() {
                    None
                } else if let Some(cell) = s.as_reference() {
                    read_ref(cell).filter(|v| !v.is_null())
                } else {
                    Some(s.clone())
                };
                match v {
                    Some(v) => {
                        if put(&mut w[rx!(b)], v) && self.ids.has_pending() {
                            pending_at!(imm as usize, pc);
                        }
                        pc = imm as usize;
                    }
                    None => pc += 1,
                }
            }
            Op::NewArray => {
                ops!(a, b, c, imm);
                let arr = Value::array(PhpArray::list_with_capacity(imm as usize));
                done!(put(&mut w[rx!(a)], arr));
            }
            Op::ArrayPush => {
                ops!(a, b, c, imm);
                array_push!(a, reg_src!(b), imm & insn::TAKE != 0);
            }
            Op::ArrayPushC => {
                ops!(a, b, c, imm);
                array_push!(a, cst_src!(imm), false);
            }
            Op::ArraySet => {
                ops!(a, b, c, imm);
                array_set!(a, reg_src!(b), reg_src!(c), imm & insn::TAKE != 0);
            }
            Op::ArraySetC => {
                ops!(a, b, c, imm);
                let key = if imm & insn::KEY_CONST != 0 { cst_src!(imm & 0x3fff) } else { reg_src!(b) };
                let val = if imm & insn::VAL_CONST != 0 { cst_src!(imm >> 14 & 0x3fff) } else { reg_src!(c) };
                array_set!(a, key, val, imm & insn::TAKE != 0);
            }
            Op::FetchConst => {
                ops!(a, b, c, imm);
                let Some(Callee::Constant(v)) = clo.sites[imm as usize].get() else { slow!() };
                let v = v.clone();
                done!(put(&mut w[rx!(a)], v));
            }
            Op::IterInit => {
                ops!(a, b, c, imm);
                let Some(arr) = w[rx!(a)].as_array() else {
                    if w[rx!(a)].is_object() {
                        defer!();
                    }
                    slow!()
                };
                if arr.is_empty() {
                    pc = imm as usize;
                } else {
                    let v = w[rx!(a)].clone();
                    let released = put(&mut w[rx!(b)], v) | put(&mut w[rk!(rx!(b) + 1)], Value::int(0));
                    done!(released);
                }
            }
            Op::SendVar => {
                ops!(a, b, c, imm);
                let n = (imm >> 16) as usize;
                let by_ref = match clo.sites[(imm & 0xffff) as usize].get() {
                    Some(&Callee::User(f)) => self.codes[f as usize].param_by_ref(n),
                    Some(&Callee::Builtin(i)) => self.isolate.registry.by_ref(i, n),
                    Some(Callee::Constant(_)) => false,
                    None => defer!(),
                };
                if by_ref {
                    let cell = make_ref(&mut w[..], Reg::from(b));
                    done!(put(&mut w[rx!(a)], Value::reference(cell)));
                } else {
                    let Some(v) = copied(&w[rx!(b)]) else { defer!() };
                    done!(put(&mut w[rx!(a)], v));
                }
            }
            // The frame-changing ones: their F handlers.
            Op::Call => return F_CALL | pc,
            Op::MethodCall => return F_METHOD | pc,
            Op::New => return F_NEW | pc,
            Op::Construct => return F_CONSTRUCT | pc,
            // A resolved builtin: its F handler (the frameless call); else
            // the first Move, as `run`'s arm.
            Op::CallRegs => {
                ops!(a, b, c, imm);
                let start = rx!(a);
                if let Some(&Callee::Builtin(_)) = clo.sites[(imm & 0xffff) as usize].get() {
                    return F_CALL_REGS | pc;
                }
                if imm & insn::ARG_CONST != 0 {
                    let v = cst!(start).clone();
                    done!(put(&mut w[rk!((imm >> 16) as usize)], v));
                } else {
                    let Some(v) = copied(&w[start]) else { slow!() };
                    done!(put(&mut w[rk!((imm >> 16) as usize)], v));
                    if self.ids.has_pending() {
                        pending!();
                    }
                }
            }
            Op::ReturnNull => {
                if self.frames.len() == self.frames.floor + 1 && (self.frames.floor == 0 || clo.code.is_generator) {
                    defer!();
                }
                ret!(Value::NULL)
            }
            // No handler here yet: `run`'s arm.
            Op::StaticCall
            | Op::StaticCallVars
            | Op::MakeClosure
            | Op::CollectVariadic
            | Op::ArrayUnpack
            | Op::ConstructSpread
            | Op::SendVarStatic
            | Op::BindGlobal
            | Op::AssignDimPath
            | Op::AssignPropDim
            | Op::UnsetDim
            | Op::DimIncDec
            | Op::Cast
            | Op::PropIncDec
            | Op::MethodArgByRef
            | Op::PropRef
            | Op::StaticArgByRef
            | Op::ArgDefault
            | Op::FuncArgByRef
            | Op::StaticVar
            | Op::IterInitRef
            | Op::IterNextRef
            | Op::FetchList
            | Op::CallValue
            | Op::AssignPropOp
            | Op::BindRef
            | Op::UnsetVar
            | Op::ToObject
            | Op::CallableArgByRef
            | Op::VarGet
            | Op::AutoGlobal
            | Op::ArrayPushRef
            | Op::AssignStaticPropOp
            | Op::DeclareClass
            | Op::DeclareFunction
            | Op::Include
            | Op::FromCallable
            | Op::Yield
            | Op::YieldFrom
            | Op::RefDim
            | Op::DimSetRef
            | Op::StaticRef
            | Op::FetchPropDyn
            | Op::AssignPropDyn
            | Op::IssetProp
            | Op::IssetDimPath
            | Op::CallSpread
            | Op::Silence
            | Op::Unsilence
            | Op::Neg
            | Op::Capture
            | Op::ClassName => defer!(),
        });
        pc
    }
}

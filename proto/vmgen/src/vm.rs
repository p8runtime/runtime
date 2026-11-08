//! The machine: instructions carrying their handler, frames over one
//! register file, and the loop that calls handlers until the entry frame
//! returns. Frame changes (calls, returns) are events the handlers raise
//! and the loop performs, so the loop owns the register file and every
//! straight-line handler gets the frame's window as a plain argument.

use crate::link::Linked;
use p8_runtime::object::{Ids, Object};
use p8_runtime::value::Value;
use std::cell::Cell;
use std::rc::Rc;

/// A frame's registers as the handlers see them: a fixed window, indexed by
/// `u8` operands with no bounds check (the translator refuses a function
/// with more registers).
pub type Window = [Value; 256];
pub const WINDOW: usize = 256;

/// A handler: the next pc, or `REFRAME | resume pc` when it raised an
/// event (`State::event`) the loop must act on before going on.
pub type Handler = for<'p> fn(&mut State<'p>, &mut Window, &'p Func<'p>, &Insn<'p>, u32) -> u32;

pub const REFRAME: u32 = 1 << 31;

/// One instruction: its exact handler and its operands. Registers are
/// window indexes; a literal operand is a pointer into the program's
/// constant pool (`k0`, `k1`), read in place as PHP reads a CONST operand.
#[derive(Clone, Copy)]
pub struct Insn<'p> {
    pub h: Handler,
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    /// A jump target, a cache index, a function index.
    pub imm: u32,
    /// Index into `TABLE`, for `--dump`.
    pub id: u16,
    pub k0: &'p Value,
    pub k1: &'p Value,
}

/// An inline cache (PHP's run-time cache slot): for a property access the
/// class seen and the slot it answered with; for a method call the class
/// seen and the function it resolved to.
pub struct Cache {
    pub class: Cell<usize>,
    pub slot: Cell<u32>,
    pub flags: Cell<u8>,
    /// The property name's constant (a pool index relative to the
    /// function's constants) or the method's call site.
    pub name: u16,
}

/// The cached property is typed: the write checks the value (cold).
pub const TYPED: u8 = 1;

impl Cache {
    pub fn new(name: u16) -> Cache {
        Cache { class: Cell::new(usize::MAX), slot: Cell::new(0), flags: Cell::new(0), name }
    }
}

/// A translated function.
pub struct Func<'p> {
    pub name: String,
    pub src: &'p p8_bytecode::Function,
    pub code: Vec<Insn<'p>>,
    /// This function's constants (thawed once, in the pool).
    pub consts: &'p [Value],
    pub caches: Vec<Cache>,
    pub ncv: u16,
    pub nregs: u16,
    pub nparams: u16,
    pub required: u16,
    /// Why the function could not be translated (its code is then one
    /// `P8_REFUSED` that faults with this).
    pub refused: Option<String>,
}

pub struct FrameRec {
    pub func: u32,
    pub base: u32,
    /// Where the frame goes on after the call it made.
    pub pc: u32,
    /// The caller's result register (absolute), or NO_RET.
    pub ret: u32,
    pub this: Option<Rc<Object>>,
}

pub const NO_RET: u32 = u32::MAX;

pub enum Event {
    None,
    Call { func: u32, args: u8, argc: u8, dst: u8, used: bool, this: Option<Rc<Object>> },
    Return(Value),
    Fault(String),
}

pub struct State<'p> {
    pub prog: &'p Linked<'p>,
    pub frames: Vec<FrameRec>,
    /// The running frame's `$this`.
    pub this: Option<Rc<Object>>,
    pub event: Event,
    pub ids: Rc<Ids>,
    pub out: Vec<u8>,
}

impl<'p> State<'p> {
    pub fn new(prog: &'p Linked<'p>) -> State<'p> {
        State { prog, frames: Vec::new(), this: None, event: Event::None, ids: Rc::new(Ids::default()), out: Vec::new() }
    }
}

/// Make a register undefined, releasing what it held.
#[inline(always)]
fn clear(r: &mut Value) {
    if !r.is_plain() {
        let _ = std::mem::replace(r, Value::UNDEF).release();
    } else {
        std::mem::forget(std::mem::replace(r, Value::UNDEF));
    }
}

/// Run `func` with `args` until it returns; its result.
pub fn run<'p>(st: &mut State<'p>, regs: &mut Vec<Value>, func: u32, args: Vec<Value>) -> Result<Value, String> {
    let entry = &st.prog.funcs[func as usize];
    if let Some(why) = &entry.refused {
        return Err(format!("refused: {why}"));
    }
    regs.clear();
    regs.resize(usize::from(entry.nregs).max(WINDOW) + WINDOW, Value::UNDEF);
    for (i, a) in args.into_iter().enumerate() {
        regs[i] = a;
    }
    st.frames.push(FrameRec { func, base: 0, pc: 0, ret: NO_RET, this: None });
    'frame: loop {
        let fr = st.frames.last().expect("a frame");
        let f: &'p Func<'p> = &st.prog.funcs[fr.func as usize];
        let base = fr.base as usize;
        let mut pc = fr.pc as usize;
        let w: &mut Window = (&mut regs[base..base + WINDOW]).try_into().expect("a window");
        let code = &f.code[..];
        loop {
            let insn = &code[pc];
            let next = (insn.h)(st, w, f, insn, pc as u32);
            if next & REFRAME == 0 {
                pc = next as usize;
                continue;
            }
            st.frames.last_mut().expect("a frame").pc = next & !REFRAME;
            match std::mem::replace(&mut st.event, Event::None) {
                Event::Call { func, args, argc, dst, used, this } => {
                    let callee = &st.prog.funcs[func as usize];
                    let nbase = base + usize::from(args);
                    let need = nbase + usize::from(callee.nregs).max(WINDOW) + WINDOW;
                    if regs.len() < need {
                        regs.resize(need, Value::UNDEF);
                    }
                    // The variables not passed are undefined (PHP initialises its CVs).
                    for r in &mut regs[nbase + usize::from(argc)..nbase + usize::from(callee.ncv)] {
                        clear(r);
                    }
                    let ret = if used { (base + usize::from(dst)) as u32 } else { NO_RET };
                    let this = std::mem::replace(&mut st.this, this);
                    st.frames.last_mut().expect("a frame").this = this;
                    st.frames.push(FrameRec { func, base: nbase as u32, pc: 0, ret, this: None });
                    continue 'frame;
                }
                Event::Return(v) => {
                    let done = st.frames.pop().expect("a frame");
                    // Locals die with the frame (its temporaries too: p8's
                    // compiler frees each where it dies, but a frame that
                    // returned through a fault path may not have).
                    let n = usize::from(f.nregs);
                    for r in &mut regs[base..base + n] {
                        clear(r);
                    }
                    st.this = None;
                    let Some(caller) = st.frames.last_mut() else {
                        return Ok(v);
                    };
                    st.this = caller.this.take();
                    if done.ret != NO_RET {
                        let slot = &mut regs[done.ret as usize];
                        clear(slot);
                        *slot = v;
                    }
                    continue 'frame;
                }
                Event::Fault(msg) => {
                    let line = f.src.lines.get(pc).copied().unwrap_or(0);
                    return Err(format!("{msg} ({} at pc {pc}, line {line})", f.name));
                }
                Event::None => return Err(format!("handler returned REFRAME with no event ({} at pc {pc})", f.name)),
            }
        }
    }
}

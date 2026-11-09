#![forbid(unsafe_code)]
//! #397's generator: V4b's kind-aware code (`proto/aot/src/b.rs`) for
//! `add`, `call` and `prop_read`, emitted as Cranelift IR and compiled by
//! Cranelift into a relocatable object file, the shape a production AOT
//! would have (no JIT, nothing executed here).
//!
//!   clif-gen <outdir> [triple]   writes <outdir>/clif.o, and per function
//!                                its CLIF and machine code (.clif, .s)
//!
//! What it emits is what V4b's Rust says, operation for operation: the
//! variables are Cranelift variables (SSA values, in registers), each a
//! (tag, payload) pair; every CV read keeps the undefined and reference
//! tests, every write the plain test (the release and the reference write
//! out of line); ints are a tag test and an overflow check inline, with
//! V4b's generic helper as the cold call; a user call is direct, with the
//! depth guard; a property read is the inline cache's class test and the
//! slot's load. Everything V4b leaves to `p8-runtime` out of line is a call
//! of a C-ABI helper (`ffi.rs`); a helper is only ever on a cold path in
//! these three programs, as in V4b.
//!
//! The one place this code knows more than V4b's Rust can say: the
//! property inline cache keeps the slot's byte offset in the object (as
//! PHP's run-time cache does), so the hit is one load at that offset where
//! V4b goes through `Object::slots()` (the inline-or-heap match and the
//! bounds check) and `Cell::peek` (take, copy, put back). Recorded in the
//! PR with the counts.

mod abi;

use abi::*;
use cranelift_codegen::Context;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::{I8, I32, I64};
use cranelift_codegen::ir::{self, AbiParam, Block, InstBuilder, MemFlagsData as MemFlags, Signature, StackSlot, StackSlotData, StackSlotKind};
use cranelift_codegen::isa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::HashMap;
use std::mem::offset_of;
use std::path::Path;

/// Where `Object::class` (the `Rc<Class>`'s pointer to its allocation)
/// sits from the object value's payload (the `Rc<Object>`'s pointer to its
/// allocation, the `RcBox`: two counts, then the object). The harness
/// checks the 16 against the live layout before it runs anything.
const CLASS_OFF: i32 = 16 + offset_of!(p8_runtime::object::Object, class) as i32;
/// `St::depth`, a `u32`.
const DEPTH_OFF: i32 = offset_of!(aot::rt::St, depth) as i32;
const MAX_DEPTH: i64 = aot::rt::MAX_DEPTH as i64;

/// The helpers, as (name, params, returns): `P` a pointer or word, `W` a
/// u32, `B` a u8 result (0 ok / false, else fault / true), `V` a value
/// result (two words).
const HELPERS: &[(&str, &str, &str)] = &[
    ("p8c_undef", "PPW", "V"),
    ("p8c_deref", "P", "V"),
    ("p8c_clone_heap", "P", "V"),
    ("p8c_assign_slow", "PPP", ""),
    ("p8c_drop", "PP", ""),
    ("p8c_less", "PPPWPW", "B"),
    ("p8c_inc", "PPPW", "B"),
    ("p8c_add1", "PPPW", "V"),
    ("p8c_too_deep", "", "V"),
    ("p8c_tail", "PPPPW", "B"),
    ("p8c_new", "PPW", "V"),
    ("p8c_not_object", "P", "V"),
    ("p8c_fill_prop", "PPW", "B"),
    ("p8c_unset_prop", "", "V"),
];

fn sig_of(m: &ObjectModule, params: &str, returns: &str) -> Signature {
    let mut s = m.make_signature();
    for c in params.chars() {
        s.params.push(AbiParam::new(if c == 'W' { I32 } else { I64 }));
    }
    for c in returns.chars() {
        match c {
            'V' => {
                s.returns.push(AbiParam::new(I64));
                s.returns.push(AbiParam::new(I64));
            }
            _ => s.returns.push(AbiParam::new(I8)),
        }
    }
    s
}

/// A compiled variable: its tag and payload.
#[derive(Clone, Copy)]
struct Cv {
    tag: Variable,
    pay: Variable,
}

/// One function being built.
struct G<'a> {
    b: FunctionBuilder<'a>,
    cx: ir::Value,
    st: ir::Value,
    fr: HashMap<&'static str, ir::FuncRef>,
    gv: HashMap<&'static str, ir::GlobalValue>,
    /// Return the fault (T_ERR).
    err: Block,
}

impl G<'_> {
    fn k(&mut self, v: i64) -> ir::Value {
        self.b.ins().iconst(I64, v)
    }

    fn k32(&mut self, v: u32) -> ir::Value {
        self.b.ins().iconst(I32, i64::from(v))
    }

    fn call(&mut self, name: &str, args: &[ir::Value]) -> Vec<ir::Value> {
        let f = self.fr[name];
        let i = self.b.ins().call(f, args);
        self.b.inst_results(i).to_vec()
    }

    fn block(&mut self) -> Block {
        self.b.create_block()
    }

    fn cold(&mut self) -> Block {
        let bl = self.b.create_block();
        self.b.set_cold_block(bl);
        bl
    }

    fn to(&mut self, bl: Block) {
        self.b.switch_to_block(bl);
    }

    /// `x cc imm` as a branch condition.
    fn test(&mut self, cc: IntCC, x: ir::Value, imm: i64) -> ir::Value {
        self.b.ins().icmp_imm_s(cc, x, imm)
    }

    fn brif(&mut self, c: ir::Value, yes: Block, no: Block) {
        self.b.ins().brif(c, yes, &[], no, &[]);
    }

    fn jump(&mut self, bl: Block) {
        self.b.ins().jump(bl, &[]);
    }

    fn var(&mut self) -> Variable {
        self.b.declare_var(I64)
    }

    /// A variable not yet assigned (Undef).
    fn cv_undef(&mut self) -> Cv {
        let cv = Cv { tag: self.var(), pay: self.var() };
        let (t, p) = (self.k(T_UNDEF), self.k(0));
        self.def(cv, t, p);
        cv
    }

    fn def(&mut self, cv: Cv, t: ir::Value, p: ir::Value) {
        self.b.def_var(cv.tag, t);
        self.b.def_var(cv.pay, p);
    }

    fn get(&mut self, cv: Cv) -> (ir::Value, ir::Value) {
        (self.b.use_var(cv.tag), self.b.use_var(cv.pay))
    }

    /// A value in a 16-byte stack slot, for a helper that takes `&Value`.
    fn spill(&mut self, t: ir::Value, p: ir::Value) -> (StackSlot, ir::Value) {
        let ss = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 3));
        self.b.ins().stack_store(I64, t, ss, 0);
        self.b.ins().stack_store(I64, p, ss, 8);
        let a = self.b.ins().stack_addr(I64, ss, 0);
        (ss, a)
    }

    fn spill_cv(&mut self, cv: Cv) -> (StackSlot, ir::Value) {
        let (t, p) = self.get(cv);
        self.spill(t, p)
    }

    /// The variable back from its slot, after a helper that took `&mut Value`.
    fn reload(&mut self, ss: StackSlot, cv: Cv) {
        let t = self.b.ins().stack_load(I64, I64, ss, 0);
        let p = self.b.ins().stack_load(I64, I64, ss, 8);
        self.def(cv, t, p);
    }

    /// `rd_cv`: a copy of a CV, undefined and reference tested (cold), a
    /// value holding memory copied by the helper (cold).
    fn rd_cv(&mut self, cv: Cv, var: u32) -> (ir::Value, ir::Value) {
        let r = Cv { tag: self.var(), pay: self.var() };
        let (t, p) = self.get(cv);
        let (undef, c2, isref, c3, heap, done) = (self.cold(), self.block(), self.cold(), self.block(), self.cold(), self.block());
        let c = self.test(IntCC::Equal, t, T_UNDEF);
        self.brif(c, undef, c2);

        self.to(undef);
        let v = self.k32(var);
        let (cx, st) = (self.cx, self.st);
        let res = self.call("p8c_undef", &[cx, st, v]);
        self.def(r, res[0], res[1]);
        self.jump(done);

        self.to(c2);
        let c = self.test(IntCC::Equal, t, T_REF);
        self.brif(c, isref, c3);

        self.to(isref);
        let (_, a) = self.spill(t, p);
        let res = self.call("p8c_deref", &[a]);
        self.def(r, res[0], res[1]);
        self.jump(done);

        self.to(c3);
        self.def(r, t, p);
        let c = self.test(IntCC::UnsignedGreaterThanOrEqual, t, FIRST_HEAP);
        self.brif(c, heap, done);

        self.to(heap);
        let (_, a) = self.spill(t, p);
        let res = self.call("p8c_clone_heap", &[a]);
        self.def(r, res[0], res[1]);
        self.jump(done);

        self.to(done);
        self.get(r)
    }

    /// `wr_cv`: a plain old value overwritten; a reference written through
    /// and a value holding memory released by the helper (cold).
    fn wr_cv(&mut self, dst: Cv, nt: ir::Value, np: ir::Value) {
        let (plain, slow, cont) = (self.block(), self.cold(), self.block());
        let ot = self.b.use_var(dst.tag);
        let c = self.test(IntCC::UnsignedLessThan, ot, FIRST_HEAP);
        self.brif(c, plain, slow);

        self.to(plain);
        self.def(dst, nt, np);
        self.jump(cont);

        self.to(slow);
        let (ss, a) = self.spill_cv(dst);
        self.call("p8c_assign_slow", &[a, nt, np]);
        self.reload(ss, dst);
        self.jump(cont);

        self.to(cont);
    }

    /// `lt_cv` as a branch: two ints inline, the rest the helper (cold).
    fn lt_cv(&mut self, a: Cv, va: u32, b: Cv, vb: u32, yes: Block, no: Block) {
        let (c1, fast, slow) = (self.block(), self.block(), self.cold());
        let (at, ap) = self.get(a);
        let (bt, bp) = self.get(b);
        let c = self.test(IntCC::Equal, at, T_INT);
        self.brif(c, c1, slow);

        self.to(c1);
        let c = self.test(IntCC::Equal, bt, T_INT);
        self.brif(c, fast, slow);

        self.to(fast);
        let c = self.b.ins().icmp(IntCC::SignedLessThan, ap, bp);
        self.brif(c, yes, no);

        self.to(slow);
        let (_, aa) = self.spill(at, ap);
        let (_, ba) = self.spill(bt, bp);
        let (va, vb) = (self.k32(va), self.k32(vb));
        let (cx, st) = (self.cx, self.st);
        let r = self.call("p8c_less", &[cx, st, aa, va, ba, vb]);
        self.brif(r[0], yes, no);
    }

    /// `inc_cv`: an int that does not overflow in place, the rest the
    /// helper (cold).
    fn inc_cv(&mut self, cv: Cv, var: u32) {
        let (fast, ok, slow, cont) = (self.block(), self.block(), self.cold(), self.block());
        let t = self.b.use_var(cv.tag);
        let c = self.test(IntCC::Equal, t, T_INT);
        self.brif(c, fast, slow);

        self.to(fast);
        let p = self.b.use_var(cv.pay);
        let one = self.k(1);
        let (r, of) = self.b.ins().sadd_overflow(p, one);
        self.brif(of, slow, ok);

        self.to(ok);
        self.b.def_var(cv.pay, r);
        self.jump(cont);

        self.to(slow);
        let (ss, a) = self.spill_cv(cv);
        let v = self.k32(var);
        let (cx, st) = (self.cx, self.st);
        let s = self.call("p8c_inc", &[cx, st, a, v]);
        self.reload(ss, cv);
        let err = self.err;
        self.brif(s[0], err, cont);

        self.to(cont);
    }

    /// `for ($i = 0; $i < $n; $i++) body`, as the bytecode has it.
    fn for_loop(&mut self, i: Cv, n: Cv, body: impl FnOnce(&mut Self)) {
        let (z, zp) = (self.k(T_INT), self.k(0));
        self.wr_cv(i, z, zp);
        let (header, b, exit) = (self.block(), self.block(), self.block());
        self.jump(header);
        self.to(header);
        self.lt_cv(i, var::I7, n, var::N7, b, exit);
        self.to(b);
        body(self);
        self.inc_cv(i, var::I7);
        self.jump(header);
        self.to(exit);
    }

    /// A result's fault test: on T_ERR to the error exit.
    fn check(&mut self, t: ir::Value) {
        let cont = self.block();
        let c = self.test(IntCC::Equal, t, T_ERR);
        let err = self.err;
        self.brif(c, err, cont);
        self.to(cont);
    }

    /// The end of `t`: the check variant's echo (the helper tests
    /// `cx.check`), the locals released, null returned.
    fn epilogue(&mut self, locals: &[Cv], i: Cv, x: Cv, vx: u32) {
        let (_, ai) = self.spill_cv(i);
        let (_, ax) = self.spill_cv(x);
        let v = self.k32(vx);
        let (cx, st) = (self.cx, self.st);
        let s = self.call("p8c_tail", &[cx, st, ai, ax, v]);
        let ok = self.block();
        let err = self.err;
        self.brif(s[0], err, ok);
        self.to(ok);
        for &cv in locals {
            let (d, c) = (self.cold(), self.block());
            let (t, p) = self.get(cv);
            let h = self.test(IntCC::UnsignedGreaterThanOrEqual, t, FIRST_HEAP);
            self.brif(h, d, c);
            self.to(d);
            self.call("p8c_drop", &[t, p]);
            self.jump(c);
            self.to(c);
        }
        let (t, p) = (self.k(T_NULL), self.k(0));
        self.b.ins().return_(&[t, p]);
    }
}

// ---- the functions, per program ---------------------------------------------------

/// `function f($a) { return $a; }`: `ret_cv`.
fn f_body(g: &mut G, a: Cv) {
    let (at, ap) = g.get(a);
    let (u, c, r, ret) = (g.cold(), g.block(), g.cold(), g.block());
    let t = g.test(IntCC::Equal, at, T_UNDEF);
    g.brif(t, u, c);

    g.to(u);
    let v = g.k32(var::FA);
    let (cx, st) = (g.cx, g.st);
    let res = g.call("p8c_undef", &[cx, st, v]);
    g.b.ins().return_(&res);

    g.to(c);
    let t = g.test(IntCC::Equal, at, T_REF);
    g.brif(t, r, ret);

    g.to(r);
    let (_, addr) = g.spill(at, ap);
    let res = g.call("p8c_deref", &[addr]);
    g.call("p8c_drop", &[at, ap]);
    g.b.ins().return_(&res);

    g.to(ret);
    g.b.ins().return_(&[at, ap]);
}

/// `$x = $i + 1`.
fn t_add(g: &mut G, n: Cv) {
    let (i, x) = (g.cv_undef(), g.cv_undef());
    g.for_loop(i, n, |g| {
        let r = Cv { tag: g.var(), pay: g.var() };
        let (fast, ok, slow, cont) = (g.block(), g.block(), g.cold(), g.block());
        let t = g.b.use_var(i.tag);
        let c = g.test(IntCC::Equal, t, T_INT);
        g.brif(c, fast, slow);

        g.to(fast);
        let p = g.b.use_var(i.pay);
        let one = g.k(1);
        let (s, of) = g.b.ins().sadd_overflow(p, one);
        g.brif(of, slow, ok);

        g.to(ok);
        let ti = g.k(T_INT);
        g.def(r, ti, s);
        g.jump(cont);

        g.to(slow);
        let (_, a) = g.spill_cv(i);
        let v = g.k32(var::I8);
        let (cx, st) = (g.cx, g.st);
        let res = g.call("p8c_add1", &[cx, st, a, v]);
        g.def(r, res[0], res[1]);
        let e = g.test(IntCC::Equal, res[0], T_ERR);
        let err = g.err;
        g.brif(e, err, cont);

        g.to(cont);
        let (rt, rp) = g.get(r);
        g.wr_cv(x, rt, rp);
    });
    g.epilogue(&[n, i, x], i, x, var::X10);
}

/// `$x = f($i)`: the argument copied, the depth guard, a direct call.
fn t_call(g: &mut G, n: Cv) {
    let (i, x) = (g.cv_undef(), g.cv_undef());
    g.for_loop(i, n, |g| {
        let (at, ap) = g.rd_cv(i, var::I8);
        let st = g.st;
        let d = g.b.ins().load(I32, MemFlags::trusted(), st, DEPTH_OFF);
        let (deep, ok) = (g.cold(), g.block());
        let c = g.test(IntCC::UnsignedGreaterThanOrEqual, d, MAX_DEPTH);
        g.brif(c, deep, ok);

        g.to(deep);
        g.call("p8c_too_deep", &[]);
        let err = g.err;
        g.jump(err);

        g.to(ok);
        let d1 = g.b.ins().iadd_imm_s(d, 1);
        g.b.ins().store(MemFlags::trusted(), d1, st, DEPTH_OFF);
        let cx = g.cx;
        let r = g.call("clif_f", &[cx, st, at, ap]);
        let d = g.b.ins().load(I32, MemFlags::trusted(), st, DEPTH_OFF);
        let d1 = g.b.ins().iadd_imm_s(d, -1);
        g.b.ins().store(MemFlags::trusted(), d1, st, DEPTH_OFF);
        g.check(r[0]);
        g.wr_cv(x, r[0], r[1]);
    });
    g.epilogue(&[n, i, x], i, x, var::X10);
}

/// `$o = new P; ... $x = $o->p`: the object test, the inline cache's class
/// test, the slot's load, unset faulting, a reference read through.
fn t_prop_read(g: &mut G, n: Cv) {
    let (o, i, x) = (g.cv_undef(), g.cv_undef(), g.cv_undef());
    let (cx, st) = (g.cx, g.st);
    let c0 = g.k32(0);
    let res = g.call("p8c_new", &[cx, st, c0]);
    g.wr_cv(o, res[0], res[1]);
    g.for_loop(i, n, |g| {
        let (isobj, notobj) = (g.block(), g.cold());
        let ot = g.b.use_var(o.tag);
        let c = g.test(IntCC::Equal, ot, T_OBJECT);
        g.brif(c, isobj, notobj);

        g.to(notobj);
        let (_, a) = g.spill_cv(o);
        g.call("p8c_not_object", &[a]);
        let err = g.err;
        g.jump(err);

        g.to(isobj);
        let op = g.b.use_var(o.pay);
        let gv = g.gv["ic_p"];
        let ic = g.b.ins().symbol_value(I64, gv);
        let cw = g.b.ins().load(I64, MemFlags::trusted(), ic, 0);
        let ow = g.b.ins().load(I64, MemFlags::trusted(), op, CLASS_OFF);
        let (hit, miss) = (g.block(), g.cold());
        let c = g.b.ins().icmp(IntCC::Equal, cw, ow);
        g.brif(c, hit, miss);

        g.to(miss);
        let (_, a) = g.spill_cv(o);
        let name = g.k32(PROP_P);
        let s = g.call("p8c_fill_prop", &[a, ic, name]);
        g.brif(s[0], err, hit);

        g.to(hit);
        let off = g.b.ins().load(I64, MemFlags::trusted(), ic, 8);
        let sa = g.b.ins().iadd(op, off);
        let stg = g.b.ins().load(I64, MemFlags::trusted(), sa, 0);
        let spy = g.b.ins().load(I64, MemFlags::trusted(), sa, 8);
        let r = Cv { tag: g.var(), pay: g.var() };
        let (unset, c1, isref, c2, heap, done) = (g.cold(), g.block(), g.cold(), g.block(), g.cold(), g.block());
        let c = g.test(IntCC::Equal, stg, T_UNDEF);
        g.brif(c, unset, c1);

        g.to(unset);
        g.call("p8c_unset_prop", &[]);
        g.jump(err);

        g.to(c1);
        let c = g.test(IntCC::Equal, stg, T_REF);
        g.brif(c, isref, c2);

        g.to(isref);
        let res = g.call("p8c_deref", &[sa]);
        g.def(r, res[0], res[1]);
        g.jump(done);

        g.to(c2);
        g.def(r, stg, spy);
        let c = g.test(IntCC::UnsignedGreaterThanOrEqual, stg, FIRST_HEAP);
        g.brif(c, heap, done);

        g.to(heap);
        let res = g.call("p8c_clone_heap", &[sa]);
        g.def(r, res[0], res[1]);
        g.jump(done);

        g.to(done);
        let (rt, rp) = g.get(r);
        g.wr_cv(x, rt, rp);
    });
    g.epilogue(&[n, o, i, x], i, x, var::X10);
}

// ---- the module ---------------------------------------------------------------

struct Gen {
    m: ObjectModule,
    fctx: FunctionBuilderContext,
    helpers: Vec<(&'static str, FuncId)>,
    data: Vec<(&'static str, DataId)>,
    out: std::path::PathBuf,
}

impl Gen {
    fn define(&mut self, name: &'static str, id: FuncId, body: fn(&mut G, Cv)) {
        let mut ctx: Context = self.m.make_context();
        ctx.func.signature = sig_of(&self.m, "PPPP", "V");
        ctx.want_disasm = true;
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut self.fctx);
            let mut fr = HashMap::new();
            for &(h, hid) in &self.helpers {
                fr.insert(h, self.m.declare_func_in_func(hid, b.func));
            }
            let mut gv = HashMap::new();
            for &(d, did) in &self.data {
                gv.insert(d, self.m.declare_data_in_func(did, b.func));
            }
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let ps = b.block_params(entry).to_vec();
            let err = b.create_block();
            b.set_cold_block(err);
            let mut g = G { b, cx: ps[0], st: ps[1], fr, gv, err };
            let a = Cv { tag: g.var(), pay: g.var() };
            g.def(a, ps[2], ps[3]);
            body(&mut g, a);
            g.to(err);
            let (t, p) = (g.k(T_ERR), g.k(0));
            g.b.ins().return_(&[t, p]);
            g.b.seal_all_blocks();
            g.b.finalize(self.m.target_config());
        }
        let clif = ctx.func.display().to_string();
        if let Err(e) = self.m.define_function(id, &mut ctx) {
            panic!("{name}: {e:?}\n{clif}");
        }
        let cc = ctx.compiled_code().expect("compiled");
        let asm = cc.vcode.clone().unwrap_or_default();
        std::fs::write(self.out.join(format!("{name}.clif")), clif).expect("write clif");
        std::fs::write(self.out.join(format!("{name}.s")), asm).expect("write asm");
        println!("{name}: {} bytes of code", cc.code_buffer().len());
        self.m.clear_context(&mut ctx);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = Path::new(args.first().map_or("out", String::as_str)).to_path_buf();
    let triple = args.get(1).map_or("x86_64-unknown-linux-gnu", String::as_str);
    std::fs::create_dir_all(&out).expect("outdir");

    let mut fb = settings::builder();
    fb.set("opt_level", "speed").expect("opt_level");
    fb.set("is_pic", "true").expect("is_pic");
    let isa = isa::lookup_by_name(triple).expect("isa").finish(settings::Flags::new(fb)).expect("flags");
    println!("isa {triple}, opt_level=speed, is_pic; CLASS_OFF={CLASS_OFF} DEPTH_OFF={DEPTH_OFF}");
    let mut m = ObjectModule::new(ObjectBuilder::new(isa, "clif", default_libcall_names()).expect("object"));

    let helpers = HELPERS
        .iter()
        .map(|&(h, ps, rs)| {
            let s = sig_of(&m, ps, rs);
            (h, m.declare_function(h, Linkage::Import, &s).expect("helper"))
        })
        .collect::<Vec<_>>();
    let sig = sig_of(&m, "PPPP", "V");
    let f_id = m.declare_function("clif_f", Linkage::Local, &sig).expect("f");
    let ic_p = m.declare_data("clif_ic_p", Linkage::Local, true, false).expect("ic");
    let mut d = DataDescription::new();
    d.define_zeroinit(16);
    m.define_data(ic_p, &d).expect("ic data");

    let mut g = Gen { m, fctx: FunctionBuilderContext::new(), helpers, data: vec![("ic_p", ic_p)], out: out.clone() };
    // `t` calls `f` directly: in its imports too.
    g.helpers.push(("clif_f", f_id));
    g.define("clif_f", f_id, f_body);
    let bodies: [fn(&mut G, Cv); 3] = [t_add, t_call, t_prop_read];
    for (&(_, sym), body) in PROGRAMS.iter().zip(bodies) {
        let id = g.m.declare_function(sym, Linkage::Export, &sig).expect("t");
        g.define(sym, id, body);
    }
    let bytes = g.m.finish().emit().expect("emit");
    std::fs::write(out.join("clif.o"), &bytes).expect("write object");
    println!("wrote {} ({} bytes)", out.join("clif.o").display(), bytes.len());
}

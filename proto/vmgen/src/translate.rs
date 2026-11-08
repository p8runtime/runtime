//! p8's bytecode (`Instr`) into the prototype's instructions: each `Instr`
//! classified by its operands' kinds and given its exact handler from the
//! generated table. Anything outside the subset refuses the function, with
//! the reason; the function then runs as one `P8_REFUSED` that faults.

use crate::handlers::TABLE;
use crate::link::Pool;
use crate::vm::{Cache, Func, Insn};
use p8_bytecode::{ClassRef, Cmp, Function, Instr, Rare, Reg, THIS as THIS_REG, const_index};
use p8_runtime::ops::BinOp;
use p8_runtime::value::Value;
use std::collections::HashMap;

/// What the translator needs of the rest of the program.
pub struct World<'w> {
    /// Lowercase function name → function index.
    pub funcs: &'w HashMap<Vec<u8>, u32>,
    /// Lowercase class name → (class index, constructor function index).
    pub classes: &'w HashMap<Vec<u8>, (u32, Option<u32>)>,
    /// Per function index: (parameters, required, any by-reference /
    /// variadic / typed parameter, is a generator).
    pub shapes: &'w [Shape],
}

#[derive(Clone, Copy)]
pub struct Shape {
    pub nparams: u16,
    pub required: u16,
    pub plain_params: bool,
    pub generator: bool,
}

enum Opnd<'p> {
    Const(&'p Value),
    Tmp(u8),
    Cv(u8),
    This,
}

impl Opnd<'_> {
    fn kind(&self) -> &'static str {
        match self {
            Opnd::Const(_) => "CONST",
            Opnd::Tmp(_) => "TMP",
            Opnd::Cv(_) => "CV",
            Opnd::This => "THIS",
        }
    }

    fn reg(&self) -> u8 {
        match self {
            Opnd::Tmp(r) | Opnd::Cv(r) => *r,
            _ => 0,
        }
    }
}

struct Tx<'p, 'w> {
    src: &'p Function,
    consts: &'p [Value],
    world: &'w World<'w>,
    caches: Vec<Cache>,
    /// Method site → its cache (shared by SendVarMethod and MethodCall).
    method_caches: HashMap<u16, u32>,
    null: &'p Value,
}

fn pick(op: &str, kinds: &[&str]) -> Result<(u16, crate::vm::Handler), String> {
    TABLE
        .iter()
        .position(|(o, k, _, _)| *o == op && *k == kinds)
        .map(|i| (i as u16, TABLE[i].3))
        .ok_or_else(|| format!("no handler P8_{op}_{}", kinds.join("_")))
}

impl<'p> Tx<'p, '_> {
    fn opnd(&self, r: Reg) -> Result<Opnd<'p>, String> {
        if r == THIS_REG {
            return Ok(Opnd::This);
        }
        if let Some(k) = const_index(r) {
            return self.consts.get(k).map(Opnd::Const).ok_or_else(|| format!("constant {k} out of range"));
        }
        let byte = u8::try_from(r).map_err(|_| format!("register {r} past the window"))?;
        Ok(if r < self.src.ncv { Opnd::Cv(byte) } else { Opnd::Tmp(byte) })
    }

    /// A destination: a temporary or a variable.
    fn dst(&self, r: Reg) -> Result<Opnd<'p>, String> {
        match self.opnd(r)? {
            o @ (Opnd::Tmp(_) | Opnd::Cv(_)) => Ok(o),
            o => Err(format!("{} as a destination", o.kind())),
        }
    }

    /// The kind of an array key operand: an int or string literal (not a
    /// numeric string: PHP's compiler normalizes those to ints), or a register.
    fn key(&self, r: Reg) -> Result<(&'static str, Opnd<'p>), String> {
        let o = self.opnd(r)?;
        Ok(match &o {
            Opnd::Const(v) if v.is_int() => ("CINT", o),
            Opnd::Const(v) if v.as_str().is_some_and(|s| !is_numeric_key(s.as_bytes())) => ("CSTR", o),
            Opnd::Const(v) => return Err(format!("literal key {v:?} (numeric string or non-scalar)")),
            Opnd::This => return Err("$this as a key".into()),
            _ => (o.kind(), o),
        })
    }

    fn insn(&self, op: &str, kinds: &[&str]) -> Result<Insn<'p>, String> {
        let (id, h) = pick(op, kinds)?;
        Ok(Insn { h, a: 0, b: 0, c: 0, d: 0, imm: 0, id, k0: self.null, k1: self.null })
    }

    /// Whether the instruction at `pc + 1` frees `reg` (the result is unused).
    fn freed_next(&self, pc: usize, reg: Reg) -> bool {
        matches!(self.src.code.get(pc + 1), Some(Instr::Free { reg: r }) if *r == reg)
    }

    fn prop_cache(&mut self, name: Reg) -> Result<u32, String> {
        let k = const_index(name).ok_or("a property name that is not a literal")?;
        let n = u16::try_from(k).map_err(|_| "property name constant index")?;
        self.caches.push(Cache::new(n));
        Ok((self.caches.len() - 1) as u32)
    }

    fn method_cache(&mut self, site: u16) -> u32 {
        if let Some(&c) = self.method_caches.get(&site) {
            return c;
        }
        self.caches.push(Cache::new(site));
        let c = (self.caches.len() - 1) as u32;
        self.method_caches.insert(site, c);
        c
    }

    fn one(&mut self, pc: usize, instr: &Instr) -> Result<Insn<'p>, String> {
        let cmp_name = |c: Cmp| -> Result<&'static str, String> {
            Ok(match c {
                Cmp::Lt => "LT",
                Cmp::Le => "LE",
                Cmp::Gt => "GT",
                Cmp::Ge => "GE",
                Cmp::Eq => "EQ",
                Cmp::NotEq => "NE",
                Cmp::Identical => "ID",
                Cmp::NotIdentical => "NI",
                Cmp::Spaceship => return Err("<=>".into()),
            })
        };
        // Two sources: `b` and `c` (their literals `k0` and `k1`).
        let two = |me: &Self, op: &str, head: &[&str], a: Reg, b: Reg| -> Result<(Insn<'p>, Opnd<'p>, Opnd<'p>), String> {
            let (x, y) = (me.opnd(a)?, me.opnd(b)?);
            if matches!(x, Opnd::This) || matches!(y, Opnd::This) {
                return Err("$this as an operand".into());
            }
            let kinds: Vec<&str> = head.iter().copied().chain([x.kind(), y.kind()]).collect();
            let mut i = me.insn(op, &kinds)?;
            if let Opnd::Const(v) = x {
                i.k0 = v;
            }
            if let Opnd::Const(v) = y {
                i.k1 = v;
            }
            Ok((i, x, y))
        };
        Ok(match *instr {
            Instr::Move { dst, src } => {
                let (d, s) = (self.dst(dst)?, self.opnd(src)?);
                let mut i = self.insn("MOVE", &[d.kind(), s.kind()])?;
                i.a = d.reg();
                i.b = s.reg();
                if let Opnd::Const(v) = s {
                    i.k0 = v;
                }
                i
            }
            Instr::LoadConst { dst, k } => {
                let d = self.dst(dst)?;
                let mut i = self.insn("MOVE", &[d.kind(), "CONST"])?;
                i.a = d.reg();
                i.k0 = self.consts.get(usize::from(k)).ok_or("LoadConst constant")?;
                i
            }
            Instr::Assign { var, src } => {
                let (Opnd::Cv(v), s) = (self.opnd(var)?, self.opnd(src)?) else { return Err("Assign to a non-variable".into()) };
                let mut i = self.insn("ASSIGN", &[s.kind(), "COPY"])?;
                i.a = v;
                i.b = s.reg();
                if let Opnd::Const(c) = s {
                    i.k0 = c;
                }
                i
            }
            Instr::AssignTake { var, src } => {
                let (Opnd::Cv(v), Opnd::Tmp(s)) = (self.opnd(var)?, self.opnd(src)?) else { return Err("AssignTake operands".into()) };
                let mut i = self.insn("ASSIGN", &["TMP", "TAKE"])?;
                i.a = v;
                i.b = s;
                i
            }
            Instr::Free { reg } => {
                let mut i = self.insn("FREE", &[])?;
                i.a = self.opnd(reg)?.reg();
                i
            }
            Instr::Jump { target } => {
                let mut i = self.insn("JMP", &[])?;
                i.imm = target;
                i
            }
            Instr::JumpIf { cond, target } | Instr::JumpIfNot { cond, target } => {
                let c = self.dst(cond)?;
                let mut i = self.insn("JMP_COND", &[c.kind(), if matches!(instr, Instr::JumpIf { .. }) { "T" } else { "F" }])?;
                i.a = c.reg();
                i.imm = target;
                i
            }
            Instr::JumpLt { .. }
            | Instr::JumpNotLt { .. }
            | Instr::JumpLe { .. }
            | Instr::JumpNotLe { .. }
            | Instr::JumpEq { .. }
            | Instr::JumpNotEq { .. }
            | Instr::JumpIdentical { .. }
            | Instr::JumpNotIdentical { .. } => {
                let (cmp, a, b, when, target) = instr.as_jump_cmp().expect("a compare-and-jump");
                let (mut i, x, y) = two(self, "JCMP", &[cmp_name(cmp)?, if when { "T" } else { "F" }], a, b)?;
                i.a = x.reg();
                i.b = y.reg();
                i.imm = target;
                i
            }
            Instr::IncJumpLt { var, b, target } | Instr::IncJumpLe { var, b, target } | Instr::DecJumpGt { var, b, target } | Instr::DecJumpGe { var, b, target } => {
                let step = match instr {
                    Instr::IncJumpLt { .. } => "INC_LT",
                    Instr::IncJumpLe { .. } => "INC_LE",
                    Instr::DecJumpGt { .. } => "DEC_GT",
                    _ => "DEC_GE",
                };
                let (Opnd::Cv(v), bound) = (self.opnd(var)?, self.opnd(b)?) else { return Err("step of a non-variable".into()) };
                let mut i = self.insn("STEP_JMP", &[step, bound.kind()])?;
                i.a = v;
                i.b = bound.reg();
                if let Opnd::Const(c) = bound {
                    i.k0 = c;
                }
                i.imm = target;
                i
            }
            Instr::Add { dst, a, b } | Instr::Sub { dst, a, b } | Instr::Mul { dst, a, b } => {
                let op = match instr {
                    Instr::Add { .. } => "ADD",
                    Instr::Sub { .. } => "SUB",
                    _ => "MUL",
                };
                let d = self.dst(dst)?;
                let (mut i, x, y) = two(self, "ARITH", &[op, d.kind()], a, b)?;
                i.a = d.reg();
                i.b = x.reg();
                i.c = y.reg();
                i
            }
            Instr::Binary { op: BinOp::Concat, dst, a, b } => {
                let d = self.dst(dst)?;
                let (mut i, x, y) = two(self, "CONCAT", &[d.kind()], a, b)?;
                i.a = d.reg();
                i.b = x.reg();
                i.c = y.reg();
                i
            }
            Instr::Compare { cmp, dst, a, b } => {
                let d = self.dst(dst)?;
                let (mut i, x, y) = two(self, "COMPARE", &[cmp_name(cmp)?, d.kind()], a, b)?;
                i.a = d.reg();
                i.b = x.reg();
                i.c = y.reg();
                i
            }
            Instr::FetchDim { dst, arr, key, quiet: false, empty: false } => {
                let (Opnd::Tmp(d), arr) = (self.opnd(dst)?, self.dst(arr)?) else { return Err("FetchDim into a variable".into()) };
                let (kk, k) = self.key(key)?;
                let mut i = self.insn("FETCH_DIM", &[arr.kind(), kk])?;
                i.a = d;
                i.b = arr.reg();
                i.c = k.reg();
                if let Opnd::Const(c) = k {
                    i.k0 = c;
                }
                i
            }
            Instr::IssetDim { dst, var, keys, n: 1 } => {
                let (Opnd::Tmp(d), var) = (self.opnd(dst)?, self.dst(var)?) else { return Err("IssetDim into a variable".into()) };
                let (kk, k) = self.key(keys)?;
                let mut i = self.insn("ISSET_DIM", &[var.kind(), kk])?;
                i.a = d;
                i.b = var.reg();
                i.c = k.reg();
                if let Opnd::Const(c) = k {
                    i.k0 = c;
                }
                i
            }
            Instr::AssignDim { var, keys, n: 1, append: false, val, dst } => {
                let (Opnd::Cv(v), val) = (self.opnd(var)?, self.opnd(val)?) else { return Err("AssignDim on a non-variable".into()) };
                let (kk, k) = self.key(keys)?;
                let Opnd::Tmp(d) = self.opnd(dst)? else { return Err("AssignDim result not a temporary".into()) };
                let res = if self.freed_next(pc, dst) { "UNUSED" } else { "USED" };
                let mut i = self.insn("ASSIGN_DIM", &[kk, val.kind(), res])?;
                i.a = v;
                i.b = val.reg();
                i.c = k.reg();
                i.d = d;
                if let Opnd::Const(c) = k {
                    i.k0 = c;
                }
                if let Opnd::Const(c) = val {
                    i.k1 = c;
                }
                i
            }
            Instr::FetchProp { dst, obj, name, quiet: false } => {
                let (Opnd::Tmp(d), o) = (self.opnd(dst)?, self.opnd(obj)?) else { return Err("FetchProp into a variable".into()) };
                if matches!(o, Opnd::Const(_)) {
                    return Err("FetchProp on a literal".into());
                }
                let cache = self.prop_cache(name)?;
                let mut i = self.insn("FETCH_PROP", &[o.kind()])?;
                i.a = d;
                i.b = o.reg();
                i.imm = cache;
                i
            }
            Instr::AssignProp { obj, name, val, dst, take } => {
                let (o, v) = (self.opnd(obj)?, self.opnd(val)?);
                if matches!(o, Opnd::Const(_)) || matches!(v, Opnd::This) {
                    return Err("AssignProp operands".into());
                }
                let Opnd::Tmp(d) = self.opnd(dst)? else { return Err("AssignProp result not a temporary".into()) };
                let res = if self.freed_next(pc, dst) { "UNUSED" } else { "USED" };
                let mode = if take {
                    if !matches!(v, Opnd::Tmp(_)) {
                        return Err("take of a non-temporary".into());
                    }
                    "TAKE"
                } else {
                    "COPY"
                };
                let cache = self.prop_cache(name)?;
                let mut i = self.insn("ASSIGN_PROP", &[o.kind(), v.kind(), mode, res])?;
                i.a = o.reg();
                i.b = v.reg();
                i.d = d;
                if let Opnd::Const(c) = v {
                    i.k0 = c;
                }
                i.imm = cache;
                i
            }
            Instr::Call { site, base, argc, dst } => {
                let site = self.src.sites.get(usize::from(site)).ok_or("call site")?;
                let target = self.world.funcs.get(site.name.as_bytes()).or_else(|| site.fallback.as_ref().and_then(|(n, _)| self.world.funcs.get(n.as_bytes())));
                let Some(&func) = target else {
                    return Err(format!("call of {} (a builtin, or not a declared function)", String::from_utf8_lossy(site.display.as_bytes())));
                };
                let shape = self.world.shapes[func as usize];
                if u16::from(argc) < shape.required || u16::from(argc) > shape.nparams {
                    return Err(format!("{argc} arguments to {}() ({} required, {} declared)", String::from_utf8_lossy(site.display.as_bytes()), shape.required, shape.nparams));
                }
                if !shape.plain_params || shape.generator {
                    return Err(format!("call of {}(): by-reference, variadic or typed parameters, or a generator", String::from_utf8_lossy(site.display.as_bytes())));
                }
                let Opnd::Tmp(d) = self.opnd(dst)? else { return Err("Call result not a temporary".into()) };
                let b = u8::try_from(base).map_err(|_| "argument base past the window")?;
                let res = if self.freed_next(pc, dst) { "UNUSED" } else { "USED" };
                let mut i = self.insn("CALL", &[res])?;
                i.a = b;
                i.b = argc;
                i.d = d;
                i.imm = func;
                i
            }
            Instr::Return { src } => {
                let s = self.opnd(src)?;
                if matches!(s, Opnd::This) {
                    return Err("return $this".into());
                }
                let mut i = self.insn("RET", &[s.kind()])?;
                i.a = s.reg();
                if let Opnd::Const(c) = s {
                    i.k0 = c;
                }
                i
            }
            Instr::ReturnNull => self.insn("RET", &["NONE"])?,
            Instr::SendVarMethod { obj, method, n, arg, var } => {
                let (o, Opnd::Tmp(a), Opnd::Cv(v)) = (self.dst(obj)?, self.opnd(arg)?, self.opnd(var)?) else { return Err("SendVarMethod operands".into()) };
                let mut i = self.insn("SEND_VAR_METHOD", &[])?;
                i.a = o.reg();
                i.b = a;
                i.c = v;
                i.d = n;
                i.imm = self.method_cache(method);
                i
            }
            Instr::MethodCall { dst, obj, method, base, argc } => {
                let (Opnd::Tmp(d), o) = (self.opnd(dst)?, self.opnd(obj)?) else { return Err("MethodCall result not a temporary".into()) };
                if matches!(o, Opnd::Const(_)) {
                    return Err("method call on a literal".into());
                }
                let res = if self.freed_next(pc, dst) { "UNUSED" } else { "USED" };
                let mut i = self.insn("METHOD_CALL", &[o.kind(), res])?;
                i.a = d;
                i.b = o.reg();
                i.c = u8::try_from(base).map_err(|_| "argument base past the window")?;
                i.d = argc;
                i.imm = self.method_cache(method);
                i
            }
            Instr::IterInit { src, iter, end } => {
                let s = self.dst(src)?;
                let Opnd::Tmp(it) = self.opnd(iter)? else { return Err("iterator not a temporary".into()) };
                if it == u8::MAX {
                    return Err("iterator position past the window".into());
                }
                let mut i = self.insn("ITER_INIT", &[s.kind()])?;
                i.a = s.reg();
                i.b = it;
                i.imm = end;
                i
            }
            Instr::IterNext { iter, val, key, has_key, end } => {
                let (Opnd::Tmp(it), Opnd::Cv(v)) = (self.opnd(iter)?, self.opnd(val)?) else { return Err("IterNext operands".into()) };
                let k = if has_key {
                    let Opnd::Cv(k) = self.opnd(key)? else { return Err("foreach key not a variable".into()) };
                    k
                } else {
                    0
                };
                let mut i = self.insn("ITER_NEXT", &[if has_key { "KEY" } else { "NOKEY" }])?;
                i.a = it;
                i.b = v;
                i.c = k;
                i.imm = end;
                i
            }
            Instr::New { dst, class } => {
                let Opnd::Tmp(d) = self.opnd(dst)? else { return Err("New into a variable".into()) };
                let (idx, ctor) = self.class_of(class)?;
                let Some(Instr::Construct { obj, .. }) = self.src.code.get(pc + 1) else { return Err("New not followed by Construct".into()) };
                if *obj != dst {
                    return Err("Construct of another register".into());
                }
                let mut i = self.insn("NEW", &[if ctor.is_some() { "CTOR" } else { "PLAIN" }])?;
                i.a = d;
                i.imm = idx;
                i
            }
            Instr::Construct { obj, base, argc } => {
                let Some(Instr::New { class, .. }) = pc.checked_sub(1).and_then(|p| self.src.code.get(p)) else { return Err("Construct not after New".into()) };
                let (_, ctor) = self.class_of(*class)?;
                let Opnd::Tmp(o) = self.opnd(obj)? else { return Err("Construct of a variable".into()) };
                let mut i = self.insn("CONSTRUCT", &[])?;
                i.a = o;
                i.b = u8::try_from(base).map_err(|_| "argument base past the window")?;
                i.c = argc;
                i.imm = match ctor {
                    Some(c) => {
                        let shape = self.world.shapes[c as usize];
                        if u16::from(argc) < shape.required || u16::from(argc) > shape.nparams || !shape.plain_params {
                            return Err("constructor arguments not in the prototype".into());
                        }
                        c
                    }
                    None => u32::MAX,
                };
                i
            }
            Instr::Echo { src } => {
                let s = self.opnd(src)?;
                if matches!(s, Opnd::This) {
                    return Err("echo $this".into());
                }
                let mut i = self.insn("ECHO", &[s.kind()])?;
                i.a = s.reg();
                if let Opnd::Const(c) = s {
                    i.k0 = c;
                }
                i
            }
            Instr::Rare { op, .. } => return Err(format!("{op:?}")),
            ref other => return Err(format!("{}", instr_name(other))),
        })
    }

    fn class_of(&self, class: u16) -> Result<(u32, Option<u32>), String> {
        match self.src.classes.get(usize::from(class)) {
            Some(ClassRef::Named(n)) => self.world.classes.get(n.lower.as_bytes()).copied().ok_or_else(|| format!("class {} not declared in the file", String::from_utf8_lossy(n.display.as_bytes()))),
            Some(other) => Err(format!("class reference {other:?}")),
            None => Err("class operand".into()),
        }
    }
}

/// The variant's name, for a refusal message.
fn instr_name(i: &Instr) -> String {
    let s = format!("{i:?}");
    s.split([' ', '{']).next().unwrap_or("?").to_string()
}

/// Whether a string literal is a canonical integer key ("123", "-5"; not
/// "0123", "-0", "1.0").
fn is_numeric_key(s: &[u8]) -> bool {
    let digits = s.strip_prefix(b"-").unwrap_or(s);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) || (digits.len() > 1 && digits[0] == b'0') || s == b"-0" {
        return false;
    }
    std::str::from_utf8(s).ok().and_then(|t| t.parse::<i64>().ok()).is_some()
}

/// The shape of a function the translator checks before a call.
pub fn shape(f: &Function) -> Shape {
    Shape {
        nparams: f.params.len() as u16,
        required: f.required as u16,
        plain_params: f.params.iter().all(|p| !p.by_ref && !p.variadic && p.ty.is_none()),
        generator: f.is_generator,
    }
}

/// `src` translated. `index` is its position in the program (for messages);
/// `consts` its constants in the pool.
pub fn translate<'p>(src: &'p Function, pool: &'p Pool, consts: &'p [Value], world: &World<'_>) -> Func<'p> {
    let name = String::from_utf8_lossy(src.name.as_bytes()).into_owned();
    let mut tx = Tx { src, consts, world, caches: Vec::new(), method_caches: HashMap::new(), null: &pool.null };
    let refused = (|| -> Result<Vec<Insn<'p>>, String> {
        if src.is_generator {
            return Err("a generator".into());
        }
        if src.returns_ref {
            return Err("returns by reference".into());
        }
        if !src.handlers.is_empty() {
            return Err("try/catch".into());
        }
        if !src.bound.is_empty() {
            return Err("a closure with bound variables".into());
        }
        if src.nregs > 255 {
            return Err(format!("{} registers", src.nregs));
        }
        if !shape(src).plain_params {
            return Err("by-reference, variadic or typed parameters".into());
        }
        let mut code = Vec::with_capacity(src.code.len());
        for (pc, instr) in src.code.iter().enumerate() {
            // (Function entry's type check is the parameters' business: refused above.)
            if matches!(instr, Instr::Rare { op: Rare::VerifyArgs, .. }) {
                return Err("VerifyArgs".into());
            }
            code.push(tx.one(pc, instr).map_err(|e| format!("pc {pc} {}: {e}", instr_name(instr)))?);
        }
        Ok(code)
    })();
    let (code, refused) = match refused {
        Ok(code) => (code, None),
        Err(why) => (vec![tx.insn("REFUSED", &[]).expect("the REFUSED handler")], Some(why)),
    };
    Func {
        name,
        src,
        code,
        consts,
        caches: tx.caches,
        ncv: src.ncv,
        nregs: src.nregs,
        nparams: src.params.len() as u16,
        required: src.required as u16,
        refused,
    }
}

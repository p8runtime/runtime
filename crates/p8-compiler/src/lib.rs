#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! AST → p8 register bytecode.
//!
//! Every `$name` a function uses is a compiled variable with a fixed
//! register; constants get registers too (filled when the frame starts),
//! so instructions only ever name registers. While compiling, a register
//! is numbered within its kind (variable, constant, temporary); `finish`
//! lays a frame out as [variables][constants][temporaries] and renumbers.
//! Temporaries are allocated like a stack, so a call's arguments are
//! always the caller's topmost registers: the callee's frame starts there.

use p8_bytecode::{CastKind, Cmp, Function, Instr, Param, Reg, Site};
use p8_runtime::ext::Registry;
use p8_runtime::ops::BinOp;
use p8_runtime::frozen::{Frozen, Text};
use p8_syntax::literal::{self, Number};
use p8_syntax::ast::{AssignOp, BinaryOp, CastKind as AstCast, Expr, Stmt, StmtId, UnaryOp};
use p8_syntax::span::Span;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

/// What the compiler cannot (yet) compile, or a compile-time error.
#[derive(Debug, Clone, PartialEq)]
pub struct CompileError {
    pub message: String,
    pub line: u32,
}

const KIND_SHIFT: u16 = 14;
const CV: u16 = 0;
const CONST: u16 = 1;
const TEMP: u16 = 2;

fn reg(kind: u16, index: usize) -> Reg {
    debug_assert!(index < (1 << KIND_SHIFT));
    (kind << KIND_SHIFT) | index as u16
}

/// A constant, as a key for sharing one register per distinct constant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ConstKey {
    Null,
    Bool(bool),
    Int(i64),
    Float(u64),
    Str(Vec<u8>),
}

struct Loop {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

/// One function being compiled.
struct FnBuilder {
    name: Text,
    code: Vec<Instr>,
    lines: Vec<u32>,
    consts: Vec<Frozen>,
    const_index: HashMap<ConstKey, usize>,
    cvs: IndexMap<Vec<u8>, usize>,
    temps: usize,
    /// Temporaries below this stay live across statements (a foreach's iterator).
    floor: usize,
    max_temps: usize,
    sites: Vec<Site>,
    site_index: HashMap<(bool, Vec<u8>), u16>,
    loops: Vec<Loop>,
    params: Vec<Param>,
    returns_ref: bool,
    line: u32,
    /// The function's declaration line.
    decl_line: u32,
}

impl FnBuilder {
    fn new(name: Text) -> Self {
        FnBuilder {
            name,
            code: Vec::new(),
            lines: Vec::new(),
            consts: Vec::new(),
            const_index: HashMap::new(),
            cvs: IndexMap::new(),
            temps: 0,
            floor: 0,
            max_temps: 0,
            sites: Vec::new(),
            site_index: HashMap::new(),
            loops: Vec::new(),
            params: Vec::new(),
            returns_ref: false,
            line: 1,
            decl_line: 1,
        }
    }

    fn emit(&mut self, instr: Instr) -> usize {
        self.code.push(instr);
        self.lines.push(self.line);
        self.code.len() - 1
    }

    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn patch(&mut self, at: usize, target: u32) {
        match self.code[at].target_mut() {
            Some(t) => *t = target,
            None => unreachable!("not a jump: {:?}", self.code[at]),
        }
    }

    /// Point every jump in `jumps` at `target`.
    fn patch_all(&mut self, jumps: Vec<usize>, target: u32) {
        for at in jumps {
            self.patch(at, target);
        }
    }

    fn cv(&mut self, name: &[u8]) -> Reg {
        let next = self.cvs.len();
        let index = *self.cvs.entry(name.to_vec()).or_insert(next);
        reg(CV, index)
    }

    fn constant(&mut self, value: Frozen) -> Reg {
        let key = match &value {
            Frozen::Null => ConstKey::Null,
            Frozen::Bool(b) => ConstKey::Bool(*b),
            Frozen::Int(i) => ConstKey::Int(*i),
            Frozen::Float(f) => ConstKey::Float(f.to_bits()),
            Frozen::Str(s) => ConstKey::Str(s.as_bytes().to_vec()),
        };
        let next = self.consts.len();
        let index = *self.const_index.entry(key).or_insert(next);
        if index == next {
            self.consts.push(value);
        }
        reg(CONST, index)
    }

    fn temp(&mut self) -> Reg {
        let r = reg(TEMP, self.temps);
        self.temps += 1;
        self.max_temps = self.max_temps.max(self.temps);
        r
    }

    fn site(&mut self, constant: bool, name: Text, display: Text) -> u16 {
        let key = (constant, name.as_bytes().to_vec());
        if let Some(&i) = self.site_index.get(&key) {
            return i;
        }
        self.sites.push(Site { name, fallback: None, display });
        let i = (self.sites.len() - 1) as u16;
        self.site_index.insert(key, i);
        i
    }

    /// Lay the frame out and renumber every register.
    fn finish(mut self, file: Arc<str>) -> Function {
        if !self.returns_ref {
            thread_returns(&mut self.code);
        }
        let ncv = self.cvs.len();
        let nconst = self.consts.len();
        let map = |r: Reg| -> Reg {
            let index = (r & ((1 << KIND_SHIFT) - 1)) as usize;
            (match r >> KIND_SHIFT {
                CV => index,
                CONST => ncv + index,
                _ => ncv + nconst + index,
            }) as Reg
        };
        for instr in &mut self.code {
            map_regs(instr, map);
        }
        fuse_steps(&mut self.code);
        let mut cv_names: Vec<(usize, Text)> =
            self.cvs.iter().map(|(n, &i)| (i, Text::from(n.as_slice()))).collect();
        cv_names.sort_by_key(|(i, _)| *i);
        let required = self.params.iter().rposition(|p| !p.has_default && !p.variadic).map_or(0, |i| i + 1);
        Function {
            name: self.name,
            params: self.params,
            returns_ref: self.returns_ref,
            code: self.code,
            lines: self.lines,
            consts: self.consts,
            consts_at: ncv as Reg,
            nregs: (ncv + nconst + self.max_temps) as Reg,
            cv_names: cv_names.into_iter().map(|(_, n)| n).collect(),
            required,
            line: self.decl_line,
            sites: self.sites,
            file,
        }
    }
}

/// A loop's `$i++` (or `$i--`) followed by its test on `$i` becomes one
/// instruction (see `Instr::IncJumpLt`). The test stays where it is, so the
/// code keeps its length and every jump target.
fn fuse_steps(code: &mut [Instr]) {
    for i in 0..code.len().saturating_sub(1) {
        code[i] = match (code[i], code[i + 1]) {
            (Instr::IncVar { var }, Instr::JumpLt { a, b, target }) if a == var && b != var => Instr::IncJumpLt { var, b, target },
            (Instr::IncVar { var }, Instr::JumpLe { a, b, target }) if a == var && b != var => Instr::IncJumpLe { var, b, target },
            // `$i > b` and `$i >= b` are compiled with the operands swapped.
            (Instr::DecVar { var }, Instr::JumpLt { a, b, target }) if b == var && a != var => Instr::DecJumpGt { var, b: a, target },
            (Instr::DecVar { var }, Instr::JumpLe { a, b, target }) if b == var && a != var => Instr::DecJumpGe { var, b: a, target },
            (other, _) => other,
        };
    }
}

/// Whether `a op b` compiles to one instruction that reads both operands
/// and then writes its result (and nothing else), so the result can go
/// straight to a variable.
fn writes_once(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Plus
            | BinaryOp::Minus
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::Pow
            | BinaryOp::Concat
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::ShiftLeft
            | BinaryOp::ShiftRight
            | BinaryOp::Spaceship
    ) || comparison(op).is_some()
}

/// Returns reached by a jump are made where the jump is: `x ? a : b` in
/// `return` position, for one, stores its result and jumps to the return.
/// A temporary moved into a register only to be returned is returned
/// directly when it is a constant. The code keeps its length (what is
/// left behind is unreachable), so no jump target moves.
fn thread_returns(code: &mut [Instr]) {
    for i in 0..code.len() {
        if let Instr::Jump { target } = code[i]
            && let ret @ (Instr::Return { .. } | Instr::ReturnNull) = code[target as usize]
        {
            code[i] = ret;
        }
    }
    for i in 0..code.len().saturating_sub(1) {
        if let (Instr::Move { dst, src }, Instr::Return { src: ret }) = (code[i], code[i + 1])
            && ret == dst
            && dst >> KIND_SHIFT == TEMP
            && src >> KIND_SHIFT == CONST
        {
            code[i] = Instr::Return { src };
        }
    }
}

/// Apply `f` to every register an instruction names.
fn map_regs(instr: &mut Instr, f: impl Fn(Reg) -> Reg) {
    use Instr::*;
    match instr {
        Move { dst, src } | Not { dst, src } | Neg { dst, src } | BitNot { dst, src } | ToBool { dst, src } | Cast { dst, src, .. } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        Assign { var, src } => {
            *var = f(*var);
            *src = f(*src);
        }
        LoadNull { dst } | NewArray { dst } => *dst = f(*dst),
        Binary { dst, a, b, .. } | Add { dst, a, b } | Sub { dst, a, b } | Mul { dst, a, b } | Compare { dst, a, b, .. } => {
            *dst = f(*dst);
            *a = f(*a);
            *b = f(*b);
        }
        AssignOp { var, src, .. } => {
            *var = f(*var);
            *src = f(*src);
        }
        IncDec { var, dst, .. } => {
            *var = f(*var);
            *dst = f(*dst);
        }
        Jump { .. } | ReturnNull | ArgDefault { .. } => {}
        IncVar { var } | DecVar { var } => *var = f(*var),
        IncJumpLt { var, b, .. } | IncJumpLe { var, b, .. } | DecJumpGt { var, b, .. } | DecJumpGe { var, b, .. } => {
            *var = f(*var);
            *b = f(*b);
        }
        JumpLt { a, b, .. }
        | JumpNotLt { a, b, .. }
        | JumpLe { a, b, .. }
        | JumpNotLe { a, b, .. }
        | JumpEq { a, b, .. }
        | JumpNotEq { a, b, .. }
        | JumpIdentical { a, b, .. }
        | JumpNotIdentical { a, b, .. } => {
            *a = f(*a);
            *b = f(*b);
        }
        JumpIf { cond, .. } | JumpIfNot { cond, .. } => *cond = f(*cond),
        JumpIfSet { src, dst, .. } => {
            *src = f(*src);
            *dst = f(*dst);
        }
        ArrayPush { arr, val } => {
            *arr = f(*arr);
            *val = f(*val);
        }
        ArraySet { arr, key, val } => {
            *arr = f(*arr);
            *key = f(*key);
            *val = f(*val);
        }
        FetchDim { dst, arr, key, .. } => {
            *dst = f(*dst);
            *arr = f(*arr);
            *key = f(*key);
        }
        AssignDim { var, keys, val, dst, .. } | AssignDimOp { var, keys, val, dst, .. } => {
            *var = f(*var);
            *keys = f(*keys);
            *val = f(*val);
            *dst = f(*dst);
        }
        IssetDim { dst, var, keys, .. } => {
            *dst = f(*dst);
            *var = f(*var);
            *keys = f(*keys);
        }
        IssetVar { dst, var } => {
            *dst = f(*dst);
            *var = f(*var);
        }
        UnsetVar { var } => *var = f(*var),
        UnsetDim { var, keys, .. } => {
            *var = f(*var);
            *keys = f(*keys);
        }
        IterInit { src, iter, .. } => {
            *src = f(*src);
            *iter = f(*iter);
        }
        IterNext { iter, val, key, .. } => {
            *iter = f(*iter);
            *val = f(*val);
            *key = f(*key);
        }
        Call { base, dst, .. } => {
            *base = f(*base);
            *dst = f(*dst);
        }
        SendVar { arg, var, .. } => {
            *arg = f(*arg);
            *var = f(*var);
        }
        Return { src } | Echo { src } | Exit { src, .. } => *src = f(*src),
        FetchConst { dst, .. } => *dst = f(*dst),
        ConcatN { dst, start, .. } => {
            *dst = f(*dst);
            *start = f(*start);
        }
        BindGlobal { var, .. } => *var = f(*var),
    }
}

/// The compiled program: the top-level code and every function declared.
pub struct Program {
    pub main: Arc<Function>,
    pub functions: Vec<Arc<Function>>,
}

pub struct Compiler<'src> {
    source: &'src [u8],
    /// The builtins (their by-reference parameters are known when compiling).
    registry: &'src Registry,
    file: Arc<str>,
    line_starts: Vec<usize>,
    functions: Vec<Arc<Function>>,
    /// Which parameters take references, for the functions declared at the
    /// top level (declared before the file runs, so a call to one is bound
    /// when compiled: its by-value arguments need no runtime check).
    /// (Each parameter's mode, and whether extra arguments go by reference.)
    by_ref: HashMap<Vec<u8>, (Vec<bool>, bool)>,
}

type Result<T> = std::result::Result<T, CompileError>;

impl<'src> Compiler<'src> {
    pub fn new(source: &'src [u8], file: &str, registry: &'src Registry) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(source.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i + 1));
        Compiler { source, registry, file: Arc::from(file), line_starts, functions: Vec::new(), by_ref: HashMap::new() }
    }

    fn line(&self, span: Span) -> u32 {
        self.line_starts.partition_point(|s| *s <= span.start) as u32
    }

    fn text(&self, span: Span) -> &'src [u8] {
        &self.source[span.start..span.end]
    }

    fn unsupported(&self, span: Span, what: &str) -> CompileError {
        CompileError { message: format!("p8 cannot compile {what} yet"), line: self.line(span) }
    }

    pub fn compile(mut self, stmts: &[StmtId]) -> Result<Program> {
        for stmt in stmts {
            if let Stmt::Function { name, params, .. } = stmt {
                let modes = params.iter().map(|p| p.by_ref).collect();
                let rest = params.last().is_some_and(|p| p.variadic && p.by_ref);
                self.by_ref.insert(self.text(name.span).to_ascii_lowercase(), (modes, rest));
            }
        }
        let mut f = FnBuilder::new(Text::from("{main}"));
        self.stmts(&mut f, stmts)?;
        f.emit(Instr::ReturnNull);
        let main = Arc::new(f.finish(self.file.clone()));
        Ok(Program { main, functions: self.functions })
    }

    fn stmts(&mut self, f: &mut FnBuilder, stmts: &[StmtId]) -> Result<()> {
        for stmt in stmts {
            self.stmt(f, stmt)?;
            f.temps = f.floor;
        }
        Ok(())
    }

    fn stmt(&mut self, f: &mut FnBuilder, stmt: &Stmt) -> Result<()> {
        f.line = self.line(stmt.span());
        match stmt {
            Stmt::Expression { expr, .. } => self.discard(f, expr)?,
            Stmt::Echo { exprs, .. } => {
                for e in *exprs {
                    let r = self.expr(f, e)?;
                    f.emit(Instr::Echo { src: r });
                }
            }
            Stmt::InlineHtml { value, .. } => {
                let r = f.constant(Frozen::Str(Text::from(*value)));
                f.emit(Instr::Echo { src: r });
            }
            Stmt::Return { expr, .. } => match expr {
                Some(e) => {
                    let r = self.expr(f, e)?;
                    f.emit(Instr::Return { src: r });
                }
                None => {
                    f.emit(Instr::ReturnNull);
                }
            },
            Stmt::If { condition, then_block, else_block, .. } => {
                let skip = self.branch(f, condition, false)?;
                f.temps = f.floor;
                self.stmts(f, then_block)?;
                match else_block {
                    Some(else_block) => {
                        let end = f.emit(Instr::Jump { target: 0 });
                        f.patch_all(skip, f.here());
                        self.stmts(f, else_block)?;
                        f.patch(end, f.here());
                    }
                    None => f.patch_all(skip, f.here()),
                }
            }
            Stmt::While { condition, body, .. } => {
                // The condition is tested at the bottom: one branch per iteration.
                let to_cond = f.emit(Instr::Jump { target: 0 });
                let body_start = f.here();
                self.loop_body(f, body, body_start)?;
                let cond_at = f.here();
                f.patch(to_cond, cond_at);
                let back = self.branch(f, condition, true)?;
                f.patch_all(back, body_start);
                f.temps = f.floor;
                self.close_loop(f, cond_at);
            }
            Stmt::DoWhile { body, condition, .. } => {
                let start = f.here();
                f.loops.push(Loop { breaks: Vec::new(), continues: Vec::new() });
                self.stmts(f, body)?;
                let cont = f.here();
                let back = self.branch(f, condition, true)?;
                f.patch_all(back, start);
                f.temps = f.floor;
                self.close_loop(f, cont);
            }
            Stmt::For { init, condition, loop_expr, body, .. } => {
                for e in *init {
                    self.discard(f, e)?;
                    f.temps = f.floor;
                }
                // Body first, then the step and the condition at the bottom.
                let to_cond = f.emit(Instr::Jump { target: 0 });
                let body_start = f.here();
                f.loops.push(Loop { breaks: Vec::new(), continues: Vec::new() });
                self.stmts(f, body)?;
                let cont = f.here();
                for e in *loop_expr {
                    self.discard(f, e)?;
                    f.temps = f.floor;
                }
                f.patch(to_cond, f.here());
                match condition.split_last() {
                    None => {
                        f.emit(Instr::Jump { target: body_start });
                    }
                    Some((last, rest)) => {
                        for e in rest {
                            self.discard(f, e)?;
                        }
                        let back = self.branch(f, last, true)?;
                        f.patch_all(back, body_start);
                    }
                }
                f.temps = f.floor;
                self.close_loop(f, cont);
            }
            Stmt::Foreach { expr, key_var, value_var, body, .. } => {
                let src = self.expr(f, expr)?;
                // The iterator's two registers stay live through the body.
                let iter = f.temp();
                let _pos = f.temp();
                let saved = f.temps;
                let init = f.emit(Instr::IterInit { src, iter, end: 0 });
                let start = f.here();
                let val = self.var_target(f, value_var)?;
                let key = match key_var {
                    Some(k) => Some(self.var_target(f, k)?),
                    None => None,
                };
                let next = f.emit(Instr::IterNext { iter, val, key: key.unwrap_or(val), has_key: key.is_some(), end: 0 });
                f.loops.push(Loop { breaks: Vec::new(), continues: Vec::new() });
                let outer_floor = std::mem::replace(&mut f.floor, saved);
                self.stmts(f, body)?;
                f.floor = outer_floor;
                f.emit(Instr::Jump { target: start });
                let end = f.here();
                f.patch(init, end);
                f.patch(next, end);
                self.close_loop(f, start);
            }
            Stmt::Block { statements, .. } => self.stmts(f, statements)?,
            Stmt::Break { level, span } | Stmt::Continue { level, span } => {
                let is_break = matches!(stmt, Stmt::Break { .. });
                let depth = match level {
                    Some(e) => match self.literal(e) {
                        Some(Frozen::Int(n)) if n >= 1 => n as usize,
                        _ => return Err(self.unsupported(*span, "break/continue with a non-literal level")),
                    },
                    None => 1,
                };
                if depth > f.loops.len() {
                    return Err(CompileError {
                        message: format!(
                            "Cannot '{}' {} level{}",
                            if is_break { "break" } else { "continue" },
                            depth,
                            if depth == 1 { "" } else { "s" }
                        ),
                        line: self.line(*span),
                    });
                }
                let at = f.emit(Instr::Jump { target: 0 });
                let target = f.loops.len() - depth;
                if is_break {
                    f.loops[target].breaks.push(at);
                } else {
                    f.loops[target].continues.push(at);
                }
            }
            Stmt::Function { name, params, body, by_ref, .. } => {
                let function = self.function(name.span, params, body, *by_ref)?;
                self.functions.push(Arc::new(function));
            }
            Stmt::Global { vars, .. } => {
                for v in *vars {
                    let Expr::Variable { name, .. } = v else {
                        return Err(self.unsupported(v.span(), "this global statement"));
                    };
                    let text = self.text(*name);
                    let var = f.cv(&text[1..]);
                    let name_reg = f.constant(Frozen::Str(Text::from(&text[1..])));
                    let name_index = name_reg & ((1 << KIND_SHIFT) - 1);
                    f.emit(Instr::BindGlobal { var, name: name_index });
                }
            }
            Stmt::Const { consts, .. } => {
                for c in *consts {
                    let name = f.constant(Frozen::Str(Text::from(self.text(c.name.span))));
                    let value = self.expr(f, c.value)?;
                    let site = f.site(false, Text::from("define"), Text::from("define"));
                    let base = f.temp();
                    let second = f.temp();
                    f.emit(Instr::Move { dst: base, src: name });
                    f.emit(Instr::Move { dst: second, src: value });
                    let dst = f.temp();
                    f.emit(Instr::Call { site, base, argc: 2, dst });
                }
            }
            Stmt::Unset { vars, .. } => {
                for v in *vars {
                    match v {
                        Expr::Variable { name, .. } => {
                            let var = f.cv(&self.text(*name)[1..]);
                            f.emit(Instr::UnsetVar { var });
                        }
                        Expr::ArrayDimFetch { .. } => {
                            let (var, keys, n, append) = self.dim_path(f, v)?;
                            if append {
                                return Err(CompileError { message: "Cannot use [] for unsetting".into(), line: f.line });
                            }
                            f.emit(Instr::UnsetDim { var, keys, n });
                        }
                        other => return Err(self.unsupported(other.span(), "this unset")),
                    }
                }
            }
            Stmt::Nop { .. } => {}
            other => return Err(self.unsupported(other.span(), "this statement")),
        }
        Ok(())
    }

    /// Compile `e` for its effects only.
    fn discard(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<()> {
        match e {
            Expr::PostInc { var, .. } | Expr::PostDec { var, .. } => {
                let var = self.inc_target(f, var)?;
                f.emit(if matches!(e, Expr::PostInc { .. }) { Instr::IncVar { var } } else { Instr::DecVar { var } });
            }
            Expr::Unary { op: op @ (UnaryOp::PreInc | UnaryOp::PreDec), expr, .. } => {
                let var = self.inc_target(f, expr)?;
                f.emit(if matches!(op, UnaryOp::PreInc) { Instr::IncVar { var } } else { Instr::DecVar { var } });
            }
            _ => {
                self.expr(f, e)?;
            }
        }
        Ok(())
    }

    /// Emit jumps taken when `cond` is `when`; returns them for patching
    /// (to one target). A comparison becomes one compare-and-jump
    /// instruction; `&&`, `||` and `!` become control flow, so no boolean
    /// is built for a condition.
    fn branch(&mut self, f: &mut FnBuilder, cond: &Expr, when: bool) -> Result<Vec<usize>> {
        let mark = f.temps;
        match cond {
            Expr::Binary { left, op, right, .. } if comparison(*op).is_some() => {
                let cmp = comparison(*op).expect("a comparison");
                let a = self.expr(f, left)?;
                let b = self.expr(f, right)?;
                f.temps = mark;
                Ok(vec![f.emit(Instr::jump_cmp(cmp, a, b, when, 0))])
            }
            Expr::Binary { left, op, right, .. }
                if matches!(op, BinaryOp::And | BinaryOp::LogicalAnd | BinaryOp::Or | BinaryOp::LogicalOr) =>
            {
                let is_and = matches!(op, BinaryOp::And | BinaryOp::LogicalAnd);
                if is_and != when {
                    // `a && b` false, or `a || b` true: either side decides.
                    let mut jumps = self.branch(f, left, when)?;
                    jumps.extend(self.branch(f, right, when)?);
                    Ok(jumps)
                } else {
                    // `a && b` true, or `a || b` false: the left side can
                    // only rule it out, the right side decides.
                    let past = self.branch(f, left, !when)?;
                    let jumps = self.branch(f, right, when)?;
                    f.patch_all(past, f.here());
                    Ok(jumps)
                }
            }
            Expr::Unary { op: UnaryOp::Not, expr, .. } => self.branch(f, expr, !when),
            _ => {
                let c = self.expr(f, cond)?;
                f.temps = mark;
                Ok(vec![f.emit(if when { Instr::JumpIf { cond: c, target: 0 } } else { Instr::JumpIfNot { cond: c, target: 0 } })])
            }
        }
    }

    fn loop_body(&mut self, f: &mut FnBuilder, body: &[StmtId], _start: u32) -> Result<()> {
        f.loops.push(Loop { breaks: Vec::new(), continues: Vec::new() });
        self.stmts(f, body)
    }

    /// Pop the innermost loop: breaks go to here, continues to `cont`.
    fn close_loop(&mut self, f: &mut FnBuilder, cont: u32) {
        let lp = f.loops.pop().expect("a loop is open");
        let end = f.here();
        for at in lp.breaks {
            f.patch(at, end);
        }
        for at in lp.continues {
            f.patch(at, cont);
        }
    }

    /// A user function.
    fn function(&mut self, name: Span, params: &[p8_syntax::ast::Param], body: &[StmtId], by_ref: bool) -> Result<Function> {
        let mut f = FnBuilder::new(Text::from(self.text(name)));
        f.returns_ref = by_ref;
        f.line = self.line(name);
        for p in params {
            let text = self.text(p.name.span);
            f.cv(&text[1..]);
            f.params.push(Param {
                name: Text::from(&text[1..]),
                by_ref: p.by_ref,
                has_default: p.default.is_some(),
                variadic: p.variadic,
            });
        }
        f.decl_line = f.line;
        // Defaults: each runs only when its argument was not passed (the
        // call checks the required ones were).
        for (i, p) in params.iter().enumerate() {
            if let Some(default) = p.default {
                let at = f.emit(Instr::ArgDefault { n: i as u8, target: 0 });
                let var = reg(CV, i);
                let value = self.expr(&mut f, default)?;
                f.emit(Instr::Assign { var, src: value });
                f.temps = f.floor;
                f.patch(at, f.here());
            }
        }
        self.stmts(&mut f, body)?;
        f.emit(Instr::ReturnNull);
        Ok(f.finish(self.file.clone()))
    }

    fn literal(&self, e: &Expr) -> Option<Frozen> {
        Some(match e {
            Expr::Integer { value, .. } => match literal::integer(value) {
                Number::Int(i) => Frozen::Int(i),
                Number::Float(x) => Frozen::Float(x),
            },
            Expr::Float { value, .. } => Frozen::Float(literal::float(value)),
            Expr::String { value, .. } => Frozen::Str(Text::from(literal::string(value))),
            Expr::Boolean { value, .. } => Frozen::Bool(*value),
            Expr::Null { .. } => Frozen::Null,
            _ => return None,
        })
    }

    /// A variable written by foreach: its register.
    fn var_target(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<Reg> {
        match e {
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => Ok(f.cv(&self.text(*name)[1..])),
            other => Err(self.unsupported(other.span(), "this foreach target")),
        }
    }

    /// `$v[k1]..[kn]`: the variable, the keys in consecutive temporaries,
    /// and whether the last dimension is `[]`.
    fn dim_path(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<(Reg, Reg, u8, bool)> {
        let mut dims = Vec::new();
        let mut cur = e;
        while let Expr::ArrayDimFetch { array, dim, .. } = cur {
            dims.push(*dim);
            cur = array;
        }
        dims.reverse();
        let Expr::Variable { name, .. } = cur else {
            return Err(self.unsupported(cur.span(), "this array write"));
        };
        let text = self.text(*name);
        if !text.starts_with(b"$") || text == b"$this" {
            return Err(self.unsupported(cur.span(), "this array write"));
        }
        let var = f.cv(&text[1..]);
        let append = dims.last().is_some_and(|d| d.is_none());
        if dims[..dims.len() - usize::from(append)].iter().any(|d| d.is_none()) {
            return Err(self.unsupported(e.span(), "[] before the last dimension"));
        }
        let keyed: Vec<&Expr> = dims.iter().flatten().copied().collect();
        if let [key] = keyed[..] {
            // One key needs no run of registers: use its own (a variable
            // key is then read when the write happens, as in PHP).
            let r = self.expr(f, key)?;
            return Ok((var, r, 1, append));
        }
        let regs: Vec<Reg> = keyed.iter().map(|_| f.temp()).collect();
        for (r, d) in regs.iter().zip(&keyed) {
            self.expr_into(f, d, *r)?;
        }
        let keys = regs.first().copied().unwrap_or_else(|| f.temp());
        Ok((var, keys, regs.len() as u8, append))
    }

    /// Compile `e` into exactly `dst`, a temporary (never a variable: a
    /// result written there directly would bypass a reference it holds).
    fn expr_into(&mut self, f: &mut FnBuilder, e: &Expr, dst: Reg) -> Result<()> {
        debug_assert_eq!(dst >> KIND_SHIFT, TEMP, "expr_into targets a temporary");
        let r = self.expr_to(f, e, Some(dst))?;
        if r != dst {
            f.emit(Instr::Move { dst, src: r });
        }
        Ok(())
    }

    /// Compile `e`; returns the register holding its value (a variable's
    /// own register for a plain read).
    fn expr(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<Reg> {
        self.expr_to(f, e, None)
    }

    /// Compile `e`, computing it straight into `hint` where the instruction
    /// allows (else the caller moves it there). Temporaries the operands
    /// used are free once the instruction that reads them is emitted.
    fn expr_to(&mut self, f: &mut FnBuilder, e: &Expr, hint: Option<Reg>) -> Result<Reg> {
        let mark = f.temps;
        if let Some(v) = self.literal(e) {
            return Ok(f.constant(v));
        }
        Ok(match e {
            Expr::Variable { name, span } => {
                let text = self.text(*name);
                if let Some(var) = text.strip_prefix(b"$") {
                    if var == b"this" {
                        return Err(self.unsupported(*span, "$this"));
                    }
                    f.cv(var)
                } else {
                    self.constant_fetch(f, text)
                }
            }
            Expr::Assign { var, expr, .. } => match var {
                Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => {
                    let target = f.cv(&self.text(*name)[1..]);
                    // Arithmetic and comparisons are one instruction that
                    // writes its result last: it can write the variable
                    // itself (through a reference, if the variable is one).
                    if let Expr::Binary { op, .. } = expr
                        && writes_once(*op)
                    {
                        let mark = f.temps;
                        let src = self.expr_to(f, expr, Some(target))?;
                        f.temps = mark;
                        if src != target {
                            f.emit(Instr::Assign { var: target, src });
                        }
                        return Ok(target);
                    }
                    let src = self.expr(f, expr)?;
                    f.emit(Instr::Assign { var: target, src });
                    target
                }
                Expr::ArrayDimFetch { .. } => {
                    let val = self.expr(f, expr)?;
                    let (target, keys, n, append) = self.dim_path(f, var)?;
                    let dst = f.temp();
                    f.emit(Instr::AssignDim { var: target, keys, n, append, val, dst });
                    dst
                }
                other => return Err(self.unsupported(other.span(), "this assignment")),
            },
            Expr::AssignOp { var, op, expr, span } => {
                let Some(bin) = assign_binop(*op) else {
                    // `??=`
                    return Err(self.unsupported(*span, "??="));
                };
                match var {
                    Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => {
                        let target = f.cv(&self.text(*name)[1..]);
                        let src = self.expr(f, expr)?;
                        f.emit(Instr::AssignOp { op: bin, var: target, src });
                        target
                    }
                    Expr::ArrayDimFetch { dim: Some(_), .. } => {
                        let val = self.expr(f, expr)?;
                        let (target, keys, n, _) = self.dim_path(f, var)?;
                        let dst = f.temp();
                        f.emit(Instr::AssignDimOp { op: bin, var: target, keys, n, val, dst });
                        dst
                    }
                    other => return Err(self.unsupported(other.span(), "this compound assignment")),
                }
            }
            Expr::Binary { left, op, right, .. } => self.binary(f, left, *op, right, hint, mark)?,
            Expr::Unary { op, expr, span } => match op {
                UnaryOp::PreInc | UnaryOp::PreDec => {
                    let var = self.inc_target(f, expr)?;
                    let dst = f.temp();
                    f.emit(Instr::IncDec { var, dst, inc: matches!(op, UnaryOp::PreInc), post: false });
                    dst
                }
                UnaryOp::Not | UnaryOp::Minus | UnaryOp::BitNot | UnaryOp::Plus => {
                    let src = self.expr(f, expr)?;
                    let dst = out(f, hint, mark);
                    let instr = match op {
                        UnaryOp::Not => Instr::Not { dst, src },
                        UnaryOp::Minus => Instr::Neg { dst, src },
                        UnaryOp::BitNot => Instr::BitNot { dst, src },
                        _ => {
                            let one = f.constant(Frozen::Int(1));
                            Instr::Binary { op: BinOp::Mul, dst, a: src, b: one }
                        }
                    };
                    f.emit(instr);
                    dst
                }
                UnaryOp::ErrorSuppress => self.expr(f, expr)?,
                UnaryOp::Reference => return Err(self.unsupported(*span, "a reference")),
            },
            Expr::PostInc { var, .. } | Expr::PostDec { var, .. } => {
                let target = self.inc_target(f, var)?;
                let dst = f.temp();
                f.emit(Instr::IncDec { var: target, dst, inc: matches!(e, Expr::PostInc { .. }), post: true });
                dst
            }
            Expr::Ternary { condition, if_true, if_false, .. } => {
                let dst = hint.unwrap_or_else(|| f.temp());
                let keep = f.temps;
                match if_true {
                    Some(t) => {
                        let else_at = self.branch(f, condition, false)?;
                        f.temps = keep;
                        self.expr_into(f, t, dst)?;
                        f.temps = keep;
                        let end = f.emit(Instr::Jump { target: 0 });
                        f.patch_all(else_at, f.here());
                        self.expr_into(f, if_false, dst)?;
                        f.temps = keep;
                        f.patch(end, f.here());
                    }
                    None => {
                        // `a ?: b`
                        self.expr_into(f, condition, dst)?;
                        f.temps = keep;
                        let end = f.emit(Instr::JumpIf { cond: dst, target: 0 });
                        self.expr_into(f, if_false, dst)?;
                        f.temps = keep;
                        f.patch(end, f.here());
                    }
                }
                dst
            }
            Expr::Array { items, .. } => {
                let dst = f.temp();
                f.emit(Instr::NewArray { dst });
                for item in *items {
                    if item.unpack || item.by_ref {
                        return Err(self.unsupported(item.span, "spread or & in an array literal"));
                    }
                    let val = self.expr(f, item.value)?;
                    match item.key {
                        Some(k) => {
                            let key = self.expr(f, k)?;
                            f.emit(Instr::ArraySet { arr: dst, key, val });
                        }
                        None => {
                            f.emit(Instr::ArrayPush { arr: dst, val });
                        }
                    }
                }
                dst
            }
            Expr::ArrayDimFetch { array, dim, span } => {
                let Some(dim) = dim else {
                    return Err(CompileError { message: "Cannot use [] for reading".into(), line: self.line(*span) });
                };
                let arr = self.expr(f, array)?;
                let key = self.expr(f, dim)?;
                let dst = out(f, hint, mark);
                f.emit(Instr::FetchDim { dst, arr, key, quiet: false });
                dst
            }
            Expr::InterpolatedString { parts, .. } => {
                if parts.is_empty() {
                    return Ok(f.constant(Frozen::Str(Text::from(""))));
                }
                let start = f.temp();
                let regs: Vec<Reg> = std::iter::once(start).chain((1..parts.len()).map(|_| f.temp())).collect();
                for (r, p) in regs.iter().zip(parts.iter()) {
                    self.expr_into(f, p, *r)?;
                }
                let dst = f.temp();
                f.emit(Instr::ConcatN { dst, start, n: parts.len() as u8 });
                dst
            }
            Expr::Call { func, args, span } => self.call(f, func, args, *span, hint, mark)?,
            Expr::Print { expr, .. } => {
                let r = self.expr(f, expr)?;
                f.emit(Instr::Echo { src: r });
                f.constant(Frozen::Int(1))
            }
            Expr::Cast { kind, expr, span } => {
                let kind = match kind {
                    AstCast::Int => CastKind::Int,
                    AstCast::Float => CastKind::Float,
                    AstCast::String => CastKind::String,
                    AstCast::Bool => CastKind::Bool,
                    AstCast::Array => CastKind::Array,
                    AstCast::Unset => CastKind::Unset,
                    _ => return Err(self.unsupported(*span, "this cast")),
                };
                let src = self.expr(f, expr)?;
                let dst = out(f, hint, mark);
                f.emit(Instr::Cast { kind, dst, src });
                dst
            }
            Expr::Isset { vars, .. } => {
                let dst = f.temp();
                let mut ends = Vec::new();
                for (i, v) in vars.iter().enumerate() {
                    match v {
                        Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => {
                            let var = f.cv(&self.text(*name)[1..]);
                            f.emit(Instr::IssetVar { dst, var });
                        }
                        Expr::ArrayDimFetch { dim: Some(_), .. } => {
                            let (var, keys, n, _) = self.dim_path(f, v)?;
                            f.emit(Instr::IssetDim { dst, var, keys, n });
                        }
                        other => return Err(self.unsupported(other.span(), "this isset")),
                    }
                    if i + 1 < vars.len() {
                        ends.push(f.emit(Instr::JumpIfNot { cond: dst, target: 0 }));
                    }
                }
                for at in ends {
                    f.patch(at, f.here());
                }
                dst
            }
            Expr::Exit { expr, .. } | Expr::Die { expr, .. } => {
                match expr {
                    Some(x) => {
                        let src = self.expr(f, x)?;
                        f.emit(Instr::Exit { src, has: true });
                    }
                    None => {
                        let src = f.constant(Frozen::Null);
                        f.emit(Instr::Exit { src, has: false });
                    }
                }
                f.constant(Frozen::Null)
            }
            other => return Err(self.unsupported(other.span(), "this expression")),
        })
    }

    fn inc_target(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<Reg> {
        match e {
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => Ok(f.cv(&self.text(*name)[1..])),
            other => Err(self.unsupported(other.span(), "++/-- on this")),
        }
    }

    fn constant_fetch(&mut self, f: &mut FnBuilder, name: &[u8]) -> Reg {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        match name.to_ascii_lowercase().as_slice() {
            b"true" => return f.constant(Frozen::Bool(true)),
            b"false" => return f.constant(Frozen::Bool(false)),
            b"null" => return f.constant(Frozen::Null),
            _ => {}
        }
        let site = f.site(true, Text::from(name), Text::from(name));
        let dst = f.temp();
        f.emit(Instr::FetchConst { site, dst });
        dst
    }

    fn binary(&mut self, f: &mut FnBuilder, left: &Expr, op: BinaryOp, right: &Expr, hint: Option<Reg>, mark: usize) -> Result<Reg> {
        let arith = |op: BinaryOp| -> Option<BinOp> {
            Some(match op {
                BinaryOp::Plus => BinOp::Add,
                BinaryOp::Minus => BinOp::Sub,
                BinaryOp::Mul => BinOp::Mul,
                BinaryOp::Div => BinOp::Div,
                BinaryOp::Mod => BinOp::Mod,
                BinaryOp::Pow => BinOp::Pow,
                BinaryOp::Concat => BinOp::Concat,
                BinaryOp::BitAnd => BinOp::BitAnd,
                BinaryOp::BitOr => BinOp::BitOr,
                BinaryOp::BitXor => BinOp::BitXor,
                BinaryOp::ShiftLeft => BinOp::Shl,
                BinaryOp::ShiftRight => BinOp::Shr,
                _ => return None,
            })
        };
        if let Some(bin) = arith(op) {
            let a = self.expr(f, left)?;
            let b = self.expr(f, right)?;
            let dst = out(f, hint, mark);
            f.emit(Instr::binary(bin, dst, a, b));
            return Ok(dst);
        }
        let cmp = match op {
            BinaryOp::Spaceship => Some(Cmp::Spaceship),
            other => comparison(other),
        };
        if let Some(cmp) = cmp {
            let a = self.expr(f, left)?;
            let b = self.expr(f, right)?;
            let dst = out(f, hint, mark);
            f.emit(Instr::Compare { cmp, dst, a, b });
            return Ok(dst);
        }
        let dst = f.temp();
        match op {
            BinaryOp::And | BinaryOp::LogicalAnd | BinaryOp::Or | BinaryOp::LogicalOr => {
                let is_and = matches!(op, BinaryOp::And | BinaryOp::LogicalAnd);
                let a = self.expr(f, left)?;
                f.emit(Instr::ToBool { dst, src: a });
                let end = f.emit(if is_and {
                    Instr::JumpIfNot { cond: dst, target: 0 }
                } else {
                    Instr::JumpIf { cond: dst, target: 0 }
                });
                let b = self.expr(f, right)?;
                f.emit(Instr::ToBool { dst, src: b });
                f.patch(end, f.here());
            }
            BinaryOp::LogicalXor => {
                let a = self.expr(f, left)?;
                let b = self.expr(f, right)?;
                let x = f.temp();
                let y = f.temp();
                f.emit(Instr::ToBool { dst: x, src: a });
                f.emit(Instr::ToBool { dst: y, src: b });
                f.emit(Instr::Compare { cmp: Cmp::NotIdentical, dst, a: x, b: y });
            }
            BinaryOp::Coalesce => {
                let a = self.quiet(f, left)?;
                let end = f.emit(Instr::JumpIfSet { src: a, dst, target: 0 });
                self.expr_into(f, right, dst)?;
                f.patch(end, f.here());
            }
            _ => return Err(self.unsupported(left.span(), "this operator")),
        }
        Ok(dst)
    }

    /// `e` read without "undefined" warnings (the left of ??, isset).
    fn quiet(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<Reg> {
        match e {
            Expr::ArrayDimFetch { array, dim: Some(dim), .. } => {
                let arr = self.quiet(f, array)?;
                let key = self.expr(f, dim)?;
                let dst = f.temp();
                f.emit(Instr::FetchDim { dst, arr, key, quiet: true });
                Ok(dst)
            }
            _ => self.expr(f, e),
        }
    }

    fn call(&mut self, f: &mut FnBuilder, func: &Expr, args: &[p8_syntax::ast::Arg], span: Span, hint: Option<Reg>, mark: usize) -> Result<Reg> {
        let Expr::Variable { name, .. } = func else {
            return Err(self.unsupported(span, "this call"));
        };
        let text = self.text(*name);
        if text.starts_with(b"$") {
            return Err(self.unsupported(span, "a variable function call"));
        }
        let text = text.strip_prefix(b"\\").unwrap_or(text);
        let lower = text.to_ascii_lowercase();
        // strlen of a literal string is a constant, as PHP compiles it
        // (a builtin cannot be redeclared, so the name means strlen).
        if lower == b"strlen"
            && let [arg] = args
            && !arg.unpack
            && arg.name.is_none()
            && let Some(Frozen::Str(s)) = self.literal(arg.value)
        {
            return Ok(f.constant(Frozen::Int(s.as_bytes().len() as i64)));
        }
        // Whether argument n is passed by reference, when the callee is known now.
        let by_ref = |n: usize| -> Option<bool> {
            if let Some((modes, rest)) = self.by_ref.get(&lower) {
                return Some(modes.get(n).copied().unwrap_or(*rest));
            }
            self.registry.lookup(&lower).map(|i| self.registry.by_ref(i, n))
        };
        let modes: Vec<Option<bool>> = (0..args.len()).map(by_ref).collect();
        let site = f.site(false, Text::from(lower), Text::from(text));
        let regs: Vec<Reg> = (0..args.len()).map(|_| f.temp()).collect();
        for (n, (arg, r)) in args.iter().zip(&regs).enumerate() {
            if arg.unpack || arg.name.is_some() {
                return Err(self.unsupported(arg.span, "spread or named arguments"));
            }
            match arg.value {
                Expr::Variable { name, .. }
                    if modes[n] != Some(false) && self.text(*name).starts_with(b"$") && self.text(*name) != b"$this" =>
                {
                    let var = f.cv(&self.text(*name)[1..]);
                    f.emit(Instr::SendVar { site, n: n as u8, arg: *r, var });
                }
                // Only a variable can be passed by reference yet.
                Expr::ArrayDimFetch { .. } | Expr::PropertyFetch { .. }
                    if modes[n] != Some(false) =>
                {
                    return Err(self.unsupported(arg.span, "passing this argument by reference"));
                }
                value => self.expr_into(f, value, *r)?,
            }
        }
        let base = regs.first().copied().unwrap_or_else(|| f.temp());
        // The arguments' registers are free again once the call has them:
        // the result may land in the first (the callee's frame is cleared
        // before its result is written).
        let dst = out(f, hint, mark);
        f.emit(Instr::Call { site, base, argc: args.len() as u8, dst });
        Ok(dst)
    }
}

/// Where an instruction's result goes: `hint`, or a temporary that reuses
/// those of its operands (`mark` is where they start; every instruction
/// reads its operands before it writes its result).
fn out(f: &mut FnBuilder, hint: Option<Reg>, mark: usize) -> Reg {
    f.temps = mark;
    hint.unwrap_or_else(|| f.temp())
}

fn comparison(op: BinaryOp) -> Option<Cmp> {
    Some(match op {
        BinaryOp::EqEq => Cmp::Eq,
        BinaryOp::NotEq => Cmp::NotEq,
        BinaryOp::EqEqEq => Cmp::Identical,
        BinaryOp::NotEqEq => Cmp::NotIdentical,
        BinaryOp::Lt => Cmp::Lt,
        BinaryOp::LtEq => Cmp::Le,
        BinaryOp::Gt => Cmp::Gt,
        BinaryOp::GtEq => Cmp::Ge,
        _ => return None,
    })
}

fn assign_binop(op: AssignOp) -> Option<BinOp> {
    Some(match op {
        AssignOp::Plus => BinOp::Add,
        AssignOp::Minus => BinOp::Sub,
        AssignOp::Mul => BinOp::Mul,
        AssignOp::Div => BinOp::Div,
        AssignOp::Mod => BinOp::Mod,
        AssignOp::Concat => BinOp::Concat,
        AssignOp::BitAnd => BinOp::BitAnd,
        AssignOp::BitOr => BinOp::BitOr,
        AssignOp::BitXor => BinOp::BitXor,
        AssignOp::ShiftLeft => BinOp::Shl,
        AssignOp::ShiftRight => BinOp::Shr,
        AssignOp::Pow => BinOp::Pow,
        AssignOp::Coalesce => return None,
    })
}

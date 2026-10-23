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

use p8_bytecode::{CastKind, ClassDef, ClassRef, Cmp, ConstDef, Function, Handler, IncludeKind, Instr, MethodDef, Param, PropDef, Reg, Site};
use p8_runtime::object::{CLASS_ABSTRACT, CLASS_FINAL, CLASS_INTERFACE, CLASS_READONLY, Visibility};
use p8_runtime::ext::Registry;
use p8_runtime::ops::BinOp;
use p8_runtime::frozen::{Frozen, Text};
use p8_syntax::literal::{self, Number};
use p8_syntax::ast::{AssignOp, BinaryOp, CastKind as AstCast, Expr, MagicConstKind, Stmt, StmtId, UnaryOp};
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

/// A `try` body (or its `catch` bodies, when it has a `finally`) being
/// compiled. Leaving it by `return`, `break` or `continue` runs its
/// `finally` there and then (the code is compiled in place), so the
/// range it protects is the code around those copies: its segments.
struct TryCtx<'a> {
    finally: Option<&'a [StmtId<'a>]>,
    /// Loops open when it started (break/continue out of them leave it).
    loops: usize,
    open: Option<u32>,
    segments: Vec<(u32, u32)>,
}

impl TryCtx<'_> {
    fn close(&mut self, at: u32) {
        if let Some(start) = self.open.take()
            && at > start
        {
            self.segments.push((start, at));
        }
    }
}

/// One function being compiled.
struct FnBuilder<'a> {
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
    /// Classes the code names, and their indices.
    classes: Vec<ClassRef>,
    class_index: HashMap<Vec<u8>, u16>,
    /// For a method: its class, as declared.
    class: Option<Text>,
    is_static: bool,
    tries: Vec<TryCtx<'a>>,
    handlers: Vec<Handler>,
    /// The classes declared in this function's code (`DeclareClass`).
    decls: Vec<Arc<ClassDef>>,
}

impl FnBuilder<'_> {
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
            classes: Vec::new(),
            class_index: HashMap::new(),
            class: None,
            is_static: false,
            tries: Vec::new(),
            handlers: Vec::new(),
            decls: Vec::new(),
        }
    }

    /// The index of a class the code names (one entry per class).
    fn class_ref(&mut self, class: ClassRef) -> u16 {
        let key = match &class {
            ClassRef::Named(lower, _) => lower.as_bytes().to_vec(),
            ClassRef::Forward(lower, _) => [b"forward:", lower.as_bytes()].concat(),
            ClassRef::Static => b"static".to_vec(),
        };
        let next = self.classes.len() as u16;
        let i = *self.class_index.entry(key).or_insert(next);
        if i == next {
            self.classes.push(class);
        }
        i
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
            // Arrays are not shared between constant registers.
            Frozen::Array(_) => {
                self.consts.push(value);
                return reg(CONST, self.consts.len() - 1);
            }
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
        for h in &mut self.handlers {
            h.slot = map(h.slot);
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
            classes: self.classes,
            class: self.class,
            is_static: self.is_static,
            handlers: self.handlers,
            decls: self.decls,
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

/// The name of a syntax node's kind (`Class`, `Try`, `Closure`, ...), for
/// "cannot compile" errors: what the php-src runner counts per feature.
fn kind(node: &impl std::fmt::Debug) -> String {
    let text = format!("{node:?}");
    text.split(|c: char| !c.is_alphanumeric()).next().unwrap_or_default().to_string()
}

/// Whether a dimension write's root is a property (`$o->p[k] = v`).
fn dim_root_is_prop(e: &Expr) -> bool {
    let mut cur = e;
    while let Expr::ArrayDimFetch { array, .. } = cur {
        cur = array;
    }
    matches!(cur, Expr::PropertyFetch { .. })
}

/// The visibility a member's modifiers give it (None: none written).
fn visibility_of(c: &Compiler, modifiers: &[p8_syntax::lexer::token::Token]) -> Option<Visibility> {
    modifiers.iter().find_map(|m| match c.text(m.span).to_ascii_lowercase().as_slice() {
        b"public" | b"var" => Some(Visibility::Public),
        b"protected" => Some(Visibility::Protected),
        b"private" => Some(Visibility::Private),
        _ => None,
    })
}

/// A declared type as PHP prints it (`?int`, `A|B`).
fn type_text(c: &Compiler, t: &p8_syntax::ast::Type) -> Vec<u8> {
    use p8_syntax::ast::Type;
    match t {
        Type::Simple(token) => c.text(token.span).to_vec(),
        Type::Name(name) => {
            let text = c.text(name.span);
            text.strip_prefix(b"\\").unwrap_or(text).to_vec()
        }
        Type::Nullable(inner) => [b"?".to_vec(), type_text(c, inner)].concat(),
        Type::Union(parts) => parts.iter().map(|p| type_text(c, p)).collect::<Vec<_>>().join(&b'|'),
        Type::Intersection(parts) => parts.iter().map(|p| type_text(c, p)).collect::<Vec<_>>().join(&b'&'),
    }
}

/// A class member's span, for errors.
fn other_span(m: &p8_syntax::ast::ClassMember) -> Span {
    use p8_syntax::ast::ClassMember::*;
    match m {
        Property { span, .. } | PropertyHook { span, .. } | Method { span, .. } | Const { span, .. } | TraitUse { span, .. } | Case { span, .. } => *span,
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

/// Reopen the try blocks `leave_tries` left (the code after the exit is
/// inside them again).
fn reenter_tries<'a>(f: &mut FnBuilder<'a>, left: Vec<TryCtx<'a>>) {
    for mut t in left.into_iter().rev() {
        t.open = Some(f.here());
        f.tries.push(t);
    }
}

/// Apply `f` to every register an instruction names.
fn map_regs(instr: &mut Instr, f: impl Fn(Reg) -> Reg) {
    use Instr::*;
    match instr {
        DeclareClass { .. } => {}
        Silence { saved } | Unsilence { saved } => *saved = f(*saved),
        Include { dst, src, .. } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        New { dst, base, .. } | StaticCall { dst, base, .. } => {
            *dst = f(*dst);
            *base = f(*base);
        }
        MethodCall { dst, obj, base, .. } => {
            *dst = f(*dst);
            *obj = f(*obj);
            *base = f(*base);
        }
        This { dst } => *dst = f(*dst),
        FetchProp { dst, obj, name, .. } | IssetProp { dst, obj, name } => {
            *dst = f(*dst);
            *obj = f(*obj);
            *name = f(*name);
        }
        AssignProp { obj, name, val, dst } | AssignPropOp { obj, name, val, dst, .. } => {
            *obj = f(*obj);
            *name = f(*name);
            *val = f(*val);
            *dst = f(*dst);
        }
        PropIncDec { obj, name, dst, .. } => {
            *obj = f(*obj);
            *name = f(*name);
            *dst = f(*dst);
        }
        AssignPropDim { base, val, dst, .. } => {
            *base = f(*base);
            *val = f(*val);
            *dst = f(*dst);
        }
        UnsetProp { obj, name } => {
            *obj = f(*obj);
            *name = f(*name);
        }
        ClassConst { dst, name, .. } | FetchStaticProp { dst, name, .. } => {
            *dst = f(*dst);
            *name = f(*name);
        }
        AssignStaticProp { name, val, dst, .. } | AssignStaticPropOp { name, val, dst, .. } => {
            *name = f(*name);
            *val = f(*val);
            *dst = f(*dst);
        }
        InstanceOf { dst, src, .. } | Clone { dst, src } => {
            *dst = f(*dst);
            *src = f(*src);
        }
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
        Return { src } | Echo { src } | Exit { src, .. } | MatchError { src } | Throw { src } | CatchIf { src, .. } => {
            *src = f(*src)
        }
        FetchConst { dst, .. } => *dst = f(*dst),
        ConcatN { dst, start, .. } => {
            *dst = f(*dst);
            *start = f(*start);
        }
        BindGlobal { var, .. } => *var = f(*var),
    }
}

pub use p8_bytecode::Program;

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
    /// Every class the file declares, in order.
    classes: Vec<Arc<ClassDef>>,
    /// The class whose members are being compiled (for self and parent).
    scope: Option<ClassScope>,
    /// Set while compiling a method: its class and whether it is static.
    method: Option<(Text, bool)>,
    /// Properties a constructor's parameters promoted (`public int $x`).
    promoted: Vec<PropDef>,
}

/// The class being compiled.
struct ClassScope {
    name: Text,
    parent: Option<Text>,
}

type Result<T> = std::result::Result<T, CompileError>;

impl<'src> Compiler<'src> {
    pub fn new(source: &'src [u8], file: &str, registry: &'src Registry) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(source.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i + 1));
        Compiler {
            source,
            registry,
            file: Arc::from(file),
            line_starts,
            functions: Vec::new(),
            by_ref: HashMap::new(),
            classes: Vec::new(),
            scope: None,
            method: None,
            promoted: Vec::new(),
        }
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
        // A file's code ends returning 1 (what including it gives).
        let one = f.constant(Frozen::Int(1));
        f.emit(Instr::Return { src: one });
        let main = Arc::new(f.finish(self.file.clone()));
        Ok(Program { main, functions: self.functions, classes: self.classes })
    }

    fn stmts<'a>(&mut self, f: &mut FnBuilder<'a>, stmts: &'a [StmtId<'a>]) -> Result<()> {
        for stmt in stmts {
            self.stmt(f, stmt)?;
            f.temps = f.floor;
        }
        Ok(())
    }

    fn stmt<'a>(&mut self, f: &mut FnBuilder<'a>, stmt: &'a Stmt<'a>) -> Result<()> {
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
            Stmt::Return { expr, .. } if f.tries.iter().any(|t| t.finally.is_some()) => {
                // The value is taken first; then each `finally` runs.
                let value = f.temp();
                match expr {
                    Some(e) => self.expr_into(f, e, value)?,
                    None => {
                        f.emit(Instr::LoadNull { dst: value });
                    }
                }
                let n = f.tries.len();
                let left = self.leave_tries(f, n)?;
                f.emit(Instr::Return { src: value });
                reenter_tries(f, left);
            }
            Stmt::Throw { expr, .. } => {
                let r = self.expr(f, expr)?;
                f.emit(Instr::Throw { src: r });
            }
            Stmt::Try { body, catches, finally, .. } => self.try_stmt(f, body, catches, *finally)?,
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
                let target = f.loops.len() - depth;
                // `try` blocks inside the loop it leaves run their `finally`.
                let n = f.tries.iter().rev().take_while(|t| t.loops > target).count();
                let left = self.leave_tries(f, n)?;
                let at = f.emit(Instr::Jump { target: 0 });
                reenter_tries(f, left);
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
            // `case` values are compared with ==, in order, until one
            // matches; then the bodies run from there (falling through).
            // A switch is a loop level for break and continue: `continue`
            // targeting it acts as `break`, as in PHP.
            Stmt::Switch { condition, cases, .. } => {
                let subject = self.expr(f, condition)?;
                let keep = f.temps;
                let mut to_body = Vec::with_capacity(cases.len());
                for case in *cases {
                    match case.condition {
                        Some(c) => {
                            let value = self.expr(f, c)?;
                            to_body.push(Some(f.emit(Instr::jump_cmp(Cmp::Eq, subject, value, true, 0))));
                            f.temps = keep;
                        }
                        None => to_body.push(None),
                    }
                }
                let no_match = f.emit(Instr::Jump { target: 0 });
                f.temps = f.floor;
                f.loops.push(Loop { breaks: Vec::new(), continues: Vec::new() });
                let mut default = None;
                for (case, jump) in cases.iter().zip(to_body) {
                    match jump {
                        Some(at) => f.patch(at, f.here()),
                        None => default = Some(f.here()),
                    }
                    self.stmts(f, case.body)?;
                    f.temps = f.floor;
                }
                let end = f.here();
                f.patch(no_match, default.unwrap_or(end));
                let lp = f.loops.pop().expect("the switch's level");
                f.patch_all(lp.breaks, end);
                f.patch_all(lp.continues, end);
            }
            // declare(strict_types=1), ticks, encoding: p8 has nothing yet
            // that they change, so only the body (if any) runs.
            Stmt::Declare { body, .. } => self.stmts(f, body)?,
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
                        Expr::PropertyFetch { target, property, .. } => {
                            let obj = self.expr(f, target)?;
                            let name = self.prop_name(f, property)?;
                            f.emit(Instr::UnsetProp { obj, name });
                        }
                        other => return Err(self.unsupported(other.span(), "this unset")),
                    }
                }
            }
            Stmt::Class { modifiers, name, extends, implements, members, .. } => {
                self.class_decl(f, name.span, modifiers, extends.as_ref(), implements, members, false)?;
            }
            Stmt::Interface { name, extends, members, .. } => {
                self.class_decl(f, name.span, &[], None, extends, members, true)?;
            }
            Stmt::Nop { .. } => {}
            other => return Err(self.unsupported(other.span(), &format!("this statement ({})", kind(other)))),
        }
        Ok(())
    }

    /// Compile `e` for its effects only.
    fn discard(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<()> {
        match e {
            Expr::PostInc { var: Expr::PropertyFetch { .. } | Expr::ClassConstFetch { .. }, .. }
            | Expr::PostDec { var: Expr::PropertyFetch { .. } | Expr::ClassConstFetch { .. }, .. }
            | Expr::Unary {
                op: UnaryOp::PreInc | UnaryOp::PreDec,
                expr: Expr::PropertyFetch { .. } | Expr::ClassConstFetch { .. },
                ..
            } => {
                self.expr(f, e)?;
            }
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

    /// Leave the innermost `n` try blocks (for a return, break or
    /// continue): each one's protected range ends here and its `finally`
    /// runs, innermost first. The caller emits the exit, then reenters.
    fn leave_tries<'a>(&mut self, f: &mut FnBuilder<'a>, n: usize) -> Result<Vec<TryCtx<'a>>> {
        let mut left = Vec::with_capacity(n);
        if !f.tries.iter().rev().take(n).any(|t| t.finally.is_some()) {
            return Ok(left);
        }
        // What the exit uses (a return's value) stays live.
        let outer_floor = std::mem::replace(&mut f.floor, f.temps);
        for _ in 0..n {
            let mut t = f.tries.pop().expect("a try block to leave");
            t.close(f.here());
            if let Some(body) = t.finally {
                self.stmts(f, body)?;
            }
            left.push(t);
        }
        f.floor = outer_floor;
        Ok(left)
    }

    /// `try { } catch (T $e) { } finally { }`. The body is protected by a
    /// handler going to the catch tests; with a `finally`, the catch
    /// bodies by one going to the `finally` that throws on. A normal exit
    /// runs the `finally` and goes on.
    fn try_stmt<'a>(
        &mut self,
        f: &mut FnBuilder<'a>,
        body: &'a [StmtId<'a>],
        catches: &'a [p8_syntax::ast::Catch<'a>],
        finally: Option<&'a [StmtId<'a>]>,
    ) -> Result<()> {
        let slot = f.temp();
        let outer_floor = std::mem::replace(&mut f.floor, f.temps);
        let depth = f.tries.len() as u16;
        let open = |f: &mut FnBuilder<'a>| {
            let ctx = TryCtx { finally, loops: f.loops.len(), open: Some(f.here()), segments: Vec::new() };
            f.tries.push(ctx);
        };
        open(f);
        self.stmts(f, body)?;
        let mut protected = f.tries.pop().expect("the try block");
        protected.close(f.here());
        let mut to_finally = vec![f.emit(Instr::Jump { target: 0 })];
        let tests = f.here();
        let mut matched = Vec::with_capacity(catches.len());
        for c in catches {
            let mut jumps = Vec::with_capacity(c.types.len());
            for ty in c.types {
                let class = self.class_name_ref(f, ty)?;
                jumps.push(f.emit(Instr::CatchIf { src: slot, class, target: 0 }));
            }
            matched.push(jumps);
        }
        let no_match = f.emit(Instr::Jump { target: 0 });
        if finally.is_some() {
            open(f);
        }
        for (c, jumps) in catches.iter().zip(matched) {
            f.patch_all(jumps, f.here());
            f.line = self.line(c.span);
            if let Some(var) = c.var {
                let var = f.cv(&self.text(var.span)[1..]);
                f.emit(Instr::Assign { var, src: slot });
            }
            f.emit(Instr::UnsetVar { var: slot });
            self.stmts(f, c.body)?;
            to_finally.push(f.emit(Instr::Jump { target: 0 }));
        }
        let catching = if finally.is_some() {
            let mut c = f.tries.pop().expect("the catch blocks");
            c.close(f.here());
            c.segments
        } else {
            Vec::new()
        };
        // An exception no catch took (or one thrown in a catch): the
        // finally, then it goes on.
        let rethrow = f.here();
        f.patch(no_match, rethrow);
        if let Some(body) = finally {
            self.stmts(f, body)?;
        }
        f.emit(Instr::Throw { src: slot });
        f.patch_all(to_finally, f.here());
        if let Some(body) = finally {
            self.stmts(f, body)?;
        }
        let first = if catches.is_empty() { rethrow } else { tests };
        for (start, end) in protected.segments {
            f.handlers.push(Handler { start, end, target: first, slot, depth });
        }
        for (start, end) in catching {
            f.handlers.push(Handler { start, end, target: rethrow, slot, depth });
        }
        f.floor = outer_floor;
        Ok(())
    }

    fn loop_body<'a>(&mut self, f: &mut FnBuilder<'a>, body: &'a [StmtId<'a>], _start: u32) -> Result<()> {
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
        if let Some((class, is_static)) = self.method.take() {
            f.class = Some(class);
            f.is_static = is_static;
        }
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
        // Constructor promotion: `public int $x` declares the property and
        // assigns the argument to it first thing.
        for (i, p) in params.iter().enumerate() {
            let Some(visibility) = visibility_of(self, p.modifiers) else { continue };
            let text = self.text(p.name.span);
            let name = Text::from(&text[1..]);
            self.promoted.push(PropDef {
                name: name.clone(),
                default: if p.ty.is_some() { None } else { Some(Frozen::Null) },
                visibility,
                is_static: false,
                readonly: p.modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"readonly")),
                ty: p.ty.map(|t| Text::from(type_text(self, t).as_slice())),
            });
            let obj = f.temp();
            f.emit(Instr::This { dst: obj });
            let key = f.constant(Frozen::Str(name));
            let dst = f.temp();
            f.emit(Instr::AssignProp { obj, name: key, val: reg(CV, i), dst });
            f.temps = f.floor;
        }
        self.stmts(&mut f, body)?;
        f.emit(Instr::ReturnNull);
        Ok(f.finish(self.file.clone()))
    }

    /// A class or interface declaration: compiled once, linked per request.
    #[allow(clippy::too_many_arguments)] // one per part of the declaration's syntax
    fn class_decl(
        &mut self,
        f: &mut FnBuilder,
        name: Span,
        modifiers: &[p8_syntax::lexer::token::Token],
        extends: Option<&p8_syntax::ast::Name>,
        implements: &[p8_syntax::ast::Name],
        members: &[p8_syntax::ast::ClassMember],
        interface: bool,
    ) -> Result<()> {
        use p8_syntax::ast::ClassMember;
        let display = Text::from(self.text(name));
        let name_of = |c: &Self, n: &p8_syntax::ast::Name| {
            let t = c.text(n.span);
            Text::from(t.strip_prefix(b"\\").unwrap_or(t))
        };
        let (parent, interfaces) = if interface {
            (None, extends.iter().map(|n| name_of(self, n)).chain(implements.iter().map(|n| name_of(self, n))).collect())
        } else {
            (extends.map(|n| name_of(self, n)), implements.iter().map(|n| name_of(self, n)).collect())
        };
        let mut flags = if interface { CLASS_INTERFACE } else { 0 };
        for m in modifiers {
            match self.text(m.span).to_ascii_lowercase().as_slice() {
                b"abstract" => flags |= CLASS_ABSTRACT,
                b"final" => flags |= CLASS_FINAL,
                b"readonly" => flags |= CLASS_READONLY,
                _ => {}
            }
        }
        let saved = self.scope.replace(ClassScope { name: display.clone(), parent: parent.clone() });
        let (mut constants, mut properties, mut methods) = (Vec::new(), Vec::new(), Vec::new());
        for member in members {
            match member {
                ClassMember::Property { modifiers, ty, entries, .. } => {
                    let words: Vec<Vec<u8>> = modifiers.iter().map(|m| self.text(m.span).to_ascii_lowercase()).collect();
                    let has = |w: &[u8]| words.iter().any(|x| x == w);
                    for e in *entries {
                        let text = self.text(e.name.span);
                        let default = match e.default {
                            Some(x) => Some(self.const_value(x)?),
                            None if ty.is_some() => None,
                            None => Some(Frozen::Null),
                        };
                        properties.push(PropDef {
                            name: Text::from(text.strip_prefix(b"$").unwrap_or(text)),
                            default,
                            visibility: visibility_of(self, modifiers).unwrap_or(Visibility::Public),
                            is_static: has(b"static"),
                            readonly: has(b"readonly") || flags & CLASS_READONLY != 0,
                            ty: ty.map(|t| Text::from(type_text(self, t).as_slice())),
                        });
                    }
                }
                ClassMember::Const { modifiers, consts, .. } => {
                    for c in *consts {
                        constants.push(ConstDef {
                            name: Text::from(self.text(c.name.span)),
                            value: self.const_value(c.value)?,
                            visibility: visibility_of(self, modifiers).unwrap_or(Visibility::Public),
                            is_final: modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"final")),
                        });
                    }
                }
                ClassMember::Method { modifiers, by_ref, name, params, body, .. } => {
                    let words: Vec<Vec<u8>> = modifiers.iter().map(|m| self.text(m.span).to_ascii_lowercase()).collect();
                    let has = |w: &[u8]| words.iter().any(|x| x == w);
                    let is_static = has(b"static");
                    self.method = Some((display.clone(), is_static));
                    let func = self.function(name.span, params, body, *by_ref)?;
                    properties.append(&mut self.promoted);
                    methods.push(MethodDef {
                        func: Arc::new(func),
                        visibility: visibility_of(self, modifiers).unwrap_or(Visibility::Public),
                        is_static,
                        is_abstract: has(b"abstract") || interface,
                        is_final: has(b"final"),
                    });
                }
                other => {
                    self.scope = saved;
                    return Err(self.unsupported(other_span(other), &format!("this class member ({})", kind(other))));
                }
            }
        }
        self.scope = saved;
        let def = ClassDef { name: display, parent, interfaces, flags, constants, properties, methods, line: self.line(name) };
        let def = Arc::new(def);
        self.classes.push(Arc::clone(&def));
        f.decls.push(def);
        f.emit(Instr::DeclareClass { class: (f.decls.len() - 1) as u16 });
        Ok(())
    }

    /// A constant expression in a declaration (a property default, a class
    /// constant): literals, and arrays of them, so far.
    fn const_value(&self, e: &Expr) -> Result<Frozen> {
        if let Some(v) = self.literal(e) {
            return Ok(v);
        }
        match e {
            Expr::Unary { op: op @ (UnaryOp::Minus | UnaryOp::Plus), expr, .. } => match (op, self.const_value(expr)?) {
                (UnaryOp::Minus, Frozen::Int(i)) => Ok(i.checked_neg().map_or(Frozen::Float(-(i as f64)), Frozen::Int)),
                (UnaryOp::Minus, Frozen::Float(x)) => Ok(Frozen::Float(-x)),
                (UnaryOp::Plus, v @ (Frozen::Int(_) | Frozen::Float(_))) => Ok(v),
                _ => Err(self.unsupported(e.span(), "this constant expression")),
            },
            Expr::Array { items, .. } => {
                let mut entries = Vec::with_capacity(items.len());
                for item in *items {
                    if item.unpack || item.by_ref {
                        return Err(self.unsupported(e.span(), "this constant expression"));
                    }
                    let key = match item.key {
                        Some(k) => Some(self.const_value(k)?),
                        None => None,
                    };
                    entries.push((key, self.const_value(item.value)?));
                }
                Ok(Frozen::Array(entries.into()))
            }
            _ => Err(self.unsupported(e.span(), "this constant expression")),
        }
    }

    /// The index of the class a name expression names: `self` and
    /// `parent` resolved now, `static` when the code runs.
    fn class_ref(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<u16> {
        let Expr::Variable { name, .. } = e else {
            return Err(self.unsupported(e.span(), "a class name in an expression"));
        };
        let text = self.text(*name);
        if text.starts_with(b"$") {
            return Err(self.unsupported(e.span(), "a class name in a variable"));
        }
        let text = text.strip_prefix(b"\\").unwrap_or(text);
        let line = self.line(*name);
        let no_scope = |word: &str| CompileError { message: format!("Cannot use \"{word}\" when no class scope is active"), line };
        let class = match text.to_ascii_lowercase().as_slice() {
            b"self" => {
                let scope = self.scope.as_ref().map(|s| s.name.clone()).or_else(|| f.class.clone()).ok_or_else(|| no_scope("self"))?;
                ClassRef::Forward(Text::from(scope.as_bytes().to_ascii_lowercase().as_slice()), scope)
            }
            b"parent" => {
                let Some(scope) = &self.scope else { return Err(no_scope("parent")) };
                let Some(parent) = scope.parent.clone() else {
                    return Err(CompileError { message: "Cannot use \"parent\" when current class scope has no parent".into(), line });
                };
                ClassRef::Forward(Text::from(parent.as_bytes().to_ascii_lowercase().as_slice()), parent)
            }
            b"static" => {
                if self.scope.is_none() && f.class.is_none() {
                    return Err(no_scope("static"));
                }
                ClassRef::Static
            }
            lower => ClassRef::Named(Text::from(lower), Text::from(text)),
        };
        Ok(f.class_ref(class))
    }

    /// A class a `catch` names.
    fn class_name_ref(&mut self, f: &mut FnBuilder, name: &p8_syntax::ast::Name) -> Result<u16> {
        let text = self.text(name.span);
        let text = text.strip_prefix(b"\\").unwrap_or(text);
        Ok(f.class_ref(ClassRef::Named(Text::from(text.to_ascii_lowercase().as_slice()), Text::from(text))))
    }

    /// A property name: a constant for `->name`, the value for `->$name`
    /// or `->{expr}`.
    fn prop_name(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<Reg> {
        match e {
            Expr::Variable { name, .. } if !self.text(*name).starts_with(b"$") => {
                Ok(f.constant(Frozen::Str(Text::from(self.text(*name)))))
            }
            other => self.expr(f, other),
        }
    }

    /// Arguments by value into consecutive temporaries: (base, count).
    fn args_by_value(&mut self, f: &mut FnBuilder, args: &[p8_syntax::ast::Arg]) -> Result<(Reg, u8)> {
        let regs: Vec<Reg> = (0..args.len().max(1)).map(|_| f.temp()).collect();
        for (arg, r) in args.iter().zip(&regs) {
            if arg.unpack || arg.name.is_some() {
                return Err(self.unsupported(arg.span, "spread or named arguments"));
            }
            self.expr_into(f, arg.value, *r)?;
        }
        Ok((regs[0], args.len() as u8))
    }

    /// `$obj->name[k1]..[kn]`: the object, the name and the keys in
    /// consecutive temporaries from the returned base; the key count and
    /// whether the last dimension is `[]`.
    fn prop_dim_path(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<(Reg, u8, bool)> {
        let mut dims = Vec::new();
        let mut cur = e;
        while let Expr::ArrayDimFetch { array, dim, .. } = cur {
            dims.push(*dim);
            cur = array;
        }
        dims.reverse();
        let Expr::PropertyFetch { target, property, .. } = cur else {
            return Err(self.unsupported(cur.span(), "this array write"));
        };
        let append = dims.last().is_some_and(|d| d.is_none());
        if dims[..dims.len() - usize::from(append)].iter().any(|d| d.is_none()) {
            return Err(self.unsupported(e.span(), "[] before the last dimension"));
        }
        let keyed: Vec<&Expr> = dims.iter().flatten().copied().collect();
        let base = f.temp();
        let name = f.temp();
        let keys: Vec<Reg> = keyed.iter().map(|_| f.temp()).collect();
        self.expr_into(f, target, base)?;
        let n = self.prop_name(f, property)?;
        f.emit(Instr::Move { dst: name, src: n });
        for (r, d) in keys.iter().zip(&keyed) {
            self.expr_into(f, d, *r)?;
        }
        Ok((base, keys.len() as u8, append))
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
                        let _ = span;
                        let dst = out(f, hint, mark);
                        f.emit(Instr::This { dst });
                        return Ok(dst);
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
                Expr::ArrayDimFetch { .. } if dim_root_is_prop(var) => {
                    let (base, n, append) = self.prop_dim_path(f, var)?;
                    let val = self.expr(f, expr)?;
                    let dst = f.temp();
                    f.emit(Instr::AssignPropDim { op: None, base, n, append, val, dst });
                    dst
                }
                Expr::ArrayDimFetch { .. } => {
                    let val = self.expr(f, expr)?;
                    let (target, keys, n, append) = self.dim_path(f, var)?;
                    let dst = f.temp();
                    f.emit(Instr::AssignDim { var: target, keys, n, append, val, dst });
                    dst
                }
                Expr::PropertyFetch { target, property, .. } => {
                    let obj = self.expr(f, target)?;
                    let name = self.prop_name(f, property)?;
                    let val = self.expr(f, expr)?;
                    let dst = f.temp();
                    f.emit(Instr::AssignProp { obj, name, val, dst });
                    dst
                }
                Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                    let class = self.class_ref(f, class)?;
                    let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                    let val = self.expr(f, expr)?;
                    let dst = f.temp();
                    f.emit(Instr::AssignStaticProp { class, name, val, dst });
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
                    Expr::ArrayDimFetch { dim: Some(_), .. } if !dim_root_is_prop(var) => {
                        let val = self.expr(f, expr)?;
                        let (target, keys, n, _) = self.dim_path(f, var)?;
                        let dst = f.temp();
                        f.emit(Instr::AssignDimOp { op: bin, var: target, keys, n, val, dst });
                        dst
                    }
                    Expr::ArrayDimFetch { .. } if dim_root_is_prop(var) => {
                        let (base, n, append) = self.prop_dim_path(f, var)?;
                        let val = self.expr(f, expr)?;
                        let dst = f.temp();
                        f.emit(Instr::AssignPropDim { op: Some(bin), base, n, append, val, dst });
                        dst
                    }
                    Expr::PropertyFetch { target, property, .. } => {
                        let obj = self.expr(f, target)?;
                        let name = self.prop_name(f, property)?;
                        let val = self.expr(f, expr)?;
                        let dst = f.temp();
                        f.emit(Instr::AssignPropOp { op: bin, obj, name, val, dst });
                        dst
                    }
                    Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                        let class = self.class_ref(f, class)?;
                        let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                        let val = self.expr(f, expr)?;
                        let dst = f.temp();
                        f.emit(Instr::AssignStaticPropOp { op: bin, class, name, val, dst });
                        dst
                    }
                    other => return Err(self.unsupported(other.span(), "this compound assignment")),
                }
            }
            Expr::Binary { left, op, right, .. } => self.binary(f, left, *op, right, hint, mark)?,
            Expr::Unary { op: op @ (UnaryOp::PreInc | UnaryOp::PreDec), expr: Expr::PropertyFetch { target, property, .. }, .. } => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name(f, property)?;
                let dst = f.temp();
                f.emit(Instr::PropIncDec { obj, name, dst, inc: matches!(op, UnaryOp::PreInc), post: false });
                dst
            }
            Expr::PostInc { var: Expr::PropertyFetch { target, property, .. }, .. }
            | Expr::PostDec { var: Expr::PropertyFetch { target, property, .. }, .. } => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name(f, property)?;
                let dst = f.temp();
                f.emit(Instr::PropIncDec { obj, name, dst, inc: matches!(e, Expr::PostInc { .. }), post: true });
                dst
            }
            // `++C::$x`, `C::$x--`: a fetch and a compound assignment of 1.
            Expr::Unary { op: op @ (UnaryOp::PreInc | UnaryOp::PreDec), expr: Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. }, .. }
                if self.text(*name).starts_with(b"$") =>
            {
                let class = self.class_ref(f, class)?;
                let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                let one = f.constant(Frozen::Int(1));
                let dst = f.temp();
                let op = if matches!(op, UnaryOp::PreInc) { BinOp::Add } else { BinOp::Sub };
                f.emit(Instr::AssignStaticPropOp { op, class, name, val: one, dst });
                dst
            }
            Expr::PostInc { var: Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. }, .. }
            | Expr::PostDec { var: Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. }, .. }
                if self.text(*name).starts_with(b"$") =>
            {
                let class = self.class_ref(f, class)?;
                let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                let one = f.constant(Frozen::Int(1));
                let dst = f.temp();
                let scratch = f.temp();
                f.emit(Instr::FetchStaticProp { dst, class, name });
                let op = if matches!(e, Expr::PostInc { .. }) { BinOp::Add } else { BinOp::Sub };
                f.emit(Instr::AssignStaticPropOp { op, class, name, val: one, dst: scratch });
                dst
            }
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
                // `@expr`: diagnostics are off while it runs.
                UnaryOp::ErrorSuppress => {
                    let saved = f.temp();
                    f.emit(Instr::Silence { saved });
                    let mut r = self.expr(f, expr)?;
                    // A variable is read here, inside, not where it is used.
                    if r >> KIND_SHIFT == CV {
                        let t = f.temp();
                        f.emit(Instr::Move { dst: t, src: r });
                        r = t;
                    }
                    f.emit(Instr::Unsilence { saved });
                    r
                }
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
            Expr::MagicConst { kind, span } => {
                let text: Vec<u8> = match kind {
                    MagicConstKind::Line => return Ok(f.constant(Frozen::Int(i64::from(self.line(*span))))),
                    MagicConstKind::File => self.file.as_bytes().to_vec(),
                    MagicConstKind::Dir => match self.file.rfind('/') {
                        Some(0) => b"/".to_vec(),
                        Some(i) => self.file.as_bytes()[..i].to_vec(),
                        None => b".".to_vec(),
                    },
                    // Outside a class, __METHOD__ is the function's name.
                    MagicConstKind::Function | MagicConstKind::Method if f.name.as_bytes() != b"{main}" => {
                        f.name.as_bytes().to_vec()
                    }
                    // No classes, traits, namespaces or property hooks yet:
                    // outside them these are empty, as in PHP.
                    _ => Vec::new(),
                };
                f.constant(Frozen::Str(Text::from(text.as_slice())))
            }
            // Arms are tried in order with ===; an arm's conditions are
            // evaluated only until one matches. No match throws.
            Expr::Match { condition, arms, .. } => {
                let subject = self.expr(f, condition)?;
                let dst = hint.unwrap_or_else(|| f.temp());
                let keep = f.temps;
                let mut ends = Vec::new();
                let mut default = None;
                for arm in *arms {
                    let Some(conditions) = arm.conditions else {
                        default = Some(arm.body);
                        continue;
                    };
                    let mut to_body = Vec::new();
                    for c in conditions {
                        let value = self.expr(f, c)?;
                        to_body.push(f.emit(Instr::jump_cmp(Cmp::Identical, subject, value, true, 0)));
                        f.temps = keep;
                    }
                    let skip = f.emit(Instr::Jump { target: 0 });
                    f.patch_all(to_body, f.here());
                    self.expr_into(f, arm.body, dst)?;
                    f.temps = keep;
                    ends.push(f.emit(Instr::Jump { target: 0 }));
                    f.patch(skip, f.here());
                }
                match default {
                    Some(body) => {
                        self.expr_into(f, body, dst)?;
                        f.temps = keep;
                    }
                    None => {
                        f.emit(Instr::MatchError { src: subject });
                    }
                }
                f.patch_all(ends, f.here());
                dst
            }
            Expr::New { class, args, .. } => {
                let class = self.class_ref(f, class)?;
                // The result's register is below the arguments: the
                // constructor's frame starts at them.
                let dst = f.temp();
                let (base, argc) = self.args_by_value(f, args)?;
                f.emit(Instr::New { dst, class, base, argc });
                dst
            }
            Expr::PropertyFetch { target, property, .. } => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name(f, property)?;
                let dst = out(f, hint, mark);
                f.emit(Instr::FetchProp { dst, obj, name, quiet: false });
                dst
            }
            Expr::MethodCall { target, method, args, .. } => {
                let obj = self.expr(f, target)?;
                let Expr::Variable { name, .. } = method else {
                    return Err(self.unsupported(method.span(), "a method name in an expression"));
                };
                let text = self.text(*name);
                if text.starts_with(b"$") {
                    return Err(self.unsupported(method.span(), "a method name in a variable"));
                }
                let site = f.site(false, Text::from(text.to_ascii_lowercase().as_slice()), Text::from(text));
                let (base, argc) = self.args_by_value(f, args)?;
                f.emit(Instr::MethodCall { dst: base, obj, method: site, base, argc });
                base
            }
            Expr::StaticCall { class, method, args, .. } => {
                let class = self.class_ref(f, class)?;
                let Expr::Variable { name, .. } = method else {
                    return Err(self.unsupported(method.span(), "a method name in an expression"));
                };
                let text = self.text(*name);
                if text.starts_with(b"$") {
                    return Err(self.unsupported(method.span(), "a method name in a variable"));
                }
                let site = f.site(false, Text::from(text.to_ascii_lowercase().as_slice()), Text::from(text));
                let (base, argc) = self.args_by_value(f, args)?;
                f.emit(Instr::StaticCall { dst: base, class, method: site, base, argc });
                base
            }
            Expr::ClassConstFetch { class, constant, .. } => {
                let Expr::Variable { name, .. } = constant else {
                    return Err(self.unsupported(constant.span(), "a class constant name in an expression"));
                };
                let text = self.text(*name);
                let class_index = self.class_ref(f, class)?;
                let dst = out(f, hint, mark);
                if let Some(prop) = text.strip_prefix(b"$") {
                    let name = f.constant(Frozen::Str(Text::from(prop)));
                    f.emit(Instr::FetchStaticProp { dst, class: class_index, name });
                } else if text.eq_ignore_ascii_case(b"class")
                    && let ClassRef::Named(_, display) | ClassRef::Forward(_, display) = &f.classes[class_index as usize]
                {
                    // `C::class` is the name, resolved when compiled (but
                    // `static::class` when the code runs).
                    let display = display.clone();
                    return Ok(f.constant(Frozen::Str(display)));
                } else {
                    let name = f.constant(Frozen::Str(Text::from(text)));
                    f.emit(Instr::ClassConst { dst, class: class_index, name });
                }
                dst
            }
            Expr::Clone { expr, .. } => {
                let src = self.expr(f, expr)?;
                let dst = out(f, hint, mark);
                f.emit(Instr::Clone { dst, src });
                dst
            }
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
                        Expr::ArrayDimFetch { dim: Some(_), .. } if !dim_root_is_prop(v) => {
                            let (var, keys, n, _) = self.dim_path(f, v)?;
                            f.emit(Instr::IssetDim { dst, var, keys, n });
                        }
                        Expr::PropertyFetch { target, property, .. } => {
                            let obj = self.quiet(f, target)?;
                            let name = self.prop_name(f, property)?;
                            f.emit(Instr::IssetProp { dst, obj, name });
                        }
                        // isset($o->p[k]...): the dimensions of a quietly read root
                        Expr::ArrayDimFetch { dim: Some(dim), array, .. } => {
                            let arr = self.quiet(f, array)?;
                            let key = self.expr(f, dim)?;
                            f.emit(Instr::IssetDim { dst, var: arr, keys: key, n: 1 });
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
            Expr::Include { kind, expr, .. } => {
                use p8_syntax::ast::IncludeKind as K;
                let kind = match kind {
                    K::Include => IncludeKind::Include,
                    K::IncludeOnce => IncludeKind::IncludeOnce,
                    K::Require => IncludeKind::Require,
                    K::RequireOnce => IncludeKind::RequireOnce,
                };
                let src = self.expr(f, expr)?;
                let dst = f.temp();
                f.emit(Instr::Include { dst, src, kind });
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
            other => return Err(self.unsupported(other.span(), &format!("this expression ({})", kind(other)))),
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
        if op == BinaryOp::Instanceof {
            let src = self.expr(f, left)?;
            let class = self.class_ref(f, right)?;
            let dst = out(f, hint, mark);
            f.emit(Instr::InstanceOf { dst, src, class });
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
            Expr::PropertyFetch { target, property, .. } => {
                let obj = self.quiet(f, target)?;
                let name = self.prop_name(f, property)?;
                let dst = f.temp();
                f.emit(Instr::FetchProp { dst, obj, name, quiet: true });
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

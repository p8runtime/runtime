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

use p8_bytecode::{
    AttrArg, AttrDef, Meta, CastKind, ClassDef, ClassRef, Cmp, ConstDef, Function, Handler, IncludeKind, Instr, MethodDef, Param, PartialArg, PartialSite, PropDef, Reg, Site, Sym, TraitRule,
    ClassName,
    CONST_EXPR, Rare,
};
use p8_runtime::object::{CLASS_ABSTRACT, CLASS_FINAL, CLASS_INTERFACE, CLASS_ENUM, CLASS_READONLY, CLASS_TRAIT, HOOK_GET, HOOK_SET, HOOK_VIRTUAL, Visibility};
use p8_runtime::ext::Registry;
use p8_runtime::ops::BinOp;
use p8_runtime::frozen::{Frozen, Text};
use p8_syntax::literal::{self, Number};
use p8_syntax::ast::{AssignOp, BinaryOp, CastKind as AstCast, Expr, MagicConstKind, Stmt, StmtId, UnaryOp};
use p8_syntax::span::Span;
use p8_runtime::fast::IndexMap;
use p8_runtime::fast::{HashMap, HashSet};
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
    Str(Text),
}

/// A function's body: statements, or an arrow function's expression.
/// The variables every scope sees as the global one.
const SUPERGLOBALS: &[&str] = &["_SERVER", "_GET", "_POST", "_COOKIE", "_FILES", "_ENV", "_REQUEST", "_SESSION"];

enum Body<'a> {
    Stmts(&'a [StmtId<'a>]),
    Expr(&'a Expr<'a>),
    /// `get => expr`: its value is returned.
    Get(&'a Expr<'a>),
    /// `set => expr`: it is stored in the property (the named one).
    Set(Text, &'a Expr<'a>),
}

struct Loop {
    /// Which loop (or switch) this is, for `goto` into one.
    id: usize,
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
    /// The declared return type.
    ret_type: Option<p8_runtime::types::TypeDecl>,
    /// The file declares strict_types=1.
    strict: bool,
    name: Text,
    /// The lowercase name's symbol.
    sym: Sym,
    code: Vec<Instr>,
    lines: Vec<u32>,
    consts: Vec<Frozen>,
    const_index: HashMap<ConstKey, usize>,
    /// Which constants the code reads as values (one bit each, noted as
    /// it is emitted); the others are only names (see `name_operand`).
    value_bits: u64,
    more_value_bits: Vec<u64>,
    cvs: IndexMap<Vec<u8>, usize>,
    temps: usize,
    /// Temporaries below this stay live across statements (a foreach's iterator).
    floor: usize,
    max_temps: usize,
    /// Temporaries holding a reference to a global (`$GLOBALS['x']…` as
    /// a variable): emptied (Free) when they go out of use, so the
    /// temporary's next value is not written into the global through it.
    ref_temps: Vec<usize>,
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
    /// A function's (not the main script's): superglobals are looked up
    /// in the global table where they are used.
    auto_globals: bool,
    is_static: bool,
    tries: Vec<TryCtx<'a>>,
    handlers: Vec<Handler>,
    /// Closures and arrow functions made in this code (`MakeClosure`).
    lambdas: Vec<Arc<Function>>,
    /// For a closure: the registers of its captured variables, in order.
    bound: Vec<Reg>,
    /// `static` variables declared so far.
    statics: u16,
    /// The code yields: a generator.
    is_generator: bool,
    /// The classes declared in this function's code (`DeclareClass`).
    decls: Vec<Arc<ClassDef>>,
    silences: Vec<(u32, u32, Reg)>,
    /// Partial applications made in this code (`Rare::Partial`).
    partials: Vec<PartialSite>,
    loops_made: usize,
    /// `label:` positions and the loops open there; `goto`s to patch.
    labels: HashMap<Vec<u8>, (u32, Vec<usize>)>,
    gotos: Vec<Goto>,
    meta: Meta,
}

/// A `goto` waiting for its label (which may come later).
struct Goto {
    label: Vec<u8>,
    at: usize,
    loops: Vec<usize>,
    line: u32,
}

impl FnBuilder<'_> {
    fn new(name: Text, sym: Sym) -> Self {
        FnBuilder {
            ret_type: None,
            strict: false,
            name,
            sym,
            // Sized for a typical function body: the first several
            // emits do not each grow the vectors.
            code: Vec::with_capacity(32),
            lines: Vec::with_capacity(32),
            consts: Vec::new(),
            const_index: HashMap::default(),
            value_bits: 0,
            more_value_bits: Vec::new(),
            cvs: IndexMap::default(),
            temps: 0,
            floor: 0,
            max_temps: 0,
            ref_temps: Vec::new(),
            sites: Vec::new(),
            site_index: HashMap::default(),
            loops: Vec::new(),
            params: Vec::new(),
            returns_ref: false,
            line: 1,
            decl_line: 1,
            classes: Vec::new(),
            class_index: HashMap::default(),
            class: None,
            auto_globals: false,
            is_static: false,
            tries: Vec::new(),
            handlers: Vec::new(),
            lambdas: Vec::new(),
            bound: Vec::new(),
            statics: 0,
            is_generator: false,
            decls: Vec::new(),
            silences: Vec::new(),
            partials: Vec::new(),
            loops_made: 0,
            labels: HashMap::default(),
            gotos: Vec::new(),
            meta: Meta::default(),
        }
    }

    /// The index of a class the code names (one entry per class).
    fn class_ref(&mut self, class: ClassRef) -> u16 {
        let key = match &class {
            ClassRef::Named(name) => name.lower.as_bytes().to_vec(),
            ClassRef::Forward(name) => [b"forward:", name.lower.as_bytes()].concat(),
            ClassRef::Static => b"static".to_vec(),
            ClassRef::Scope => b"scope".to_vec(),
            ClassRef::ParentScope => b"parent scope".to_vec(),
            ClassRef::Dynamic(r) => format!("dynamic:{r}").into_bytes(),
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

    fn push_loop(&mut self) {
        self.loops_made += 1;
        self.loops.push(Loop { id: self.loops_made, breaks: Vec::new(), continues: Vec::new() });
    }

    fn open_loops(&self) -> Vec<usize> {
        self.loops.iter().map(|l| l.id).collect()
    }

    /// Point each `goto` at its label: one in the same loops or outside
    /// them (never into a loop or switch).
    /// Each kind of register is numbered in 14 bits and the frame in 16: a
    /// function past that cannot be compiled (an error, never registers
    /// silently shared).
    fn check_size(&self) -> Result<()> {
        let limit = 1usize << KIND_SHIFT;
        let (cvs, consts, temps) = (self.cvs.len(), self.consts.len(), self.max_temps);
        if cvs >= limit || consts >= limit || temps >= limit || cvs + consts + temps > usize::from(u16::MAX) {
            return Err(CompileError {
                message: format!("p8 cannot compile a function this large yet ({cvs} variables, {consts} constants, {temps} temporaries; at most {} each)", limit - 1),
                line: self.decl_line,
            });
        }
        Ok(())
    }

    fn resolve_gotos(&mut self) -> Result<()> {
        for g in std::mem::take(&mut self.gotos) {
            let Some((target, loops)) = self.labels.get(&g.label) else {
                return Err(CompileError { message: format!("'goto' to undefined label '{}'", String::from_utf8_lossy(&g.label)), line: g.line });
            };
            if !g.loops.starts_with(loops) {
                return Err(CompileError { message: "'goto' into loop or switch statement is disallowed".into(), line: g.line });
            }
            let target = *target;
            self.patch(g.at, target);
        }
        Ok(())
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
        // The name is copied only for a variable not seen before.
        let index = match self.cvs.get(name) {
            Some(&i) => i,
            None => {
                let next = self.cvs.len();
                self.cvs.insert(name.to_vec(), next);
                next
            }
        };
        let r = reg(CV, index);
        // In a function, a superglobal is the global table's variable at
        // each use (`unset($GLOBALS['_GET'])` then `$_GET = …` makes it
        // again, where a binding made once would keep the old one).
        if self.auto_globals && name.first() == Some(&b'_') && SUPERGLOBALS.iter().any(|g| g.as_bytes() == name) {
            let n = self.constant(Frozen::Str(Text::from(name)));
            self.emit(Instr::rare(Rare::AutoGlobal, r, n, 0, 0));
        }
        r
    }

    /// A constant the code reads as a value (copied into a call's
    /// registers).
    fn constant(&mut self, value: Frozen) -> Reg {
        let index = self.constant_slot(value);
        self.mark_value(index);
        reg(CONST, index)
    }

    /// A constant only used as a name: the name operand of a property,
    /// class constant or static property instruction (see
    /// `name_operand`), or `global $x`'s. It is never copied into a
    /// register, unless some code also asks for it as a value.
    fn name_constant(&mut self, value: Frozen) -> Reg {
        reg(CONST, self.constant_slot(value))
    }

    fn constant_slot(&mut self, value: Frozen) -> usize {
        let key = match &value {
            Frozen::Null => ConstKey::Null,
            Frozen::Bool(b) => ConstKey::Bool(*b),
            Frozen::Int(i) => ConstKey::Int(*i),
            Frozen::Float(f) => ConstKey::Float(f.to_bits()),
            Frozen::Str(s) => ConstKey::Str(s.clone()),
            // Arrays are not shared between constant registers.
            Frozen::Array(_) => {
                self.consts.push(value);
                return self.consts.len() - 1;
            }
        };
        let next = self.consts.len();
        let index = *self.const_index.entry(key).or_insert(next);
        if index == next {
            self.consts.push(value);
        }
        index
    }

    fn temp(&mut self) -> Reg {
        let r = reg(TEMP, self.temps);
        self.temps += 1;
        self.max_temps = self.max_temps.max(self.temps);
        r
    }

    /// The temporaries from `to` on are out of use: a reference one of
    /// them holds is dropped now (Free), so whatever is written there next
    /// lands in the register, not in the referenced variable.
    fn reset_temps(&mut self, to: usize) {
        let mut i = 0;
        while i < self.ref_temps.len() {
            if self.ref_temps[i] >= to {
                let t = self.ref_temps.swap_remove(i);
                self.emit(Instr::Free { reg: reg(TEMP, t) });
            } else {
                i += 1;
            }
        }
        self.temps = to;
    }

    fn site(&mut self, constant: bool, name: Text, sym: Sym, display: Text) -> u16 {
        // One site per spelling: a call's messages name it as written
        // (`$o->foobar()` after `$o->FooBar()` says "foobar").
        let key = (constant, display.as_bytes().to_vec());
        if let Some(&i) = self.site_index.get(&key) {
            return i;
        }
        self.sites.push(Site { name, sym, fallback: None, display });
        let i = (self.sites.len() - 1) as u16;
        self.site_index.insert(key, i);
        i
    }

    /// Constant `i` is read as a value somewhere (not only as a name).
    fn mark_value(&mut self, i: usize) {
        if i < 64 {
            self.value_bits |= 1 << i;
        } else {
            let word = (i - 64) / 64;
            if self.more_value_bits.len() <= word {
                self.more_value_bits.resize(word + 1, 0);
            }
            self.more_value_bits[word] |= 1 << ((i - 64) % 64);
        }
    }

    fn is_value(&self, i: usize) -> bool {
        if i < 64 { self.value_bits >> i & 1 != 0 } else { self.more_value_bits.get((i - 64) / 64).is_some_and(|w| w >> ((i - 64) % 64) & 1 != 0) }
    }

    /// Debug builds: every constant operand an instruction reads as a value
    /// (all but the name operand of an instruction that reads its name from
    /// the constants) was asked for with `constant`, not `name_constant`.
    #[cfg(debug_assertions)]
    fn check_values(&self) {
        for instr in &self.code {
            let found: [std::cell::Cell<Reg>; 8] = Default::default();
            let count = std::cell::Cell::new(0);
            map_regs(&mut instr.clone(), |r| {
                if r >> KIND_SHIFT == CONST && count.get() < found.len() {
                    found[count.get()].set(r);
                    count.set(count.get() + 1);
                }
                r
            });
            let mut name = name_operand(instr);
            for cell in &found[..count.get()] {
                let r = cell.get();
                if name == Some(r) {
                    name = None;
                    continue;
                }
                let index = (r & ((1 << KIND_SHIFT) - 1)) as usize;
                assert!(self.is_value(index), "constant {:?} is read as a value by {instr:?} but was made as a name", self.consts[index]);
            }
        }
    }

    /// Lay the frame out and renumber every register. `order` is scratch
    /// space the compiler keeps for every function's constant order.
    fn finish(mut self, file: Arc<str>, order: &mut ConstOrder) -> Function {
        if !self.returns_ref {
            thread_returns(&mut self.code);
        }
        let ncv = self.cvs.len();
        let nconst = self.consts.len();
        #[cfg(debug_assertions)]
        self.check_values();
        // The constants some code reads as a value come first: a call
        // copies those into its registers. The others (property and class
        // constant names, which their instructions read from the
        // constants) come after, and are never copied. Most functions have
        // none of the latter: nothing moves then.
        let value_consts = (0..nconst).filter(|&i| self.is_value(i)).count();
        let moved = value_consts < nconst;
        if moved {
            let (position, work) = (&mut order.position, &mut order.work);
            position.clear();
            let (mut next_value, mut next_name) = (0, value_consts as u32);
            for i in 0..nconst {
                let slot = if self.is_value(i) { &mut next_value } else { &mut next_name };
                position.push(*slot);
                *slot += 1;
            }
            // Move the constants to their places, in place (cycle by cycle).
            work.clear();
            work.extend_from_slice(position);
            for i in 0..nconst {
                while work[i] as usize != i {
                    let j = work[i] as usize;
                    self.consts.swap(i, j);
                    work.swap(i, j);
                }
            }
        }
        let position = &order.position;
        let map = |r: Reg| -> Reg {
            let index = (r & ((1 << KIND_SHIFT) - 1)) as usize;
            (match r >> KIND_SHIFT {
                CV => index,
                CONST if moved => ncv + position[index] as usize,
                CONST => ncv + index,
                _ => ncv + nconst + index,
            }) as Reg
        };
        for instr in &mut self.code {
            map_regs(instr, map);
            // `global $x` names its variable by constant index.
            if moved && let Instr::BindGlobal { name, .. } = instr {
                *name = position[*name as usize] as u16;
            }
        }
        for h in &mut self.handlers {
            h.slot = map(h.slot);
        }
        for s in &mut self.silences {
            s.2 = map(s.2);
        }
        for c in &mut self.classes {
            if let ClassRef::Dynamic(r) = c {
                *r = map(*r);
            }
        }
        fuse_steps(&mut self.code);
        let mut cv_names: Vec<(usize, Text)> =
            self.cvs.iter().map(|(n, &i)| (i, Text::from(n.as_slice()))).collect();
        cv_names.sort_by_key(|(i, _)| *i);
        let required = self.params.iter().rposition(|p| !p.has_default && !p.variadic).map_or(0, |i| i + 1);
        Function {
            name: self.name,
            sym: self.sym,
            ret_type: self.ret_type,
            strict: self.strict,
            params: self.params,
            returns_ref: self.returns_ref,
            code: self.code,
            lines: self.lines,
            consts: self.consts,
            consts_at: ncv as Reg,
            value_consts: value_consts as Reg,
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
            lambdas: self.lambdas,
            is_generator: self.is_generator,
            bound: self.bound,
            meta: self.meta,
            decls: self.decls,
            silences: self.silences,
            partials: self.partials,
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
/// Whether a dimension path has `[]` before its last dimension.
fn has_inner_append(e: &Expr) -> bool {
    let Expr::ArrayDimFetch { array, .. } = e else { return false };
    let mut cur = *array;
    while let Expr::ArrayDimFetch { array, dim, .. } = cur {
        if dim.is_none() {
            return true;
        }
        cur = array;
    }
    false
}

/// Whether a dimension path starts at a plain variable (not `$this`).
fn dim_root_is_var(c: &Compiler, e: &Expr) -> bool {
    let mut cur = e;
    while let Expr::ArrayDimFetch { array, .. } = cur {
        if c.globals_dim(cur).is_some() {
            return true;
        }
        cur = array;
    }
    matches!(cur, Expr::Variable { name, .. } if c.text(*name).starts_with(b"$") && c.text(*name) != b"$this")
}

fn dim_root_is_prop(e: &Expr) -> bool {
    let mut cur = e;
    while let Expr::ArrayDimFetch { array, .. } = cur {
        cur = array;
    }
    matches!(cur, Expr::PropertyFetch { .. })
}

/// The visibility a member's modifiers give it (None: none written).
/// The targets a builtin attribute class (by resolved name) may be put
/// on (TARGET_CLASS = 1, FUNCTION 2, METHOD 4, PROPERTY 8, CLASS_CONSTANT
/// 16, PARAMETER 32); None for any other class.
fn builtin_attribute_targets(name: &[u8]) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_slice() {
        b"attribute" | b"allowdynamicproperties" => 1,
        b"returntypewillchange" | b"override" => 4,
        b"sensitiveparameter" => 32,
        b"deprecated" => 2 | 4 | 16,
        _ => return None,
    })
}

fn attr_target_names(flags: u32) -> String {
    ["class", "function", "method", "property", "class constant", "parameter"]
        .iter()
        .enumerate()
        .filter(|(i, _)| flags & (1 << i) != 0)
        .map(|(_, n)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

fn visibility_of(c: &Compiler, modifiers: &[p8_syntax::lexer::token::Token]) -> Option<Visibility> {
    modifiers.iter().find_map(|m| match c.text(m.span).to_ascii_lowercase().as_slice() {
        b"public" | b"var" => Some(Visibility::Public),
        b"protected" => Some(Visibility::Protected),
        b"private" => Some(Visibility::Private),
        _ => None,
    })
}

/// A property's set visibility as written (`private(set)`). Two are a
/// fatal error, as zend_add_member_modifier reports them.
fn set_visibility_of(c: &Compiler, modifiers: &[p8_syntax::lexer::token::Token]) -> Result<Option<Visibility>> {
    use p8_syntax::lexer::token::TokenKind;
    let mut found = None;
    for m in modifiers {
        let v = match m.kind {
            TokenKind::PublicSet => Visibility::Public,
            TokenKind::ProtectedSet => Visibility::Protected,
            TokenKind::PrivateSet => Visibility::Private,
            _ => continue,
        };
        if found.is_some() {
            return Err(CompileError { message: "Multiple access type modifiers are not allowed".into(), line: c.line(m.span) });
        }
        found = Some(v);
    }
    Ok(found)
}

/// The set visibility a property keeps, as zend_declare_typed_property
/// settles it: a public readonly one is protected(set) unless declared
/// otherwise, and one no narrower than the visibility is dropped. An
/// untyped property cannot have one, nor one wider than its visibility.
fn settled_set_visibility(what: (&[u8], &[u8]), visibility: Visibility, declared: Option<Visibility>, readonly: bool, typed: bool, line: u32) -> Result<Option<Visibility>> {
    let Some(set) = declared else {
        return Ok((readonly && visibility == Visibility::Public).then_some(Visibility::Protected));
    };
    let what = format!("{}::${}", String::from_utf8_lossy(what.0), String::from_utf8_lossy(what.1));
    if !typed {
        return Err(CompileError { message: format!("Property with asymmetric visibility {what} must have type"), line });
    }
    if visibility.rank() > set.rank() {
        return Err(CompileError { message: format!("Visibility of property {what} must not be weaker than set visibility"), line });
    }
    Ok((set != visibility).then_some(set))
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

/// `ns\\name` (just `name` in the global namespace).
fn join(ns: &[u8], name: &[u8]) -> Vec<u8> {
    if ns.is_empty() { name.to_vec() } else { [ns, b"\\", name].concat() }
}

/// Whether a destructuring pattern takes anything by reference (`[&$a]`,
/// `['k' => [&$b]]`).
fn has_ref_items(items: &[p8_syntax::ast::ArrayItem]) -> bool {
    items.iter().any(|i| i.by_ref || matches!(i.value, Expr::Array { items, .. } if has_ref_items(items)))
}

/// Whether a call's arguments are `...` (the first-class callable syntax).
/// The pipe's call (the last instruction) gets its argument as a
/// temporary, which a by-reference parameter refuses (ZEND_SEND_VAL_EX):
/// the check goes right before it, once the callee is known there.
fn pipe_send_check(f: &mut FnBuilder) {
    let Some(last) = f.code.len().checked_sub(1) else { return };
    let check = match f.code[last] {
        Instr::Call { site, .. } => Instr::rare(Rare::PipeArgByRef, 0, site, 0, 0),
        Instr::MethodCall { obj, method, .. } => Instr::rare(Rare::PipeArgByRef, obj, method, 1, 0),
        Instr::StaticCall { class, method, .. } => Instr::rare(Rare::PipeArgByRef, 0, method, 2, class),
        Instr::CallValue { callee, .. } => Instr::rare(Rare::PipeArgByRef, callee, 0, 3, 0),
        _ => return,
    };
    let line = f.lines[last];
    f.code.insert(last, check);
    f.lines.insert(last, line);
}

fn is_placeholder(args: &[p8_syntax::ast::Arg]) -> bool {
    matches!(args, [arg] if matches!(arg.value, Expr::VariadicPlaceholder { .. }) && arg.name.is_none())
}

/// Whether a call is a partial application (PHP 8.6): a `?` among its
/// arguments, or `...` beside others.
fn is_partial(args: &[p8_syntax::ast::Arg]) -> bool {
    !is_placeholder(args) && args.iter().any(|a| matches!(a.value, Expr::Placeholder { .. } | Expr::VariadicPlaceholder { .. }))
}

/// Whether an expression makes a partial application anywhere in it.
fn makes_partial(e: &Expr) -> bool {
    match e {
        Expr::Call { args, .. } | Expr::StaticCall { args, .. } | Expr::MethodCall { args, .. } | Expr::New { args, .. } => {
            is_partial(args) || args.iter().any(|a| makes_partial(a.value))
        }
        Expr::Array { items, .. } => items.iter().any(|i| i.key.is_some_and(makes_partial) || makes_partial(i.value)),
        Expr::Binary { left, right, .. } => makes_partial(left) || makes_partial(right),
        Expr::Unary { expr, .. } | Expr::Cast { expr, .. } => makes_partial(expr),
        Expr::Ternary { condition, if_true, if_false, .. } => makes_partial(condition) || if_true.is_some_and(makes_partial) || makes_partial(if_false),
        _ => false,
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
/// Where each constant of the function being finished goes (`position`),
/// and room to move them there: kept by the compiler for every function.
#[derive(Default)]
struct ConstOrder {
    position: Vec<u32>,
    work: Vec<u32>,
}

/// The operand of `instr` that only names something (a property, a class
/// constant, a static property): read from the function's constants when
/// it is one, never as a register (see `Function::value_consts`).
fn name_operand(instr: &Instr) -> Option<Reg> {
    use Instr::*;
    match *instr {
        FetchProp { name, .. }
        | AssignProp { name, .. }
        | AssignPropOp { name, .. }
        | PropIncDec { name, .. }
        | IssetProp { name, .. }
        | UnsetProp { name, .. }
        | ClassConst { name, .. }
        | FetchStaticProp { name, .. }
        | AssignStaticProp { name, .. }
        | AssignStaticPropOp { name, .. } => Some(name),
        _ => None,
    }
}

fn map_regs(instr: &mut Instr, f: impl Fn(Reg) -> Reg) {
    use Instr::*;
    match instr {
        DeclareClass { .. } | DeclareFunction { .. } => {}
        MakeClosure { dst, .. } => *dst = f(*dst),
        BindRef { var, src } => {
            *var = f(*var);
            *src = f(*src);
        }
        RefDim { var, root, keys, .. } => {
            *var = f(*var);
            *root = f(*root);
            *keys = f(*keys);
        }
        DimSetRef { root, keys, src, .. } => {
            *root = f(*root);
            *keys = f(*keys);
            *src = f(*src);
        }
        IterInitRef { src, iter, .. } => {
            *src = f(*src);
            *iter = f(*iter);
        }
        IterNextRef { iter, val, key, .. } => {
            *iter = f(*iter);
            *val = f(*val);
            *key = f(*key);
        }
        StaticVar { var, .. } | CollectVariadic { var } => *var = f(*var),
        DimIncDec { var, keys, dst, .. } => {
            *var = f(*var);
            *keys = f(*keys);
            *dst = f(*dst);
        }
        CallSpread { dst, callee, args } => {
            *dst = f(*dst);
            *callee = f(*callee);
            *args = f(*args);
        }
        ArrayUnpack { arr, src } | Eval { dst: arr, src } => {
            *arr = f(*arr);
            *src = f(*src);
        }
        FetchList { dst, arr, key } => {
            *dst = f(*dst);
            *arr = f(*arr);
            *key = f(*key);
        }
        Yield { dst, value, key, .. } => {
            *dst = f(*dst);
            *value = f(*value);
            *key = f(*key);
        }
        YieldFrom { dst, src } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        FromCallable { dst, src } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        Capture { closure, var, .. } => {
            *closure = f(*closure);
            *var = f(*var);
        }
        CallValue { dst, callee, base, .. } => {
            *dst = f(*dst);
            *callee = f(*callee);
            *base = f(*base);
        }
        Silence { saved } | Unsilence { saved } => *saved = f(*saved),
        Include { dst, src, .. } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        Rare { a, b, c, .. } => {
            *a = f(*a);
            *b = f(*b);
            *c = f(*c);
        }
        New { dst, .. } => *dst = f(*dst),
        Construct { obj, base, .. } => {
            *obj = f(*obj);
            *base = f(*base);
        }
        ConstructSpread { obj, args } => {
            *obj = f(*obj);
            *args = f(*args);
        }
        StaticCall { dst, base, .. } => {
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
        AssignProp { obj, name, val, dst, .. } | AssignPropOp { obj, name, val, dst, .. } => {
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
        Assign { var, src } | AssignTake { var, src } => {
            *var = f(*var);
            *src = f(*src);
        }
        Free { reg } => *reg = f(*reg),
        LoadNull { dst } | NewArray { dst, .. } => *dst = f(*dst),
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
        ArrayPush { arr, val, .. } => {
            *arr = f(*arr);
            *val = f(*val);
        }
        ArraySet { arr, key, val, .. } => {
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
        SendVar { arg, var, .. } | SendVarStatic { arg, var, .. } => {
            *arg = f(*arg);
            *var = f(*var);
        }
        SendVarMethod { obj, arg, var, .. } => {
            *obj = f(*obj);
            *arg = f(*arg);
            *var = f(*var);
        }
        MethodArgByRef { dst, obj, .. } => {
            *dst = f(*dst);
            *obj = f(*obj);
        }
        StaticArgByRef { dst, .. } | FuncArgByRef { dst, .. } => *dst = f(*dst),
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

/// Whether a call's arguments go as one array (`...` or a named argument).
fn needs_spread(args: &[p8_syntax::ast::Arg]) -> bool {
    args.iter().any(|a| a.unpack || a.name.is_some())
}

/// Whether `$a[…]…`'s root is a static property (`C::$a[k]`).
fn dim_root_is_static(c: &Compiler, e: &Expr) -> bool {
    let mut cur = e;
    while let Expr::ArrayDimFetch { array, .. } = cur {
        cur = array;
    }
    match cur {
        Expr::ClassConstFetch { constant: Expr::Variable { name, .. }, .. } => c.text(*name).starts_with(b"$"),
        // `C::${expr}`: named when it runs.
        Expr::ClassConstFetch { constant: Expr::IndirectVariable { .. }, .. } => true,
        _ => false,
    }
}

/// Whether a hook's source uses its property's stored value: `$this->name`,
/// or `parent::$name::` (a parent's plain property is the stored value).
fn uses_own_property(source: &[u8], name: &[u8]) -> bool {
    let word_at = |needle: &[u8]| {
        source.windows(needle.len()).enumerate().any(|(i, w)| {
            w.eq_ignore_ascii_case(needle) && source.get(i + needle.len()).is_none_or(|&c| !(c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80))
        })
    };
    word_at(&[b"$this->".as_slice(), name].concat()) || word_at(&[b"parent::$".as_slice(), name].concat())
}

/// After an operator used a value an expression made (a call's result,
/// a new object) from a temporary: release it, as PHP frees the operand
/// at once (a destructor runs before the statement goes on). Not when
/// the operator wrote its result into the same register.
fn free_operand(f: &mut FnBuilder, e: &Expr, val: Reg, dst: Reg) {
    if val != dst {
        free_made(f, e, val);
    }
}

/// Whether register `r` is a temporary: an expression's value, which the
/// instruction consuming it may take (move) when that is its only read.
fn is_temp(r: Reg) -> bool {
    r >> KIND_SHIFT == TEMP
}

/// After storing a value an expression made (an object, likely) from a
/// temporary: release the temporary's copy.
fn free_made(f: &mut FnBuilder, e: &Expr, val: Reg) {
    if val >> KIND_SHIFT == TEMP && made(e) {
        f.emit(Instr::Free { reg: val });
    }
}

/// Whether `e` makes a value its temporary may be the only holder of: an
/// object (new, clone, a call's result), or an array literal with one in
/// it (else the temporary keeps the objects alive after the array they
/// were put in goes). A literal of variables and constants needs no
/// release: what is in it is held elsewhere too.
fn made(e: &Expr) -> bool {
    match e {
        Expr::New { .. } | Expr::Clone { .. } | Expr::Call { .. } | Expr::MethodCall { .. } | Expr::StaticCall { .. } | Expr::NullsafeMethodCall { .. } => true,
        Expr::Array { items, .. } => items.iter().any(|i| made(i.value)),
        _ => false,
    }
}

pub struct Compiler<'src> {
    source: &'src [u8],
    /// Where the source names a superglobal (`$_GET`…): (offset, index in
    /// SUPERGLOBALS), sorted. Made on first use, by one scan of the file
    /// (each function's body was scanned once per name before).
    superglobals: Option<Vec<(usize, u8)>>,
    /// The temporaries holding references a spread call's list was given
    /// (freed after the call).
    spread_refs: Vec<Reg>,
    /// Scratch for laying out each function's constants.
    const_order: ConstOrder,
    /// The assignment being compiled is a statement's: its value is not
    /// used, so the variable itself can stand for it.
    unused_result: bool,
    /// Where the data after `__halt_compiler();` starts (__COMPILER_HALT_OFFSET__).
    halt_offset: Option<usize>,
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
    /// The top-level `function` statements (by address): only these are
    /// declared before the file runs; any other is declared when reached.
    top_functions: HashSet<usize>,
    /// The top-level class, interface, trait and enum statements: only
    /// these can be bound before the file runs (a class in an `if` is
    /// declared when reached).
    top_classes: HashSet<usize>,
    /// The class statement being compiled is one of those.
    declaring_top: bool,
    /// The classes the file declares at its top level, in order (bound
    /// early when their parents are known).
    classes: Vec<Arc<ClassDef>>,
    /// The class whose members are being compiled (for self and parent).
    scope: Option<ClassScope>,
    /// Set while compiling a method: its class and whether it is static.
    method: Option<(Text, bool)>,
    /// Properties a constructor's parameters promoted (`public int $x`).
    promoted: Vec<PropDef>,
    /// The namespace the code is in (empty: the global one), and its
    /// imports by lowercase alias: classes (and namespaces), functions,
    /// constants (`use`, `use function`, `use const`).
    ns: Vec<u8>,
    /// eval()'s code: it returns null at its end, not 1.
    pub eval: bool,
    /// The enum being declared's backing type (for class_decl).
    enum_backing: Option<Text>,
    use_classes: HashMap<Vec<u8>, Vec<u8>>,
    use_functions: HashMap<Vec<u8>, Vec<u8>>,
    use_consts: HashMap<Vec<u8>, Vec<u8>>,
    /// Anonymous classes declared so far (each one's name ends with its number).
    anonymous: u32,
    /// The function being compiled is a set hook: no parameter list means `$value`.
    hook_value: bool,
    /// A reference taken for `unset($obj->p[k])`: an uninitialized
    /// property is left alone, not modified.
    ref_for_unset: bool,
    /// The file declares strict_types=1.
    strict: bool,
    /// The declared return type of the function about to be compiled.
    pending_ret: Option<p8_runtime::types::TypeDecl>,
    /// The doc comment, attributes and last line of the function about to be compiled.
    pending_meta: Option<Meta>,
}

/// The class being compiled.
struct ClassScope {
    name: Text,
    parent: Option<Text>,
    /// A trait: `self` is the class using it, known when the code runs.
    in_trait: bool,
}

type Result<T> = std::result::Result<T, CompileError>;

impl<'src> Compiler<'src> {
    /// The symbol of `name` as it is (a constant's key).
    fn sym(&self, name: &[u8]) -> Sym {
        self.registry.symbols().intern(name)
    }

    /// The symbol of a function or class name (any case).
    fn sym_lower(&self, name: &[u8]) -> Sym {
        self.registry.symbols().intern_lower(name)
    }

    /// The call site of a function or method named `display` (any case).
    fn named_site(&self, f: &mut FnBuilder, display: &[u8]) -> u16 {
        let lower = display.to_ascii_lowercase();
        let sym = self.sym(&lower);
        let display = Text::from(display);
        let lower = if lower.as_slice() == display.as_bytes() { display.clone() } else { Text::from(lower) };
        f.site(false, lower, sym, display)
    }

    /// A class named in code: its symbol, lowercase name and the name as written.
    fn named(&self, display: Text) -> ClassName {
        let lower = display.as_bytes().to_ascii_lowercase();
        let lower = if lower.as_slice() == display.as_bytes() { display.clone() } else { Text::from(lower) };
        ClassName { sym: self.sym(lower.as_bytes()), lower, display }
    }

    pub fn new(source: &'src [u8], file: &str, registry: &'src Registry) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(memchr::memchr_iter(b'\n', source).map(|i| i + 1));
        Compiler {
            source,
            superglobals: None,
            spread_refs: Vec::new(),
            const_order: ConstOrder::default(),
            unused_result: false,
            registry,
            file: Arc::from(file),
            line_starts,
            functions: Vec::new(),
            by_ref: HashMap::default(),
            top_functions: HashSet::default(),
            top_classes: HashSet::default(),
            declaring_top: false,
            classes: Vec::new(),
            scope: None,
            method: None,
            promoted: Vec::new(),
            ns: Vec::new(),
            eval: false,
            enum_backing: None,
            use_classes: HashMap::default(),
            use_functions: HashMap::default(),
            use_consts: HashMap::default(),
            anonymous: 0,
            hook_value: false,
            ref_for_unset: false,
            halt_offset: None,
            strict: false,
            pending_ret: None,
            pending_meta: None,
        }
    }

    /// A class name as written, fully qualified: through the imports and
    /// the current namespace (`\\A` is A itself, `namespace\\A` the
    /// current namespace's).
    fn class_name(&self, text: &[u8]) -> Vec<u8> {
        if let Some(full) = text.strip_prefix(b"\\") {
            return full.to_vec();
        }
        if text.len() > 10 && text[..10].eq_ignore_ascii_case(b"namespace\\") {
            return join(&self.ns, &text[10..]);
        }
        let (first, rest) = match text.iter().position(|&c| c == b'\\') {
            Some(at) => (&text[..at], Some(&text[at..])),
            None => (text, None),
        };
        if let Some(full) = self.use_classes.get(&first.to_ascii_lowercase()) {
            return [full.as_slice(), rest.unwrap_or(b"")].concat();
        }
        join(&self.ns, text)
    }

    /// A function or constant name as written: its full name, and the
    /// global name to fall back to when that is not defined (an
    /// unqualified name in a namespace).
    fn ns_name(&self, text: &[u8], imports: &HashMap<Vec<u8>, Vec<u8>>, lower_alias: bool) -> (Vec<u8>, Option<Vec<u8>>) {
        if let Some(full) = text.strip_prefix(b"\\") {
            return (full.to_vec(), None);
        }
        if text.contains(&b'\\') {
            return (self.class_name(text), None);
        }
        // The import named so, if any (lowercased for function aliases;
        // copied only when there are imports and the name has capitals).
        let imported = if imports.is_empty() {
            None
        } else if lower_alias && text.iter().any(u8::is_ascii_uppercase) {
            imports.get(&text.to_ascii_lowercase())
        } else {
            imports.get(text)
        };
        if let Some(full) = imported {
            return (full.clone(), None);
        }
        if self.ns.is_empty() {
            return (text.to_vec(), None);
        }
        (join(&self.ns, text), Some(text.to_vec()))
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
        // declare(strict_types=1) (the file's first statement) applies to all of it.
        for stmt in stmts {
            if let Stmt::Declare { declares, .. } = stmt
                && declares.iter().any(|d| self.text(d.key.span).eq_ignore_ascii_case(b"strict_types") && matches!(self.literal(d.value), Some(Frozen::Int(1))))
            {
                self.strict = true;
            }
        }
        self.halt_offset = stmts.iter().find_map(|s| match s {
            Stmt::HaltCompiler { span } => Some(span.end),
            _ => None,
        });
        // Each top-level function's by-reference parameters, by its full name.
        let mut ns: Vec<u8> = Vec::new();
        let scan = |c: &mut Self, ns: &[u8], stmt: &Stmt| {
            // Declared as the file loads (early binding), as PHP does it:
            // not a class that implements interfaces (a __toString makes it
            // Stringable) or uses traits, nor an enum (it implements
            // UnitEnum); those are declared when their statement runs.
            let uses_traits = |members: &[p8_syntax::ast::ClassMember]| members.iter().any(|m| matches!(m, p8_syntax::ast::ClassMember::TraitUse { .. }));
            let stringable = |c: &Self, members: &[p8_syntax::ast::ClassMember]| {
                members.iter().any(|m| matches!(m, p8_syntax::ast::ClassMember::Method { name, .. } if c.text(name.span).eq_ignore_ascii_case(b"__tostring")))
            };
            let early = match stmt {
                Stmt::Class { implements, members, .. } => implements.is_empty() && !uses_traits(members) && !stringable(c, members),
                Stmt::Interface { extends, .. } => extends.is_empty(),
                Stmt::Trait { members, .. } => !uses_traits(members),
                _ => false,
            };
            if early {
                c.top_classes.insert(stmt as *const Stmt as usize);
            }
            if let Stmt::Function { name, params, .. } = stmt {
                c.top_functions.insert(stmt as *const Stmt as usize);
                let modes = params.iter().map(|p| p.by_ref).collect();
                let rest = params.last().is_some_and(|p| p.variadic && p.by_ref);
                let full = join(ns, c.text(name.span));
                c.by_ref.insert(full.to_ascii_lowercase(), (modes, rest));
            }
        };
        for stmt in stmts {
            match stmt {
                Stmt::Namespace { name, body, .. } => {
                    let named = name.map(|n| self.text(n.span).to_vec()).unwrap_or_default();
                    match body {
                        Some(body) => {
                            for s in *body {
                                scan(&mut self, &named, s);
                            }
                        }
                        None => ns = named,
                    }
                }
                other => scan(&mut self, &ns, other),
            }
        }
        let mut f = FnBuilder::new(Text::from("{main}"), self.sym(b"{main}"));
        f.strict = self.strict;
        // A file's code runs in the scope that includes it, a function's
        // too: its superglobals are bound like a function's.
        self.bind_superglobals(&mut f, 0, self.source.len());
        self.stmts(&mut f, stmts)?;
        // A file's code ends returning 1 (what including it gives); eval()'s
        // returns null.
        if self.eval {
            f.emit(Instr::ReturnNull);
        } else {
            let one = f.constant(Frozen::Int(1));
            f.emit(Instr::Return { src: one });
        }
        f.resolve_gotos()?;
        f.check_size()?;
        let main = Arc::new(f.finish(self.file.clone(), &mut self.const_order));
        Ok(Program { main, functions: self.functions, classes: self.classes })
    }

    fn stmts<'a>(&mut self, f: &mut FnBuilder<'a>, stmts: &'a [StmtId<'a>]) -> Result<()> {
        for stmt in stmts {
            self.stmt(f, stmt)?;
            f.reset_temps(f.floor);
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
                if f.ret_type.as_ref().is_some_and(|t| !t.is_void()) {
                    f.emit(Instr::rare(Rare::VerifyReturn, value, u16::from(expr.is_none()), 0, 0));
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
                // A function returning by reference returns the place's reference.
                Some(e)
                    if f.returns_ref
                        && match e {
                            Expr::Variable { name, .. } => self.text(*name).starts_with(b"$") && self.text(*name) != b"$this",
                            Expr::PropertyFetch { .. } | Expr::StaticCall { .. } | Expr::Call { .. } | Expr::MethodCall { .. } => true,
                            Expr::ArrayDimFetch { .. } => true,
                            Expr::ClassConstFetch { constant: Expr::Variable { name, .. }, .. } => self.text(*name).starts_with(b"$"),
                            _ => false,
                        } =>
                {
                    let (r, _) = self.ref_source(f, e, e.span())?;
                    f.emit(Instr::rare(Rare::ReturnRef, r, 0, 0, 0));
                }
                Some(e) => {
                    let r = self.expr(f, e)?;
                    if f.ret_type.as_ref().is_some_and(|t| t.is_void()) {
                        return Err(CompileError { message: "A void function must not return a value".into(), line: f.line });
                    }
                    if f.ret_type.is_some() {
                        f.emit(Instr::rare(Rare::VerifyReturn, r, 0, 0, 0));
                    }
                    f.emit(Instr::Return { src: r });
                }
                None => {
                    if f.ret_type.as_ref().is_some_and(|t| !t.is_void()) {
                        let null = f.constant(Frozen::Null);
                        f.emit(Instr::rare(Rare::VerifyReturn, null, 2, 0, 0));
                    }
                    f.emit(Instr::ReturnNull);
                }
            },
            Stmt::If { condition, then_block, else_block, .. } => {
                let skip = self.branch(f, condition, false)?;
                f.reset_temps(f.floor);
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
                f.reset_temps(f.floor);
                self.close_loop(f, cond_at);
            }
            Stmt::DoWhile { body, condition, .. } => {
                let start = f.here();
                f.push_loop();
                self.stmts(f, body)?;
                let cont = f.here();
                let back = self.branch(f, condition, true)?;
                f.patch_all(back, start);
                f.reset_temps(f.floor);
                self.close_loop(f, cont);
            }
            Stmt::For { init, condition, loop_expr, body, .. } => {
                for e in *init {
                    self.discard(f, e)?;
                    f.reset_temps(f.floor);
                }
                // Body first, then the step and the condition at the bottom.
                let to_cond = f.emit(Instr::Jump { target: 0 });
                let body_start = f.here();
                f.push_loop();
                self.stmts(f, body)?;
                let cont = f.here();
                for e in *loop_expr {
                    self.discard(f, e)?;
                    f.reset_temps(f.floor);
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
                f.reset_temps(f.floor);
                self.close_loop(f, cont);
            }
            Stmt::Foreach { expr, key_var, value_var, body, .. }
                if matches!(value_var, Expr::Unary { op: UnaryOp::Reference, .. }) || matches!(value_var, Expr::Array { items, .. } if has_ref_items(items)) =>
            {
                // By reference (`as &$v`, or `as ['k' => &$v]` taken apart
                // from the element itself): the elements of the array
                // itself, so a property's or an element's is made a
                // reference first.
                let src = match expr {
                    Expr::PropertyFetch { target, property, .. } => {
                        let r = f.temp();
                        let obj = self.expr(f, target)?;
                        let name = self.prop_name(f, property)?;
                        f.emit(Instr::rare(Rare::PropRef, r, obj, name, 0));
                        r
                    }
                    Expr::ArrayDimFetch { .. } if self.can_ref(expr) => {
                        let r = f.temp();
                        let (root, keys, count, append, held) = self.ref_dim_path(f, expr)?;
                        f.emit(Instr::RefDim { var: r, root, keys, n: count, append });
                        if let Some(h) = held {
                            f.emit(Instr::Free { reg: h });
                        }
                        r
                    }
                    Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                        let r = f.temp();
                        let class = self.class_ref(f, class)?;
                        let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                        f.emit(Instr::rare(Rare::StaticRef, r, name, 0, class));
                        r
                    }
                    _ => self.expr(f, expr)?,
                };
                let iter = f.temp();
                let _cell = f.temp();
                let saved = f.temps;
                let pattern = match value_var {
                    Expr::Array { items, .. } => Some((*items, f.temp())),
                    _ => None,
                };
                let init = f.emit(Instr::IterInitRef { src, iter, end: 0 });
                let start = f.here();
                let val = match (pattern, value_var) {
                    (Some((_, r)), _) => r,
                    (None, Expr::Unary { expr: target, .. }) => self.var_target(f, target)?,
                    (None, other) => return Err(self.unsupported(other.span(), "this foreach target")),
                };
                let key = match key_var {
                    Some(k) => Some(self.var_target(f, k)?),
                    None => None,
                };
                let next = f.emit(Instr::IterNextRef { iter, val, key: key.unwrap_or(val), has_key: key.is_some(), end: 0 });
                f.push_loop();
                let outer_floor = std::mem::replace(&mut f.floor, saved);
                if let Some((items, r)) = pattern {
                    self.destructure(f, items, r, Some((r, &[])))?;
                    f.reset_temps(f.floor);
                }
                self.stmts(f, body)?;
                f.floor = outer_floor;
                f.emit(Instr::Jump { target: start });
                let end = f.here();
                f.patch(init, end);
                f.patch(next, end);
                // The element's reference goes with the loop (as PHP frees
                // its temporary), so the last element is a plain value again.
                if let Some((_, r)) = pattern {
                    f.emit(Instr::Free { reg: r });
                }
                self.close_loop(f, start);
                // The loop is done with what it iterated (PHP frees it here).
                f.emit(Instr::Free { reg: iter });
                if src >> KIND_SHIFT == TEMP {
                    f.emit(Instr::Free { reg: src });
                }
            }
            Stmt::Foreach { expr, key_var, value_var, body, .. } => {
                let src = self.expr(f, expr)?;
                // The iterator's two registers stay live through the body.
                let iter = f.temp();
                let _pos = f.temp();
                // `as [$a, $b]`: the element lands here, then is taken apart.
                let pattern = match value_var {
                    Expr::Array { items, .. } => Some((*items, f.temp())),
                    _ => None,
                };
                // `as $this->p`, `as $a[k]`, `as C::$s` (`=> $o->k` too): the
                // element lands in a temporary, then is assigned, value first
                // (as zend_compile_foreach does).
                let plain = |c: &Self, e: &Expr| matches!(e, Expr::Variable { name, .. } if c.text(*name).starts_with(b"$") && c.text(*name) != b"$this");
                let value_place = (pattern.is_none() && !plain(self, value_var)).then(|| f.temp());
                // (When the value goes through a temporary the key does too: the
                // value is assigned before it, and may read the old key.)
                let key_place = key_var.filter(|k| value_place.is_some() || !plain(self, k)).map(|_| f.temp());
                let saved = f.temps;
                let init = f.emit(Instr::IterInit { src, iter, end: 0 });
                let start = f.here();
                let val = match (pattern, value_place) {
                    (Some((_, r)), _) => r,
                    (None, Some(t)) => t,
                    (None, None) => self.var_target(f, value_var)?,
                };
                let key = match (key_var, key_place) {
                    (Some(_), Some(t)) => Some(t),
                    (Some(k), None) => Some(self.var_target(f, k)?),
                    (None, _) => None,
                };
                let next = f.emit(Instr::IterNext { iter, val, key: key.unwrap_or(val), has_key: key.is_some(), end: 0 });
                f.push_loop();
                let outer_floor = std::mem::replace(&mut f.floor, saved);
                if let Some(t) = value_place {
                    self.assign_from(f, value_var, t)?;
                    f.reset_temps(f.floor);
                }
                if let (Some(k), Some(t)) = (key_var, key_place) {
                    self.assign_from(f, k, t)?;
                    f.reset_temps(f.floor);
                }
                if let Some((items, r)) = pattern {
                    self.destructure(f, items, r, None)?;
                    f.reset_temps(f.floor);
                }
                self.stmts(f, body)?;
                f.floor = outer_floor;
                f.emit(Instr::Jump { target: start });
                let end = f.here();
                f.patch(init, end);
                f.patch(next, end);
                self.close_loop(f, start);
                // The loop is done with what it iterated (PHP frees it here).
                f.emit(Instr::Free { reg: iter });
                if src >> KIND_SHIFT == TEMP {
                    f.emit(Instr::Free { reg: src });
                }
            }
            Stmt::Block { statements, .. } => self.stmts(f, statements)?,
            // The script ends here; what follows is data.
            Stmt::HaltCompiler { .. } => {
                let one = f.constant(Frozen::Int(1));
                f.emit(Instr::Return { src: one });
            }
            Stmt::Label { name, span } => {
                let label = self.text(name.span).to_vec();
                if f.labels.contains_key(&label) {
                    return Err(CompileError { message: format!("Label '{}' already defined", String::from_utf8_lossy(&label)), line: self.line(*span) });
                }
                let loops = f.open_loops();
                f.labels.insert(label, (f.here(), loops));
            }
            Stmt::Goto { label, span } => {
                let at = f.emit(Instr::Jump { target: 0 });
                let (label, loops, line) = (self.text(label.span).to_vec(), f.open_loops(), self.line(*span));
                f.gotos.push(Goto { label, at, loops, line });
            }
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
            Stmt::Function { attributes, name, params, body, by_ref, return_type, doc_comment, span } => {
                let full = join(&self.ns, self.text(name.span));
                let line = self.line(name.span);
                let meta = self.meta_of(attributes, *doc_comment, *span, 2)?;
                self.pending_ret = return_type.map(|t| self.type_decl(t));
                self.pending_meta = Some(meta);
                let (function, _) = self.function_with(Text::from(full.as_slice()), line, params, Body::Stmts(body), *by_ref, &[])?;
                if self.top_functions.contains(&(stmt as *const Stmt as usize)) {
                    self.functions.push(Arc::new(function));
                } else {
                    f.lambdas.push(Arc::new(function));
                    f.emit(Instr::DeclareFunction { func: (f.lambdas.len() - 1) as u16 });
                }
            }
            Stmt::Namespace { name, body, .. } => {
                self.ns = name.map(|n| self.text(n.span).to_vec()).unwrap_or_default();
                self.use_classes.clear();
                self.use_functions.clear();
                self.use_consts.clear();
                if let Some(body) = body {
                    self.stmts(f, body)?;
                    self.ns.clear();
                    self.use_classes.clear();
                    self.use_functions.clear();
                    self.use_consts.clear();
                }
            }
            Stmt::Use { uses, kind, .. } => {
                use p8_syntax::ast::UseKind;
                for u in *uses {
                    // From the name's tokens, not its span: a group's item
                    // (`use A\{B, C}`) spans the braces between them.
                    let full: Vec<u8> = u.name.parts.iter().flat_map(|t| self.text(t.span).iter().copied()).collect();
                    let full = full.strip_prefix(b"\\").unwrap_or(&full).to_vec();
                    let alias = match u.alias {
                        Some(a) => self.text(a.span).to_vec(),
                        None => full.rsplit(|&c| c == b'\\').next().unwrap_or(&full).to_vec(),
                    };
                    let kind = if u.kind == UseKind::Normal { *kind } else { u.kind };
                    match kind {
                        UseKind::Normal => self.use_classes.insert(alias.to_ascii_lowercase(), full),
                        UseKind::Function => self.use_functions.insert(alias.to_ascii_lowercase(), full),
                        UseKind::Const => self.use_consts.insert(alias, full),
                    };
                }
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
                            f.reset_temps(keep);
                        }
                        None => to_body.push(None),
                    }
                }
                let no_match = f.emit(Instr::Jump { target: 0 });
                f.reset_temps(f.floor);
                f.push_loop();
                let mut default = None;
                for (case, jump) in cases.iter().zip(to_body) {
                    match jump {
                        Some(at) => f.patch(at, f.here()),
                        None => default = Some(f.here()),
                    }
                    self.stmts(f, case.body)?;
                    f.reset_temps(f.floor);
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
                    // `global $$name`: `$$name = &$GLOBALS[$name]`.
                    if let Expr::IndirectVariable { name, .. } = v {
                        let n = self.expr(f, name)?;
                        f.emit(Instr::rare(Rare::VarBind, n, 0, 1, 0));
                        continue;
                    }
                    let Expr::Variable { name, .. } = v else {
                        return Err(self.unsupported(v.span(), "this global statement"));
                    };
                    let text = self.text(*name);
                    let var = f.cv(&text[1..]);
                    let name_reg = f.name_constant(Frozen::Str(Text::from(&text[1..])));
                    let name_index = name_reg & ((1 << KIND_SHIFT) - 1);
                    f.emit(Instr::BindGlobal { var, name: name_index });
                }
            }
            Stmt::Const { consts, .. } => {
                for c in *consts {
                    let full = join(&self.ns, self.text(c.name.span));
                    let name = f.constant(Frozen::Str(Text::from(full.as_slice())));
                    let value = self.expr(f, c.value)?;
                    let site = f.site(false, Text::from("define"), self.sym(b"define"), Text::from("define"));
                    let base = f.temp();
                    let second = f.temp();
                    f.emit(Instr::Move { dst: base, src: name });
                    f.emit(Instr::Move { dst: second, src: value });
                    let dst = f.temp();
                    f.emit(Instr::Call { site, base, argc: 2, dst });
                }
            }
            Stmt::Static { vars, .. } => {
                for v in *vars {
                    // `static $x = 1` may come as an assignment.
                    let (target, default) = match v.var {
                        Expr::Assign { var, expr, .. } if v.default.is_none() => (*var, Some(*expr)),
                        other => (other, v.default),
                    };
                    let var = self.var_target(f, target)?;
                    let index = f.statics;
                    f.statics += 1;
                    let at = f.emit(Instr::StaticVar { var, index, skip: 0 });
                    if let Some(init) = default {
                        let value = self.expr(f, init)?;
                        f.emit(Instr::Assign { var, src: value });
                    }
                    f.patch(at, f.here());
                }
            }
            Stmt::Unset { vars, .. } => {
                for v in *vars {
                    if let Some(name) = self.globals_dim(v) {
                        let n = self.expr(f, name)?;
                        f.emit(Instr::rare(Rare::VarUnset, n, 1, 0, 0));
                        continue;
                    }
                    match v {
                        // `unset($_POST)`: the global itself, which stays
                        // bound here (an auto-global).
                        Expr::Variable { name, .. } if SUPERGLOBALS.iter().any(|g| self.text(*name).get(1..) == Some(g.as_bytes())) => {
                            let n = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                            f.emit(Instr::rare(Rare::VarUnset, n, 1, 0, 0));
                        }
                        Expr::Variable { name, .. } => {
                            let var = f.cv(&self.text(*name)[1..]);
                            f.emit(Instr::UnsetVar { var });
                        }
                        // (A property's dimensions: through a reference to it.)
                        Expr::ArrayDimFetch { .. } => {
                            self.ref_for_unset = true;
                            let path = self.ref_dim_path(f, v);
                            self.ref_for_unset = false;
                            let (var, keys, n, append, held) = path?;
                            if append {
                                return Err(CompileError { message: "Cannot use [] for unsetting".into(), line: f.line });
                            }
                            f.emit(Instr::UnsetDim { var, keys, n });
                            if let Some(h) = held {
                                f.emit(Instr::Free { reg: h });
                            }
                        }
                        Expr::PropertyFetch { target, property, .. } => {
                            let obj = self.expr(f, target)?;
                            let name = self.prop_name_operand(f, property)?;
                            f.emit(Instr::UnsetProp { obj, name });
                        }
                        Expr::IndirectVariable { name, .. } => {
                            let n = self.expr(f, name)?;
                            f.emit(Instr::rare(Rare::VarUnset, n, 0, 0, 0));
                        }
                        other => return Err(self.unsupported(other.span(), "this unset")),
                    }
                }
            }
            Stmt::Class { .. } | Stmt::Interface { .. } | Stmt::Trait { .. } | Stmt::Enum { .. } if !self.declaring_top && self.top_classes.contains(&(stmt as *const Stmt as usize)) => {
                self.declaring_top = true;
                let r = self.stmt(f, stmt);
                self.declaring_top = false;
                r?;
            }
            Stmt::Class { attributes, modifiers, name, extends, implements, members, doc_comment, span } => {
                let (display, line) = self.declared_name(name.span);
                let flags = if self.has_attribute(attributes, b"allowdynamicproperties") { p8_runtime::object::CLASS_ALLOW_DYNAMIC } else { 0 };
                self.class_decl(f, display, line, modifiers, extends.as_ref(), implements, members, flags, (*attributes, *doc_comment, *span))?;
            }
            Stmt::Interface { attributes, name, extends, members, doc_comment, span } => {
                let (display, line) = self.declared_name(name.span);
                self.class_decl(f, display, line, &[], None, extends, members, CLASS_INTERFACE, (*attributes, *doc_comment, *span))?;
            }
            Stmt::Trait { attributes, name, members, doc_comment, span } => {
                let (display, line) = self.declared_name(name.span);
                self.class_decl(f, display, line, &[], None, &[], members, CLASS_TRAIT, (*attributes, *doc_comment, *span))?;
            }
            Stmt::Enum { attributes, name, backed_type, implements, members, doc_comment, span } => {
                self.enum_backing = backed_type.map(|t| Text::from(type_text(self, t).to_ascii_lowercase().as_slice()));
                let (display, line) = self.declared_name(name.span);
                self.class_decl(f, display, line, &[], None, implements, members, CLASS_ENUM | CLASS_FINAL, (*attributes, *doc_comment, *span))?;
            }
            Stmt::Nop { .. } => {}
            other => return Err(self.unsupported(other.span(), &format!("this statement ({})", kind(other)))),
        }
        Ok(())
    }

    /// Compile `e` for its effects only.
    fn discard(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<()> {
        match e {
            Expr::PostInc { var: Expr::PropertyFetch { .. } | Expr::ClassConstFetch { .. } | Expr::ArrayDimFetch { .. }, .. }
            | Expr::PostDec { var: Expr::PropertyFetch { .. } | Expr::ClassConstFetch { .. } | Expr::ArrayDimFetch { .. }, .. }
            | Expr::Unary {
                op: UnaryOp::PreInc | UnaryOp::PreDec,
                expr: Expr::PropertyFetch { .. } | Expr::ClassConstFetch { .. } | Expr::ArrayDimFetch { .. },
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
                self.unused_result = matches!(e, Expr::Assign { .. } | Expr::AssignRef { .. });
                let r = self.expr(f, e)?;
                self.unused_result = false;
                if r >> KIND_SHIFT == TEMP {
                    f.emit(Instr::Free { reg: r });
                }
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
                f.reset_temps(mark);
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
                f.reset_temps(mark);
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
        f.push_loop();
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
    /// The file's superglobal references (see `superglobals`): every `$`
    /// followed by a superglobal's name, as a prefix (`$_GETx` counts, as
    /// the body scan it replaces counted it).
    /// Bind the superglobals named between `start` and `end` of the source
    /// to the global variables, first thing (as `global $_SERVER;`).
    fn bind_superglobals(&mut self, f: &mut FnBuilder, start: usize, end: usize) {
        // Which are named there: from the file's one scan.
        let refs = self.superglobal_refs();
        let from = refs.partition_point(|&(at, _)| at < start);
        let named = refs[from..].iter().take_while(|&&(at, _)| at < end).fold(0u16, |m, &(_, i)| m | 1 << i);
        for (i, name) in SUPERGLOBALS.iter().enumerate() {
            if named & (1 << i) != 0 {
                let var = f.cv(name.as_bytes());
                let name_reg = f.name_constant(Frozen::Str(Text::from(*name)));
                f.emit(Instr::BindGlobal { var, name: name_reg & ((1 << KIND_SHIFT) - 1) });
            }
        }
    }

    fn superglobal_refs(&mut self) -> &[(usize, u8)] {
        let source = self.source;
        self.superglobals.get_or_insert_with(|| {
            let mut refs = Vec::new();
            for at in memchr::memchr_iter(b'$', source) {
                let rest = &source[at + 1..];
                // Every name in SUPERGLOBALS starts with `_`.
                if rest.first() != Some(&b'_') {
                    continue;
                }
                if let Some(i) = SUPERGLOBALS.iter().position(|name| rest.starts_with(name.as_bytes())) {
                    refs.push((at, i as u8));
                }
            }
            refs
        })
    }

    fn function(&mut self, name: Span, params: &[p8_syntax::ast::Param], body: &[StmtId], by_ref: bool) -> Result<Function> {
        let text = Text::from(self.text(name));
        let line = self.line(name);
        Ok(self.function_with(text, line, params, Body::Stmts(body), by_ref, &[])?.0)
    }

    /// A function, closure or arrow function: its code, and (for an arrow
    /// function) the variables it takes from where it is made.
    fn function_with<'a>(
        &mut self,
        name: Text,
        line: u32,
        params: &[p8_syntax::ast::Param],
        body: Body<'a>,
        by_ref: bool,
        uses: &[p8_syntax::ast::ClosureUse],
    ) -> Result<(Function, Vec<Text>)> {
        let sym = self.sym_lower(name.as_bytes());
        let mut f = FnBuilder::new(name, sym);
        f.meta = self.pending_meta.take().unwrap_or_default();
        f.returns_ref = by_ref;
        f.strict = self.strict;
        f.ret_type = self.pending_ret.take();
        f.line = line;
        if let Some((class, is_static)) = self.method.take() {
            f.class = Some(class);
            f.is_static = is_static;
        }
        for p in params {
            // A default making a partial application is checked as the
            // constant expression it is.
            if let Some(d) = p.default
                && makes_partial(d)
            {
                self.check_attr_const(d)?;
            }
            let text = self.text(p.name.span);
            f.cv(&text[1..]);
            let ty = p.ty.map(|t| self.type_decl(t));
            let default = p.default.and_then(|d| self.const_value(d).ok());
            let default_const = p.default.and_then(|d| self.default_const_name(d));
            let default_src = if default.is_none() { p.default.and_then(|d| self.default_src(d)).map(|s| Text::from(s.as_slice())) } else { None };
            let default_ns = default_src.as_ref().filter(|_| !self.ns.is_empty()).map(|_| Text::from(self.ns.as_slice()));
            let promoted = visibility_of(self, p.modifiers).is_some()
                || p.modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"readonly"))
                || set_visibility_of(self, p.modifiers)?.is_some();
            let attrs = self.attr_defs(p.attributes, 32)?;
            f.params.push(Param {
                name: Text::from(&text[1..]),
                by_ref: p.by_ref,
                has_default: p.default.is_some(),
                variadic: p.variadic,
                ty,
                default,
                default_const,
                default_src,
                default_ns,
                promoted,
                attrs,
            });
        }
        // A set hook written without its parameter takes `$value`.
        if std::mem::take(&mut self.hook_value) && params.is_empty() {
            f.cv(b"value");
            f.params.push(Param { name: Text::from("value"), by_ref: false, has_default: false, variadic: false, ty: None, default: None, default_const: None, default_src: None, default_ns: None, promoted: false, attrs: Vec::new() });
        }
        f.decl_line = f.line;
        if params.last().is_some_and(|p| p.variadic) {
            f.emit(Instr::CollectVariadic { var: reg(CV, params.len() - 1) });
        }
        if f.params.iter().any(|p| p.ty.is_some()) {
            f.emit(Instr::rare(Rare::VerifyArgs, 0, 0, 0, 0));
        }
        // Defaults: each runs only when its argument was not passed (the
        // call checks the required ones were).
        for (i, p) in params.iter().enumerate() {
            if let Some(default) = p.default {
                let at = f.emit(Instr::ArgDefault { n: i as u8, target: 0 });
                let var = reg(CV, i);
                let value = self.expr(&mut f, default)?;
                f.emit(Instr::Assign { var, src: value });
                f.reset_temps(f.floor);
                f.patch(at, f.here());
            }
        }
        // Constructor promotion: `public int $x` declares the property and
        // assigns the argument to it first thing.
        for (i, p) in params.iter().enumerate() {
            if !f.params[i].promoted {
                continue;
            }
            let visibility = visibility_of(self, p.modifiers).unwrap_or(Visibility::Public);
            let text = self.text(p.name.span);
            let name = Text::from(&text[1..]);
            let readonly = p.modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"readonly"));
            let class = self.scope.as_ref().map_or_else(|| Text::from(""), |s| s.name.clone());
            let set_declared = set_visibility_of(self, p.modifiers)?;
            let set_visibility = settled_set_visibility((class.as_bytes(), name.as_bytes()), visibility, set_declared, readonly, p.ty.is_some(), line)?;
            self.promoted.push(PropDef {
                name: name.clone(),
                default: if p.ty.is_some() { None } else { Some(Frozen::Null) },
                code: None,
                hooks: 0,
                visibility,
                set_visibility,
                is_static: false,
                readonly,
                // private(set) is implicitly final.
                is_final: p.modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"final")) || set_visibility == Some(Visibility::Private),
                ty: p.ty.map(|t| Text::from(type_text(self, t).as_slice())),
                ty_decl: p.ty.map(|t| self.type_decl(t)),
                meta: Meta { attrs: f.params[i].attrs.clone(), ..Meta::default() },
            });
            let obj = f.temp();
            f.emit(Instr::This { dst: obj });
            let key = f.name_constant(Frozen::Str(name));
            let dst = f.temp();
            f.emit(Instr::AssignProp { obj, name: key, val: reg(CV, i), dst, take: false });
            f.reset_temps(f.floor);
        }
        // `use ($a, &$b)`: variables after the parameters, set from the
        // closure's captured values on each call.
        for u in uses {
            let text = self.text(u.var.span);
            let r = f.cv(&text[1..]);
            f.bound.push(r & ((1 << KIND_SHIFT) - 1));
        }
        // Superglobals: the global variable in every scope, found at each
        // use (FnBuilder::cv).
        f.auto_globals = true;
        let mut captures = Vec::new();
        match body {
            Body::Stmts(body) => {
                self.stmts(&mut f, body)?;
                // Falling off the end returns null: checked against the type
                // (none returned), but for void.
                if f.ret_type.as_ref().is_some_and(|t| !t.is_void()) {
                    let null = f.constant(Frozen::Null);
                    f.emit(Instr::rare(Rare::VerifyReturn, null, 1, 0, 0));
                }
                f.emit(Instr::ReturnNull);
            }
            Body::Get(e) => {
                let r = self.expr(&mut f, e)?;
                f.emit(Instr::Return { src: r });
            }
            Body::Set(name, e) => {
                let obj = f.temp();
                f.emit(Instr::This { dst: obj });
                let key = f.name_constant(Frozen::Str(name));
                let val = self.expr(&mut f, e)?;
                let dst = f.temp();
                f.emit(Instr::AssignProp { obj, name: key, val, dst, take: false });
                f.emit(Instr::ReturnNull);
            }
            Body::Expr(e) => {
                let r = self.expr(&mut f, e)?;
                if f.ret_type.as_ref().is_some_and(|t| !t.is_void()) {
                    f.emit(Instr::rare(Rare::VerifyReturn, r, 0, 0, 0));
                }
                f.emit(Instr::Return { src: r });
                // An arrow function takes every variable it uses (but its
                // parameters) by value from where it is made.
                let nparams = f.params.len();
                for (name, &i) in &f.cvs {
                    if i >= nparams {
                        captures.push(Text::from(name.as_slice()));
                        f.bound.push(i as Reg);
                    }
                }
            }
        }
        f.resolve_gotos()?;
        f.check_size()?;
        Ok((f.finish(self.file.clone(), &mut self.const_order), captures))
    }

    /// Take apart the array in `src` into the targets of `items` (keyed,
    /// or by position; an empty slot is skipped; nested patterns too).
    /// `root`: the variable the array is in and the keys down to it, for
    /// by-reference targets (`[&$a, [&$b]] = $v` binds into `$v`).
    fn destructure(&mut self, f: &mut FnBuilder, items: &[p8_syntax::ast::ArrayItem], src: Reg, root: Option<(Reg, &[Reg])>) -> Result<()> {
        let mut index = 0;
        for item in items {
            if let Expr::Error { .. } = item.value {
                index += 1;
                continue;
            }
            if item.unpack || (item.by_ref && root.is_none()) {
                return Err(self.unsupported(item.span, "this destructuring by reference"));
            }
            let key = match item.key {
                Some(k) => self.expr(f, k)?,
                None => {
                    let k = f.constant(Frozen::Int(index));
                    index += 1;
                    k
                }
            };
            if let (true, Some((root, path)), Expr::Variable { name, .. }) = (item.by_ref, root, item.value) {
                let var = f.cv(&self.text(*name)[1..]);
                if path.is_empty() {
                    f.emit(Instr::RefDim { var, root, keys: key, n: 1, append: false });
                } else {
                    // The keys in a run of registers.
                    let keys: Vec<Reg> = path.iter().chain([&key]).map(|_| f.temp()).collect();
                    for (&dst, &src) in keys.iter().zip(path.iter().chain([&key])) {
                        f.emit(Instr::Move { dst, src });
                    }
                    f.emit(Instr::RefDim { var, root, keys: keys[0], n: keys.len() as u8, append: false });
                }
                continue;
            }
            // zend_verify_list_assign_target: a call is not a target.
            if matches!(item.value, Expr::Call { .. } | Expr::MethodCall { .. } | Expr::StaticCall { .. } | Expr::Binary { op: BinaryOp::Pipe, .. }) {
                return Err(CompileError { message: "Can't use function return value in write context".into(), line: self.line(item.value.span()) });
            }
            let elem = f.temp();
            match item.value {
                // Fetched for writing, its references made below: no warning
                // when it is not there yet.
                Expr::Array { items, .. } if root.is_some() && items.iter().any(|i| i.by_ref) => {
                    f.emit(Instr::FetchDim { dst: elem, arr: src, key, quiet: true, empty: false });
                }
                _ => {
                    f.emit(Instr::FetchList { dst: elem, arr: src, key });
                }
            }
            match item.value {
                Expr::Array { items, .. } => {
                    let path: Vec<Reg> = root.map(|(_, p)| p.iter().copied().chain([key]).collect()).unwrap_or_default();
                    self.destructure(f, items, elem, root.map(|(r, _)| (r, path.as_slice())))?
                }
                target => self.assign_from(f, target, elem)?,
            }
        }
        Ok(())
    }

    /// `target = <the value in val>`, for destructuring.
    /// `$a[x][][y] = v` is `$a[x][] = [y => v]`: an append before the last
    /// dimension starts a new array the rest of the path builds. Whether
    /// `target` had one (then it is assigned).
    fn assign_inner_append(&mut self, f: &mut FnBuilder, target: &Expr, val: Reg) -> Result<bool> {
        let mut outer = Vec::new();
        let mut cur = target;
        let mut split = None;
        while let Expr::ArrayDimFetch { array, dim, .. } = cur
            && self.globals_dim(cur).is_none()
        {
            if dim.is_none() && !outer.is_empty() {
                split = Some((cur, outer.len()));
            }
            outer.push(*dim);
            cur = array;
        }
        let Some((append_at, rest)) = split else { return Ok(false) };
        // The dimensions after the split, innermost first.
        let mut value = val;
        for dim in outer[..rest].iter() {
            let arr = f.temp();
            f.emit(Instr::NewArray { dst: arr, n: 0 });
            match dim {
                Some(k) => {
                    let key = self.expr(f, k)?;
                    f.emit(Instr::ArraySet { arr, key, val: value, take: false });
                }
                None => {
                    f.emit(Instr::ArrayPush { arr, val: value, take: false });
                }
            }
            value = arr;
        }
        self.assign_from(f, append_at, value)?;
        Ok(true)
    }

    fn assign_from(&mut self, f: &mut FnBuilder, target: &Expr, val: Reg) -> Result<()> {
        if let Some(name) = self.globals_dim(target) {
            let var = self.global_ref(f, name)?;
            f.emit(Instr::Assign { var, src: val });
            return Ok(());
        }
        if self.assign_inner_append(f, target, val)? {
            return Ok(());
        }
        match target {
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") && self.text(*name) != b"$this" => {
                let var = f.cv(&self.text(*name)[1..]);
                f.emit(Instr::Assign { var, src: val });
            }
            Expr::ArrayDimFetch { .. } if dim_root_is_prop(target) => {
                let (base, n, append) = self.prop_dim_path(f, target)?;
                let dst = f.temp();
                f.emit(Instr::AssignPropDim { op: None, base, n, append, val, dst });
            }
            Expr::ArrayDimFetch { .. } => {
                let (var, keys, n, append) = self.dim_path(f, target)?;
                let dst = f.temp();
                f.emit(Instr::AssignDim { var, keys, n, append, val, dst });
                self.free_static_root(f, target, var);
            }
            Expr::PropertyFetch { target: obj, property, .. } => {
                let obj = self.expr(f, obj)?;
                let name = self.prop_name_operand(f, property)?;
                let dst = f.temp();
                f.emit(Instr::AssignProp { obj, name, val, dst, take: false });
            }
            Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                let class = self.class_ref(f, class)?;
                let name = f.name_constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                let dst = f.temp();
                f.emit(Instr::AssignStaticProp { class, name, val, dst });
            }
            Expr::IndirectVariable { name, .. } => {
                let var = self.var_ref(f, name)?;
                f.emit(Instr::Assign { var, src: val });
            }
            other => return Err(self.unsupported(other.span(), "this destructuring target")),
        }
        Ok(())
    }

    /// `MakeClosure` of `func` into a new temporary (not a variable: a
    /// `use` of the variable being assigned takes its old value).
    fn make_closure(&mut self, f: &mut FnBuilder, mut func: Function, is_static: bool) -> Reg {
        // `static function`: ZEND_ACC_STATIC (reflection shows it).
        func.is_static |= is_static;
        f.lambdas.push(Arc::new(func));
        let dst = f.temp();
        f.emit(Instr::MakeClosure { dst, func: (f.lambdas.len() - 1) as u16, is_static });
        dst
    }

    /// The name PHP 8.4 gives a closure: where it is made.
    fn closure_name(&self, f: &FnBuilder, line: u32) -> Text {
        let place = if f.name.as_bytes() == b"{main}" {
            self.file.to_string()
        } else {
            let name = String::from_utf8_lossy(f.name.as_bytes());
            match &f.class {
                Some(c) => format!("{}::{name}()", String::from_utf8_lossy(c.as_bytes())),
                None => format!("{name}()"),
            }
        };
        Text::from(format!("{{closure:{place}:{line}}}").as_bytes())
    }

    /// A declared class's name in the current namespace, and its line.
    fn declared_name(&self, name: Span) -> (Text, u32) {
        (Text::from(join(&self.ns, self.text(name)).as_slice()), self.line(name))
    }

    /// `new class(args) { ... }`: declare the class (once; PHP names it
    /// "class@anonymous", or after its parent, then a NUL, where it is),
    /// and make one.
    fn anonymous_class(&mut self, f: &mut FnBuilder, class: &Expr, args: &[p8_syntax::ast::Arg], hint: Option<Reg>) -> Result<Reg> {
        let Expr::AnonymousClass { attributes, modifiers, extends, implements, members, span, .. } = class else { unreachable!("an anonymous class") };
        let line = self.line(*span);
        let base = match extends {
            Some(n) => self.class_name(self.text(n.span)),
            None => b"class".to_vec(),
        };
        let name = [base.as_slice(), b"@anonymous\0", self.file.as_bytes(), format!(":{line}${:x}", self.anonymous).as_bytes()].concat();
        self.anonymous += 1;
        let display = Text::from(name.as_slice());
        // Its methods are compiled in the middle of this function's code.
        let method = self.method.take();
        let promoted = std::mem::take(&mut self.promoted);
        let declared = self.class_decl(f, display.clone(), line, modifiers, extends.as_ref(), implements, members, 0, (*attributes, None, *span));
        self.method = method;
        self.promoted = promoted;
        let def = declared?;
        // Declared when the expression runs, not before the file does.
        self.classes.retain(|c| !Arc::ptr_eq(c, &def));
        let class = f.class_ref(ClassRef::Named(self.named(display)));
        let dst = match hint {
            Some(h) if h >> KIND_SHIFT == TEMP => h,
            _ => f.temp(),
        };
        f.emit(Instr::New { dst, class });
        if needs_spread(args) {
            let list = self.spread_list(f, args, None)?;
            f.emit(Instr::ConstructSpread { obj: dst, args: list });
        } else {
            let (base, argc) = self.args_by_value(f, args)?;
            f.emit(Instr::Construct { obj: dst, base, argc });
        }
        Ok(dst)
    }

    /// A class or interface declaration: compiled once, linked per request.
    #[allow(clippy::too_many_arguments)] // one per part of the declaration's syntax
    fn class_decl(
        &mut self,
        f: &mut FnBuilder,
        display: Text,
        line: u32,
        modifiers: &[p8_syntax::lexer::token::Token],
        extends: Option<&p8_syntax::ast::Name>,
        implements: &[p8_syntax::ast::Name],
        members: &[p8_syntax::ast::ClassMember],
        class_kind: u32,
        decl: (&[p8_syntax::ast::AttributeGroup], Option<Span>, Span),
    ) -> Result<Arc<ClassDef>> {
        use p8_syntax::ast::{ClassMember, TraitAdaptation};
        // Taken now: a class its methods declare is not a top-level one.
        let top = std::mem::take(&mut self.declaring_top);
        let interface = class_kind == CLASS_INTERFACE;
        let name_of = |c: &Self, n: &p8_syntax::ast::Name| Text::from(c.class_name(c.text(n.span)).as_slice());
        let (parent, interfaces) = if interface {
            (None, extends.iter().map(|n| name_of(self, n)).chain(implements.iter().map(|n| name_of(self, n))).collect())
        } else {
            (extends.map(|n| name_of(self, n)), implements.iter().map(|n| name_of(self, n)).collect())
        };
        let mut flags = class_kind;
        for m in modifiers {
            match self.text(m.span).to_ascii_lowercase().as_slice() {
                b"abstract" => flags |= CLASS_ABSTRACT,
                b"final" => flags |= CLASS_FINAL,
                b"readonly" => flags |= CLASS_READONLY,
                _ => {}
            }
        }
        let saved = self.scope.replace(ClassScope { name: display.clone(), parent: parent.clone(), in_trait: class_kind == CLASS_TRAIT });
        let (mut constants, mut properties, mut methods) = (Vec::new(), Vec::new(), Vec::new());
        let (mut traits, mut trait_rules) = (Vec::new(), Vec::new());
        let mut cases = Vec::new();
        let mut case_meta = Vec::new();
        let enum_backing = self.enum_backing.take();
        let class_meta = self.meta_of(decl.0, decl.1, decl.2, 1)?;
        for member in members {
            match member {
                ClassMember::Property { attributes, modifiers, ty, entries, doc_comment, .. } => {
                    let words: Vec<Vec<u8>> = modifiers.iter().map(|m| self.text(m.span).to_ascii_lowercase()).collect();
                    let has = |w: &[u8]| words.iter().any(|x| x == w);
                    let set_declared = set_visibility_of(self, modifiers)?;
                    if has(b"static") && set_declared.is_some() {
                        let line = entries.first().map_or(line, |e| self.line(e.name.span));
                        return Err(CompileError { message: "Static property may not have asymmetric visibility".into(), line });
                    }
                    let visibility = visibility_of(self, modifiers).unwrap_or(Visibility::Public);
                    let readonly = has(b"readonly") || flags & CLASS_READONLY != 0;
                    for (n, e) in entries.iter().enumerate() {
                        let text = self.text(e.name.span);
                        let (default, code) = match e.default {
                            Some(x) => {
                                let (v, code) = self.const_init(x)?;
                                (Some(v), code)
                            }
                            None if ty.is_some() => (None, None),
                            None => (Some(Frozen::Null), None),
                        };
                        let meta = self.meta_of(attributes, if n == 0 { *doc_comment } else { None }, e.span, 8)?;
                        let name = Text::from(text.strip_prefix(b"$").unwrap_or(text));
                        let set_visibility =
                            settled_set_visibility((display.as_bytes(), name.as_bytes()), visibility, set_declared, readonly && !has(b"static"), ty.is_some(), self.line(e.name.span))?;
                        properties.push(PropDef {
                            meta,
                            name,
                            default,
                            code,
                            hooks: 0,
                            visibility,
                            set_visibility,
                            is_static: has(b"static"),
                            readonly,
                            // private(set) is implicitly final.
                            is_final: has(b"final") || set_visibility == Some(Visibility::Private),
                            ty: ty.map(|t| Text::from(type_text(self, t).as_slice())),
                            ty_decl: ty.map(|t| self.type_decl(t)),
                        });
                    }
                }
                ClassMember::PropertyHook { attributes, modifiers, ty, name, default, hooks, doc_comment, span } => {
                    let text = self.text(name.span);
                    let pname = Text::from(text.strip_prefix(b"$").unwrap_or(text));
                    let visibility = visibility_of(self, modifiers).unwrap_or(Visibility::Public);
                    let line = self.line(name.span);
                    let set_declared = set_visibility_of(self, modifiers)?;
                    let set_visibility = settled_set_visibility((display.as_bytes(), pname.as_bytes()), visibility, set_declared, false, ty.is_some(), line)?;
                    let fatal = |message: &str| CompileError { message: message.to_string(), line };
                    let has = |c: &Self, w: &[u8]| modifiers.iter().any(|m| c.text(m.span).eq_ignore_ascii_case(w));
                    let prop_abstract = has(self, b"abstract");
                    if interface {
                        if has(self, b"final") {
                            return Err(fatal("Property in interface cannot be final"));
                        }
                        if visibility != Visibility::Public {
                            return Err(fatal("Property in interface cannot be protected or private"));
                        }
                        if prop_abstract {
                            return Err(fatal("Property in interface cannot be explicitly abstract. All interface members are implicitly abstract"));
                        }
                    }
                    if prop_abstract && !hooks.iter().any(|h| matches!(h.body, p8_syntax::ast::PropertyHookBody::None)) {
                        return Err(fatal(&format!(
                            "Abstract property {}::${} must specify at least one abstract hook",
                            String::from_utf8_lossy(display.as_bytes()),
                            String::from_utf8_lossy(pname.as_bytes())
                        )));
                    }
                    if hooks.is_empty() {
                        return Err(fatal("Property hook list must not be empty"));
                    }
                    if has(self, b"static") {
                        return Err(fatal("Cannot declare hooks for static property"));
                    }
                    if has(self, b"readonly") {
                        return Err(fatal("Hooked properties cannot be readonly"));
                    }
                    let mut seen: Vec<Vec<u8>> = Vec::new();
                    let mut bits = 0;
                    // Backed when a hook uses `$this->name` (or is `set => …`).
                    let mut backed = false;
                    for h in *hooks {
                        let hline = self.line(h.span);
                        let hfatal = |message: String| CompileError { message, line: hline };
                        let hname = self.text(h.name.span).to_ascii_lowercase();
                        if seen.contains(&hname) {
                            return Err(hfatal(format!("Cannot redeclare property hook \"{}\"", String::from_utf8_lossy(&hname))));
                        }
                        seen.push(hname.clone());
                        for m in h.modifiers {
                            let w = self.text(m.span).to_ascii_lowercase();
                            if matches!(w.as_slice(), b"static" | b"public" | b"protected" | b"private" | b"readonly" | b"abstract") {
                                return Err(hfatal(format!("Cannot use the {} modifier on a property hook", String::from_utf8_lossy(&w))));
                            }
                        }
                        let what = format!("{}::${}", String::from_utf8_lossy(display.as_bytes()), String::from_utf8_lossy(pname.as_bytes()));
                        if hname == b"get" && !h.params.is_empty() {
                            return Err(hfatal(format!("get hook of property {what} must not have a parameter list")));
                        }
                        if hname == b"set" && h.params.iter().any(|p| p.variadic) {
                            let p = h.params.iter().find(|p| p.variadic).expect("found");
                            return Err(hfatal(format!("Parameter {} of set hook {what} must not be variadic", String::from_utf8_lossy(self.text(p.name.span)))));
                        }
                        let hook_final = h.modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"final"));
                        let is_abstract = matches!(h.body, p8_syntax::ast::PropertyHookBody::None);
                        if is_abstract && (hook_final || (has(self, b"final") && !interface)) {
                            return Err(fatal("Property hook cannot be both abstract and final"));
                        }
                        if is_abstract && visibility == Visibility::Private {
                            return Err(fatal("Property hook cannot be both abstract and private"));
                        }
                        let kind = self.text(h.name.span).to_ascii_lowercase();
                        let bit = match kind.as_slice() {
                            b"get" => HOOK_GET,
                            b"set" => HOOK_SET,
                            _ => {
                                return Err(CompileError {
                                    message: format!(
                                        "Unknown hook \"{}\" for property {}::${}, expected \"get\" or \"set\"",
                                        String::from_utf8_lossy(self.text(h.name.span)),
                                        String::from_utf8_lossy(display.as_bytes()),
                                        String::from_utf8_lossy(pname.as_bytes())
                                    ),
                                    line: self.line(h.span),
                                });
                            }
                        };
                        let body = match h.body {
                            // Abstract: a method the class (or a subclass) implements.
                            p8_syntax::ast::PropertyHookBody::None => Body::Stmts(&[]),
                            p8_syntax::ast::PropertyHookBody::Statements(s) => Body::Stmts(s),
                            p8_syntax::ast::PropertyHookBody::Expr(e) if bit == HOOK_GET => Body::Get(e),
                            p8_syntax::ast::PropertyHookBody::Expr(e) => {
                                backed = true;
                                Body::Set(pname.clone(), e)
                            }
                        };
                        if uses_own_property(self.text(h.span), pname.as_bytes()) {
                            backed = true;
                        }
                        if !is_abstract {
                            bits |= bit;
                        }
                        self.method = Some((display.clone(), false));
                        self.hook_value = bit == HOOK_SET;
                        let hname = Text::from([b"$", pname.as_bytes(), b"::", kind.as_slice()].concat().as_slice());
                        let line = self.line(h.span);
                        let r = self.function_with(hname, line, h.params, body, h.by_ref, &[]);
                        self.hook_value = false;
                        self.method = None;
                        let (func, _) = r?;
                        methods.push(MethodDef {
                            func: Arc::new(func),
                            visibility,
                            is_static: false,
                            is_abstract,
                            is_final: hook_final,
                        });
                    }
                    // A default value is stored: backed too.
                    if !backed && bits != 0 && default.is_none() {
                        bits |= HOOK_VIRTUAL;
                    }
                    // A virtual property has a set visibility only with both
                    // hooks (checked after inheritance when there is a parent).
                    if parent.is_none() && set_visibility.is_some() && bits & HOOK_VIRTUAL != 0 && seen.len() < 2 {
                        let what = if seen.iter().any(|h| h == b"get") { "Read-only" } else { "Write-only" };
                        return Err(CompileError {
                            message: format!(
                                "{what} virtual property {}::${} must not specify asymmetric visibility",
                                String::from_utf8_lossy(display.as_bytes()),
                                String::from_utf8_lossy(pname.as_bytes())
                            ),
                            line: hooks.last().map_or(line, |h| self.line(h.span)),
                        });
                    }
                    let (default, code) = match default {
                        Some(x) => {
                            let (v, code) = self.const_init(x)?;
                            (Some(v), code)
                        }
                        None if ty.is_some() || bits & HOOK_VIRTUAL != 0 => (None, None),
                        None => (Some(Frozen::Null), None),
                    };
                    if !interface {
                        let meta = self.meta_of(attributes, *doc_comment, *span, 8)?;
                        properties.push(PropDef {
                            meta,
                            name: pname,
                            default,
                            code,
                            hooks: bits,
                            visibility,
                            set_visibility,
                            is_static: false,
                            readonly: modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"readonly")),
                            // private(set) is implicitly final.
                            is_final: modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"final")) || set_visibility == Some(Visibility::Private),
                            ty: ty.map(|t| Text::from(type_text(self, t).as_slice())),
                            ty_decl: ty.map(|t| self.type_decl(t)),
                        });
                    }
                }
                ClassMember::Const { attributes, modifiers, consts, doc_comment, .. } => {
                    for c in *consts {
                        let (value, code) = self.const_init(c.value)?;
                        let meta = self.meta_of(attributes, *doc_comment, c.span, 16)?;
                        constants.push(ConstDef {
                            meta,
                            name: Text::from(self.text(c.name.span)),
                            value,
                            code,
                            visibility: visibility_of(self, modifiers).unwrap_or(Visibility::Public),
                            is_final: modifiers.iter().any(|m| self.text(m.span).eq_ignore_ascii_case(b"final")),
                        });
                    }
                }
                ClassMember::Method { attributes, modifiers, by_ref, name, params, body, return_type, doc_comment, span } => {
                    let words: Vec<Vec<u8>> = modifiers.iter().map(|m| self.text(m.span).to_ascii_lowercase()).collect();
                    let has = |w: &[u8]| words.iter().any(|x| x == w);
                    let is_static = has(b"static");
                    let meta = self.meta_of(attributes, *doc_comment, *span, 4)?;
                    self.method = Some((display.clone(), is_static));
                    self.pending_ret = return_type.map(|t| self.type_decl(t));
                    self.pending_meta = Some(meta);
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
                ClassMember::Case { attributes, name, value, doc_comment, span } => {
                    let value = match value {
                        Some(v) => Some(self.const_value(v)?),
                        None => None,
                    };
                    case_meta.push(self.meta_of(attributes, *doc_comment, *span, 16)?);
                    cases.push((Text::from(self.text(name.span)), value));
                }
                ClassMember::TraitUse { traits: used, adaptations, .. } => {
                    for t in *used {
                        traits.push(name_of(self, t));
                    }
                    for a in *adaptations {
                        let (method, insteadof, alias, visibility) = match a {
                            TraitAdaptation::Precedence { method, insteadof, .. } => (method, insteadof.iter().map(|n| name_of(self, n)).collect(), None, None),
                            TraitAdaptation::Alias { method, alias, visibility, .. } => {
                                let visibility = visibility.and_then(|v| match self.text(v.span).to_ascii_lowercase().as_slice() {
                                    b"public" => Some(Visibility::Public),
                                    b"protected" => Some(Visibility::Protected),
                                    b"private" => Some(Visibility::Private),
                                    _ => None,
                                });
                                (method, Vec::new(), alias.map(|t| Text::from(self.text(t.span))), visibility)
                            }
                        };
                        trait_rules.push(TraitRule {
                            trait_name: method.trait_name.as_ref().map(|n| name_of(self, n)),
                            method: Text::from(self.text(method.method.span)),
                            insteadof,
                            alias,
                            visibility,
                        });
                    }
                }
            }
        }
        self.scope = saved;
        let sym = self.sym_lower(display.as_bytes());
        let deps = parent.iter().chain(&interfaces).chain(&traits).map(|n| self.sym_lower(n.as_bytes())).collect();
        let def = ClassDef { name: display, sym, deps, parent, interfaces, flags, constants, properties, methods, traits, trait_rules, cases, enum_backing, line, meta: class_meta, case_meta };
        let def = Arc::new(def);
        if top {
            self.classes.push(Arc::clone(&def));
        }
        f.decls.push(Arc::clone(&def));
        f.emit(Instr::DeclareClass { class: (f.decls.len() - 1) as u16 });
        Ok(def)
    }

    /// A constant expression in a declaration (a property default, a class
    /// constant): literals, and arrays of them, so far.
    /// A declaration's constant expression: its value now when it is a
    /// literal; otherwise the code computing it (`self` is this class),
    /// run when PHP evaluates it.
    fn const_init(&mut self, e: &Expr) -> Result<(Frozen, Option<Arc<Function>>)> {
        if let Ok(v) = self.const_value(e) {
            return Ok((v, None));
        }
        let line = self.line(e.span());
        let method = self.method.take();
        let r = self.function_with(Text::from(CONST_EXPR), line, &[], Body::Expr(e), false, &[]);
        self.method = method;
        Ok((Frozen::Null, Some(Arc::new(r?.0))))
    }

    /// `__DIR__`: the file's directory.
    fn dir(&self) -> Vec<u8> {
        match self.file.rfind('/') {
            Some(0) => b"/".to_vec(),
            Some(i) => self.file.as_bytes()[..i].to_vec(),
            None => b".".to_vec(),
        }
    }

    /// Whether `e` is known at compile time throughout: literals, the
    /// file's magic constants, and arrays, concatenations and signs of
    /// those (what const_value folds). An array of such is one constant
    /// (Composer's class map: `__DIR__ . '/..' . '/x.php'`, 15,000 of them).
    fn foldable(&self, e: &Expr) -> bool {
        match e {
            Expr::MagicConst { kind: MagicConstKind::File | MagicConstKind::Dir | MagicConstKind::Line | MagicConstKind::Namespace, .. } => true,
            Expr::Binary { left, op: BinaryOp::Concat, right, .. } => self.foldable(left) && self.foldable(right),
            Expr::Unary { op: UnaryOp::Minus | UnaryOp::Plus, expr, .. } => is_literal(expr),
            Expr::Array { items, .. } => items.iter().all(|i| {
                !i.unpack && !i.by_ref && i.key.is_none_or(|k| is_literal(k) && !float_key(k)) && self.foldable(i.value)
            }),
            other => is_literal(other),
        }
    }

    fn const_value(&self, e: &Expr) -> Result<Frozen> {
        if let Some(v) = self.literal(e) {
            return Ok(v);
        }
        match e {
            Expr::MagicConst { kind: MagicConstKind::File, .. } => return Ok(Frozen::Str(Text::from(self.file.as_bytes()))),
            Expr::MagicConst { kind: MagicConstKind::Dir, .. } => return Ok(Frozen::Str(Text::from(self.dir().as_slice()))),
            Expr::MagicConst { kind: MagicConstKind::Line, span } => return Ok(Frozen::Int(i64::from(self.line(*span)))),
            Expr::MagicConst { kind: MagicConstKind::Namespace, .. } => return Ok(Frozen::Str(Text::from(self.ns.as_slice()))),
            _ => {}
        }
        match e {
            Expr::Unary { op: op @ (UnaryOp::Minus | UnaryOp::Plus), expr, .. } => match (op, self.const_value(expr)?) {
                (UnaryOp::Minus, Frozen::Int(i)) => Ok(i.checked_neg().map_or(Frozen::Float(-(i as f64)), Frozen::Int)),
                (UnaryOp::Minus, Frozen::Float(x)) => Ok(Frozen::Float(p8_runtime::ops::negate_float(x))),
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
            Expr::Binary { left, op, right, .. } => {
                let op = match op {
                    BinaryOp::Plus => BinOp::Add,
                    BinaryOp::Minus => BinOp::Sub,
                    BinaryOp::Mul => BinOp::Mul,
                    BinaryOp::Concat => BinOp::Concat,
                    BinaryOp::BitAnd => BinOp::BitAnd,
                    BinaryOp::BitOr => BinOp::BitOr,
                    BinaryOp::BitXor => BinOp::BitXor,
                    BinaryOp::ShiftLeft => BinOp::Shl,
                    BinaryOp::ShiftRight => BinOp::Shr,
                    _ => return Err(self.unsupported(e.span(), "this constant expression")),
                };
                let (a, b) = (self.const_value(left)?.to_value(), self.const_value(right)?.to_value());
                let mut diags = Vec::new();
                match p8_runtime::ops::binary(op, &a, &b, &mut diags) {
                    Ok(p8_runtime::value::Value::Int(i)) if diags.is_empty() => Ok(Frozen::Int(i)),
                    Ok(p8_runtime::value::Value::Float(x)) if diags.is_empty() => Ok(Frozen::Float(x)),
                    Ok(p8_runtime::value::Value::Str(s)) if diags.is_empty() => Ok(Frozen::Str(Text::from(s.as_bytes()))),
                    _ => Err(self.unsupported(e.span(), "this constant expression")),
                }
            }
            // `C::class` names a class known now (an enum case's value).
            Expr::ClassConstFetch { class: Expr::Variable { name: class, .. }, constant: Expr::Variable { name, .. }, .. }
                if self.text(*name).eq_ignore_ascii_case(b"class")
                    && !self.text(*class).starts_with(b"$")
                    && !matches!(self.text(*class).to_ascii_lowercase().as_slice(), b"self" | b"parent" | b"static") =>
            {
                Ok(Frozen::Str(Text::from(self.class_name(self.text(*class)).as_slice())))
            }
            _ => Err(self.unsupported(e.span(), "this constant expression")),
        }
    }

    /// A parameter default that is a constant, named as reflection shows
    /// it: global and namespaced constants resolved (an unqualified one in
    /// a namespace is the namespaced name), `self::`/`parent::`/`static::`
    /// as written, other classes resolved.
    fn default_const_name(&self, e: &Expr) -> Option<Text> {
        match e {
            Expr::Variable { name, .. } => {
                let text = self.text(*name);
                let bare = text.strip_prefix(b"\\").unwrap_or(text);
                if text.starts_with(b"$") || matches!(bare.to_ascii_lowercase().as_slice(), b"true" | b"false" | b"null") {
                    return None;
                }
                Some(Text::from(self.ns_name(text, &self.use_consts, false).0.as_slice()))
            }
            Expr::ClassConstFetch { class: Expr::Variable { name: class, .. }, constant: Expr::Variable { name, .. }, .. } => {
                let (class, name) = (self.text(*class), self.text(*name));
                if class.starts_with(b"$") || name.starts_with(b"$") || name.eq_ignore_ascii_case(b"class") {
                    return None;
                }
                let class = match class.to_ascii_lowercase().as_slice() {
                    b"self" | b"parent" | b"static" => class.to_vec(),
                    _ => self.class_name(class),
                };
                Some(Text::from([class.as_slice(), b"::", name].concat().as_slice()))
            }
            _ => None,
        }
    }

    /// A parameter's default as PHP source that means the same anywhere
    /// in its namespace: the expression as written, its constant and
    /// class names resolved (`K` → `\App\K`, `C::X` → `\App\C::X`; `self`,
    /// `parent` and `static` stay). An unqualified constant that falls
    /// back to the global one stays unqualified. None for what a constant
    /// expression may not hold, or holds that depends on where it is
    /// written (magic constants, closures).
    fn default_src(&self, e: &Expr) -> Option<Vec<u8>> {
        let span = e.span();
        let mut edits: Vec<(Span, Vec<u8>)> = Vec::new();
        self.default_names(e, &mut edits)?;
        edits.sort_by_key(|(s, _)| s.start);
        let mut out = Vec::new();
        let mut at = span.start;
        for (s, text) in edits {
            out.extend_from_slice(&self.source[at..s.start]);
            out.extend_from_slice(&text);
            at = s.end;
        }
        out.extend_from_slice(&self.source[at..span.end]);
        Some(out)
    }

    /// default_src's names to replace, and None for what it cannot write.
    fn default_names(&self, e: &Expr, edits: &mut Vec<(Span, Vec<u8>)>) -> Option<()> {
        let class = |s: &Self, name: Span, edits: &mut Vec<(Span, Vec<u8>)>| -> Option<()> {
            let text = s.text(name);
            if text.starts_with(b"$") {
                return None;
            }
            if !matches!(text.to_ascii_lowercase().as_slice(), b"self" | b"parent" | b"static") {
                edits.push((name, [b"\\".as_slice(), &s.class_name(text)].concat()));
            }
            Some(())
        };
        match e {
            Expr::Integer { .. } | Expr::Float { .. } | Expr::String { .. } | Expr::Boolean { .. } | Expr::Null { .. } => Some(()),
            Expr::Variable { name, .. } => {
                let text = self.text(*name);
                let bare = text.strip_prefix(b"\\").unwrap_or(text);
                if text.starts_with(b"$") {
                    return None;
                }
                if matches!(bare.to_ascii_lowercase().as_slice(), b"true" | b"false" | b"null") {
                    return Some(());
                }
                let (full, fallback) = self.ns_name(text, &self.use_consts, false);
                if fallback.is_none() {
                    edits.push((*name, [b"\\".as_slice(), &full].concat()));
                }
                Some(())
            }
            Expr::ClassConstFetch { class: Expr::Variable { name, .. }, constant: Expr::Variable { name: c, .. }, .. } if !self.text(*c).starts_with(b"$") => class(self, *name, edits),
            Expr::New { class: Expr::Variable { name, .. }, args, .. } => {
                class(self, *name, edits)?;
                args.iter().try_for_each(|a| if a.unpack { None } else { self.default_names(a.value, edits) })
            }
            Expr::Array { items, .. } => items.iter().try_for_each(|i| {
                if i.by_ref {
                    return None;
                }
                if let Some(k) = i.key {
                    self.default_names(k, edits)?;
                }
                self.default_names(i.value, edits)
            }),
            Expr::Unary { op, expr, .. } if !matches!(op, UnaryOp::PreInc | UnaryOp::PreDec | UnaryOp::ErrorSuppress | UnaryOp::Reference) => self.default_names(expr, edits),
            Expr::Binary { left, right, .. } => {
                self.default_names(left, edits)?;
                self.default_names(right, edits)
            }
            Expr::Ternary { condition, if_true, if_false, .. } => {
                self.default_names(condition, edits)?;
                if let Some(t) = if_true {
                    self.default_names(t, edits)?;
                }
                self.default_names(if_false, edits)
            }
            Expr::ArrayDimFetch { array, dim: Some(dim), .. } => {
                self.default_names(array, edits)?;
                self.default_names(dim, edits)
            }
            Expr::PropertyFetch { target, property: Expr::Variable { name, .. }, .. }
            | Expr::NullsafePropertyFetch { target, property: Expr::Variable { name, .. }, .. }
                if !self.text(*name).starts_with(b"$") =>
            {
                self.default_names(target, edits)
            }
            Expr::Cast { expr, .. } => self.default_names(expr, edits),
            _ => None,
        }
    }

    /// The index of the class a name expression names: `self` and
    /// `parent` resolved now, `static` when the code runs.
    fn class_ref(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<u16> {
        let name = match e {
            Expr::Variable { name, .. } if !self.text(*name).starts_with(b"$") => name,
            _ => {
                let r = self.expr(f, e)?;
                return Ok(f.class_ref(ClassRef::Dynamic(r)));
            }
        };
        let text = self.text(*name);
        let text = text.strip_prefix(b"\\").unwrap_or(text);
        let line = self.line(*name);
        let no_scope = |word: &str| CompileError { message: format!("Cannot use \"{word}\" when no class scope is active"), line };
        let in_trait = self.scope.as_ref().is_some_and(|s| s.in_trait);
        // zend_is_scope_known: a file's own code may be included from a
        // method and a closure may be bound to a class, so outside a class
        // their self, parent and static are resolved when they run.
        let scope_unknown = self.scope.is_none()
            && f.class.is_none()
            && (f.name.as_bytes() == b"{main}" || f.name.as_bytes().starts_with(b"{closure"));
        let class = match text.to_ascii_lowercase().as_slice() {
            b"self" if in_trait || scope_unknown => ClassRef::Scope,
            b"parent" if in_trait || scope_unknown => ClassRef::ParentScope,
            b"static" if scope_unknown => ClassRef::Static,
            b"self" => {
                let scope = self.scope.as_ref().map(|s| s.name.clone()).or_else(|| f.class.clone()).ok_or_else(|| no_scope("self"))?;
                ClassRef::Forward(self.named(scope))
            }
            b"parent" => {
                let Some(scope) = &self.scope else { return Err(no_scope("parent")) };
                let Some(parent) = scope.parent.clone() else {
                    return Err(CompileError { message: "Cannot use \"parent\" when current class scope has no parent".into(), line });
                };
                ClassRef::Forward(self.named(parent))
            }
            b"static" => {
                if self.scope.is_none() && f.class.is_none() {
                    return Err(no_scope("static"));
                }
                ClassRef::Static
            }
            _ => {
                let full = self.class_name(self.text(*name));
                ClassRef::Named(self.named(Text::from(full.as_slice())))
            }
        };
        Ok(f.class_ref(class))
    }

    /// A class a `catch` names.
    fn class_name_ref(&mut self, f: &mut FnBuilder, name: &p8_syntax::ast::Name) -> Result<u16> {
        let full = self.class_name(self.text(name.span));
        Ok(f.class_ref(ClassRef::Named(self.named(Text::from(full.as_slice())))))
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

    /// `prop_name` for the name operand of a property instruction (see
    /// `name_operand`): a literal name is a name-only constant.
    fn prop_name_operand(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<Reg> {
        match e {
            Expr::Variable { name, .. } if !self.text(*name).starts_with(b"$") => {
                Ok(f.name_constant(Frozen::Str(Text::from(self.text(*name)))))
            }
            other => self.expr(f, other),
        }
    }

    /// Arguments by value into consecutive temporaries: (base, count).
    /// A call with `...$spread` arguments: the arguments as one list,
    /// and `callee` called on it.
    fn spread_call(&mut self, f: &mut FnBuilder, callee: Reg, args: &[p8_syntax::ast::Arg]) -> Result<Reg> {
        let outer = std::mem::take(&mut self.spread_refs);
        let list = self.spread_list(f, args, Some(callee))?;
        let dst = f.temp();
        f.emit(Instr::CallSpread { dst, callee, args: list });
        // The list and the elements' references in it let go, so a
        // reference only the call needed is not one after.
        f.emit(Instr::Free { reg: list });
        for r in std::mem::replace(&mut self.spread_refs, outer) {
            f.emit(Instr::Free { reg: r });
        }
        Ok(dst)
    }

    /// The arguments of a call with `...` in it, as one array. With the
    /// `callee`, a variable, element or property passed where it takes a
    /// reference goes in as one (asked when the call runs).
    fn spread_list(&mut self, f: &mut FnBuilder, args: &[p8_syntax::ast::Arg], callee: Option<Reg>) -> Result<Reg> {
        let list = f.temp();
        f.emit(Instr::NewArray { dst: list, n: 0 });
        let mut named = false;
        for (n, arg) in args.iter().enumerate() {
            let line = self.line(arg.span);
            if named && arg.name.is_none() {
                let message = if arg.unpack { "Cannot use argument unpacking after named arguments" } else { "Cannot use positional argument after named argument" };
                return Err(CompileError { message: message.into(), line });
            }
            // Positional arguments come before any `...`: n is the parameter;
            // a named one is asked about by its name.
            if let Some(callee) = callee
                && !arg.unpack
                && self.can_ref(arg.value)
            {
                let flag = f.temp();
                let key = arg.name.map(|name| f.constant(Frozen::Str(Text::from(self.text(name.span)))));
                match key {
                    Some(key) => f.emit(Instr::rare(Rare::CallableArgByRef, flag, callee, key, u16::MAX)),
                    None => f.emit(Instr::rare(Rare::CallableArgByRef, flag, callee, 0, n as u16)),
                };
                let (by_ref, by_value) = match arg.value {
                    Expr::Variable { name, .. } => (f.cv(&self.text(*name)[1..]), None),
                    value => {
                        let r = f.temp();
                        self.ref_or_value_with(f, value, r, flag)?;
                        self.spread_refs.push(r);
                        (r, Some(r))
                    }
                };
                let value_at = f.emit(Instr::JumpIfNot { cond: flag, target: 0 });
                match key {
                    Some(key) => f.emit(Instr::rare(Rare::ArraySetRef, list, key, by_ref, 0)),
                    None => f.emit(Instr::rare(Rare::ArrayPushRef, list, by_ref, 0, 0)),
                };
                let end = f.emit(Instr::Jump { target: 0 });
                f.patch(value_at, f.here());
                let v = match by_value {
                    Some(r) => r,
                    None => self.expr(f, arg.value)?,
                };
                match key {
                    Some(key) => {
                        named = true;
                        f.emit(Instr::ArraySet { arr: list, key, val: v, take: false })
                    }
                    None => f.emit(Instr::ArrayPush { arr: list, val: v, take: false }),
                };
                f.patch(end, f.here());
                continue;
            }
            let v = self.expr(f, arg.value)?;
            match arg.name {
                // A named argument: its name is the key (matched to the
                // parameters when the call is made).
                Some(n) => {
                    named = true;
                    let key = f.constant(Frozen::Str(Text::from(self.text(n.span))));
                    f.emit(Instr::ArraySet { arr: list, key, val: v, take: false });
                }
                None if arg.unpack => {
                    f.emit(Instr::ArrayUnpack { arr: list, src: v });
                }
                None => {
                    f.emit(Instr::ArrayPush { arr: list, val: v, take: false });
                }
            }
        }
        Ok(list)
    }

    /// The register `&e` takes its reference from (BindRef and co. make
    /// it one), and whether it is a call's result (a reference only when
    /// the function returns by reference).
    fn ref_source(&mut self, f: &mut FnBuilder, e: &Expr, span: Span) -> Result<(Reg, bool)> {
        if let Some(name) = self.globals_dim(e) {
            return Ok((self.global_ref(f, name)?, false));
        }
        match e {
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") && self.text(*name) != b"$this" => Ok((f.cv(&self.text(*name)[1..]), false)),
            Expr::ArrayDimFetch { .. } => {
                let t = f.temp();
                let (root, keys, n, append, held) = self.ref_dim_path(f, e)?;
                f.emit(Instr::RefDim { var: t, root, keys, n, append });
                if let Some(h) = held {
                    f.emit(Instr::Free { reg: h });
                }
                Ok((t, false))
            }
            Expr::PropertyFetch { target, property, .. } => {
                let for_unset = std::mem::take(&mut self.ref_for_unset);
                let obj = self.expr(f, target)?;
                let name = self.prop_name(f, property)?;
                let dst = f.temp();
                f.emit(Instr::rare(Rare::PropRef, dst, obj, name, u16::from(for_unset)));
                Ok((dst, false))
            }
            Expr::IndirectVariable { name, .. } => Ok((self.var_ref(f, name)?, false)),
            // `&$this`: a reference to a copy (the object), as in PHP.
            Expr::Variable { name, .. } if self.text(*name) == b"$this" => {
                let t = f.temp();
                f.emit(Instr::This { dst: t });
                Ok((t, false))
            }
            Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                let class = self.class_ref(f, class)?;
                let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                let dst = f.temp();
                f.emit(Instr::rare(Rare::StaticRef, dst, name, 0, class));
                Ok((dst, false))
            }
            Expr::ClassConstFetch { class, constant: Expr::IndirectVariable { name, .. }, .. } => {
                let class = self.class_ref(f, class)?;
                let name = self.expr(f, name)?;
                let dst = f.temp();
                f.emit(Instr::rare(Rare::StaticRef, dst, name, 0, class));
                Ok((dst, false))
            }
            Expr::Call { .. } | Expr::MethodCall { .. } | Expr::StaticCall { .. } | Expr::NullsafeMethodCall { .. } => {
                let r = self.expr(f, e)?;
                // Mark the call (the last instruction) as wanting a reference.
                let last = f.code.len() - 1;
                let writes_r = match f.code[last] {
                    Instr::Call { dst, .. } | Instr::MethodCall { dst, .. } | Instr::StaticCall { dst, .. } | Instr::CallSpread { dst, .. } => dst == r,
                    _ => false,
                };
                if writes_r {
                    f.code.insert(last, Instr::rare(Rare::WantRef, r, 0, 0, 0));
                    let line = f.lines[last];
                    f.lines.insert(last, line);
                }
                Ok((r, true))
            }
            _ => Err(self.unsupported(span, "this assignment by reference")),
        }
    }

    /// `var = &…` with the reference source in `src`; the expression's register.
    fn bind_ref(&mut self, f: &mut FnBuilder, var: &Expr, src: Reg, call: bool, span: Span) -> Result<Reg> {
        Ok(match var {
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") && self.text(*name) != b"$this" => {
                let var = f.cv(&self.text(*name)[1..]);
                f.emit(if call { Instr::rare(Rare::BindRefResult, var, src, 0, 0) } else { Instr::BindRef { var, src } });
                // A temporary holding the reference lets go of it: a later
                // write to that register would go through to the variable.
                if src >> KIND_SHIFT == TEMP {
                    f.emit(Instr::Free { reg: src });
                }
                var
            }
            Expr::ArrayDimFetch { .. } => {
                let (root, keys, n, append, held) = self.ref_dim_path(f, var)?;
                f.emit(Instr::DimSetRef { root, keys, n, append, src });
                if let Some(h) = held {
                    f.emit(Instr::Free { reg: h });
                }
                src
            }
            Expr::PropertyFetch { target, property, .. } => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name(f, property)?;
                f.emit(Instr::rare(Rare::AssignPropRef, obj, name, src, 0));
                src
            }
            Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                let class = self.class_ref(f, class)?;
                let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                f.emit(Instr::rare(Rare::AssignStaticRef, name, src, 0, class));
                src
            }
            // `$$name = &...`: the variable named at run time.
            Expr::IndirectVariable { name, .. } => {
                let n = self.expr(f, name)?;
                f.emit(Instr::rare(Rare::VarBind, n, src, 0, 0));
                src
            }
            _ => return Err(self.unsupported(span, "this assignment by reference")),
        })
    }

    /// A declared type for checking: its T_* bits and class names (resolved
    /// in this namespace; `self` and `parent` kept, resolved when checked).
    fn type_decl(&self, t: &p8_syntax::ast::Type) -> p8_runtime::types::TypeDecl {
        use p8_runtime::types::*;
        use p8_syntax::ast::Type;
        let mut d = p8_runtime::types::TypeDecl::new(0, Vec::new(), Vec::new(), false);
        fn add(c: &Compiler, t: &Type, d: &mut p8_runtime::types::TypeDecl) {
            match t {
                Type::Simple(token) => add_word(c, c.text(token.span), d),
                Type::Name(name) => add_word(c, c.text(name.span), d),
                Type::Nullable(inner) => {
                    add(c, inner, d);
                    d.mask |= T_NULL;
                }
                Type::Union(parts) => {
                    for p in *parts {
                        add(c, p, d);
                    }
                }
                Type::Intersection(parts) => {
                    d.intersection = true;
                    for p in *parts {
                        add(c, p, d);
                    }
                }
            }
        }
        fn add_word(c: &Compiler, word: &[u8], d: &mut p8_runtime::types::TypeDecl) {
            let bit = match word.to_ascii_lowercase().as_slice() {
                b"int" => T_INT,
                b"float" => T_FLOAT,
                b"string" => T_STRING,
                b"bool" => T_BOOL,
                b"false" => T_FALSE,
                b"true" => T_TRUE,
                b"null" => T_NULL,
                b"array" => T_ARRAY,
                b"callable" => T_CALLABLE,
                b"object" => T_OBJECT,
                b"mixed" => T_MIXED,
                b"void" => T_VOID,
                b"never" => T_NEVER,
                b"static" => T_STATIC,
                b"iterable" => {
                    d.classes.push(Text::from("Traversable"));
                    d.written.push(Text::from("Traversable"));
                    T_ARRAY
                }
                // In a class (not a trait): its name, or its parent's.
                b"self" | b"parent" => {
                    let known = c.scope.as_ref().filter(|s| !s.in_trait).and_then(|s| {
                        if word.eq_ignore_ascii_case(b"self") { Some(s.name.clone()) } else { s.parent.clone() }
                    });
                    d.classes.push(known.unwrap_or_else(|| Text::from(word.to_ascii_lowercase().as_slice())));
                    d.written.push(Text::from(word.to_ascii_lowercase().as_slice()));
                    0
                }
                _ => {
                    // (class_name reads a leading backslash as fully qualified.)
                    let full = Text::from(c.class_name(word).as_slice());
                    d.classes.push(full.clone());
                    d.written.push(full);
                    0
                }
            };
            d.mask |= bit;
        }
        add(self, t, &mut d);
        p8_runtime::types::TypeDecl::new(d.mask, d.classes, d.written, d.intersection)
    }

    /// Call the callable [a, b] (a method named at run time).
    fn call_pair(&mut self, f: &mut FnBuilder, a: Reg, b: Reg, args: &[p8_syntax::ast::Arg]) -> Result<Reg> {
        let callee = f.temp();
        f.emit(Instr::NewArray { dst: callee, n: 0 });
        f.emit(Instr::ArrayPush { arr: callee, val: a, take: false });
        f.emit(Instr::ArrayPush { arr: callee, val: b, take: false });
        if needs_spread(args) {
            return self.spread_call(f, callee, args);
        }
        let (base, argc) = self.value_args(f, args, callee)?;
        f.emit(Instr::CallValue { dst: base, callee, base, argc });
        Ok(base)
    }

    /// A method call's arguments. The method is known only when the call
    /// runs, so an argument that can be a reference is decided then: a
    /// variable by `send`'s instruction; an element or property by
    /// `asks`'s, branching to the reference or the value.
    fn method_args(
        &mut self,
        f: &mut FnBuilder,
        args: &[p8_syntax::ast::Arg],
        send: impl Fn(u8, Reg, Reg) -> Instr,
        asks: impl Fn(Reg, u8) -> Instr,
    ) -> Result<(Reg, u8)> {
        let regs: Vec<Reg> = (0..args.len().max(1)).map(|_| f.temp()).collect();
        for (n, (arg, r)) in args.iter().zip(&regs).enumerate() {
            if arg.unpack || arg.name.is_some() {
                return Err(self.unsupported(arg.span, "spread or named arguments"));
            }
            match arg.value {
                Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") && !matches!(self.text(*name), b"$this" | b"$GLOBALS") => {
                    let var = f.cv(&self.text(*name)[1..]);
                    f.emit(send(n as u8, *r, var));
                }
                value @ (Expr::ArrayDimFetch { .. } | Expr::PropertyFetch { .. }) if self.can_ref_arg(value) => {
                    let flag = f.temp();
                    f.emit(asks(flag, n as u8));
                    self.ref_or_value_with(f, value, *r, flag)?;
                }
                value => self.expr_into(f, value, *r)?,
            }
        }
        Ok((regs[0], args.len() as u8))
    }

    /// The arguments of a call of a value (`$f(...)`, `[$o, 'm'](...)`): a
    /// variable, element or property goes as a reference when the callable
    /// takes that parameter by one, asked as the call runs (PHP's
    /// SEND_VAR_EX for a function known only then).
    fn value_args(&mut self, f: &mut FnBuilder, args: &[p8_syntax::ast::Arg], callee: Reg) -> Result<(Reg, u8)> {
        let regs: Vec<Reg> = (0..args.len().max(1)).map(|_| f.temp()).collect();
        for (n, (arg, r)) in args.iter().zip(&regs).enumerate() {
            match arg.value {
                value if self.can_ref_arg(value) && !matches!(value, Expr::Variable { name, .. } if matches!(self.text(*name), b"$this" | b"$GLOBALS")) => {
                    let flag = f.temp();
                    f.emit(Instr::rare(Rare::CallableArgByRef, flag, callee, 0, n as u16));
                    self.ref_or_value_with(f, value, *r, flag)?;
                }
                value => self.expr_into(f, value, *r)?,
            }
        }
        Ok((regs[0], args.len() as u8))
    }

    /// Argument `value` into `r`: a reference when `ask` (whose `dst` is
    /// filled in) says the callee takes one, else its value.
    fn ref_or_value(&mut self, f: &mut FnBuilder, value: &Expr, r: Reg, ask: Instr) -> Result<()> {
        let flag = f.temp();
        let ask = match ask {
            Instr::FuncArgByRef { site, n, .. } => Instr::FuncArgByRef { dst: flag, site, n },
            other => other,
        };
        f.emit(ask);
        self.ref_or_value_with(f, value, r, flag)
    }

    /// The branch on `flag`: the element or property as a reference, or its value.
    fn ref_or_value_with(&mut self, f: &mut FnBuilder, value: &Expr, r: Reg, flag: Reg) -> Result<()> {
        let by_value = f.emit(Instr::JumpIfNot { cond: flag, target: 0 });
        let appends = matches!(value, Expr::ArrayDimFetch { dim: None, .. });
        match value {
            Expr::PropertyFetch { target, property, .. } => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name(f, property)?;
                f.emit(Instr::rare(Rare::PropRef, r, obj, name, 0));
            }
            _ => {
                let (root, keys, count, append, held) = self.ref_dim_path(f, value)?;
                f.emit(Instr::RefDim { var: r, root, keys, n: count, append });
                if let Some(h) = held {
                    f.emit(Instr::Free { reg: h });
                }
            }
        }
        let end = f.emit(Instr::Jump { target: 0 });
        f.patch(by_value, f.here());
        if appends {
            // `$a[]` to a parameter that takes a value: PHP's error, when
            // the call shows it.
            let message = f.constant(Frozen::Str(Text::from("Cannot use [] for reading")));
            f.emit(Instr::rare(Rare::ThrowError, message, 0, 0, 0));
        } else {
            self.expr_into(f, value, r)?;
        }
        f.patch(end, f.here());
        Ok(())
    }

    /// An element or property that can be made a reference: a property,
    /// or a dimension path (no `[]` read) rooted in a variable, a property
    /// or a static property.
    /// An argument whose callee is found as the call runs may be `$a[]`
    /// (a new element, when the parameter takes a reference).
    fn can_ref_arg(&self, e: &Expr) -> bool {
        match e {
            Expr::ArrayDimFetch { array, dim: None, .. } => self.can_ref(array),
            _ => self.can_ref(e),
        }
    }

    fn can_ref(&self, e: &Expr) -> bool {
        let mut cur = e;
        while let Expr::ArrayDimFetch { array, dim, .. } = cur {
            if dim.is_none() {
                return false;
            }
            cur = array;
        }
        match cur {
            // `$GLOBALS` as a whole is a read-only copy, never a reference.
            Expr::Variable { name, .. } if std::ptr::eq(cur, e) && self.text(*name) == b"$GLOBALS" => false,
            Expr::Variable { name, .. } => self.text(*name).starts_with(b"$") && self.text(*name) != b"$this",
            Expr::PropertyFetch { .. } => true,
            Expr::ClassConstFetch { constant: Expr::Variable { name, .. }, .. } => !std::ptr::eq(cur, e) && self.text(*name).starts_with(b"$"),
            _ => false,
        }
    }

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
        while let Expr::ArrayDimFetch { array, dim, .. } = cur
            && self.globals_dim(cur).is_none()
        {
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
            Expr::String { value, .. } => Frozen::Str(Text::from(&*literal::string(value))),
            Expr::Boolean { value, .. } => Frozen::Bool(*value),
            Expr::Null { .. } => Frozen::Null,
            _ => return None,
        })
    }

    /// A variable written by foreach: its register.
    fn var_target(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<Reg> {
        match e {
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => Ok(f.cv(&self.text(*name)[1..])),
            other => Err(self.unsupported(other.span(), &format!("this foreach target ({})", kind(other)))),
        }
    }

    /// `$v[k1]..[kn]`: the variable, the keys in consecutive temporaries,
    /// and whether the last dimension is `[]`.
    /// dim_path for a reference (`&$o->a[k]`, `C::$a[k] = &…`): the root
    /// may be a property, made a reference in a temporary (also returned,
    /// to release after).
    fn ref_dim_path(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<(Reg, Reg, u8, bool, Option<Reg>)> {
        if !dim_root_is_prop(e) && !dim_root_is_static(self, e) {
            let (root, keys, n, append) = self.dim_path(f, e)?;
            return Ok((root, keys, n, append, None));
        }
        let mut dims = Vec::new();
        let mut cur = e;
        while let Expr::ArrayDimFetch { array, dim, .. } = cur
            && self.globals_dim(cur).is_none()
        {
            dims.push(*dim);
            cur = array;
        }
        dims.reverse();
        let (root, _) = self.ref_source(f, cur, cur.span())?;
        let append = dims.last().is_some_and(|d| d.is_none());
        if dims[..dims.len() - usize::from(append)].iter().any(|d| d.is_none()) {
            return Err(self.unsupported(e.span(), "[] before the last dimension"));
        }
        let keyed: Vec<&Expr> = dims.iter().flatten().copied().collect();
        let regs: Vec<Reg> = keyed.iter().map(|_| f.temp()).collect();
        for (r, d) in regs.iter().zip(&keyed) {
            self.expr_into(f, d, *r)?;
        }
        let keys = regs.first().copied().unwrap_or_else(|| f.temp());
        Ok((root, keys, regs.len() as u8, append, Some(root)))
    }

    /// After the write (or read) through a dim_path rooted in a static
    /// property: let go of the temporary holding a reference to it, which
    /// a later expression reusing the register would otherwise write
    /// through.
    fn free_static_root(&mut self, f: &mut FnBuilder, e: &Expr, root: Reg) {
        let mut cur = e;
        while let Expr::ArrayDimFetch { array, .. } = cur
            && self.globals_dim(cur).is_none()
        {
            cur = array;
        }
        if dim_root_is_static(self, cur) {
            f.emit(Instr::Free { reg: root });
        }
    }

    fn dim_path(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<(Reg, Reg, u8, bool)> {
        let mut dims = Vec::new();
        let mut cur = e;
        while let Expr::ArrayDimFetch { array, dim, .. } = cur
            && self.globals_dim(cur).is_none()
        {
            dims.push(*dim);
            cur = array;
        }
        dims.reverse();
        let var = self.dim_root(f, cur)?;
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

    /// `${'name'}` is `$name`, a compiled variable (not `$this`).
    fn literal_var(&self, f: &mut FnBuilder, name: &Expr) -> Option<Reg> {
        match self.literal(name) {
            Some(Frozen::Str(s)) if s.as_bytes() != b"this" => Some(f.cv(s.as_bytes())),
            _ => None,
        }
    }

    /// `$$name`: a temporary holding a reference to the variable it names.
    fn var_ref(&mut self, f: &mut FnBuilder, name: &Expr) -> Result<Reg> {
        if let Some(cv) = self.literal_var(f, name) {
            return Ok(cv);
        }
        let n = self.expr(f, name)?;
        let dst = f.temp();
        f.emit(Instr::rare(Rare::VarRef, dst, n, 0, 0));
        Ok(dst)
    }

    /// The attributes of a declaration: each name resolved, each argument
    /// a constant expression (computed when reflection asks).
    ///
    /// `target` is what the attributes are on (ATTR_TARGET_*): the builtin
    /// attribute classes are checked against it, as PHP does when compiling.
    fn attr_defs(&mut self, groups: &[p8_syntax::ast::AttributeGroup], target: u32) -> Result<Vec<AttrDef>> {
        let mut out: Vec<AttrDef> = Vec::new();
        // PHP reports a bad builtin attribute at the declaration it is on: the first
        // token after the last group.
        let decl_line = groups.last().map_or(1, |g| {
            let mut at = g.span.end;
            while self.source.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            self.line(Span::new(at, at))
        });
        for group in groups {
            for a in group.attributes {
                let name = Text::from(self.class_name(self.text(a.name.span)).as_slice());
                let line = self.line(a.span);
                if let Some(allowed) = builtin_attribute_targets(name.as_bytes()) {
                    let shown = String::from_utf8_lossy(name.as_bytes());
                    if allowed & target == 0 {
                        let message = format!("Attribute \"{shown}\" cannot target {} (allowed targets: {})", attr_target_names(target), attr_target_names(allowed));
                        return Err(CompileError { message, line: decl_line });
                    }
                    if out.iter().any(|o| o.name.as_bytes().eq_ignore_ascii_case(name.as_bytes())) {
                        return Err(CompileError { message: format!("Attribute \"{shown}\" must not be repeated"), line: decl_line });
                    }
                }
                let mut args = Vec::with_capacity(a.args.len());
                for arg in a.args {
                    if arg.unpack {
                        return Err(CompileError { message: "Cannot use unpacking in attribute argument list".into(), line: self.line(arg.span) });
                    }
                    self.check_attr_const(arg.value)?;
                    let (value, code) = self.const_init(arg.value)?;
                    args.push(AttrArg { name: arg.name.map(|t| Text::from(self.text(t.span))), value, code });
                }
                out.push(AttrDef { name, args, line });
            }
        }
        Ok(out)
    }

    /// An attribute argument is a constant expression: no calls, variables
    /// or assignments.
    fn check_attr_const(&self, e: &Expr) -> Result<()> {
        let invalid = || CompileError { message: "Constant expression contains invalid operations".into(), line: self.line(e.span()) };
        match e {
            // A first-class callable or partial application of a named
            // function or static method is one (PHP 8.5, 8.6), its bound
            // arguments constant expressions too (zend_compile_const_expr_fcc).
            Expr::Call { func, args, .. } | Expr::StaticCall { class: func, args, .. } if is_placeholder(args) || is_partial(args) => {
                if matches!(func, Expr::Variable { name, .. } if self.text(*name).starts_with(b"$")) {
                    return Err(invalid());
                }
                for arg in *args {
                    if !matches!(arg.value, Expr::Placeholder { .. } | Expr::VariadicPlaceholder { .. }) {
                        self.check_attr_const(arg.value)?;
                    }
                }
                Ok(())
            }
            Expr::Call { .. } | Expr::MethodCall { .. } | Expr::StaticCall { .. } | Expr::Assign { .. } | Expr::Closure { .. } | Expr::ArrowFunction { .. } => Err(invalid()),
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") && self.text(*name) != b"$this" => Err(invalid()),
            Expr::Array { items, .. } => {
                for item in *items {
                    if let Some(k) = item.key {
                        self.check_attr_const(k)?;
                    }
                    self.check_attr_const(item.value)?;
                }
                Ok(())
            }
            Expr::Binary { left, right, .. } => self.check_attr_const(left).and_then(|()| self.check_attr_const(right)),
            Expr::Unary { expr, .. } => self.check_attr_const(expr),
            _ => Ok(()),
        }
    }

    /// What reflection reads of a declaration: its doc comment, attributes
    /// and (from `span`) last line.
    fn meta_of(&mut self, groups: &[p8_syntax::ast::AttributeGroup], doc: Option<Span>, span: Span, target: u32) -> Result<Meta> {
        let attrs = self.attr_defs(groups, target)?;
        let doc = doc.map(|d| Text::from(self.text(d)));
        Ok(Meta { doc, attrs, end_line: self.line(Span::new(span.end.saturating_sub(1), span.end)) })
    }

    /// Whether the attribute groups name `attr` (lowercase, no namespace:
    /// the global attribute classes).
    fn has_attribute(&self, groups: &[p8_syntax::ast::AttributeGroup], attr: &[u8]) -> bool {
        groups.iter().flat_map(|g| g.attributes.iter()).any(|a| {
            let name = self.text(a.name.span);
            name.strip_prefix(b"\\").unwrap_or(name).eq_ignore_ascii_case(attr)
        })
    }

    /// `$GLOBALS[name]`: the name expression. It is the global variable of
    /// that name, read, written and unset in place (PHP 8.1's $GLOBALS).
    fn globals_dim<'e>(&self, e: &Expr<'e>) -> Option<&'e Expr<'e>> {
        match e {
            Expr::ArrayDimFetch { array: Expr::Variable { name, .. }, dim: Some(d), .. } if self.text(*name) == b"$GLOBALS" => Some(*d),
            _ => None,
        }
    }

    /// The global variable `name` (an expression) as a reference.
    fn global_ref(&mut self, f: &mut FnBuilder, name: &Expr) -> Result<Reg> {
        let n = self.expr(f, name)?;
        let dst = f.temp();
        f.emit(Instr::rare(Rare::VarRef, dst, n, 1, 0));
        // Dropped when the temporary goes out of use (reset_temps).
        f.ref_temps.push(usize::from(dst & ((1 << KIND_SHIFT) - 1)));
        Ok(dst)
    }

    /// The global variable `name`'s value (quietly for isset and ??).
    fn global_get(&mut self, f: &mut FnBuilder, name: &Expr, quiet: bool) -> Result<Reg> {
        let n = self.expr(f, name)?;
        let dst = f.temp();
        f.emit(Instr::rare(Rare::VarGet, dst, n, Reg::from(quiet) | 2, 0));
        Ok(dst)
    }

    /// `$$name` read (quietly for isset and ??).
    fn var_get(&mut self, f: &mut FnBuilder, name: &Expr, quiet: bool) -> Result<Reg> {
        if let Some(cv) = self.literal_var(f, name) {
            if quiet {
                return Ok(cv);
            }
            let dst = f.temp();
            f.emit(Instr::Move { dst, src: cv });
            return Ok(dst);
        }
        let n = self.expr(f, name)?;
        let dst = f.temp();
        f.emit(Instr::rare(Rare::VarGet, dst, n, Reg::from(quiet), 0));
        Ok(dst)
    }

    /// Whether a call's name is a builtin function's (which cannot be redeclared).
    fn is_builtin(&self, func: &Expr) -> bool {
        let Expr::Variable { name, .. } = func else { return false };
        let text = self.text(*name);
        if text.starts_with(b"$") {
            return false;
        }
        let (full, fallback) = self.ns_name(text, &self.use_functions, true);
        let name = fallback.unwrap_or(full).to_ascii_lowercase();
        self.registry.lookup(&name).is_some()
    }

    /// The register a dimension write starts from: a variable, or a
    /// temporary holding `$this`, a reference to a static property, or a
    /// call's result (written to and dropped, unless it is an object).
    fn dim_root(&mut self, f: &mut FnBuilder, root: &Expr) -> Result<Reg> {
        if let Some(name) = self.globals_dim(root) {
            return self.global_ref(f, name);
        }
        match root {
            Expr::Variable { name, .. } if self.text(*name) == b"$this" => {
                let dst = f.temp();
                f.emit(Instr::This { dst });
                Ok(dst)
            }
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => Ok(f.cv(&self.text(*name)[1..])),
            Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                let class = self.class_ref(f, class)?;
                let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                let dst = f.temp();
                f.emit(Instr::rare(Rare::StaticRef, dst, name, 0, class));
                Ok(dst)
            }
            // `C::${expr}[k] = v`: the static property named when it runs.
            Expr::ClassConstFetch { class, constant: Expr::IndirectVariable { name, .. }, .. } => {
                let class = self.class_ref(f, class)?;
                let name = self.expr(f, name)?;
                let dst = f.temp();
                f.emit(Instr::rare(Rare::StaticRef, dst, name, 0, class));
                Ok(dst)
            }
            Expr::Call { func, .. } if self.is_builtin(func) => {
                Err(CompileError { message: "Cannot use result of built-in function in write context".into(), line: self.line(root.span()) })
            }
            // (A pipe is a call: its result written into, GH-22490.)
            Expr::Call { .. } | Expr::MethodCall { .. } | Expr::StaticCall { .. } | Expr::NullsafeMethodCall { .. } | Expr::Binary { op: BinaryOp::Pipe, .. } | Expr::Znode { .. } => {
                let r = self.expr(f, root)?;
                let dst = f.temp();
                f.emit(Instr::Move { dst, src: r });
                Ok(dst)
            }
            Expr::IndirectVariable { name, .. } => self.var_ref(f, name),
            Expr::Variable { .. } => Err(self.unsupported(root.span(), "this array write")),
            _ => Err(CompileError { message: "Cannot use temporary expression in write context".into(), line: self.line(root.span()) }),
        }
    }

    /// Compile `e` into exactly `dst`, a temporary (never a variable: a
    /// result written there directly would bypass a reference it holds).
    fn expr_into(&mut self, f: &mut FnBuilder, e: &Expr, dst: Reg) -> Result<()> {
        debug_assert_eq!(dst >> KIND_SHIFT, TEMP, "expr_into targets a temporary");
        // A value already computed (the pipe's): moved where it goes, as
        // PHP sends a TMP (no copy left behind).
        if let Expr::Znode { reg, once: true, .. } = e {
            f.emit(Instr::AssignTake { var: dst, src: *reg as Reg });
            return Ok(());
        }
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
        if let Some(name) = self.globals_dim(e) {
            return self.global_get(f, name, false);
        }
        Ok(match e {
            // `$GLOBALS` as a whole: a read-only copy of the global variables.
            Expr::Variable { name, .. } if self.text(*name) == b"$GLOBALS" => {
                let dst = out(f, hint, mark);
                f.emit(Instr::rare(Rare::Globals, dst, 0, 0, 0));
                dst
            }
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
            Expr::Assign { var, expr, .. } => match (var, std::mem::take(&mut self.unused_result)) {
                // A used value: the value assigned, even when a destructor
                // the overwrite runs changes the variable (PHP's result).
                (Expr::Variable { name, .. }, false) if self.text(*name).starts_with(b"$") && self.text(*name) != b"$this" && !matches!(expr, Expr::Binary { .. }) => {
                    let target = f.cv(&self.text(*name)[1..]);
                    let src = self.expr(f, expr)?;
                    if src >> KIND_SHIFT == TEMP {
                        f.emit(Instr::Assign { var: target, src });
                        src
                    } else {
                        f.emit(Instr::Assign { var: target, src });
                        target
                    }
                }
                (var, _) => match var {
                // `[$a, $b] = …` / `list($a, $b) = …`: the value first, then
                // each target left to right; the result is the value.
                Expr::Array { items, .. } => {
                    // By-reference items bind into the source variable itself.
                    let root = match expr {
                        Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") && self.text(*name) != b"$this" => {
                            Some(f.cv(&self.text(*name)[1..]))
                        }
                        _ => None,
                    };
                    let src = self.expr(f, expr)?;
                    let value = if src >> KIND_SHIFT == TEMP {
                        src
                    } else {
                        let t = f.temp();
                        f.emit(Instr::Move { dst: t, src });
                        t
                    };
                    self.destructure(f, items, value, root.map(|r| (r, &[][..])))?;
                    value
                }
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
                        f.reset_temps(mark);
                        if src != target {
                            f.emit(Instr::Assign { var: target, src });
                        }
                        return Ok(target);
                    }
                    let src = self.expr(f, expr)?;
                    f.emit(if src >> KIND_SHIFT == TEMP { Instr::AssignTake { var: target, src } } else { Instr::Assign { var: target, src } });
                    target
                }
                Expr::ArrayDimFetch { array: Expr::Variable { name: g, .. }, dim: Some(name), .. } if self.text(*g) == b"$GLOBALS" => {
                    let target = self.global_ref(f, name)?;
                    let src = self.expr(f, expr)?;
                    f.emit(Instr::Assign { var: target, src });
                    target
                }
                Expr::IndirectVariable { name, .. } => {
                    let target = self.var_ref(f, name)?;
                    let src = self.expr(f, expr)?;
                    f.emit(Instr::Assign { var: target, src });
                    target
                }
                Expr::ArrayDimFetch { .. } if has_inner_append(var) => {
                    let val = self.expr(f, expr)?;
                    let dst = f.temp();
                    f.emit(Instr::Move { dst, src: val });
                    self.assign_from(f, var, val)?;
                    dst
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
                    self.free_static_root(f, var, target);
                    free_made(f, expr, val);
                    dst
                }
                Expr::PropertyFetch { target, property, .. } => {
                    let obj = self.expr(f, target)?;
                    let name = self.prop_name_operand(f, property)?;
                    let val = self.expr(f, expr)?;
                    let dst = f.temp();
                    // The value's temporary is read only here.
                    // (A temporary is taken: released there, so no Free.)
                    f.emit(Instr::AssignProp { obj, name, val, dst, take: is_temp(val) });
                    dst
                }
                Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                    let class = self.class_ref(f, class)?;
                    let name = f.name_constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                    let val = self.expr(f, expr)?;
                    let dst = f.temp();
                    f.emit(Instr::AssignStaticProp { class, name, val, dst });
                    dst
                }
                other => return Err(self.unsupported(other.span(), "this assignment")),
            },
            },
            Expr::AssignRef { var, expr, span } if self.globals_dim(var).is_some() => {
                let _ = std::mem::take(&mut self.unused_result);
                let name = self.globals_dim(var).expect("checked");
                let (src, _) = self.ref_source(f, expr, *span)?;
                let n = self.expr(f, name)?;
                f.emit(Instr::rare(Rare::GlobalBind, n, src, 0, 0));
                src
            }
            Expr::AssignRef { var, expr, span } => match (var, expr) {
                (Expr::Variable { name, .. }, Expr::Variable { name: src, .. })
                    if self.text(*name).starts_with(b"$") && self.text(*src).starts_with(b"$") && self.text(*src) != b"$this" =>
                {
                    let var = f.cv(&self.text(*name)[1..]);
                    let src = f.cv(&self.text(*src)[1..]);
                    f.emit(Instr::BindRef { var, src });
                    var
                }
                // (`$v = &$GLOBALS['x']` is the global itself: ref_source.)
                (Expr::Variable { name, .. }, Expr::ArrayDimFetch { .. })
                    if self.text(*name).starts_with(b"$") && !dim_root_is_prop(expr) && self.globals_dim(expr).is_none() =>
                {
                    let var = f.cv(&self.text(*name)[1..]);
                    let (root, keys, n, append) = self.dim_path(f, expr)?;
                    f.emit(Instr::RefDim { var, root, keys, n, append });
                    self.free_static_root(f, expr, root);
                    var
                }
                (Expr::ArrayDimFetch { .. }, Expr::Variable { name: src, .. }) if self.text(*src).starts_with(b"$") && !dim_root_is_prop(var) => {
                    let src = f.cv(&self.text(*src)[1..]);
                    let (root, keys, n, append) = self.dim_path(f, var)?;
                    f.emit(Instr::DimSetRef { root, keys, n, append, src });
                    self.free_static_root(f, var, root);
                    src
                }
                _ => {
                    let unused = std::mem::take(&mut self.unused_result);
                    let (src, call) = self.ref_source(f, expr, *span)?;
                    // `$a =& f()` whose value is used: the value, even when
                    // a destructor the rebinding runs unsets $a.
                    if call && !unused && matches!(var, Expr::Variable { .. }) {
                        let held = f.temp();
                        f.emit(Instr::Move { dst: held, src });
                        self.bind_ref(f, var, src, call, *span)?;
                        held
                    } else {
                        self.bind_ref(f, var, src, call, *span)?
                    }
                }
            },
            Expr::AssignOp { var, op, expr, span } => {
                let Some(bin) = assign_binop(*op) else {
                    // `$a ??= v`: assigned only when $a is not set.
                    let _ = span;
                    // A call the target is read from (`f()->p ??= v`,
                    // `(1 |> f(...))[0] ??= v`) runs once, its result read
                    // and written (zend_compile_assign_coalesce memoizes).
                    // (A builtin's result is no write target: the error
                    // below says so, as PHP's compiler does.)
                    // (assert() compiles to a call like a user function's.)
                    let builtin = |e: &Expr| {
                        matches!(e, Expr::Call { func, .. } if self.is_builtin(func)
                            && !matches!(func, Expr::Variable { name, .. } if self.text(*name).strip_prefix(b"\\").unwrap_or(self.text(*name)).eq_ignore_ascii_case(b"assert")))
                    };
                    let is_call = |e: &Expr| !builtin(e) && matches!(e, Expr::Call { .. } | Expr::MethodCall { .. } | Expr::StaticCall { .. } | Expr::NullsafeMethodCall { .. } | Expr::Binary { op: BinaryOp::Pipe, .. });
                    let root;
                    let memo;
                    let var: &Expr = match var {
                        Expr::PropertyFetch { target, property, span } if is_call(target) => {
                            let r = self.expr(f, target)?;
                            root = Expr::Znode { reg: r as u32, once: false, span: target.span() };
                            memo = Expr::PropertyFetch { target: &root, property, span: *span };
                            &memo
                        }
                        Expr::ArrayDimFetch { array, dim, span } if is_call(array) => {
                            let r = self.expr(f, array)?;
                            root = Expr::Znode { reg: r as u32, once: false, span: array.span() };
                            memo = Expr::ArrayDimFetch { array: &root, dim: *dim, span: *span };
                            &memo
                        }
                        other => other,
                    };
                    let current = self.quiet(f, var)?;
                    let dst = f.temp();
                    let set = f.emit(Instr::JumpIfSet { src: current, dst, target: 0 });
                    let val = self.expr(f, expr)?;
                    self.assign_from(f, var, val)?;
                    f.emit(Instr::Move { dst, src: val });
                    f.patch(set, f.here());
                    return Ok(dst);
                };
                match var {
                    Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => {
                        let target = f.cv(&self.text(*name)[1..]);
                        let src = self.expr(f, expr)?;
                        f.emit(Instr::AssignOp { op: bin, var: target, src });
                        target
                    }
                    Expr::ArrayDimFetch { array: Expr::Variable { name: g, .. }, dim: Some(name), .. } if self.text(*g) == b"$GLOBALS" => {
                        let target = self.global_ref(f, name)?;
                        let src = self.expr(f, expr)?;
                        f.emit(Instr::AssignOp { op: bin, var: target, src });
                        target
                    }
                    Expr::IndirectVariable { name, .. } => {
                        let target = self.var_ref(f, name)?;
                        let src = self.expr(f, expr)?;
                        f.emit(Instr::AssignOp { op: bin, var: target, src });
                        target
                    }
                    // `$a[] op= v` appends `null op v`.
                    Expr::ArrayDimFetch { dim: None, .. } => {
                        let val = self.expr(f, expr)?;
                        let null = f.temp();
                        f.emit(Instr::LoadNull { dst: null });
                        let dst = f.temp();
                        f.emit(Instr::Binary { op: bin, dst, a: null, b: val });
                        self.assign_from(f, var, dst)?;
                        dst
                    }
                    Expr::ArrayDimFetch { dim: Some(_), .. } if !dim_root_is_prop(var) => {
                        let val = self.expr(f, expr)?;
                        let (target, keys, n, _) = self.dim_path(f, var)?;
                        let dst = f.temp();
                        f.emit(Instr::AssignDimOp { op: bin, var: target, keys, n, val, dst });
                        self.free_static_root(f, var, target);
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
                        let name = self.prop_name_operand(f, property)?;
                        let val = self.expr(f, expr)?;
                        let dst = f.temp();
                        f.emit(Instr::AssignPropOp { op: bin, obj, name, val, dst });
                        dst
                    }
                    Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                        let class = self.class_ref(f, class)?;
                        let name = f.name_constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                        let val = self.expr(f, expr)?;
                        let dst = f.temp();
                        f.emit(Instr::AssignStaticPropOp { op: bin, class, name, val, dst });
                        dst
                    }
                    other => return Err(self.unsupported(other.span(), "this compound assignment")),
                }
            }
            Expr::Binary { left, op: BinaryOp::Pipe, right, span } => self.pipe(f, left, right, *span, hint, mark)?,
            Expr::Binary { left, op, right, .. } => self.binary(f, left, *op, right, hint, mark)?,
            Expr::Znode { reg, .. } => *reg as Reg,
            Expr::Unary { op: op @ (UnaryOp::PreInc | UnaryOp::PreDec), expr: Expr::PropertyFetch { target, property, .. }, .. } => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name_operand(f, property)?;
                let dst = f.temp();
                f.emit(Instr::PropIncDec { obj, name, dst, inc: matches!(op, UnaryOp::PreInc), post: false });
                dst
            }
            Expr::PostInc { var: Expr::PropertyFetch { target, property, .. }, .. }
            | Expr::PostDec { var: Expr::PropertyFetch { target, property, .. }, .. } => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name_operand(f, property)?;
                let dst = f.temp();
                f.emit(Instr::PropIncDec { obj, name, dst, inc: matches!(e, Expr::PostInc { .. }), post: true });
                dst
            }
            // `++C::$x`, `C::$x--`: a fetch and a compound assignment of 1.
            Expr::Unary { op: op @ (UnaryOp::PreInc | UnaryOp::PreDec), expr: Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. }, .. }
                if self.text(*name).starts_with(b"$") =>
            {
                let class = self.class_ref(f, class)?;
                let name = f.name_constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
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
                let name = f.name_constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                let one = f.constant(Frozen::Int(1));
                let dst = f.temp();
                let scratch = f.temp();
                f.emit(Instr::FetchStaticProp { dst, class, name });
                let op = if matches!(e, Expr::PostInc { .. }) { BinOp::Add } else { BinOp::Sub };
                f.emit(Instr::AssignStaticPropOp { op, class, name, val: one, dst: scratch });
                dst
            }
            // `++$a[k]`, `$o->p[k]--`, `++C::$s[k]`: a property or static
            // root is reached through a reference, released after.
            Expr::Unary { op: op @ (UnaryOp::PreInc | UnaryOp::PreDec), expr: target @ Expr::ArrayDimFetch { .. }, .. }
                if self.globals_dim(target).is_none() =>
            {
                self.dim_incdec(f, target, matches!(op, UnaryOp::PreInc), false)?
            }
            Expr::PostInc { var: target @ Expr::ArrayDimFetch { .. }, .. } | Expr::PostDec { var: target @ Expr::ArrayDimFetch { .. }, .. }
                if self.globals_dim(target).is_none() =>
            {
                self.dim_incdec(f, target, matches!(e, Expr::PostInc { .. }), true)?
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
                    let start = f.emit(Instr::Silence { saved }) as u32;
                    let mut r = self.expr(f, expr)?;
                    // A variable is read here, inside, not where it is used.
                    if r >> KIND_SHIFT == CV {
                        let t = f.temp();
                        f.emit(Instr::Move { dst: t, src: r });
                        r = t;
                    }
                    let end = f.emit(Instr::Unsilence { saved }) as u32;
                    f.silences.push((start, end, saved));
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
                        f.reset_temps(keep);
                        self.expr_into(f, t, dst)?;
                        f.reset_temps(keep);
                        let end = f.emit(Instr::Jump { target: 0 });
                        f.patch_all(else_at, f.here());
                        self.expr_into(f, if_false, dst)?;
                        f.reset_temps(keep);
                        f.patch(end, f.here());
                    }
                    None => {
                        // `a ?: b`
                        self.expr_into(f, condition, dst)?;
                        f.reset_temps(keep);
                        let end = f.emit(Instr::JumpIf { cond: dst, target: 0 });
                        self.expr_into(f, if_false, dst)?;
                        f.reset_temps(keep);
                        f.patch(end, f.here());
                    }
                }
                dst
            }
            // An array known at compile time (`['a' => 1, 'b' => [2, 3]]`,
            // `[__DIR__ . '/x.php']`) is one constant, as PHP folds it: a
            // large one (100,000 words, a 15,000-class map) takes a single
            // register.
            Expr::Array { items, .. } if !items.is_empty() && self.foldable(e) && self.const_value(e).is_ok() => {
                let value = self.const_value(e)?;
                f.constant(value)
            }
            Expr::Array { items, .. } => {
                let dst = f.temp();
                // A list literal's elements are all pushed: room for them.
                let list = items.iter().all(|i| i.key.is_none() && !i.unpack && !i.by_ref);
                let n = if list { u16::try_from(items.len()).unwrap_or(0) } else { 0 };
                f.emit(Instr::NewArray { dst, n });
                for item in *items {
                    if item.unpack {
                        let src = self.expr(f, item.value)?;
                        f.emit(Instr::ArrayUnpack { arr: dst, src });
                        continue;
                    }
                    if item.by_ref {
                        // `[&$x]`, `[k => &$x]`: the element is a reference.
                        let (src, _) = self.ref_source(f, item.value, item.span)?;
                        match item.key {
                            Some(k) => {
                                let key = self.expr(f, k)?;
                                f.emit(Instr::rare(Rare::ArraySetRef, dst, key, src, 0));
                            }
                            None => {
                                f.emit(Instr::rare(Rare::ArrayPushRef, dst, src, 0, 0));
                            }
                        }
                        continue;
                    }
                    let val = self.expr(f, item.value)?;
                    match item.key {
                        Some(k) => {
                            let key = self.expr(f, k)?;
                            // The element's temporary is read only here.
                            f.emit(Instr::ArraySet { arr: dst, key, val, take: is_temp(val) });
                        }
                        None => {
                            f.emit(Instr::ArrayPush { arr: dst, val, take: is_temp(val) });
                        }
                    }
                    // (A temporary is taken, so released already: no Free.)
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
                f.emit(Instr::FetchDim { dst, arr, key, quiet: false, empty: false });
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
            // `command` runs as shell_exec("command").
            Expr::ShellExec { parts, .. } => {
                let base = f.temp();
                match parts.len() {
                    0 => {
                        let empty = f.constant(Frozen::Str(Text::from("")));
                        f.emit(Instr::Move { dst: base, src: empty });
                    }
                    1 => self.expr_into(f, parts[0], base)?,
                    n => {
                        let start = f.temp();
                        let regs: Vec<Reg> = std::iter::once(start).chain((1..n).map(|_| f.temp())).collect();
                        for (r, p) in regs.iter().zip(parts.iter()) {
                            self.expr_into(f, p, *r)?;
                        }
                        f.emit(Instr::ConcatN { dst: base, start, n: n as u8 });
                    }
                }
                let site = self.named_site(f, b"shell_exec");
                let dst = out(f, hint, mark);
                f.emit(Instr::Call { site, base, argc: 1, dst });
                dst
            }
            Expr::MagicConst { kind, span } => {
                let text: Vec<u8> = match kind {
                    MagicConstKind::Line => return Ok(f.constant(Frozen::Int(i64::from(self.line(*span))))),
                    MagicConstKind::File => self.file.as_bytes().to_vec(),
                    MagicConstKind::Dir => self.dir(),
                    MagicConstKind::Method if f.name.as_bytes() != b"{main}" => match &f.class {
                        Some(c) => [c.as_bytes(), b"::", f.name.as_bytes()].concat(),
                        // Outside a class, __METHOD__ is the function's name.
                        None => f.name.as_bytes().to_vec(),
                    },
                    MagicConstKind::Function if f.name.as_bytes() != b"{main}" => f.name.as_bytes().to_vec(),
                    MagicConstKind::Namespace => self.ns.clone(),
                    // In a trait: the class using it, when the code runs.
                    MagicConstKind::Class if self.scope.as_ref().is_some_and(|s| s.in_trait) => {
                        let class = f.class_ref(ClassRef::Scope);
                        let name = f.name_constant(Frozen::Str(Text::from("class")));
                        let dst = f.temp();
                        f.emit(Instr::ClassConst { dst, class, name });
                        return Ok(dst);
                    }
                    MagicConstKind::Class => self.scope.as_ref().map(|s| s.name.as_bytes().to_vec()).unwrap_or_default(),
                    MagicConstKind::Trait => match &self.scope {
                        Some(s) if s.in_trait => s.name.as_bytes().to_vec(),
                        _ => Vec::new(),
                    },
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
                        f.reset_temps(keep);
                    }
                    let skip = f.emit(Instr::Jump { target: 0 });
                    f.patch_all(to_body, f.here());
                    self.expr_into(f, arm.body, dst)?;
                    f.reset_temps(keep);
                    ends.push(f.emit(Instr::Jump { target: 0 }));
                    f.patch(skip, f.here());
                }
                match default {
                    Some(body) => {
                        self.expr_into(f, body, dst)?;
                        f.reset_temps(keep);
                    }
                    None => {
                        f.emit(Instr::MatchError { src: subject });
                    }
                }
                f.patch_all(ends, f.here());
                dst
            }
            Expr::New { args, span, .. } if is_placeholder(args) || is_partial(args) => {
                return Err(CompileError { message: "Cannot create Closure for new expression".into(), line: self.line(*span) });
            }
            Expr::NullsafeMethodCall { args, span, .. } if is_placeholder(args) || is_partial(args) => {
                return Err(CompileError { message: "Cannot combine nullsafe operator with Closure creation".into(), line: self.line(*span) });
            }
            Expr::New { class, args, .. } if matches!(class, Expr::AnonymousClass { .. }) => self.anonymous_class(f, class, args, hint)?,
            Expr::New { class, args, .. } => {
                let class = self.class_ref(f, class)?;
                // The result's register is below the arguments: the
                // constructor's frame starts at them. A temporary it is
                // wanted in will do (no copy left behind to keep the
                // object alive); a variable is assigned only after.
                let dst = match hint {
                    Some(h) if h >> KIND_SHIFT == TEMP => h,
                    _ => f.temp(),
                };
                f.emit(Instr::New { dst, class });
                if needs_spread(args) {
                    let list = self.spread_list(f, args, None)?;
                    f.emit(Instr::ConstructSpread { obj: dst, args: list });
                } else {
                    let (base, argc) = self.args_by_value(f, args)?;
                    f.emit(Instr::Construct { obj: dst, base, argc });
                }
                dst
            }
            Expr::PropertyFetch { target, property, .. } => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name_operand(f, property)?;
                let dst = out(f, hint, mark);
                f.emit(Instr::FetchProp { dst, obj, name, quiet: false });
                dst
            }
            Expr::MethodCall { target, method, args, .. } if needs_spread(args) && !is_partial(args) => {
                let obj = self.expr(f, target)?;
                let name = self.prop_name(f, method)?;
                let pair = f.temp();
                f.emit(Instr::NewArray { dst: pair, n: 0 });
                f.emit(Instr::ArrayPush { arr: pair, val: obj, take: false });
                f.emit(Instr::ArrayPush { arr: pair, val: name, take: false });
                self.spread_call(f, pair, args)?
            }
            // `C::m(...$a)`, `parent::__construct(...$a)`: the callable
            // [class, 'm']; self, parent and static stay keywords, resolved
            // where the call runs (as PHP's callables do: $this and the
            // called class go along).
            Expr::StaticCall { class: class_expr @ Expr::Variable { .. }, method: Expr::Variable { name: method, .. }, args, .. }
                if needs_spread(args) && !is_partial(args) && !self.text(*method).starts_with(b"$") =>
            {
                let Expr::Variable { name: written, .. } = class_expr else { unreachable!("matched a name") };
                let written = self.text(*written);
                let name = if written.starts_with(b"$") {
                    // `$class::m(...$a)`: the class (or object) it holds.
                    self.expr(f, class_expr)?
                } else if matches!(written.to_ascii_lowercase().as_slice(), b"self" | b"parent" | b"static") {
                    // Resolved where the call runs (the callable keeps the keyword).
                    self.class_ref(f, class_expr)?;
                    f.constant(Frozen::Str(Text::from(written.to_ascii_lowercase().as_slice())))
                } else {
                    f.constant(Frozen::Str(Text::from(self.class_name(written).as_slice())))
                };
                let m = f.constant(Frozen::Str(Text::from(self.text(*method))));
                self.call_pair(f, name, m, args)?
            }
            Expr::MethodCall { target, method, args, span } if is_placeholder(args) || is_partial(args) => {
                // `$o->m(...)`: a Closure of [$o, 'm'] (`$o->m(?)`: its
                // partial application).
                let obj = self.expr(f, target)?;
                let name = self.prop_name(f, method)?;
                let pair = f.temp();
                f.emit(Instr::NewArray { dst: pair, n: 0 });
                f.emit(Instr::ArrayPush { arr: pair, val: obj, take: false });
                f.emit(Instr::ArrayPush { arr: pair, val: name, take: false });
                if is_partial(args) {
                    return self.partial(f, pair, args, *span);
                }
                let dst = f.temp();
                f.emit(Instr::FromCallable { dst, src: pair });
                dst
            }
            Expr::StaticCall { class: class_expr @ Expr::Variable { name: class, .. }, method: Expr::Variable { name: method, .. }, args, span }
                if is_placeholder(args) || is_partial(args) =>
            {
                // `C::m(...)`: a Closure of "C::m", the class name resolved
                // as written here (self, parent and static stay keywords);
                // `$c::m(...)`: of [$c, 'm'].
                let written = self.text(*class);
                let src = if written.starts_with(b"$") {
                    let class = self.expr(f, class_expr)?;
                    let name = f.constant(Frozen::Str(Text::from(self.text(*method))));
                    let pair = f.temp();
                    f.emit(Instr::NewArray { dst: pair, n: 0 });
                    f.emit(Instr::ArrayPush { arr: pair, val: class, take: false });
                    f.emit(Instr::ArrayPush { arr: pair, val: name, take: false });
                    pair
                } else {
                    let class = if matches!(written.to_ascii_lowercase().as_slice(), b"self" | b"parent" | b"static") {
                        written.to_ascii_lowercase()
                    } else {
                        self.class_name(written)
                    };
                    let text = [class.as_slice(), b"::", self.text(*method)].concat();
                    f.constant(Frozen::Str(Text::from(text.as_slice())))
                };
                if is_partial(args) {
                    return self.partial(f, src, args, *span);
                }
                let dst = f.temp();
                f.emit(Instr::FromCallable { dst, src });
                dst
            }
            Expr::MethodCall { target, method, args, .. } => {
                let obj = self.expr(f, target)?;
                let text = match method {
                    Expr::Variable { name, .. } => self.text(*name),
                    _ => b"$",
                };
                if text.starts_with(b"$") {
                    // `$o->$m()`: the callable [$o, $m].
                    let m = self.expr(f, method)?;
                    return self.call_pair(f, obj, m, args);
                }
                let site = self.named_site(f, text);
                let (base, argc) = self.method_args(
                    f,
                    args,
                    |n, arg, var| Instr::SendVarMethod { obj, method: site, n, arg, var },
                    |dst, n| Instr::MethodArgByRef { dst, obj, method: site, n },
                )?;
                f.emit(Instr::MethodCall { dst: base, obj, method: site, base, argc });
                base
            }
            // `parent::$prop::get()` / `::set($v)`: the parent's hook (or
            // its stored value when the parent's property has none).
            Expr::StaticCall { class: Expr::ClassConstFetch { class: parent @ Expr::Variable { name: p, .. }, constant: Expr::Variable { name: prop, .. }, .. }, method: Expr::Variable { name: m, .. }, args, .. }
                if self.text(*p).eq_ignore_ascii_case(b"parent")
                    && self.text(*prop).starts_with(b"$")
                    && matches!(self.text(*m).to_ascii_lowercase().as_slice(), b"get" | b"set") =>
            {
                if self.scope.is_none() {
                    return Err(CompileError { message: "Cannot use \"parent\" when no class scope is active".into(), line: self.line(*p) });
                }
                let class = self.class_ref(f, parent)?;
                let display = [self.text(*prop), b"::", self.text(*m).to_ascii_lowercase().as_slice()].concat();
                let site = self.named_site(f, &display);
                let (base, argc) = self.args_by_value(f, args)?;
                f.emit(Instr::StaticCall { dst: base, class, method: site, base, argc });
                base
            }
            Expr::StaticCall { class: class_expr, method, args, .. } => {
                let class = self.class_ref(f, class_expr)?;
                let text = match method {
                    Expr::Variable { name, .. } => self.text(*name),
                    _ => b"$",
                };
                if text.starts_with(b"$") {
                    // `C::$m()`: the callable [C::class, $m].
                    let name = match &f.classes[class as usize] {
                        ClassRef::Named(name) | ClassRef::Forward(name) => {
                            let display = name.display.clone();
                            f.constant(Frozen::Str(display))
                        }
                        _ => {
                            let dst = f.temp();
                            let class_const = f.name_constant(Frozen::Str(Text::from("class")));
                            f.emit(Instr::ClassConst { dst, class, name: class_const });
                            dst
                        }
                    };
                    let m = self.expr(f, method)?;
                    return self.call_pair(f, name, m, args);
                }
                let site = self.named_site(f, text);
                let (base, argc) = self.method_args(
                    f,
                    args,
                    |n, arg, var| Instr::SendVarStatic { class, method: site, n, arg, var },
                    |dst, n| Instr::StaticArgByRef { dst, class, method: site, n },
                )?;
                f.emit(Instr::StaticCall { dst: base, class, method: site, base, argc });
                base
            }
            // `C::${expr}`, `C::$$v`: a static property named when it runs.
            Expr::ClassConstFetch { class, constant: Expr::IndirectVariable { name, .. }, .. } => {
                let class_index = self.class_ref(f, class)?;
                let name = self.expr(f, name)?;
                let dst = out(f, hint, mark);
                f.emit(Instr::FetchStaticProp { dst, class: class_index, name });
                dst
            }
            Expr::ClassConstFetch { class, constant, .. } => {
                let Expr::Variable { name, .. } = constant else {
                    return Err(self.unsupported(constant.span(), "a class constant name in an expression"));
                };
                let text = self.text(*name);
                let class_index = self.class_ref(f, class)?;
                let dst = out(f, hint, mark);
                if let Some(prop) = text.strip_prefix(b"$") {
                    let name = f.name_constant(Frozen::Str(Text::from(prop)));
                    f.emit(Instr::FetchStaticProp { dst, class: class_index, name });
                } else if text.eq_ignore_ascii_case(b"class")
                    && let ClassRef::Named(ClassName { display, .. }) | ClassRef::Forward(ClassName { display, .. }) = &f.classes[class_index as usize]
                {
                    // `C::class` is the name, resolved when compiled (but
                    // `static::class` when the code runs).
                    let display = display.clone();
                    return Ok(f.constant(Frozen::Str(display)));
                } else {
                    let name = f.name_constant(Frozen::Str(Text::from(text)));
                    f.emit(Instr::ClassConst { dst, class: class_index, name });
                }
                dst
            }
            // `throw` as an expression: it never yields a value.
            Expr::Throw { expr, .. } => {
                let src = self.expr(f, expr)?;
                f.emit(Instr::Throw { src });
                let dst = out(f, hint, mark);
                f.emit(Instr::LoadNull { dst });
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
                    AstCast::Object => {
                        let src = self.expr(f, expr)?;
                        let dst = out(f, hint, mark);
                        f.emit(Instr::rare(Rare::ToObject, dst, src, 0, 0));
                        return Ok(dst);
                    }
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
                        Expr::ArrayDimFetch { array: Expr::Variable { name: g, .. }, dim: Some(name), .. } if self.text(*g) == b"$GLOBALS" => {
                            let var = self.global_get(f, name, true)?;
                            f.emit(Instr::IssetVar { dst, var });
                        }
                        Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => {
                            let var = f.cv(&self.text(*name)[1..]);
                            f.emit(Instr::IssetVar { dst, var });
                        }
                        Expr::ArrayDimFetch { dim: Some(_), .. } if dim_root_is_var(self, v) => {
                            let (var, keys, n, _) = self.dim_path(f, v)?;
                            f.emit(Instr::IssetDim { dst, var, keys, n });
                            self.free_static_root(f, v, var);
                        }
                        // `?->` reads as `->` here: null has no property set.
                        Expr::PropertyFetch { target, property, .. } | Expr::NullsafePropertyFetch { target, property, .. } => {
                            let obj = self.quiet(f, target)?;
                            let name = self.prop_name_operand(f, property)?;
                            f.emit(Instr::IssetProp { dst, obj, name });
                        }
                        Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. } if self.text(*name).starts_with(b"$") => {
                            let class = self.class_ref(f, class)?;
                            let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                            f.emit(Instr::rare(Rare::IssetStatic, dst, name, 0, class));
                        }
                        Expr::ClassConstFetch { class, constant: Expr::IndirectVariable { name, .. }, .. } => {
                            let class = self.class_ref(f, class)?;
                            let name = self.expr(f, name)?;
                            f.emit(Instr::rare(Rare::IssetStatic, dst, name, 0, class));
                        }
                        // isset($o->p[k]...): the dimensions of a quietly read root
                        Expr::ArrayDimFetch { dim: Some(dim), array, .. } => {
                            let arr = self.quiet(f, array)?;
                            let key = self.expr(f, dim)?;
                            f.emit(Instr::IssetDim { dst, var: arr, keys: key, n: 1 });
                        }
                        Expr::IndirectVariable { name, .. } => {
                            let var = self.var_get(f, name, true)?;
                            f.emit(Instr::IssetVar { dst, var });
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
            Expr::Closure { attributes, is_static, by_ref, params, uses, body, span, return_type } => {
                let line = self.line(*span);
                let name = self.closure_name(f, line);
                let method = self.method.take();
                let meta = self.meta_of(attributes, None, *span, 2)?;
                self.pending_ret = return_type.map(|t| self.type_decl(t));
                self.pending_meta = Some(meta);
                let (func, _) = self.function_with(name, line, params, Body::Stmts(body), *by_ref, uses)?;
                self.method = method;
                let dst = self.make_closure(f, func, *is_static);
                for u in *uses {
                    let var = f.cv(&self.text(u.var.span)[1..]);
                    f.emit(Instr::Capture { closure: dst, var, by_ref: u.by_ref, quiet: false });
                }
                dst
            }
            Expr::ArrowFunction { attributes, is_static, by_ref, params, expr, span, return_type } => {
                let line = self.line(*span);
                let name = self.closure_name(f, line);
                let method = self.method.take();
                let meta = self.meta_of(attributes, None, *span, 2)?;
                self.pending_ret = return_type.map(|t| self.type_decl(t));
                self.pending_meta = Some(meta);
                let (func, captures) = self.function_with(name, line, params, Body::Expr(expr), *by_ref, &[])?;
                self.method = method;
                let dst = self.make_closure(f, func, *is_static);
                for name in captures {
                    let var = f.cv(name.as_bytes());
                    f.emit(Instr::Capture { closure: dst, var, by_ref: false, quiet: true });
                }
                dst
            }
            Expr::Yield { key, value, from: false, .. } => {
                f.is_generator = true;
                let value = match value {
                    Some(v) => self.expr(f, v)?,
                    None => f.constant(Frozen::Null),
                };
                let (key, has_key) = match key {
                    Some(k) => (self.expr(f, k)?, true),
                    None => (value, false),
                };
                let dst = f.temp();
                f.emit(Instr::Yield { dst, value, key, has_key });
                dst
            }
            Expr::Yield { value, from: true, span, .. } => {
                f.is_generator = true;
                let Some(value) = value else { return Err(self.unsupported(*span, "yield from without a value")) };
                let src = self.expr(f, value)?;
                let dst = f.temp();
                f.emit(Instr::YieldFrom { dst, src });
                dst
            }
            // empty($x): not set, or false when converted.
            Expr::Empty { expr, .. } => {
                let v = match expr {
                    // `empty($c[k])`: the container read quietly, its last
                    // dimension with isset's key checks (FetchDim's `empty`).
                    Expr::ArrayDimFetch { array, dim: Some(dim), .. } if self.globals_dim(expr).is_none() => {
                        let arr = self.quiet(f, array)?;
                        let key = self.expr(f, dim)?;
                        let v = f.temp();
                        f.emit(Instr::FetchDim { dst: v, arr, key, quiet: true, empty: true });
                        v
                    }
                    _ => self.quiet(f, expr)?,
                };
                let t = f.temp();
                f.emit(Instr::LoadNull { dst: t });
                let set = f.emit(Instr::JumpIfSet { src: v, dst: t, target: 0 });
                f.patch(set, f.here());
                let dst = f.temp();
                f.emit(Instr::Not { dst, src: t });
                dst
            }
            Expr::Eval { expr, .. } => {
                let src = self.expr(f, expr)?;
                let dst = f.temp();
                f.emit(Instr::Eval { dst, src });
                dst
            }
            Expr::NullsafePropertyFetch { target, property, .. } => {
                let obj = self.expr(f, target)?;
                let dst = f.temp();
                let null = f.constant(Frozen::Null);
                let is_null = f.emit(Instr::jump_cmp(Cmp::Identical, obj, null, true, 0));
                let name = self.prop_name_operand(f, property)?;
                f.emit(Instr::FetchProp { dst, obj, name, quiet: false });
                let end = f.emit(Instr::Jump { target: 0 });
                f.patch(is_null, f.here());
                f.emit(Instr::LoadNull { dst });
                f.patch(end, f.here());
                dst
            }
            Expr::NullsafeMethodCall { target, method, args, .. } => {
                // Null: no call, and its arguments are not evaluated.
                let obj = self.expr(f, target)?;
                let dst = f.temp();
                let null = f.constant(Frozen::Null);
                let is_null = f.emit(Instr::jump_cmp(Cmp::Identical, obj, null, true, 0));
                let Expr::Variable { name, .. } = method else {
                    return Err(self.unsupported(method.span(), "a method name in an expression"));
                };
                let text = self.text(*name);
                let site = self.named_site(f, text);
                let (base, argc) = self.method_args(
                    f,
                    args,
                    |n, arg, var| Instr::SendVarMethod { obj, method: site, n, arg, var },
                    |dst, n| Instr::MethodArgByRef { dst, obj, method: site, n },
                )?;
                f.emit(Instr::MethodCall { dst: base, obj, method: site, base, argc });
                f.emit(Instr::Move { dst, src: base });
                let end = f.emit(Instr::Jump { target: 0 });
                f.patch(is_null, f.here());
                f.emit(Instr::LoadNull { dst });
                f.patch(end, f.here());
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
            Expr::IndirectVariable { name, .. } => self.var_get(f, name, false)?,
            other => return Err(self.unsupported(other.span(), &format!("this expression ({})", kind(other)))),
        })
    }

    fn dim_incdec(&mut self, f: &mut FnBuilder, target: &Expr, inc: bool, post: bool) -> Result<Reg> {
        let (var, keys, n, append, held) = self.ref_dim_path(f, target)?;
        if append {
            return Err(self.unsupported(target.span(), "++/-- on []"));
        }
        let dst = f.temp();
        f.emit(Instr::DimIncDec { var, keys, n, dst, inc, post });
        if let Some(h) = held {
            f.emit(Instr::Free { reg: h });
        }
        Ok(dst)
    }

    fn inc_target(&mut self, f: &mut FnBuilder, e: &Expr) -> Result<Reg> {
        if let Some(name) = self.globals_dim(e) {
            return self.global_ref(f, name);
        }
        match e {
            Expr::Variable { name, .. } if self.text(*name).starts_with(b"$") => Ok(f.cv(&self.text(*name)[1..])),
            Expr::IndirectVariable { name, .. } => self.var_ref(f, name),
            other => Err(self.unsupported(other.span(), "++/-- on this")),
        }
    }

    fn constant_fetch(&mut self, f: &mut FnBuilder, name: &[u8]) -> Reg {
        let bare = name.strip_prefix(b"\\").unwrap_or(name);
        match bare.to_ascii_lowercase().as_slice() {
            b"true" => return f.constant(Frozen::Bool(true)),
            b"false" => return f.constant(Frozen::Bool(false)),
            b"null" => return f.constant(Frozen::Null),
            _ => {}
        }
        if bare == b"__COMPILER_HALT_OFFSET__"
            && let Some(at) = self.halt_offset
        {
            return f.constant(Frozen::Int(at as i64));
        }
        let (full, fallback) = self.ns_name(name, &self.use_consts, false);
        // A namespace's part is case-insensitive; the constant's own is not.
        let key = p8_runtime::ext::const_key(&full).into_owned();
        let site = f.site(true, Text::from(key.as_slice()), self.sym(&key), Text::from(full.as_slice()));
        if let Some(global) = fallback {
            f.sites[site as usize].fallback = Some((Text::from(global.as_slice()), self.sym(&p8_runtime::ext::const_key(&global))));
        }
        let dst = f.temp();
        f.emit(Instr::FetchConst { site, dst });
        dst
    }

    /// `$value |> $callable` (zend_compile_pipe): the value computed first
    /// (into a temporary of its own, so a by-reference parameter refuses
    /// it), then the call. A first-class callable on the right
    /// (`f(...)`, `$o->m(...)`, `C::m(...)`) becomes that call with the
    /// value as its argument; anything else is called as a value,
    /// `($callable)($value)`.
    fn pipe(&mut self, f: &mut FnBuilder, left: &Expr, right: &Expr, span: Span, hint: Option<Reg>, mark: usize) -> Result<Reg> {
        if let Expr::ArrowFunction { span: arrow, .. } = right
            && !self.parenthesized(*arrow)
        {
            return Err(CompileError { message: "Arrow functions on the right hand side of |> must be parenthesized".into(), line: self.line(*arrow) });
        }
        let value = self.expr(f, left)?;
        // ZEND_QM_ASSIGN of a variable: a temporary, never a reference.
        let value = if value >> KIND_SHIFT == TEMP {
            value
        } else {
            let t = f.temp();
            f.emit(Instr::Move { dst: t, src: value });
            t
        };
        let znode = Expr::Znode { reg: value as u32, once: true, span: left.span() };
        let arg = [p8_syntax::ast::Arg { name: None, value: &znode, unpack: false, span: left.span() }];
        // assert() gets its own source as an extra argument: never
        // rewritten (zend_is_pipe_optimizable_callable_name).
        let is_assert = |func: &Expr| matches!(func, Expr::Variable { name, .. } if self.text(*name).trim_ascii_start().strip_prefix(b"\\").unwrap_or(self.text(*name)).eq_ignore_ascii_case(b"assert"));
        // zend_partial_apply: a first-class callable or a partial
        // application with one placeholder on the right becomes the call
        // with the value in the placeholder's place (`$v |> f(1, ?)` is
        // `f(1, $v)`; `name: ?` passes it by that name). Not with more
        // placeholders (a missing argument), nor `...` after named
        // arguments (a positional one would follow them).
        let applied = |args: &[p8_syntax::ast::Arg<'_>]| -> Option<usize> {
            if !is_placeholder(args) && !is_partial(args) {
                return None;
            }
            let mut found = None;
            let mut named = false;
            for (i, a) in args.iter().enumerate() {
                named |= a.name.is_some();
                if matches!(a.value, Expr::Placeholder { .. } | Expr::VariadicPlaceholder { .. }) {
                    if found.is_some() || (named && matches!(a.value, Expr::VariadicPlaceholder { .. })) {
                        return None;
                    }
                    found = Some(i);
                }
            }
            found
        };
        let replaced = |args: &[p8_syntax::ast::Arg<'_>], at: usize| -> Vec<p8_syntax::ast::Arg<'_>> {
            args.iter().enumerate().map(|(i, a)| if i == at { p8_syntax::ast::Arg { name: a.name, value: &znode, unpack: false, span: a.span } } else { *a }).collect()
        };
        let pfa_args;
        let call = match right {
            Expr::Call { func, args, span } if !is_assert(func) && let Some(at) = applied(args) => {
                pfa_args = replaced(args, at);
                Expr::Call { func, args: &pfa_args, span: *span }
            }
            Expr::MethodCall { target, method, args, span } if let Some(at) = applied(args) => {
                pfa_args = replaced(args, at);
                Expr::MethodCall { target, method, args: &pfa_args, span: *span }
            }
            Expr::StaticCall { class, method, args, span } if let Some(at) = applied(args) => {
                pfa_args = replaced(args, at);
                Expr::StaticCall { class, method, args: &pfa_args, span: *span }
            }
            callable => {
                let callee = self.expr(f, callable)?;
                let callee = Expr::Znode { reg: callee as u32, once: true, span: callable.span() };
                let call = Expr::Call { func: &callee, args: &arg, span };
                let r = self.expr_to(f, &call, hint)?;
                pipe_send_check(f);
                return Ok(r);
            }
        };
        let _ = mark;
        let r = self.expr_to(f, &call, hint)?;
        pipe_send_check(f);
        Ok(r)
    }

    /// Whether the expression at `span` is written in parentheses.
    fn parenthesized(&self, span: Span) -> bool {
        let before = &self.source[..span.start.min(self.source.len())];
        let after = &self.source[span.end.min(self.source.len())..];
        before.trim_ascii_end().ends_with(b"(") && after.trim_ascii_start().starts_with(b")")
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
            // Arithmetic and string operators get numbers and strings, whose
            // release can't be seen: no Free on this hot path.
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
            free_operand(f, left, a, dst);
            free_operand(f, right, b, dst);
            return Ok(dst);
        }
        if op == BinaryOp::Instanceof {
            let src = self.expr(f, left)?;
            let class = self.class_ref(f, right)?;
            let dst = out(f, hint, mark);
            f.emit(Instr::InstanceOf { dst, src, class });
            free_operand(f, left, src, dst);
            return Ok(dst);
        }
        let dst = f.temp();
        match op {
            BinaryOp::And | BinaryOp::LogicalAnd | BinaryOp::Or | BinaryOp::LogicalOr => {
                let is_and = matches!(op, BinaryOp::And | BinaryOp::LogicalAnd);
                let a = self.expr(f, left)?;
                f.emit(Instr::ToBool { dst, src: a });
                free_operand(f, left, a, dst);
                let end = f.emit(if is_and {
                    Instr::JumpIfNot { cond: dst, target: 0 }
                } else {
                    Instr::JumpIf { cond: dst, target: 0 }
                });
                let b = self.expr(f, right)?;
                f.emit(Instr::ToBool { dst, src: b });
                free_operand(f, right, b, dst);
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
        if let Some(name) = self.globals_dim(e) {
            return self.global_get(f, name, true);
        }
        match e {
            Expr::ArrayDimFetch { array, dim: Some(dim), .. } => {
                let arr = self.quiet(f, array)?;
                let key = self.expr(f, dim)?;
                let dst = f.temp();
                f.emit(Instr::FetchDim { dst, arr, key, quiet: true, empty: false });
                Ok(dst)
            }
            Expr::PropertyFetch { target, property, .. } => {
                let obj = self.quiet(f, target)?;
                let name = self.prop_name_operand(f, property)?;
                let dst = f.temp();
                f.emit(Instr::FetchProp { dst, obj, name, quiet: true });
                Ok(dst)
            }
            Expr::IndirectVariable { name, .. } => self.var_get(f, name, true),
            _ => self.expr(f, e),
        }
    }

    /// The callable name a function call by name resolves to, as a register:
    /// `use function` and the namespace applied; in a namespace an
    /// unqualified `f` is ns\f when that exists as the code runs, else the
    /// global f (as any call).
    fn function_callee(&mut self, f: &mut FnBuilder, name: Span) -> Result<Reg> {
        let (full, fallback) = self.ns_name(self.text(name), &self.use_functions, true);
        let full = f.constant(Frozen::Str(Text::from(full.as_slice())));
        Ok(match fallback {
            // `f(...$a)` in a namespace: ns\f when it exists as
            // the call runs, else the global f (as any call).
            Some(global) => {
                let global = f.constant(Frozen::Str(Text::from(global.as_slice())));
                let exists = f.constant(Frozen::Str(Text::from("function_exists")));
                let base = f.temp();
                f.emit(Instr::Move { dst: base, src: full });
                f.emit(Instr::CallValue { dst: base, callee: exists, base, argc: 1 });
                let callee = f.temp();
                let other = f.emit(Instr::JumpIfNot { cond: base, target: 0 });
                f.emit(Instr::Move { dst: callee, src: full });
                let end = f.emit(Instr::Jump { target: 0 });
                f.patch(other, f.here());
                f.emit(Instr::Move { dst: callee, src: global });
                f.patch(end, f.here());
                callee
            }
            None => full,
        })
    }

    /// A partial application of the callable in `callee` (PHP 8.6): its
    /// arguments checked as zend_compile_args_ex checks them, the values
    /// bound now (a reference where the callee takes one), and a
    /// Rare::Partial making the Closure (ZEND_CALLABLE_CONVERT_PARTIAL).
    fn partial(&mut self, f: &mut FnBuilder, callee: Reg, args: &[p8_syntax::ast::Arg], span: Span) -> Result<Reg> {
        let line = self.line(span);
        let error = |message: &str| CompileError { message: message.into(), line };
        let (mut named, mut variadic) = (false, false);
        let mut site = Vec::new();
        for arg in args {
            if arg.unpack {
                return Err(error(if named { "Cannot use argument unpacking after named arguments" } else { "Cannot combine partial application and unpacking" }));
            }
            let is_variadic = matches!(arg.value, Expr::VariadicPlaceholder { .. });
            if arg.name.is_some() {
                named = true;
                if variadic {
                    return Err(error("Variadic placeholder must be last"));
                }
            } else {
                if named && !is_variadic {
                    return Err(error("Cannot use positional argument after named argument"));
                }
                if variadic {
                    return Err(error(if is_variadic { "Variadic placeholder may only appear once" } else { "Variadic placeholder must be last" }));
                }
            }
            if is_variadic {
                variadic = true;
                continue;
            }
            site.push(PartialArg {
                name: arg.name.map(|t| Text::from(self.text(t.span))),
                placeholder: matches!(arg.value, Expr::Placeholder { .. }),
            });
        }
        let written: Vec<&p8_syntax::ast::Arg> = args.iter().filter(|a| !matches!(a.value, Expr::VariadicPlaceholder { .. })).collect();
        let regs: Vec<Reg> = (0..written.len().max(1)).map(|_| f.temp()).collect();
        for (n, (arg, r)) in written.iter().zip(&regs).enumerate() {
            match arg.value {
                Expr::Placeholder { .. } => {}
                value if self.can_ref_arg(value) && !matches!(value, Expr::Variable { name, .. } if matches!(self.text(*name), b"$this" | b"$GLOBALS")) => {
                    // Positional arguments come first: n is the parameter;
                    // a named one is asked about by its name.
                    let flag = f.temp();
                    match arg.name {
                        Some(name) => {
                            let key = f.constant(Frozen::Str(Text::from(self.text(name.span))));
                            f.emit(Instr::rare(Rare::CallableArgByRef, flag, callee, key, u16::MAX));
                        }
                        None => {
                            f.emit(Instr::rare(Rare::CallableArgByRef, flag, callee, 0, n as u16));
                        }
                    }
                    self.ref_or_value_with(f, value, *r, flag)?;
                }
                value => self.expr_into(f, value, *r)?,
            }
        }
        let name = self.partial_name(f, line);
        f.partials.push(PartialSite { args: site, variadic, name, line });
        let dst = f.temp();
        f.emit(Instr::rare(Rare::Partial, dst, callee, regs[0], (f.partials.len() - 1) as u16));
        Ok(dst)
    }

    /// zend_compile_partial_name: `{closure:pfa:<where>:<line>}`, where
    /// is the function making it (`C::m()`, `f()`; a closure as named)
    /// or, at the top level, the file.
    fn partial_name(&self, f: &FnBuilder, line: u32) -> Text {
        let place = if f.name.as_bytes() == b"{main}" {
            self.file.to_string()
        } else {
            let name = String::from_utf8_lossy(f.name.as_bytes());
            if f.name.as_bytes().starts_with(b"{closure") {
                name.into_owned()
            } else {
                match &f.class {
                    Some(c) => format!("{}::{name}()", String::from_utf8_lossy(c.as_bytes())),
                    None => format!("{name}()"),
                }
            }
        };
        Text::from(format!("{{closure:pfa:{place}:{line}}}").as_bytes())
    }

    fn call(&mut self, f: &mut FnBuilder, func: &Expr, args: &[p8_syntax::ast::Arg], span: Span, hint: Option<Reg>, mark: usize) -> Result<Reg> {
        if is_partial(args) {
            let callee = match func {
                Expr::Variable { name, .. } if !self.text(*name).starts_with(b"$") => self.function_callee(f, *name)?,
                other => self.expr(f, other)?,
            };
            return self.partial(f, callee, args, span);
        }
        // `f(...)`: a Closure of the function.
        if is_placeholder(args) {
            let src = match func {
                Expr::Variable { name, .. } if !self.text(*name).starts_with(b"$") => self.function_callee(f, *name)?,
                other => self.expr(f, other)?,
            };
            let dst = f.temp();
            f.emit(Instr::FromCallable { dst, src });
            return Ok(dst);
        }
        // A call of a value: `$f()`, `(function () {})()`, `$o->p()()`.
        let dynamic = match func {
            Expr::Variable { name, .. } => self.text(*name).starts_with(b"$"),
            _ => true,
        };
        if needs_spread(args) {
            let callee = match func {
                Expr::Variable { name, .. } if !dynamic => self.function_callee(f, *name)?,
                other => self.expr(f, other)?,
            };
            return self.spread_call(f, callee, args);
        }
        if dynamic {
            let callee = self.expr(f, func)?;
            let (base, argc) = self.value_args(f, args, callee)?;
            f.emit(Instr::CallValue { dst: base, callee, base, argc });
            let _ = (span, hint, mark);
            return Ok(base);
        }
        let Expr::Variable { name, .. } = func else { unreachable!("a name") };
        let text = self.text(*name);
        let (full, fallback) = self.ns_name(text, &self.use_functions, true);
        let text = full.as_slice();
        let lower = text.to_ascii_lowercase();
        let fallback = fallback.map(|g| g.to_ascii_lowercase());
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
            let global = fallback.as_deref().unwrap_or(&lower);
            if let Some((modes, rest)) = self.by_ref.get(global) {
                return Some(modes.get(n).copied().unwrap_or(*rest));
            }
            self.registry.lookup(global).map(|i| self.registry.by_ref(i, n))
        };
        let modes: Vec<Option<bool>> = (0..args.len()).map(by_ref).collect();
        // One text for both when the name is written in lowercase.
        let display = Text::from(text);
        let lower_text = if lower.as_slice() == text { display.clone() } else { Text::from(lower.as_slice()) };
        let site = f.site(false, lower_text, self.sym(&lower), display);
        if let Some(global) = &fallback {
            f.sites[site as usize].fallback = Some((Text::from(global.as_slice()), self.sym(global)));
        }
        // assert() gets its source as a hidden last argument (the message
        // of the AssertionError it throws).
        let assert_text = (fallback.as_deref().unwrap_or(&lower) == b"assert" && args.len() <= 2).then(|| self.text(span).to_vec());
        let extra = match &assert_text {
            Some(_) => 3 - args.len().min(2),
            None => 0,
        };
        let all: Vec<Reg> = (0..args.len() + extra).map(|_| f.temp()).collect();
        for (n, (arg, r)) in args.iter().zip(&all[..args.len()]).enumerate() {
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
                Expr::IndirectVariable { name, .. } if modes[n] != Some(false) => {
                    let nm = self.expr(f, name)?;
                    f.emit(Instr::rare(Rare::VarRef, *r, nm, 0, 0));
                }
                // An element or property for a function not known yet: the
                // function, found when the call runs, decides.
                value @ (Expr::ArrayDimFetch { .. } | Expr::PropertyFetch { .. }) if modes[n].is_none() && self.can_ref_arg(value) => {
                    self.ref_or_value(f, value, *r, Instr::FuncArgByRef { dst: 0, site, n: n as u8 })?;
                }
                // An array element or property passed by reference becomes one.
                Expr::ArrayDimFetch { .. } if modes[n] == Some(true) => {
                    let (root, keys, count, append, held) = self.ref_dim_path(f, arg.value)?;
                    f.emit(Instr::RefDim { var: *r, root, keys, n: count, append });
                    if let Some(h) = held {
                        f.emit(Instr::Free { reg: h });
                    }
                }
                Expr::PropertyFetch { target, property, .. } if modes[n] == Some(true) => {
                    let obj = self.expr(f, target)?;
                    let name = self.prop_name(f, property)?;
                    f.emit(Instr::rare(Rare::PropRef, *r, obj, name, 0));
                }
                Expr::ClassConstFetch { class, constant: Expr::Variable { name, .. }, .. }
                    if modes[n] == Some(true) && self.text(*name).starts_with(b"$") =>
                {
                    let class = self.class_ref(f, class)?;
                    let name = f.constant(Frozen::Str(Text::from(&self.text(*name)[1..])));
                    f.emit(Instr::rare(Rare::StaticRef, *r, name, 0, class));
                }
                value => self.expr_into(f, value, *r)?,
            }
        }
        if let Some(source) = assert_text {
            let mut i = args.len();
            if args.len() < 2 {
                let null = f.constant(Frozen::Null);
                f.emit(Instr::Move { dst: all[i], src: null });
                i += 1;
            }
            let text = f.constant(Frozen::Str(Text::from(source.as_slice())));
            f.emit(Instr::Move { dst: all[i], src: text });
        }
        let base = all.first().copied().unwrap_or_else(|| f.temp());
        // The arguments' registers are free again once the call has them:
        // the result may land in the first (the callee's frame is cleared
        // before its result is written).
        let dst = out(f, hint, mark);
        f.emit(Instr::Call { site, base, argc: all.len() as u8, dst });
        Ok(dst)
    }
}

/// Where an instruction's result goes: `hint`, or a temporary that reuses
/// those of its operands (`mark` is where they start; every instruction
/// reads its operands before it writes its result).
fn out(f: &mut FnBuilder, hint: Option<Reg>, mark: usize) -> Reg {
    f.reset_temps(mark);
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

/// Whether `e` is a literal (what Compiler::literal makes a constant of),
/// without making it: a string literal's value is a copy.
fn is_literal(e: &Expr) -> bool {
    matches!(e, Expr::Integer { .. } | Expr::Float { .. } | Expr::String { .. } | Expr::Boolean { .. } | Expr::Null { .. })
}

/// Whether literal `e` is a float (an integer literal can overflow to one).
fn float_key(e: &Expr) -> bool {
    match e {
        Expr::Float { .. } => true,
        Expr::Integer { value, .. } => matches!(literal::integer(value), Number::Float(_)),
        _ => false,
    }
}
